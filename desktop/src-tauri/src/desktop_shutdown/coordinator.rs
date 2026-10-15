//! One process-wide stop operation, shared by every exit/reset/update caller.
//! A native watchdog publishes terminal failure independently of stuck cleanup.
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const TOTAL_BUDGET: Duration = Duration::from_secs(20);
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static OPERATION: Mutex<Option<Arc<Operation>>> = Mutex::new(None);

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StopSnapshot {
    pub operation_id: String,
    pub reason: String,
    pub state: String,
    pub stage: String,
    pub elapsed_ms: u64,
    pub errors: Vec<String>,
}
struct State {
    snapshot: StopSnapshot,
    worker_finished: bool,
}
pub(crate) struct Operation {
    started: Instant,
    deadline: Instant,
    state: Mutex<State>,
    changed: Condvar,
}
impl Operation {
    pub(super) fn remaining(&self, limit: Duration) -> Duration {
        limit.min(self.deadline.saturating_duration_since(Instant::now()))
    }
    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }
    pub(super) fn stage(&self, stage: &str) {
        let mut state = self.state.lock().unwrap();
        if state.snapshot.state == "running" {
            state.snapshot.stage = stage.into();
        }
        drop(state);
        eprintln!("FutureOS: cleanup stage={stage}");
        self.emit_progress();
    }
    fn emit_progress(&self) {
        #[cfg(feature = "gui")]
        if let Some(app) = crate::APP_HANDLE.get() {
            use tauri::Emitter;
            let _ = app.emit("desktop-shutdown-progress", self.snapshot());
        }
    }
    pub(super) fn failed(&self) -> bool {
        self.state.lock().unwrap().snapshot.state == "failed"
    }
    fn finish(&self, result: Result<(), String>, worker_finished: bool) {
        let mut state = self.state.lock().unwrap();
        state.worker_finished |= worker_finished;
        if state.snapshot.state == "running" {
            let result = if result.is_ok() && Instant::now() >= self.deadline {
                Err(format!("Desktop cleanup deadline exceeded during {}; restart/reset/install require process exit", state.snapshot.stage))
            } else {
                result
            };
            match result {
                Ok(()) => {
                    state.snapshot.state = "stopped".into();
                    state.snapshot.stage = "complete".into();
                }
                Err(error) => {
                    *super::STOP_FAILURE.lock().unwrap() = Some(error.clone());
                    state.snapshot.state = "failed".into();
                    state.snapshot.errors.push(error);
                }
            }
            state.snapshot.elapsed_ms = self.started.elapsed().as_millis() as u64;
        }
        self.changed.notify_all();
    }
    pub(crate) fn wait(&self) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        while state.snapshot.state == "running" {
            state = self.changed.wait(state).unwrap();
        }
        if state.snapshot.state == "stopped" {
            Ok(())
        } else {
            Err(state.snapshot.errors.join("; "))
        }
    }
    #[cfg(any(feature = "gui", test))]
    fn snapshot(&self) -> StopSnapshot {
        let state = self.state.lock().unwrap();
        let mut snapshot = state.snapshot.clone();
        if snapshot.state == "running" {
            snapshot.elapsed_ms = self.started.elapsed().as_millis() as u64;
        }
        snapshot
    }
}
#[cfg(any(feature = "gui", test))]
pub(crate) fn snapshot() -> Option<StopSnapshot> {
    OPERATION.lock().unwrap().as_ref().map(|op| op.snapshot())
}
pub(crate) fn begin(reason: &'static str) -> Arc<Operation> {
    begin_with(reason, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
    })
}
pub(super) fn begin_with(
    reason: &'static str,
    build: impl FnOnce() -> std::io::Result<tokio::runtime::Runtime> + Send + 'static,
) -> Arc<Operation> {
    begin_with_budget(reason, build, TOTAL_BUDGET)
}
fn begin_with_budget(
    reason: &'static str,
    build: impl FnOnce() -> std::io::Result<tokio::runtime::Runtime> + Send + 'static,
    budget: Duration,
) -> Arc<Operation> {
    let mut current = OPERATION.lock().unwrap();
    if let Some(operation) = current.as_ref() {
        return operation.clone();
    }
    let started = Instant::now();
    let operation = Arc::new(Operation {
        started,
        deadline: started + budget,
        state: Mutex::new(State {
            worker_finished: false,
            snapshot: StopSnapshot {
                operation_id: format!(
                    "stop-{}-{}",
                    std::process::id(),
                    NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ),
                reason: reason.into(),
                state: "running".into(),
                stage: "admission".into(),
                elapsed_ms: 0,
                errors: vec![],
            },
        }),
        changed: Condvar::new(),
    });
    // Linearize producer admission before returning the shared operation.
    super::ADMISSION.lock().unwrap().closed = true;
    *current = Some(operation.clone());
    drop(current);
    let watcher = operation.clone();
    if let Err(error) = std::thread::Builder::new().name("desktop-stop-deadline".into()).spawn(move || {
        let mut state = watcher.state.lock().unwrap();
        while state.snapshot.state == "running" {
            let remaining = watcher.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let stage = state.snapshot.stage.clone(); drop(state);
                watcher.finish(Err(format!("Desktop cleanup deadline exceeded during {stage}; restart/reset/install require process exit")), false);
                return;
            }
            state = watcher.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }) {
        operation.finish(Err(format!("Cannot start Desktop cleanup watchdog: {error}")), true);
        return operation;
    }
    let worker = operation.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("desktop-stop".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                super::perform_stop(reason, build, &worker)
            }))
            .map_err(|_| {
                "Desktop cleanup worker panicked; instance ownership may remain until process exit"
                    .to_owned()
            })
            .and_then(|result| result.map_err(|error| error.to_string()));
            worker.finish(result, true);
            worker.emit_progress();
        })
    {
        operation.finish(
            Err(format!("Cannot start Desktop cleanup worker: {error}")),
            true,
        );
    }
    operation
}

