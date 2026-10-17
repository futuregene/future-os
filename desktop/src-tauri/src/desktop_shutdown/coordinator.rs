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
    if action.is_some() && snapshot().is_none_or(|operation| operation.state != "failed") {
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

    /// The cleanup runtime this host uses for a Desktop stop, in the shape the
    /// coordinator takes it: a current-thread runtime the stop can block on.
    ///
    /// Named rather than inlined in one test so more than one fixture executes the
    /// same builder, and so its body is covered by whichever of them really reaches
    /// `build()`.
    fn current_thread_runtime() -> std::io::Result<tokio::runtime::Runtime> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
    }

    #[test]
    fn an_expired_budget_can_never_publish_cleanup_success() {
        let _home = crate::auth_store::test_support::HomeGuard::new("stop-expired");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        // `begin_with_budget` joins an operation that is already current, so a
        // sibling test's leftover stop would make this fixture pass without ever
        // running its own worker. Start from a clean registry: the point is an
        // *expired* budget during real cleanup.
        reset_for_test();
        let operation = begin_with_budget("fixture", current_thread_runtime, Duration::ZERO);
        assert!(operation.wait().unwrap_err().contains("deadline exceeded"));
        assert_eq!(snapshot().unwrap().state, "failed");
        // Let this operation's worker finish before the next test can join it.
        reset_for_test();
    }

    /// The remote-shutdown deadline arm in `desktop_shutdown.rs`'s `perform_stop` — the
    /// lines that record `Remote shutdown deadline exceeded` and fall back to the
    /// immediate `remote::stop()` — cannot fire under the production budget, and this
    /// test is the supervisor-granted (test-only, §S11.6 ask 1) way to reach it.
    ///
    /// Why it cannot fire there: the arm's deadline is
    /// `operation.remaining(Duration::from_secs(2))` = 2 s, while the future it bounds —
    /// `remote::stop_gracefully` → `send_mobile_disconnect_notice` — is itself capped at
    /// `DISCONNECT_NOTICE_TIMEOUT` (750 ms), so the outer timeout always observes a
    /// completed future. `begin_with_budget` is the only knob that changes that, which is
    /// why the grant covers this file rather than a production seam.
    ///
    /// The bridge is the shell's supported fake (`install_fake_state`), and its tasks —
    /// the broker's accept loop and the `async_nats` connection — live on the runtime
    /// below, which this test does not poll again until the stop has finished. That is the
    /// arm's real condition (a connection that is not making progress), made
    /// deterministic: `publish` queues, `flush` waits for a reply nobody reads, and the
    /// sub-750 ms budget is gone before it can arrive.
    #[test]
    fn a_budget_below_the_disconnect_notice_cap_expires_the_remote_publish() {
        let _home = crate::auth_store::test_support::HomeGuard::new("stop-remote-deadline");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        super::super::reset_for_test();

        let bridge_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime to install the fake bridge on");
        let bridge = bridge_runtime.block_on(crate::remote::test_support::install_fake_state(
            "stop-remote-deadline",
        ));

        // Strictly below the 750 ms cap the production path always has available.
        let operation = begin_with_budget(
            "fixture",
            current_thread_runtime,
            Duration::from_millis(100),
        );
        assert!(
            operation.wait().is_err(),
            "the budget is spent before the bounded publish can finish"
        );
        let state = operation.state.lock().unwrap();
        let (state, timed_out) = operation
            .changed
            .wait_timeout_while(state, Duration::from_secs(5), |state| {
                !state.worker_finished
            })
            .unwrap();
        assert!(
            !timed_out.timed_out() && state.worker_finished,
            "the worker must return through the arm, not hang in the publish"
        );
        drop(state);

        let failure = super::super::STOP_FAILURE.lock().unwrap().clone();
        let failure = failure.expect("a failed stop publishes its reason");
        assert!(
            failure.contains("Remote shutdown deadline exceeded"),
            "the remote stage must be the one that ran out of budget: {failure}"
        );
        drop(bridge);
        super::super::reset_for_test();
    }

    /// A stop that is allowed to finish really does build its cleanup runtime and
    /// reach a terminal state — the property the expired-budget test cannot show,
    /// because its budget is gone before the worker gets that far.
    #[test]
    fn a_stop_with_a_budget_reaches_the_runtime_builder_and_finishes() {
        static BUILDER_CALLS: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let _home = crate::auth_store::test_support::HomeGuard::new("stop-reaches-builder");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        reset_for_test();
        let operation = begin_with("fixture", || {
            BUILDER_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            current_thread_runtime()
        });
        let result = operation.wait();
        assert_eq!(
            BUILDER_CALLS.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "the cleanup must build its runtime exactly once: {result:?}"
        );
        let state = snapshot().expect("the operation is published").state;
        assert!(
            state == "stopped" || state == "failed",
            "a finished stop must publish a terminal state, not {state}"
        );
        reset_for_test();
    }

    /// A quit cannot begin while another maintenance action owns the barrier, and a
    /// quit that repeats while a stop is already running is *answered* with that stop
    /// instead of starting a second one.
    #[cfg(feature = "gui")]
    #[test]
    fn a_quit_cannot_start_a_second_stop() {
        let _home = crate::auth_store::test_support::HomeGuard::new("stop-second-quit");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        reset_for_test();
        let claimed = claim_action("restart").unwrap();
        assert!(
            begin_quit().is_none(),
            "a quit cannot begin while another maintenance action owns the barrier"
        );
        drop(claimed);
        let operation = begin_quit().expect("the quit begins once the barrier is free");
        let repeated = begin_quit().expect("the repeat quit is answered, not refused");
        assert!(
            Arc::ptr_eq(&operation, &repeated),
            "the second quit must join the running stop instead of starting another"
        );
        operation.wait().unwrap();
        reset_for_test();
    }

    /// A worker that *succeeds* after the budget is gone must not publish cleanup
    /// success: the deadline rewrite in `Operation::finish` is the operation's only
    /// fail-safe, and what it publishes is what `stop()`/`wait()` report.
    ///
    /// Reached by construction rather than through `begin_with_budget`. There the
    /// watchdog publishes the same failure at the deadline, and which of the two
    /// threads takes the state lock first is a scheduling race (measured: the
    /// watchdog is the older waiter and normally wins, so the worker's late
    /// success is discarded before `finish` can rewrite it). The rewrite itself is
    /// deterministic, so the operation is built directly — no watchdog exists to
    /// race — and the result is asserted through the same published failure the
    /// callers read.
    #[test]
    fn a_success_after_the_deadline_is_rewritten_into_a_failure() {
        // This fixture asserts on the process-global `STOP_FAILURE`, and every neighbour
        // that creates or drops a `HomeGuard` clears it (`HomeGuard` → `cancel_test_tasks`
        // → `desktop_shutdown::reset_for_test`). Without the guard the clear can land
        // between `finish` publishing the reason and the read below — measured 9/10 runs
        // of this module red before the guard was added, green with `--test-threads=1`.
        // Holding the HOME lock is what serialises it, exactly as the neighbouring
        // `begin_with` fixtures do.
        let _home = crate::auth_store::test_support::HomeGuard::new("stop-late-success");
        let started = Instant::now();
        let operation = Operation {
            started,
            deadline: started,
            state: Mutex::new(State {
                worker_finished: false,
                snapshot: StopSnapshot {
                    operation_id: "stop-fixture-1".into(),
                    reason: "fixture".into(),
                    state: "running".into(),
                    stage: "remote".into(),
                    elapsed_ms: 0,
                    errors: vec![],
                },
            }),
            changed: Condvar::new(),
        };

        operation.finish(Ok(()), true);

        let snapshot = operation.snapshot();
        let published = super::super::STOP_FAILURE.lock().unwrap().clone();
        // Clear the process-global failure before judging, so a failing assertion
        // here cannot leave the next test reading this one's reason.
        reset_for_test();
        assert_eq!(snapshot.state, "failed");
        assert_eq!(snapshot.stage, "remote", "the stage that ran out of budget");
        let error = published.expect("a rewritten deadline failure is published");
        assert!(
            error.contains("deadline exceeded during remote"),
            "the reason must name the stage that ran out of budget: {error}"
        );
        assert!(
            snapshot.errors.contains(&error),
            "the published reason is the operation's own: {:?}",
            snapshot.errors
        );
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
