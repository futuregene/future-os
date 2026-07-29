use std::{
    future::Future,
    sync::{atomic::AtomicBool, Arc},
};

use anyhow::{bail, Result};
use parking_lot::Mutex;
use tokio::sync::mpsc;

use super::{RunControl, RunLease, RunSnapshot};

struct RuntimeTask {
    lease: RunLease,
    // Retained by the runtime so a future explicit force-cancel policy can
    // terminate a cancellation-stuck task without recovering a JoinHandle
    // from ServerSession. Cooperative cancellation remains the normal path.
    _abort_handle: tokio::task::AbortHandle,
}

/// Authoritative per-session execution runtime.
///
/// The control state and task ownership live together behind short locks.
/// Model/tool/persistence futures run outside those locks; only the matching
/// run lease can finalize state or release its task slot.
pub struct SessionRuntime {
    control: RunControl,
    task: Mutex<Option<RuntimeTask>>,
}

impl SessionRuntime {
    pub fn new(is_streaming: Arc<AtomicBool>) -> Self {
        Self {
            control: RunControl::new(is_streaming),
            task: Mutex::new(None),
        }
    }

    pub fn begin(
        &self,
        requested_run_id: Option<&str>,
        client_request_id: Option<&str>,
    ) -> Result<RunLease> {
        // Lock order is always task -> control. The completion monitor holds
        // the same task lock while releasing control state, so no new begin
        // can observe Idle before the old task slot is handed back.
        let task = self.task.lock();
        if let Some(active) = task.as_ref() {
            bail!(
                "runtime task {} at epoch {} has not exited",
                active.lease.run_id,
                active.lease.epoch
            );
        }
        self.control.begin(requested_run_id, client_request_id)
    }

    pub fn request_lease(&self, client_request_id: &str) -> Option<RunLease> {
        self.control.request_lease(client_request_id)
    }

    pub fn install_cancellation(
        &self,
        lease: &RunLease,
        interrupt_tx: mpsc::Sender<()>,
        interrupt_flag: Arc<AtomicBool>,
    ) -> bool {
        self.control
            .install_cancellation(lease, interrupt_tx, interrupt_flag)
    }

    pub fn steer(
        &self,
        expected_run_id: Option<&str>,
        steering_tx: &mpsc::Sender<String>,
        message: String,
    ) -> Result<()> {
        self.control.steer(expected_run_id, steering_tx, message)
    }

    pub fn follow_up(
        &self,
        expected_run_id: Option<&str>,
        follow_up_tx: &mpsc::Sender<String>,
        message: String,
    ) -> Result<bool> {
        self.control
            .follow_up(expected_run_id, follow_up_tx, message)
    }

    /// Request cooperative cancellation and arm the bounded acknowledgement
    /// timer. The task remains owned and the session remains unavailable until
    /// that same lease finalizes.
    pub fn request_abort(self: &Arc<Self>, expected_run_id: Option<&str>) -> Result<()> {
        let Some(snapshot) = self.control.abort_expected(expected_run_id)? else {
            return Ok(());
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let runtime = self.clone();
            let lease = RunLease {
                run_id: snapshot.run_id,
                epoch: snapshot.epoch,
            };
            handle.spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                if runtime.control.snapshot().is_some_and(|active| {
                    active.run_id == lease.run_id
                        && active.epoch == lease.epoch
                        && active.phase == super::RunPhase::Cancelling
                }) {
                    let _ = runtime
                        .control
                        .mark_stuck(&lease, "cancellation acknowledgement timed out");
                }
            });
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Option<RunSnapshot> {
        self.control.snapshot()
    }

    pub fn begin_finalizing(&self, lease: &RunLease) -> bool {
        self.control.begin_finalizing(lease)
    }

    pub fn finish(&self, lease: &RunLease) -> bool {
        if self.task.lock().is_some() {
            return false;
        }
        self.control.finish(lease)
    }

    pub fn mark_stuck(&self, lease: &RunLease, reason: &str) -> bool {
        self.control.mark_stuck(lease, reason)
    }

    pub fn mark_persistence_degraded(&self, lease: &RunLease, reason: &str) -> bool {
        self.control.mark_persistence_degraded(lease, reason)
    }

