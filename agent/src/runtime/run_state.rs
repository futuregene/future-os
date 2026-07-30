use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc,
};

use anyhow::{bail, Result};
use parking_lot::Mutex;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPhase {
    Starting,
    Running,
    Cancelling,
    CancellationStuck,
    PersistenceDegraded,
    Finalizing,
}

impl RunPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::CancellationStuck => "cancellation_stuck",
            Self::PersistenceDegraded => "persistence_degraded",
            Self::Finalizing => "finalizing",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLease {
    pub run_id: String,
    pub epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSnapshot {
    pub run_id: String,
    pub epoch: u64,
    pub phase: RunPhase,
}

struct ActiveRun {
    lease: RunLease,
    client_request_id: String,
    phase: RunPhase,
    interrupt_tx: Option<mpsc::Sender<()>>,
    interrupt_flag: Option<Arc<AtomicBool>>,
}

#[derive(Default)]
struct ControlState {
    epoch: u64,
    active: Option<ActiveRun>,
    recent_requests: VecDeque<(String, RunLease)>,
}

/// The authoritative lifecycle for one session.
///
/// `is_streaming` remains as a compatibility projection for older RPC clients;
/// acceptance and completion decisions must use this state machine instead.
pub struct RunControl {
    state: Mutex<ControlState>,
    is_streaming: Arc<AtomicBool>,
    active_tasks: AtomicUsize,
    stale_epoch_drops: AtomicU64,
    /// Count of runs that entered PersistenceDegraded (a run terminal could not
    /// be committed). Observability metric for the "persistence degraded"
    /// acceptance criterion; expected to stay 0 on healthy storage.
    persistence_degraded: AtomicU64,
}

impl RunControl {
    pub fn new(is_streaming: Arc<AtomicBool>) -> Self {
        Self {
            state: Mutex::new(ControlState::default()),
            is_streaming,
            active_tasks: AtomicUsize::new(0),
            stale_epoch_drops: AtomicU64::new(0),
            persistence_degraded: AtomicU64::new(0),
        }
    }

    pub fn begin(
        &self,
        requested_run_id: Option<&str>,
        client_request_id: Option<&str>,
    ) -> Result<RunLease> {
        let mut state = self.state.lock();
        let client_request_id = client_request_id.unwrap_or_default();
        if !client_request_id.is_empty()
            && (state
                .active
                .as_ref()
                .is_some_and(|active| active.client_request_id == client_request_id)
                || state
                    .recent_requests
                    .iter()
                    .any(|(request_id, _)| request_id == client_request_id))
        {
            bail!("client request `{client_request_id}` was already accepted");
        }
        if let Some(active) = &state.active {
            bail!(
                "agent run {} is {}; wait for it to finish before starting another run",
                active.lease.run_id,
                active.phase.as_str()
            );
        }

        state.epoch = state.epoch.wrapping_add(1).max(1);
        let lease = RunLease {
            run_id: requested_run_id
                .filter(|id| !id.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(crate::utils::generate_id),
            epoch: state.epoch,
        };
        state.active = Some(ActiveRun {
            lease: lease.clone(),
            client_request_id: client_request_id.to_string(),
            phase: RunPhase::Starting,
            interrupt_tx: None,
            interrupt_flag: None,
        });
        self.is_streaming.store(true, Ordering::Release);
        self.active_tasks.fetch_add(1, Ordering::Relaxed);
        tracing::info!(
            run_id = %lease.run_id,
            run_epoch = lease.epoch,
            phase = RunPhase::Starting.as_str(),
            "session run accepted"
        );
        Ok(lease)
    }

    pub fn install_cancellation(
        &self,
        lease: &RunLease,
        interrupt_tx: mpsc::Sender<()>,
        interrupt_flag: Arc<AtomicBool>,
    ) -> bool {
        let mut state = self.state.lock();
        let Some(active) = state.active.as_mut() else {
            return self.stale(lease, "install_cancellation");
        };
        if active.lease != *lease {
            return self.stale(lease, "install_cancellation");
        }
        active.interrupt_tx = Some(interrupt_tx);
        active.interrupt_flag = Some(interrupt_flag);
        active.phase = RunPhase::Running;
        true
    }

    /// Request cancellation without making the session idle. Only the matching
    /// run task may later transition through finalizing and release the session.
    #[cfg(test)]
    pub fn abort(&self) -> Option<RunSnapshot> {
        self.abort_expected(None).ok().flatten()
    }

    pub fn abort_expected(&self, expected_run_id: Option<&str>) -> Result<Option<RunSnapshot>> {
        let (snapshot, tx, flag) = {
            let mut state = self.state.lock();
            let Some(active) = state.active.as_mut() else {
                if let Some(expected) = nonempty(expected_run_id) {
                    bail!("run `{expected}` is no longer active");
                }
                return Ok(None);
            };
            validate_run_id(&active.lease.run_id, expected_run_id)?;
            active.phase = RunPhase::Cancelling;
            (
                RunSnapshot {
                    run_id: active.lease.run_id.clone(),
                    epoch: active.lease.epoch,
                    phase: active.phase,
                },
                active.interrupt_tx.clone(),
                active.interrupt_flag.clone(),
            )
        };

        if let Some(tx) = tx {
            let _ = tx.try_send(());
        }
        if let Some(flag) = flag {
            flag.store(true, Ordering::SeqCst);
        }
        tracing::info!(
            run_id = %snapshot.run_id,
            run_epoch = snapshot.epoch,
            phase = snapshot.phase.as_str(),
            "session run cancellation requested"
        );
        Ok(Some(snapshot))
    }

    /// Validate the canonical run and enqueue steering while holding the same
    /// short control lock. This prevents a late command from being accepted
    /// after its run finalized and then leaking into the next run's queue.
    ///
    /// Steering is only accepted while the run can still drain its queue
    /// (`Starting` / `Running`). Once it is cancelling or finalizing the loop is
    /// unwinding and will never read the queue again, so enqueueing would silently
    /// drop the message — reject it instead and let the caller retry as a fresh
    /// prompt after the run releases.
    pub fn steer(
        &self,
        expected_run_id: Option<&str>,
        steering_tx: &mpsc::Sender<String>,
        message: String,
    ) -> Result<()> {
        let state = self.state.lock();
        let active = state
            .active
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("there is no active run to steer"))?;
        validate_run_id(&active.lease.run_id, expected_run_id)?;
        if !accepts_control(active.phase) {
            bail!(
                "run {} is {}; steering is accepted only while starting or running",
                active.lease.run_id,
                active.phase.as_str()
            );
        }
        steering_tx
            .try_send(message)
            .map_err(|error| anyhow::anyhow!("unable to enqueue steering message: {error}"))?;
        if let Some(tx) = &active.interrupt_tx {
            let _ = tx.try_send(());
        }
        Ok(())
    }

    /// Returns false only for the legacy "follow up while idle" behavior, which
    /// asks ServerSession to start a fresh prompt. A run-scoped request can
    /// never silently turn into a new run, and is rejected (rather than silently
    /// dropped) when the active run can no longer drain its queue.
    pub fn follow_up(
        &self,
        expected_run_id: Option<&str>,
        follow_up_tx: &mpsc::Sender<String>,
        message: String,
    ) -> Result<bool> {
        let state = self.state.lock();
        let Some(active) = state.active.as_ref() else {
            if let Some(expected) = nonempty(expected_run_id) {
                bail!("run `{expected}` is no longer active");
            }
            return Ok(false);
        };
        validate_run_id(&active.lease.run_id, expected_run_id)?;
        if !accepts_control(active.phase) {
            bail!(
                "run {} is {}; follow-up is accepted only while starting or running",
                active.lease.run_id,
                active.phase.as_str()
            );
        }
        follow_up_tx
            .try_send(message)
            .map_err(|error| anyhow::anyhow!("unable to enqueue follow-up message: {error}"))?;
        Ok(true)
    }

    /// Fence all final messages, persistence, and terminal events. A late task
    /// from another epoch must return immediately when this returns false.
    pub fn begin_finalizing(&self, lease: &RunLease) -> bool {
        let mut state = self.state.lock();
        let Some(active) = state.active.as_mut() else {
            return self.stale(lease, "begin_finalizing");
        };
        if active.lease != *lease {
            return self.stale(lease, "begin_finalizing");
        }
        active.phase = RunPhase::Finalizing;
        true
    }

    pub fn mark_stuck(&self, lease: &RunLease, reason: &str) -> bool {
        let mut state = self.state.lock();
        let Some(active) = state.active.as_mut() else {
            return false;
        };
        if active.lease != *lease {
            return false;
        }
        active.phase = RunPhase::CancellationStuck;
        tracing::error!(
            run_id = %lease.run_id,
            run_epoch = lease.epoch,
            reason,
            phase = RunPhase::CancellationStuck.as_str(),
            "session run could not be confirmed stopped; restart or operator intervention required"
        );
        true
    }

    pub fn mark_persistence_degraded(&self, lease: &RunLease, reason: &str) -> bool {
        let mut state = self.state.lock();
        let Some(active) = state.active.as_mut() else {
            return false;
        };
        if active.lease != *lease {
            return false;
        }
        if active.phase == RunPhase::PersistenceDegraded {
            return true;
        }
        active.phase = RunPhase::PersistenceDegraded;
        self.persistence_degraded.fetch_add(1, Ordering::Relaxed);
        tracing::error!(
            run_id = %lease.run_id,
            run_epoch = lease.epoch,
            reason,
            phase = RunPhase::PersistenceDegraded.as_str(),
            "session persistence could not be committed; restart or operator intervention required"
        );
        true
    }

    pub fn finish(&self, lease: &RunLease) -> bool {
        let mut state = self.state.lock();
        let Some(active) = state.active.as_ref() else {
            return self.stale(lease, "finish");
        };
        if active.lease != *lease {
            return self.stale(lease, "finish");
        }
        let finished = state.active.take().expect("active run checked above");
        if !finished.client_request_id.is_empty() {
            state
                .recent_requests
                .push_back((finished.client_request_id, finished.lease));
            // Transport retries are expected shortly after a response. Keep a
            // small bounded window; durable idempotency can later move into the
            // append-only run journal without changing this API.
            while state.recent_requests.len() > 64 {
                state.recent_requests.pop_front();
            }
        }
        self.is_streaming.store(false, Ordering::Release);
        self.active_tasks.fetch_sub(1, Ordering::Relaxed);
        tracing::info!(
            run_id = %lease.run_id,
            run_epoch = lease.epoch,
            phase = "idle",
            "session run released"
        );
        true
    }

    pub fn snapshot(&self) -> Option<RunSnapshot> {
        self.state.lock().active.as_ref().map(|active| RunSnapshot {
            run_id: active.lease.run_id.clone(),
            epoch: active.lease.epoch,
            phase: active.phase,
        })
    }

    pub fn request_lease(&self, client_request_id: &str) -> Option<RunLease> {
        if client_request_id.is_empty() {
            return None;
        }
        let state = self.state.lock();
        state
            .active
            .as_ref()
            .filter(|active| active.client_request_id == client_request_id)
            .map(|active| active.lease.clone())
            .or_else(|| {
                state
                    .recent_requests
                    .iter()
                    .find(|(request_id, _)| request_id == client_request_id)
                    .map(|(_, lease)| lease.clone())
            })
    }

    pub fn active_task_count(&self) -> usize {
        self.active_tasks.load(Ordering::Relaxed)
    }

    pub fn stale_epoch_drop_count(&self) -> u64 {
        self.stale_epoch_drops.load(Ordering::Relaxed)
    }

    pub fn persistence_degraded_count(&self) -> u64 {
        self.persistence_degraded.load(Ordering::Relaxed)
    }

    fn stale(&self, lease: &RunLease, operation: &'static str) -> bool {
        self.stale_epoch_drops.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(
            run_id = %lease.run_id,
            run_epoch = lease.epoch,
            operation,
            "dropped stale run completion"
        );
        false
    }
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

fn validate_run_id(active_run_id: &str, expected_run_id: Option<&str>) -> Result<()> {
    if let Some(expected) = nonempty(expected_run_id) {
        if expected != active_run_id {
            bail!("run `{expected}` is no longer active; current run is `{active_run_id}`");
        }
    }
    Ok(())
}

/// Whether a run-scoped control command (steer / follow-up) can still reach the
/// run loop's queues. Only `Starting` and `Running` drain them; once cancelling
/// or finalizing the loop is unwinding and an enqueue would be silently lost.
fn accepts_control(phase: RunPhase) -> bool {
    matches!(phase, RunPhase::Starting | RunPhase::Running)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abort_does_not_release_the_active_run() {
        let streaming = Arc::new(AtomicBool::new(false));
        let control = RunControl::new(streaming.clone());
        let lease = control.begin(Some("run-a"), Some("request-a")).unwrap();

        let snapshot = control.abort().unwrap();
        assert_eq!(snapshot.phase, RunPhase::Cancelling);
        assert!(streaming.load(Ordering::Acquire));
        assert!(control.begin(Some("run-b"), Some("request-b")).is_err());

        assert!(control.begin_finalizing(&lease));
        assert!(control.finish(&lease));
        assert!(!streaming.load(Ordering::Acquire));
        assert_eq!(control.active_task_count(), 0);
    }

    #[test]
    fn stale_epoch_cannot_finish_newer_run() {
        let control = RunControl::new(Arc::new(AtomicBool::new(false)));
        let old = control.begin(Some("run-a"), None).unwrap();
        assert!(control.begin_finalizing(&old));
        assert!(control.finish(&old));

        let current = control.begin(Some("run-b"), None).unwrap();
        assert!(!control.begin_finalizing(&old));
        assert!(!control.finish(&old));
        assert_eq!(control.snapshot().unwrap().run_id, "run-b");
        assert_eq!(control.stale_epoch_drop_count(), 2);
        assert!(control.begin_finalizing(&current));
        assert!(control.finish(&current));
    }

    #[test]
    fn completed_client_request_is_idempotently_remembered() {
        let control = RunControl::new(Arc::new(AtomicBool::new(false)));
        let lease = control.begin(Some("run-a"), Some("request-a")).unwrap();
        assert!(control.begin_finalizing(&lease));
        assert!(control.finish(&lease));
        assert_eq!(control.request_lease("request-a"), Some(lease));
        assert!(control.begin(Some("run-b"), Some("request-a")).is_err());
    }

    #[test]
    fn persistence_failure_keeps_session_unavailable() {
        let streaming = Arc::new(AtomicBool::new(false));
        let control = RunControl::new(streaming.clone());
        let lease = control.begin(Some("run-a"), Some("request-a")).unwrap();
        assert!(control.begin_finalizing(&lease));
        assert!(control.mark_persistence_degraded(&lease, "disk full"));

        assert_eq!(
            control.snapshot().unwrap().phase,
            RunPhase::PersistenceDegraded
        );
        assert!(streaming.load(Ordering::Acquire));
        assert!(control.begin(Some("run-b"), Some("request-b")).is_err());
        assert_eq!(control.active_task_count(), 1);
        // The degraded transition is counted for observability.
        assert_eq!(control.persistence_degraded_count(), 1);
        assert!(control.mark_persistence_degraded(&lease, "same failure reported twice"));
        assert_eq!(
            control.persistence_degraded_count(),
            1,
            "one run entering degraded state is counted once"
        );
    }

    #[test]
    fn abort_resend_stress_never_has_two_active_tasks() {
        let control = RunControl::new(Arc::new(AtomicBool::new(false)));
        for iteration in 0..100 {
            let lease = control
                .begin(
                    Some(&format!("run-{iteration}")),
                    Some(&format!("request-{iteration}")),
                )
                .unwrap();
            assert_eq!(control.active_task_count(), 1);
            control.abort();
            assert!(control
                .begin(Some("must-not-start"), Some("new-request"))
                .is_err());
            assert_eq!(control.active_task_count(), 1);
            assert!(control.begin_finalizing(&lease));
            assert!(control.finish(&lease));
            assert_eq!(control.active_task_count(), 0);
        }
    }

    #[test]
    fn stale_run_scoped_controls_cannot_touch_current_queues_or_cancel() {
        let control = RunControl::new(Arc::new(AtomicBool::new(false)));
        let current = control
            .begin(Some("run-current"), Some("request-current"))
            .unwrap();
        let (steering_tx, mut steering_rx) = mpsc::channel(4);
        let (follow_up_tx, mut follow_up_rx) = mpsc::channel(4);

        assert!(control
            .steer(Some("run-old"), &steering_tx, "stale steer".to_string())
            .is_err());
        assert!(control
            .follow_up(
                Some("run-old"),
                &follow_up_tx,
                "stale follow-up".to_string()
            )
            .is_err());
        assert!(control.abort_expected(Some("run-old")).is_err());
        assert!(steering_rx.try_recv().is_err());
        assert!(follow_up_rx.try_recv().is_err());
        assert_eq!(control.snapshot().unwrap().phase, RunPhase::Starting);

        assert!(control
            .steer(
                Some("run-current"),
                &steering_tx,
                "current steer".to_string()
            )
            .is_ok());
        assert_eq!(steering_rx.try_recv().unwrap(), "current steer");
        assert!(control
            .abort_expected(Some("run-current"))
            .unwrap()
            .is_some());
        assert!(control.begin_finalizing(&current));
        assert!(control.finish(&current));
    }

    #[test]
    fn scoped_follow_up_cannot_turn_into_a_new_run_after_terminal() {
        let control = RunControl::new(Arc::new(AtomicBool::new(false)));
        let (follow_up_tx, mut follow_up_rx) = mpsc::channel(1);

        assert!(control
            .follow_up(Some("finished-run"), &follow_up_tx, "late".to_string())
            .is_err());
        assert!(follow_up_rx.try_recv().is_err());
        assert!(!control
            .follow_up(None, &follow_up_tx, "legacy idle prompt".to_string())
            .unwrap());
    }

    #[test]
    fn steer_and_follow_up_rejected_once_run_is_cancelling_or_finalizing() {
        let control = RunControl::new(Arc::new(AtomicBool::new(false)));
        let lease = control.begin(Some("run-1"), None).unwrap();
        let (steering_tx, mut steering_rx) = mpsc::channel(4);
        let (follow_up_tx, mut follow_up_rx) = mpsc::channel(4);

        // Accepted while the loop can still drain its queues.
        assert!(control
            .steer(Some("run-1"), &steering_tx, "ok".to_string())
            .is_ok());
        assert_eq!(steering_rx.try_recv().unwrap(), "ok");

        // Cancelling: the loop is unwinding, so enqueueing would silently drop
        // the message — reject instead and never touch the queues.
        control.abort();
        assert!(control
            .steer(Some("run-1"), &steering_tx, "lost".to_string())
            .is_err());
        assert!(control
            .follow_up(Some("run-1"), &follow_up_tx, "lost".to_string())
            .is_err());
        assert!(steering_rx.try_recv().is_err());
        assert!(follow_up_rx.try_recv().is_err());

        // Finalizing: same — reject and keep the queues empty.
        assert!(control.begin_finalizing(&lease));
        assert!(control
            .steer(Some("run-1"), &steering_tx, "lost".to_string())
            .is_err());
        assert!(control
            .follow_up(Some("run-1"), &follow_up_tx, "lost".to_string())
            .is_err());
        assert!(steering_rx.try_recv().is_err());
        assert!(follow_up_rx.try_recv().is_err());
    }
}