#[cfg(test)]
pub(super) fn reset_for_test() {
    let mut current = OPERATION.lock().unwrap();
    if let Some(operation) = current.as_ref() {
        let state = operation.state.lock().unwrap();
        let (state, _) = operation
            .changed
            .wait_timeout_while(state, TOTAL_BUDGET, |state| !state.worker_finished)
            .unwrap();
        assert!(
            state.worker_finished,
            "cleanup must finish before test HOME changes"
        );
    }
    *current = None;
}

// Serialize destructive follow-up actions after the common cleanup barrier.
// GUI close requests must not exit between cleanup and installer handoff.
#[cfg(feature = "gui")]
static ACTION: Mutex<Option<&'static str>> = Mutex::new(None);
#[cfg(feature = "gui")]
pub(crate) struct ActionGuard;
#[cfg(feature = "gui")]
impl Drop for ActionGuard {
    fn drop(&mut self) {
        *ACTION.lock().unwrap() = None;
    }
}
#[cfg(feature = "gui")]
pub(crate) fn claim_action(reason: &'static str) -> Result<ActionGuard, crate::AppError> {
    let mut action = ACTION.lock().unwrap();
    if let Some(active) = *action {
        return Err(format!("Desktop {active} is already in progress").into());
    }
    if let Some(operation) = snapshot() {
        if matches!(operation.reason.as_str(), "quit" | "exit" | "headless_exit")
            || reason == "update"
        {
            return Err(
                "Desktop is already stopping; cannot begin another maintenance action".into(),
            );
        }
    }
    *action = Some(reason);
    Ok(ActionGuard)
}
#[cfg(all(feature = "gui", test))]
pub(crate) fn maintenance_active() -> bool {
    ACTION.lock().unwrap().is_some()
}

#[cfg(feature = "gui")]
pub(crate) fn begin_quit() -> Option<Arc<Operation>> {
    // Same lock order as claim_action: an exit cannot slip between claiming
    // maintenance and establishing its stop operation, or vice versa.
    let action = ACTION.lock().unwrap();
    if action.is_some() && !snapshot().is_some_and(|operation| operation.state == "failed") {
        return None;
    }
    Some(begin("quit"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_callers_share_one_worker_reason_and_terminal_result() {
        let _home = crate::auth_store::test_support::HomeGuard::new("stop-singleflight");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let (entered, started) = std::sync::mpsc::channel();
        let (release, ready) = std::sync::mpsc::channel();
        let operation = begin_with("fixture", move || {
            entered.send(()).unwrap();
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
        });
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let second = begin("update");
        assert!(Arc::ptr_eq(&operation, &second));
        assert_eq!(snapshot().unwrap().reason, "fixture");
        assert!(super::super::admit().is_err());
        let waiter = std::thread::spawn(move || second.wait());
        release.send(()).unwrap();
        operation.wait().unwrap();
        waiter.join().unwrap().unwrap();
        assert_eq!(snapshot().unwrap().state, "stopped");
        assert!(Arc::ptr_eq(&begin("reset"), &operation));
    }
    #[test]
    fn stalled_worker_reports_failure_and_late_completion_never_reopens_the_gate() {
        let _home = crate::auth_store::test_support::HomeGuard::new("stop-deadline");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let (release, ready) = std::sync::mpsc::channel();
        let operation = begin_with_budget(
            "fixture",
            move || {
                ready.recv_timeout(Duration::from_secs(5)).unwrap();
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
            },
            Duration::from_millis(500),
        );
        let error = operation.wait().unwrap_err();
        assert!(error.contains("deadline exceeded"));
        let initial = operation.snapshot();
        release.send(()).unwrap();
        // Observe actual worker teardown before HOME may change, without
        // accepting a late success as permission for installation/restart.
        let state = operation.state.lock().unwrap();
        let (state, _) = operation
            .changed
            .wait_timeout_while(state, Duration::from_secs(5), |s| !s.worker_finished)
            .unwrap();
        assert!(state.worker_finished);
        drop(state);
        assert_eq!(operation.wait().unwrap_err(), error);
        assert_eq!(operation.snapshot().operation_id, initial.operation_id);
        assert!(super::super::STOP_FAILURE.lock().unwrap().is_some());
    }
    #[cfg(feature = "gui")]
    #[test]
    fn destructive_actions_are_exclusive_and_release_after_failure() {
        let _home = crate::auth_store::test_support::HomeGuard::new("stop-actions");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let action = claim_action("update").unwrap();
        assert!(maintenance_active());
        assert!(claim_action("reset").is_err());
        drop(action);
        assert!(!maintenance_active());
        assert!(claim_action("restart").is_ok());
        let operation = begin_quit().unwrap();
        assert!(claim_action("reset").is_err());
        operation.wait().unwrap();
        assert!(claim_action("update").is_err());
    }

    #[test]
    fn an_expired_budget_can_never_publish_cleanup_success() {
        let _home = crate::auth_store::test_support::HomeGuard::new("stop-expired");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let operation = begin_with_budget(
            "fixture",
            || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
            },
            Duration::ZERO,
        );
        assert!(operation.wait().unwrap_err().contains("deadline exceeded"));
        assert_eq!(snapshot().unwrap().state, "failed");
    }

    #[test]
    fn worker_panic_is_shared_terminal_failure() {
        let _home = crate::auth_store::test_support::HomeGuard::new("stop-panic");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let operation = begin_with("fixture", || panic!("injected cleanup panic"));
        assert!(operation.wait().unwrap_err().contains("panicked"));
        assert!(begin("reset").wait().is_err());
    }
}