    /// Spawn and register the only task allowed for this session. The monitor
    /// is runtime-owned, so a panic cannot silently orphan the lifecycle state.
    pub fn spawn(
        self: &Arc<Self>,
        lease: RunLease,
        future: impl Future<Output = ()> + Send + 'static,
    ) -> Result<()> {
        let mut slot = self.task.lock();
        if let Some(active) = slot.as_ref() {
            bail!(
                "runtime already owns task {} at epoch {}",
                active.lease.run_id,
                active.lease.epoch
            );
        }
        // Reserve ownership before the future can be polled. This prevents a
        // rejected registration from leaving an already-running orphan task.
        let task = tokio::spawn(future);
        let abort_handle = task.abort_handle();
        *slot = Some(RuntimeTask {
            lease: lease.clone(),
            _abort_handle: abort_handle,
        });
        drop(slot);

        let runtime = self.clone();
        tokio::spawn(async move {
            let outcome = task.await;
            let mut slot = runtime.task.lock();
            if !slot.as_ref().is_some_and(|active| active.lease == lease) {
                return;
            }
            match outcome {
                Err(error) => {
                    let _ = runtime.mark_stuck(&lease, &error.to_string());
                }
                Ok(()) => match runtime.snapshot() {
                    Some(active)
                        if active.run_id == lease.run_id
                            && active.epoch == lease.epoch
                            && active.phase == super::RunPhase::Finalizing =>
                    {
                        let _ = runtime.control.finish(&lease);
                    }
                    Some(active)
                        if active.run_id == lease.run_id
                            && active.epoch == lease.epoch
                            && matches!(
                                active.phase,
                                super::RunPhase::CancellationStuck
                                    | super::RunPhase::PersistenceDegraded
                            ) => {}
                    Some(active)
                        if active.run_id == lease.run_id && active.epoch == lease.epoch =>
                    {
                        let _ = runtime.mark_stuck(&lease, "run task exited without finalizing");
                    }
                    _ => {}
                },
            }
            *slot = None;
        });
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn owns_task(&self, lease: &RunLease) -> bool {
        self.task
            .lock()
            .as_ref()
            .is_some_and(|active| active.lease == *lease)
    }

    #[cfg(test)]
    pub(crate) fn active_task_count(&self) -> usize {
        self.control.active_task_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use tokio::sync::Notify;

    #[tokio::test]
    async fn runtime_owns_task_until_matching_run_finishes() {
        let streaming = Arc::new(AtomicBool::new(false));
        let runtime = Arc::new(SessionRuntime::new(streaming.clone()));
        let lease = runtime.begin(Some("run-owned"), None).unwrap();
        let started = Arc::new(Notify::new());
        let finalize = Arc::new(Notify::new());
        let finalizing = Arc::new(Notify::new());
        let exit = Arc::new(Notify::new());
        let task_runtime = runtime.clone();
        let task_lease = lease.clone();
        let task_started = started.clone();
        let task_finalize = finalize.clone();
        let task_finalizing = finalizing.clone();
        let task_exit = exit.clone();

        runtime
            .spawn(lease.clone(), async move {
                task_started.notify_one();
                task_finalize.notified().await;
                assert!(task_runtime.begin_finalizing(&task_lease));
                task_finalizing.notify_one();
                task_exit.notified().await;
            })
            .unwrap();
        started.notified().await;
        assert!(runtime.owns_task(&lease));
        assert_eq!(runtime.active_task_count(), 1);
        assert!(streaming.load(Ordering::Acquire));

        finalize.notify_one();
        finalizing.notified().await;
        assert!(runtime.begin(Some("run-too-early"), None).is_err());

        exit.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while runtime.owns_task(&lease) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(runtime.snapshot().is_none());
        assert_eq!(runtime.active_task_count(), 0);
        assert!(!streaming.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn task_exit_without_finalization_becomes_explicit_fault() {
        let runtime = Arc::new(SessionRuntime::new(Arc::new(AtomicBool::new(false))));
        let lease = runtime.begin(Some("run-orphan"), None).unwrap();
        runtime.spawn(lease.clone(), async {}).unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if runtime
                    .snapshot()
                    .is_some_and(|run| run.phase == super::super::RunPhase::CancellationStuck)
                    && !runtime.owns_task(&lease)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn persistence_degraded_survives_task_monitor_exit() {
        let runtime = Arc::new(SessionRuntime::new(Arc::new(AtomicBool::new(false))));
        let lease = runtime.begin(Some("run-degraded"), None).unwrap();
        let task_runtime = runtime.clone();
        let task_lease = lease.clone();
        runtime
            .spawn(lease.clone(), async move {
                assert!(task_runtime.begin_finalizing(&task_lease));
                assert!(
                    task_runtime.mark_persistence_degraded(&task_lease, "injected write failure")
                );
            })
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while runtime.owns_task(&lease) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            runtime.snapshot().unwrap().phase,
            super::super::RunPhase::PersistenceDegraded
        );
        assert!(runtime.begin(Some("run-must-not-start"), None).is_err());
    }
}
