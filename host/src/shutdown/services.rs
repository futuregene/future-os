//! Ordered shared service teardown. Entries drive the async phase and own
//! runtime shutdown and presentation cleanup between the two service barriers.
use super::Operation;
use crate::request_scope::{active_callers, clear_owned_runs, close_admission, stop_owned_runs};
use std::time::{Duration, Instant};

pub struct ServiceStop<'a> {
    operation: &'a Operation,
    reason: &'static str,
    errors: Vec<String>,
}

impl<'a> ServiceStop<'a> {
    pub fn prepare(
        operation: &'a Operation,
        reason: &'static str,
        freeze_presentation: impl FnOnce(),
    ) -> Result<Self, crate::Error> {
        if let Some(error) = super::failure() {
            return Err(error.into());
        }
        // Cancel the Agent immediately and bound the Desktop caller drain separately.
        let deadline = Instant::now() + operation.remaining(Duration::from_secs(3));
        close_admission();
        crate::tasks::stop_admission();
        crate::remote_peer::runtime::stop_admission();
        crate::runtime::freeze_tasks();
        freeze_presentation();
        // Cancel accepted Agent commands while waiting for their Desktop callers;
        // a blocked shell or approval must not prevent global cancellation.
        crate::agent_host::host().request_stop();
        operation.stage("callers");
        crate::request_scope::wait_for_callers(deadline);
        Ok(Self {
            operation,
            reason,
            errors: Vec::new(),
        })
    }

    pub fn runtime_unavailable(&mut self, error: std::io::Error) {
        self.errors
            .push(format!("Desktop cleanup runtime unavailable: {error}"));
        crate::remote::stop();
    }

    pub fn record_error(&mut self, error: String) {
        self.errors.push(error);
    }

    pub async fn disconnect(&mut self) {
        let Self {
            operation,
            reason,
            errors,
        } = self;
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
        if let Err(error) =
            crate::remote::wait_for_tasks(operation.remaining(Duration::from_secs(2))).await
        {
            errors.push(format!("Remote server cleanup failed: {error}"));
        }
        if let Err(error) =
            crate::remote_peer::runtime::shutdown(operation.remaining(Duration::from_secs(2))).await
        {
            errors.push(format!("Remote controller cleanup failed: {error}"));
        }
        if !crate::agent_host::host().owns_agent() {
            operation.stage("external_runs");
            errors.extend(stop_owned_runs(operation.remaining(Duration::from_secs(3))).await);
        }
    }

    pub fn finish(self) -> Result<(), crate::Error> {
        let Self {
            operation,
            mut errors,
            ..
        } = self;
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
        let active = active_callers();
        if active != 0 {
            errors.push(format!(
                "Desktop commands did not finish before shutdown ({active} still active)"
            ));
        }
        if operation.failed() {
            errors.push("Desktop cleanup exceeded its shared deadline".into());
        }
        if errors.is_empty() {
            clear_owned_runs();
            Ok(())
        } else {
            let error = errors.join("; ");
            super::record_failure(error.clone());
            Err(error.into())
        }
    }
}

#[cfg(test)]
use crate::request_scope::{
    admit, close_admission_guard_for_test, is_stopping, own_run, owned_count, reconcile_owned_runs,
    release_run,
};
#[cfg(test)]
fn reset_for_test() {
    super::reset_for_test();
    super::clear_failure_for_test();
    crate::request_scope::reset_for_test();
    crate::tasks::reset_for_test();
    crate::remote_peer::runtime::reset_after_shutdown_for_test();
    crate::remote::reset_after_shutdown_for_test();
}
#[cfg(test)]
fn stop(reason: &'static str) -> Result<(), crate::Error> {
    stop_with_runtime(reason, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
    })
}
#[cfg(test)]
fn stop_with_runtime(
    reason: &'static str,
    build: impl FnOnce() -> std::io::Result<tokio::runtime::Runtime> + Send + 'static,
) -> Result<(), crate::Error> {
    super::begin(reason, move |operation| {
        let mut services =
            ServiceStop::prepare(operation, reason, || {}).map_err(|e| e.to_string())?;
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
    })
    .wait()
    .map_err(Into::into)
}
#[cfg(test)]
mod tests {
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
        assert!(crate::shutdown::failure().is_none());
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
            &crate::agent_client::raw_agent_addr(),
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
            &crate::agent_client::raw_agent_addr(),
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
        assert!(crate::shutdown::failure().is_some());
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

    #[tokio::test]
    async fn reconcile_keeps_runs_whose_state_could_not_be_read() {
        use crate::agent_bridge::test_support::Reply;
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-reconcile-down");
        super::reset_for_test();
        let mock = crate::agent_bridge::test_support::mock_agent();
        let client = future_rpc::connection::connect_verified(
            &crate::agent_client::raw_agent_addr(),
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

    #[tokio::test]
    async fn unconfirmed_runs_report_the_transport_outcome() {
        use crate::agent_bridge::test_support::Reply;
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-transport");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let client = future_rpc::connection::connect_verified(
            &crate::agent_client::raw_agent_addr(),
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
    #[test]
    fn quit_admission_guard_closes_admission_and_restores_the_flag() {
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-admission-guard");
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

#[cfg(test)]
#[path = "service_entry_tests.rs"]
mod entry_tests;
