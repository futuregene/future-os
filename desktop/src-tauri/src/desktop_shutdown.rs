//! Stop Desktop producers before draining the Agent. External ownership is
//! preserved; only prompts submitted by this Desktop receive exact-run aborts.
use future_rpc::connection::VerifiedAgentClient;
use std::{
    collections::HashMap,
    sync::{Condvar, LazyLock, Mutex},
    time::{Duration, Instant},
};

type OwnedRun = (String, String);
struct Admission {
    closed: bool,
    active: usize,
}
static ADMISSION: Mutex<Admission> = Mutex::new(Admission {
    closed: false,
    active: 0,
});
static CHANGED: Condvar = Condvar::new();
static OWNED: LazyLock<Mutex<HashMap<OwnedRun, VerifiedAgentClient>>> =
    LazyLock::new(Default::default);
mod coordinator;
pub(crate) use coordinator::begin;
#[cfg(feature = "gui")]
pub(crate) use coordinator::{begin_quit, claim_action};
#[cfg(feature = "gui")]
pub(crate) use coordinator::{snapshot, StopSnapshot};
static STOP_FAILURE: Mutex<Option<String>> = Mutex::new(None);

pub(crate) struct CommandGuard;
impl Drop for CommandGuard {
    fn drop(&mut self) {
        ADMISSION.lock().unwrap().active -= 1;
        CHANGED.notify_all();
    }
}
#[allow(clippy::result_large_err)] // Tonic callers preserve native Status errors.
pub(crate) fn admit() -> Result<CommandGuard, tonic::Status> {
    let mut admission = ADMISSION.lock().unwrap();
    if admission.closed {
        return Err(tonic::Status::unavailable("Desktop is stopping"));
    }
    admission.active += 1;
    Ok(CommandGuard)
}
pub(crate) fn is_stopping() -> bool {
    ADMISSION.lock().unwrap().closed
}
pub(crate) fn own_run(session: String, run: String, client: VerifiedAgentClient) {
    if !session.is_empty() && !run.is_empty() {
        OWNED.lock().unwrap().insert((session, run), client);
    }
}
pub(crate) fn release_run(session: &str, run: &str) {
    OWNED.lock().unwrap().remove(&(session.into(), run.into()));
}
pub(crate) fn owned_count() -> usize {
    OWNED.lock().unwrap().len()
}

/// A collector can fail or hand queued work to an observer. Reconcile even
/// locally terminal rows, but never confuse a lost connection with a terminal.
pub(crate) async fn reconcile_owned_runs() {
    let runs: Vec<_> = OWNED
        .lock()
        .unwrap()
        .iter()
        .map(|(key, client)| (key.clone(), client.clone()))
        .collect();
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        for ((session, run), mut client) in runs {
            let command = crate::agent_bridge::get_run_state_command(session.clone(), run.clone());
            let Ok(Ok(response)) =
                tokio::time::timeout(Duration::from_secs(1), client.execute_command(command)).await
            else {
                continue;
            };
            let response = response.into_inner();
            if response.success
                && owned_run_is_terminal(&future_rpc::decode::response_data(&response), &run)
            {
                release_run(&session, &run);
            }
        }
    })
    .await;
}

fn owned_run_is_terminal(state: &serde_json::Value, run: &str) -> bool {
    // Active/queued evidence takes precedence over terminal snapshots.
    if state
        .pointer("/activeRun/runId")
        .and_then(serde_json::Value::as_str)
        == Some(run)
        || state
            .get("queuedRuns")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|runs| {
                runs.iter().any(|entry| {
                    entry.get("runId").and_then(serde_json::Value::as_str) == Some(run)
                })
            })
    {
        return false;
    }
    state
        .pointer("/interruptedRun/runId")
        .and_then(serde_json::Value::as_str)
        == Some(run)
        || (state
            .pointer("/requestedRun/runId")
            .and_then(serde_json::Value::as_str)
            == Some(run)
            && matches!(
                state
                    .pointer("/requestedRun/status")
                    .and_then(serde_json::Value::as_str),
                Some("completed" | "cancelled" | "error" | "failed")
            ))
}

