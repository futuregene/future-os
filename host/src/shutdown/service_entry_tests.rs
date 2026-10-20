//! Original shared shutdown coordinator assertions, independent of Desktop.
use super::{reset_for_test, ServiceStop};
use crate::shutdown::Operation;
use std::sync::Arc;
use std::time::Duration;

fn perform_stop(
    reason: &'static str,
    build: impl FnOnce() -> std::io::Result<tokio::runtime::Runtime>,
    operation: &Operation,
) -> Result<(), String> {
    let mut services = ServiceStop::prepare(operation, reason, || {}).map_err(|e| e.to_string())?;
    let runtime = match build() {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            services.runtime_unavailable(error);
            None
        }
    };
    if let Some(runtime) = &runtime {
        runtime.block_on(services.disconnect());
    }
    if let Some(runtime) = runtime {
        runtime.shutdown_timeout(operation.remaining(Duration::from_secs(1)));
    }
    services.finish().map_err(|e| e.to_string())
}
fn begin(reason: &'static str) -> Arc<Operation> {
    begin_with(reason, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
    })
}
fn begin_with(
    reason: &'static str,
    build: impl FnOnce() -> std::io::Result<tokio::runtime::Runtime> + Send + 'static,
) -> Arc<Operation> {
    crate::shutdown::begin(reason, move |operation| {
        perform_stop(reason, build, operation)
    })
}
fn begin_with_budget(
    reason: &'static str,
    build: impl FnOnce() -> std::io::Result<tokio::runtime::Runtime> + Send + 'static,
    budget: Duration,
) -> Arc<Operation> {
    crate::shutdown::begin_with_test_budget(
        reason,
        move |operation| perform_stop(reason, build, operation),
        budget,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shutdown::snapshot;
    use std::sync::Arc;
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
        assert!(crate::request_scope::admit().is_err());
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
        assert!(operation.wait_for_worker_for_test(Duration::from_secs(5)));
        assert_eq!(operation.wait().unwrap_err(), error);
        assert_eq!(operation.snapshot().operation_id, initial.operation_id);
        assert!(crate::shutdown::failure().is_some());
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
        super::reset_for_test();

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
        assert!(
            operation.wait_for_worker_for_test(Duration::from_secs(5)),
            "the worker must return through the arm, not hang in the publish"
        );

        let failure = crate::shutdown::failure();
        let failure = failure.expect("a failed stop publishes its reason");
        assert!(
            failure.contains("Remote shutdown deadline exceeded"),
            "the remote stage must be the one that ran out of budget: {failure}"
        );
        drop(bridge);
        super::reset_for_test();
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

    /// A worker that *succeeds* after the budget is gone must not publish cleanup
    /// success: the deadline rewrite in `Operation::finish` is the operation's only
    /// fail-safe, and what it publishes is what `stop()`/`wait()` report.
    ///
    #[test]
    fn worker_panic_is_shared_terminal_failure() {
        let _home = crate::auth_store::test_support::HomeGuard::new("stop-panic");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let operation = begin_with("fixture", || panic!("injected cleanup panic"));
        assert!(operation.wait().unwrap_err().contains("panicked"));
        assert!(begin("reset").wait().is_err());
    }
}
