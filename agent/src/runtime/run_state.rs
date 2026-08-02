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
    pub run_sequence: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSnapshot {
    pub run_id: String,
    pub epoch: u64,
    pub run_sequence: Option<u64>,
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
        self.begin_with_sequence(requested_run_id, client_request_id, None)
    }

    pub fn begin_with_sequence(
        &self,
        requested_run_id: Option<&str>,
        client_request_id: Option<&str>,
        run_sequence: Option<u64>,
    ) -> Result<RunLease> {
        let mut state = self.state.lock();
        let client_request_id = client_request_id.unwrap_or_default();
        // Idempotency: reject a client request id we already accepted. A
        // cancellation-stuck run never completed, so it must NOT count as
        // already-accepted — a transport retry of the same id has to go through
        // (the stuck lease is released as a dead lease just below).
        let already_accepted = !client_request_id.is_empty()
            && (state.active.as_ref().is_some_and(|active| {
                active.client_request_id == client_request_id
                    && active.phase != RunPhase::CancellationStuck
            }) || state
                .recent_requests
                .iter()
                .any(|(request_id, _)| request_id == client_request_id));
        if already_accepted {
            bail!("client request `{client_request_id}` was already accepted");
        }
        // Self-heal a cancellation-stuck predecessor. `SessionRuntime::begin` holds
        // the task lock and only reaches here when the task slot is empty — which
        // means the stuck run's completion monitor has already returned, so there
        // is no live writer to race. A stuck run never finalizes on its own, so
        // without this the session would stay locked until the agent restarts.
        // `PersistenceDegraded` is intentionally NOT released (fail-closed: a
        // failed persistence commit needs operator eyes, not silent recovery).
        let stuck_dead = state
            .active
            .as_ref()
            .filter(|active| active.phase == RunPhase::CancellationStuck)
            .map(|active| (active.lease.run_id.clone(), active.lease.epoch));
        if let Some((dead_run_id, dead_epoch)) = stuck_dead {
            state.active.take();
            self.is_streaming.store(false, Ordering::Release);
            self.active_tasks.fetch_sub(1, Ordering::Relaxed);
            tracing::warn!(
                run_id = %dead_run_id,
                run_epoch = dead_epoch,
                "self-heal: releasing cancellation-stuck dead lease on new begin"
            );
            // Do NOT push the dead client_request_id into `recent_requests`: a
            // stuck run did not complete, so a retry of the same request id must
            // be allowed through (the idempotency check above already exempts it).
        } else if let Some(active) = &state.active {
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
            run_sequence,
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

    /// Request cancellation without making the session idle. Only the matching
    /// run task may later transition through finalizing and release the session.
    ///
    /// If the run is already finalizing or terminal (`CancellationStuck` /
    /// `PersistenceDegraded`) the cancellation point has passed, so this returns
    /// `Ok(None)` (a no-op "nothing to cancel") without touching the phase or
    /// arming the stuck-detection timer — clobbering `Finalizing` back to
    /// `Cancelling` there is exactly what produces a spurious
    /// `cancellation_stuck` on the error path.
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
            // A run that is already finalizing or terminal is past the point where
            // cancellation can reach it; the loop is unwinding and will release the
            // session itself. Aborting here would clobber `Finalizing` back to
            // `Cancelling` and arm a pointless 30s timer — and that clobber is what
            // turns a clean error-path finalize into a spurious
            // `cancellation_stuck` (see the completion monitor). Treat it as no-op.
            if !abortable(active.phase) {
                tracing::debug!(
                    run_id = %active.lease.run_id,
                    phase = active.phase.as_str(),
                    "abort ignored: run is finalizing or terminal"
                );
                return Ok(None);
            }
            active.phase = RunPhase::Cancelling;
            (
                RunSnapshot {
                    run_id: active.lease.run_id.clone(),
                    epoch: active.lease.epoch,
                    run_sequence: active.lease.run_sequence,
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

    pub fn recover_persistence_degraded(&self, lease: &RunLease) -> bool {
        let mut state = self.state.lock();
        let Some(active) = state.active.as_mut() else {
            return false;
        };
        if active.lease != *lease || active.phase != RunPhase::PersistenceDegraded {
            return false;
        }
        active.phase = RunPhase::Finalizing;
        drop(state);
        self.finish(lease)
    }

    pub fn snapshot(&self) -> Option<RunSnapshot> {
        self.state.lock().active.as_ref().map(|active| RunSnapshot {
            run_id: active.lease.run_id.clone(),
            epoch: active.lease.epoch,
            run_sequence: active.lease.run_sequence,
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

/// Whether an `abort` can still meaningfully request cancellation. A run that is
/// already finalizing or terminal (`CancellationStuck` / `PersistenceDegraded`) is
/// past the cancellation point — aborting it would only clobber its phase and arm
/// a useless timer — so `abort_expected` treats those as a no-op. `Cancelling` is
/// included so a repeated abort stays idempotent while the loop drains.
fn abortable(phase: RunPhase) -> bool {
    matches!(
        phase,
        RunPhase::Starting | RunPhase::Running | RunPhase::Cancelling
    )
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
    fn abort_is_noop_once_finalizing() {
        let streaming = Arc::new(AtomicBool::new(true));
        let control = RunControl::new(streaming.clone());
        let lease = control.begin(Some("run-1"), Some("req-1")).unwrap();
        assert!(control.begin_finalizing(&lease));

        // Aborting a finalizing run must not clobber the phase back to Cancelling
        // and must report no active snapshot to cancel.
        assert!(control.abort_expected(Some("run-1")).unwrap().is_none());
        assert_eq!(control.snapshot().unwrap().phase, RunPhase::Finalizing);

        // The run still finalizes and releases normally.
        assert!(control.finish(&lease));
        assert!(control.snapshot().is_none());
        assert!(!streaming.load(Ordering::Acquire));
    }

    #[test]
    fn abort_is_noop_on_stuck_and_degraded() {
        let control = RunControl::new(Arc::new(AtomicBool::new(false)));
        let lease = control.begin(Some("run-1"), None).unwrap();
        control.mark_stuck(&lease, "test");
        assert_eq!(
            control.snapshot().unwrap().phase,
            RunPhase::CancellationStuck
        );

        assert!(control.abort_expected(Some("run-1")).unwrap().is_none());
        assert_eq!(
            control.snapshot().unwrap().phase,
            RunPhase::CancellationStuck,
            "abort must not move a stuck run back to Cancelling"
        );

        // Degraded likewise: abort is a no-op, phase unchanged.
        let control = RunControl::new(Arc::new(AtomicBool::new(false)));
        let lease = control.begin(Some("run-2"), None).unwrap();
        assert!(control.begin_finalizing(&lease));
        control.mark_persistence_degraded(&lease, "disk full");
        assert!(control.abort_expected(Some("run-2")).unwrap().is_none());
        assert_eq!(
            control.snapshot().unwrap().phase,
            RunPhase::PersistenceDegraded
        );
    }

    #[test]
    fn abort_stays_idempotent_while_cancelling() {
        let control = RunControl::new(Arc::new(AtomicBool::new(false)));
        let _lease = control.begin(Some("run-1"), None).unwrap();
        assert!(control.abort_expected(Some("run-1")).unwrap().is_some());
        assert_eq!(control.snapshot().unwrap().phase, RunPhase::Cancelling);
        // A second abort while still cancelling stays actionable (idempotent),
        // not a no-op — Cancelling is abortable.
        assert!(control.abort_expected(Some("run-1")).unwrap().is_some());
        assert_eq!(control.snapshot().unwrap().phase, RunPhase::Cancelling);
    }

    #[test]
    fn begin_releases_cancellation_stuck_dead_lease() {
        let streaming = Arc::new(AtomicBool::new(false));
        let control = RunControl::new(streaming.clone());
        let first = control.begin(Some("run-a"), Some("req-a")).unwrap();
        control.mark_stuck(&first, "test");
        assert_eq!(control.active_task_count(), 1);
        assert!(streaming.load(Ordering::Acquire));

        // A new begin self-heals: the dead stuck lease is released and the new run
        // starts, with invariants reset (no task-count or streaming leak).
        let second = control.begin(Some("run-b"), Some("req-b")).unwrap();
        assert_eq!(second.run_id, "run-b");
        assert_eq!(control.active_task_count(), 1);
        assert!(streaming.load(Ordering::Acquire));
        assert_eq!(control.snapshot().unwrap().phase, RunPhase::Starting);

        assert!(control.begin_finalizing(&second));
        assert!(control.finish(&second));
        assert_eq!(control.active_task_count(), 0);
        assert!(!streaming.load(Ordering::Acquire));
    }

    #[test]
    fn begin_allows_same_request_id_after_stuck() {
        let control = RunControl::new(Arc::new(AtomicBool::new(false)));
        let first = control.begin(Some("run-a"), Some("req-retry")).unwrap();
        control.mark_stuck(&first, "test");

        // The stuck run never completed, so retrying with the same client request
        // id must be accepted (not rejected as "already accepted").
        let second = control.begin(Some("run-a2"), Some("req-retry")).unwrap();
        assert_eq!(second.run_id, "run-a2");
    }

    #[test]
    fn begin_does_not_release_persistence_degraded() {
        let streaming = Arc::new(AtomicBool::new(false));
        let control = RunControl::new(streaming.clone());
        let lease = control.begin(Some("run-a"), Some("req-a")).unwrap();
        assert!(control.begin_finalizing(&lease));
        control.mark_persistence_degraded(&lease, "disk full");

        // Fail-closed: a degraded run is NOT self-healed; the session stays locked
        // for operator intervention.
        assert!(control.begin(Some("run-b"), Some("req-b")).is_err());
        assert_eq!(
            control.snapshot().unwrap().phase,
            RunPhase::PersistenceDegraded
        );
        assert_eq!(control.active_task_count(), 1);
        assert_eq!(control.persistence_degraded_count(), 1);
    }
}