/// Cancellation acknowledgments are not proof of completion. Retain unknown
/// runs and return their exact identities to the final installation barrier.
async fn stop_owned_runs(budget: Duration) -> Vec<String> {
    use futures::{stream, StreamExt};
    let runs: Vec<_> = OWNED
        .lock()
        .unwrap()
        .iter()
        .map(|(key, client)| (key.clone(), client.clone()))
        .collect();
    let deadline = tokio::time::Instant::now() + budget;
    stream::iter(runs)
        .map(|((session, run), mut client)| async move {
            let mut last_error = "no verified terminal state".to_owned();
            let stopped = tokio::time::timeout_at(deadline, async {
                loop {
                    // Both commands are exact-run fenced. A run_not_queued reply
                    // is normal if the queued run became active or already ended.
                    for kind in ["cancel_queued_run", "abort", "get_state"] {
                        let command = future_rpc::proto::RpcCommand {
                            r#type: kind.into(),
                            session_id: session.clone(),
                            run_id: run.clone(),
                            ..Default::default()
                        };
                        match tokio::time::timeout(
                            Duration::from_millis(500),
                            client.execute_command(command),
                        )
                        .await
                        {
                            Ok(Ok(response)) => {
                                let response = response.into_inner();
                                if kind == "get_state"
                                    && response.success
                                    && owned_run_is_terminal(
                                        &future_rpc::decode::response_data(&response),
                                        &run,
                                    )
                                {
                                    return;
                                }
                                if !response.success {
                                    last_error = response.error;
                                } else if kind == "get_state" {
                                    last_error =
                                        "get_state did not confirm the exact run terminal".into();
                                }
                            }
                            Ok(Err(error)) => last_error = error.to_string(),
                            Err(_) => last_error = format!("{kind} timed out"),
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await;
            if stopped.is_ok() {
                release_run(&session, &run);
                None
            } else {
                Some(format!(
                    "Desktop run {session}/{run} could not be confirmed stopped: {last_error}"
                ))
            }
        })
        .buffer_unordered(8)
        .filter_map(async |error| error)
        .collect()
        .await
}

pub(crate) fn stop(reason: &'static str) -> Result<(), crate::AppError> {
    begin(reason).wait().map_err(Into::into)
}
#[cfg(test)]
fn stop_with_runtime(
    reason: &'static str,
    build: impl FnOnce() -> std::io::Result<tokio::runtime::Runtime> + Send + 'static,
) -> Result<(), crate::AppError> {
    coordinator::begin_with(reason, build)
        .wait()
        .map_err(Into::into)
}
fn perform_stop(
    reason: &'static str,
    build: impl FnOnce() -> std::io::Result<tokio::runtime::Runtime>,
    operation: &coordinator::Operation,
) -> Result<(), crate::AppError> {
    if let Some(error) = STOP_FAILURE.lock().unwrap().clone() {
        return Err(error.into());
    }
    // Cancel the Agent immediately and bound the Desktop caller drain separately.
    let deadline = Instant::now() + operation.remaining(Duration::from_secs(3));
    let mut errors = Vec::new();
    ADMISSION.lock().unwrap().closed = true;
    crate::tasks::stop_admission();
    crate::runtime::freeze_tasks();
    #[cfg(all(feature = "gui", not(test)))]
    if let Some(manager) = crate::terminal::server::manager() {
        manager.freeze();
    }
    // Cancel accepted Agent commands while waiting for their Desktop callers;
    // a blocked shell or approval must not prevent global cancellation.
    crate::agent_host::host().request_stop();
    operation.stage("callers");
    {
        let mut admission = ADMISSION.lock().unwrap();
        admission.closed = true;
        while admission.active != 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            admission = CHANGED.wait_timeout(admission, remaining).unwrap().0;
        }
    }
    let cleanup_runtime = match build() {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            errors.push(format!("Desktop cleanup runtime unavailable: {error}"));
            crate::remote::stop();
            None
        }
    };
    if let Some(runtime) = &cleanup_runtime {
        runtime.block_on(async {
            operation.stage("remote");
            if tokio::time::timeout(
                operation.remaining(Duration::from_secs(2)),
                crate::remote::stop_gracefully(reason),
            )
            .await
            .is_err()
            {
                errors.push("Remote shutdown deadline exceeded".into());
                crate::remote::stop();
            }
            if !crate::agent_host::host().owns_agent() {
                operation.stage("external_runs");
                errors.extend(stop_owned_runs(operation.remaining(Duration::from_secs(3))).await);
            }
        });
    }
    #[cfg(all(feature = "gui", not(test)))]
    if let Some(manager) = crate::terminal::server::manager() {
        operation.stage("terminals");
        if let Err(error) =
            manager.shutdown_checked(operation.remaining(crate::terminal::manager::CLOSE_GRACE))
        {
            errors.push(error);
        }
    }
    if let Some(runtime) = cleanup_runtime {
        runtime.shutdown_timeout(operation.remaining(Duration::from_secs(1)));
    }
    // Stop waits until the native service scope has released storage and lock.
    operation.stage("agent");
    if let Err(error) = crate::agent_host::host().stop_until(operation.deadline()) {
        errors.push(error.to_string());
    }
    operation.stage("background");
    if let Err(error) =
        crate::runtime::cancel_all_tasks(operation.remaining(Duration::from_secs(5)))
    {
        errors.push(error.to_string());
    }
    // A command may finish while the Agent and tracked tasks are drained.
    // Require zero callers at the final barrier, not at the earlier wait budget.
    operation.stage("verification");
    let active = ADMISSION.lock().unwrap().active;
    if active != 0 {
        errors.push(format!(
            "Desktop commands did not finish before shutdown ({active} still active)"
        ));
    }
    if operation.failed() {
        errors.push("Desktop cleanup exceeded its shared deadline".into());
    }
    if errors.is_empty() {
        OWNED.lock().unwrap().clear();
        Ok(())
    } else {
        let error = errors.join("; ");
        *STOP_FAILURE.lock().unwrap() = Some(error.clone());
        Err(error.into())
    }
}

#[cfg(all(test, feature = "gui"))]
pub(crate) fn fail_for_test(error: &str) {
    *STOP_FAILURE.lock().unwrap() = Some(error.to_owned());
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    coordinator::reset_for_test();
    let mut admission = ADMISSION.lock().unwrap();
    // Some read-only mock RPC tests intentionally share no HOME fixture.
    // Preserve their live guards rather than resetting the counter.
    admission.closed = false;
    *STOP_FAILURE.lock().unwrap() = None;
    OWNED.lock().unwrap().clear();
    crate::tasks::reset_for_test();
}

/// Test-only: close admission exactly as `perform_stop` does, so the arms that
/// react to a *stopping* process can be exercised.
///
/// The only production writer of `ADMISSION.closed` is `perform_stop` (a real
/// shutdown: `tasks::stop_admission`, `runtime::freeze_tasks`, the Agent's
/// `request_stop` and a caller drain), and `reset_for_test` above only *clears*
/// the flag — which is why an arm guarded by `is_stopping()` had no seam at all.
/// This setter is the missing half; the clearing half already exists.
///
/// Deliberately not `cfg(feature = "gui")`: `tasks` is compiled in both flavours
/// and the arm this reaches is not gui-gated.
///
/// **Nothing in production may call this.** It is the same shape as the crate's
/// other test-only setters, and it is paired with [`reset_for_test`] — a fixture
/// that closes admission must restore it, because the flag is process-global and
/// `admit()` rejects every command while it is set.
#[cfg(test)]
pub(crate) fn close_admission_for_test() {
    ADMISSION.lock().unwrap().closed = true;
}

/// Test-only RAII half of [`close_admission_for_test`]: closes admission for as
/// long as the guard lives and hands the flag back on drop.
///
/// The setter alone leaves a fixture responsible for restoring process-global
/// state on every path, including a panicking assertion. `reset_for_test` would
/// also restore it, but it additionally stops tasks and clears the publishable
/// stop, which is not what a fixture that merely borrowed the flag should do to
/// the tests running beside it. This guard touches exactly the bit it changed,
/// and only when it changed it: a process that was already stopping stays
/// stopping.
#[cfg(test)]
pub(crate) struct AdmissionClosedForTest {
    was_closed: bool,
}

#[cfg(test)]
impl Drop for AdmissionClosedForTest {
    fn drop(&mut self) {
        if !self.was_closed {
            ADMISSION.lock().unwrap().closed = false;
        }
    }
}

#[cfg(test)]
pub(crate) fn close_admission_guard_for_test() -> AdmissionClosedForTest {
    let mut admission = ADMISSION.lock().unwrap();
    let was_closed = admission.closed;
    admission.closed = true;
    AdmissionClosedForTest { was_closed }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ownership_requires_positive_terminal_evidence_and_preserves_active_or_queued_work() {
        use serde_json::json;
        assert!(!super::owned_run_is_terminal(&json!({}), "run"));
        let terminal = json!({"requestedRun":{"runId":"run","status":"completed"}});
        assert!(super::owned_run_is_terminal(&terminal, "run"));
        assert!(!super::owned_run_is_terminal(
            &json!({"activeRun":{"runId":"run"},"requestedRun":{"runId":"run","status":"completed"}}),
            "run"
        ));
        assert!(!super::owned_run_is_terminal(
            &json!({"queuedRuns":[{"runId":"run"}],"requestedRun":{"runId":"run","status":"completed"}}),
            "run"
        ));
        assert!(!super::owned_run_is_terminal(
            &json!({"requestedRun":{"runId":"run","status":"unknown"}}),
            "run"
        ));
        assert!(super::owned_run_is_terminal(
            &json!({"interruptedRun":{"runId":"run"}}),
            "run"
        ));
    }

    #[test]
    fn command_finishing_during_later_cleanup_does_not_create_a_sticky_failure() {
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-late-command");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let command = super::admit().unwrap();
        super::stop_with_runtime("fixture", || {
            drop(command);
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
        })
        .unwrap();
        assert!(super::STOP_FAILURE.lock().unwrap().is_none());
    }

    #[test]
    fn a_command_still_active_after_all_cleanup_keeps_the_install_barrier_closed() {
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-active-command");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let command = super::admit().unwrap();
        let error = super::stop("fixture").unwrap_err();
        assert!(error.to_string().contains("still active"));
        assert!(super::stop("fixture").is_err());
        drop(command);
    }

    #[tokio::test]
    async fn queued_ownership_is_retained_until_verified_terminal_state() {
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-owned-queued");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let client = future_rpc::connection::connect_verified(
            &crate::agent_bridge::raw_agent_addr(),
            future_rpc::connection::EXPECTED_AGENT_VERSION,
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap();
        super::own_run("session".into(), "run".into(), client);
        mock.push_run_state("run", serde_json::json!({"queuedRuns":[{"runId":"run"}]}));
        super::reconcile_owned_runs().await;
        assert_eq!(super::owned_count(), 1);
        mock.push_run_state(
            "run",
            serde_json::json!({"requestedRun":{"runId":"run","status":"completed"}}),
        );
        super::reconcile_owned_runs().await;
        assert_eq!(super::owned_count(), 0);
    }

    #[tokio::test]
    async fn prompt_acknowledgment_cannot_resurrect_already_completed_ownership() {
        use crate::agent_bridge::test_support::Reply;
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-owned-ack");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let (entered, arrived) = tokio::sync::oneshot::channel();
        let (response, data) = tokio::sync::oneshot::channel();
        mock.push("prompt", Reply::Deferred { entered, data });
        let mut client = crate::agent_bridge::connect_agent().await.unwrap();
        let task = tokio::spawn(async move {
            client
                .execute_command(future_rpc::proto::RpcCommand {
                    r#type: "prompt".into(),
                    session_id: "session".into(),
                    requested_run_id: "run".into(),
                    ..Default::default()
                })
                .await
        });
        arrived.await.unwrap();
        assert_eq!(super::owned_count(), 1);
        super::release_run("session", "run");
        response.send(serde_json::json!({"run_id":"run"})).unwrap();
        assert!(task.await.unwrap().unwrap().into_inner().success);
        assert_eq!(super::owned_count(), 0);
    }

    #[tokio::test]
    async fn external_stop_confirms_terminals_and_retains_unknown_or_wrong_run_results() {
        use crate::agent_bridge::test_support::{mock_agent, Reply};
        let _home = crate::auth_store::test_support::HomeGuard::new("external-stop-confirm");
        let mock = mock_agent();
        let client = future_rpc::connection::connect_verified(
            &crate::agent_bridge::raw_agent_addr(),
            future_rpc::connection::EXPECTED_AGENT_VERSION,
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap();
        super::own_run("session".into(), "run".into(), client.clone());
        // Queue -> active is not a fatal cancellation rejection. Only the
        // final authoritative state, scoped to this run, proves completion.
        mock.push("cancel_queued_run", Reply::Reject("run_not_queued".into()));
        mock.push_run_state(
            "run",
            serde_json::json!({"requestedRun":{"runId":"run","status":"cancelled"}}),
        );
        assert!(super::stop_owned_runs(std::time::Duration::from_secs(1))
            .await
            .is_empty());
        assert_eq!(super::owned_count(), 0);
        for request in mock.requests() {
            if request.r#type != "get_agent_readiness" {
                assert_eq!(request.session_id, "session");
                assert_eq!(request.run_id, "run");
            }
        }
        super::own_run("session".into(), "run".into(), client);
        mock.push_run_state(
            "run",
            serde_json::json!({"requestedRun":{"runId":"someone-else","status":"completed"}}),
        );
        let errors = super::stop_owned_runs(std::time::Duration::from_millis(100)).await;
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("session/run"));
        assert_eq!(super::owned_count(), 1);
        // The same failure must reach the public cleanup barrier and remain
        // sticky for update/reset/restart, without discarding the ledger.
        let error = tokio::task::spawn_blocking(|| super::stop("fixture"))
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("could not be confirmed stopped"));
        assert!(super::STOP_FAILURE.lock().unwrap().is_some());
        assert_eq!(super::owned_count(), 1);
    }

    #[test]
    fn runtime_failure_still_drops_background_work_and_blocks_retry() {
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-runtime-failure");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        // Open the process-global runtime registry first: a sibling test that froze
        // it would make `runtime::spawn` below a no-op, so this fixture's spawned
        // future would never run and the test would assert about nothing.
        crate::runtime::cancel_test_tasks();
        struct Done(std::sync::mpsc::Sender<()>);
        impl Drop for Done {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let (tx, completed) = std::sync::mpsc::channel();
        let done = Done(tx);
        let _task = crate::runtime::spawn(async move {
            let _done = done;
            std::future::pending::<()>().await;
        });
        let error = super::stop_with_runtime("fixture", || {
            Err(std::io::Error::other("injected runtime failure"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("injected runtime failure"));
        completed
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(super::stop("fixture").is_err());
        assert!(super::is_stopping());
    }

    /// Reconcile must **retain** a run whose state could not be read: a transport
    /// failure is not evidence of a terminal state, so the row stays owned and the
    /// next pass retries it instead of releasing work that may still be running.
    #[tokio::test]
    async fn reconcile_keeps_runs_whose_state_could_not_be_read() {
        use crate::agent_bridge::test_support::Reply;
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-reconcile-down");
        super::reset_for_test();
        let mock = crate::agent_bridge::test_support::mock_agent();
        let client = future_rpc::connection::connect_verified(
            &crate::agent_bridge::raw_agent_addr(),
            future_rpc::connection::EXPECTED_AGENT_VERSION,
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap();
        super::own_run("session".into(), "run".into(), client);
        mock.push(
            "get_state#run",
            Reply::Status(tonic::Code::Unavailable, "agent transport down"),
        );
        super::reconcile_owned_runs().await;
        assert_eq!(
            super::owned_count(),
            1,
            "an unreadable state must not release the run"
        );
    }

    #[test]
    fn closing_admission_rejects_new_commands_without_losing_active_guards() {
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-admission");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        super::reset_for_test();
        let command = super::admit().unwrap();
        super::ADMISSION.lock().unwrap().closed = true;
        assert!(super::admit().is_err());
        drop(command);
        assert!(super::is_stopping());
        // A closed admission with **no** published stop operation is the state
        // the status poll reports as "stopping". `coordinator::begin` sets both
        // globals in one critical section, but they are separate: this fixture
        // closes admission alone, which is the only construction of that state.
        #[cfg(feature = "gui")]
        {
            let status = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime for the status poll")
                .block_on(crate::agent_supervisor::agent_status());
            assert_eq!(status.phase, "stopping");
            assert!(
                status.shutdown.is_none(),
                "the stopping arm has no stop snapshot: {:?}",
                status.shutdown
            );
        }
        super::reset_for_test();
    }

    /// A run whose cancellation cannot be confirmed must say *why* in the message
    /// the installer barrier shows: a transport status and a per-command timeout are
    /// both evidence, and neither may be replaced by the "no verified terminal
    /// state" default — that default only describes a run that answered normally.
    #[tokio::test]
    async fn unconfirmed_runs_report_the_transport_outcome() {
        use crate::agent_bridge::test_support::Reply;
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-transport");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let client = future_rpc::connection::connect_verified(
            &crate::agent_bridge::raw_agent_addr(),
            future_rpc::connection::EXPECTED_AGENT_VERSION,
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap();
        super::own_run("session".into(), "run".into(), client);
        // A transport-level rejection for the first command, and a command that
        // never answers for the second: both are recorded as the run's reason.
        mock.push(
            "cancel_queued_run",
            Reply::Status(tonic::Code::Unavailable, "agent transport down"),
        );
        let (entered, _arrived) = tokio::sync::oneshot::channel();
        let (_response, data) = tokio::sync::oneshot::channel();
        mock.push("abort", Reply::Deferred { entered, data });

        let errors = super::stop_owned_runs(std::time::Duration::from_millis(800)).await;
        assert_eq!(errors.len(), 1, "{errors:?}");
        // Hoisted: an `assert!` argument that is only an expression (here
        // `errors[0]`) is evaluated on failure alone, so a passing run would
        // never execute it.
        let reason = errors[0].clone();
        assert!(reason.contains("session/run"), "{reason}");
        assert!(
            !reason.contains("no verified terminal state"),
            "a transport outcome must replace the default reason: {reason}"
        );
        // The run is retained: an unconfirmed cancel is not a stop.
        assert_eq!(super::owned_count(), 1);
    }

    /// The seam's two halves. Closing admission must be visible to both readers
    /// (`is_stopping` for the event-loop arms, `admit` for every command), and
    /// dropping the guard must restore the flag it found rather than assume it was
    /// open — the flag is process-global and other tests assert it is false.
    ///
    /// This is the seam the `RunEvent::Ready` arm's `return` in
    /// `gui::dispatch_run_event` is measured through: its only production input is
    /// a quit that begins before readiness, which no fixture can race reliably.
    #[test]
    fn quit_admission_guard_closes_admission_and_restores_the_flag() {
        let was_stopping = super::is_stopping();
        {
            let _closed = super::close_admission_guard_for_test();
            assert!(super::is_stopping(), "the guard must close admission");
            let refused = super::admit().err().expect("closed admission refuses");
            assert_eq!(refused.code(), tonic::Code::Unavailable, "{refused}");
        }
        assert_eq!(
            super::is_stopping(),
            was_stopping,
            "the guard must restore the flag it found"
        );
    }
}
