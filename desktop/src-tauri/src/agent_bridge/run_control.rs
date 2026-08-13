//! Run control: abort an in-flight agent run, mark a run failed, and wait for
//! the agent to confirm idle before snapshotting. These back the abort command
//! and the parent module's prompt finalization.

use super::client::{
    connect_agent, get_state_command, map_rpc_error, run_control_command, RpcResponseExt,
};
use super::replica::AGENT_REPLICAS;
use crate::store;

async fn canonical_active_run_id(
    client: &mut crate::agent_proto::FutureAgentClient<tonic::transport::Channel>,
    session_id: &str,
) -> Result<Option<String>, crate::AppError> {
    let response = client
        .execute_command(get_state_command(session_id.to_string()))
        .await
        .map_err(|status| map_rpc_error("Unable to read Future Agent run state", status))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the state request.")?;
    let state: serde_json::Value = future_rpc::decode::response_data(&response);
    Ok(state
        .get("activeRun")
        .and_then(|run| run.get("runId"))
        .and_then(|run_id| run_id.as_str())
        .filter(|run_id| !run_id.is_empty())
        .map(str::to_string))
}

pub(super) async fn abort_agent_thread(
    thread_id: &str,
    local_run_id: Option<&str>,
) -> Result<(), crate::AppError> {
    let thread =
        store::get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let session_id = thread.agent_session_id.unwrap_or(thread.id);
    let mut client = connect_agent().await?;
    let canonical_run_id = match local_run_id.filter(|run_id| !run_id.is_empty()) {
        Some(local_run_id) => Some(
            AGENT_REPLICAS
                .canonical_for_local(local_run_id)
                // GUI-originated runs use their SQLite id as the Agent's
                // requested/canonical id. A mapping is only required for the
                // synthetic local rows used while observing another client.
                .unwrap_or_else(|| local_run_id.to_string()),
        ),
        None => canonical_active_run_id(&mut client, &session_id).await?,
    };
    client
        .execute_command(run_control_command("abort", session_id, canonical_run_id))
        .await
        // Transport-level Unavailable → AgentUnavailable, so `abort_run`
        // still cancels the run locally when the agent died after the shared
        // channel was established.
        .map_err(|status| map_rpc_error("Unable to abort Future Agent run", status))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the abort request.")?;
    Ok(())
}

/// Abort an in-flight run for an already-resolved agent session id. Unlike
/// [`abort_agent_thread`] it takes the session id directly (the quit guard reads
/// it from `store::active_run_sessions`, so there is no thread to reload) and
/// does not touch the store — on force-quit we only need the agent to stop
/// streaming before the process exits; startup convergence settles the run rows
/// on the next launch. Best-effort at the call site: aborting a session that
/// already finished is a harmless no-op on the agent side.
pub(crate) async fn abort_session(session_id: &str) -> Result<(), crate::AppError> {
    let mut client = connect_agent().await?;
    let canonical_run_id = canonical_active_run_id(&mut client, session_id).await?;
    client
        .execute_command(run_control_command(
            "abort",
            session_id.to_string(),
            canonical_run_id,
        ))
        .await
        .map_err(|error| format!("Unable to abort Future Agent session: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the abort request.")?;
    Ok(())
}

/// Abort an in-flight agent run, then mark its store run cancelled. A missing
/// agent (e.g. the backend is down) is tolerated — the run is still cancelled
/// locally so the UI doesn't strand on a "running" row.
pub async fn abort_run(
    thread_id: String,
    run_id: String,
) -> Result<store::RunRecord, crate::AppError> {
    if let Err(error) = abort_agent_thread(&thread_id, Some(&run_id)).await {
        if !is_agent_unavailable_error(&error) {
            return Err(error);
        }
        eprintln!("FutureOS agent abort skipped because agent is unavailable: {error}");
    }
    // Compare-and-set: only cancel a run that isn't already terminal. If the run
    // finished (completed/failed) in the window before the user's stop landed,
    // leave that terminal state intact — cancelling it would rewrite a successful
    // reply as "stopped" and cascade-cancel its approvals/tool_calls.
    // Either way, return the run's real current state.
    store::update_run_status_if_active(store::UpdateRunStatusInput {
        run_id: run_id.clone(),
        status: "cancelled".to_string(),
        error_message: Some("Terminated by user.".to_string()),
        error_type: Some("abort_requested".to_string()),
    })?;
    store::get_run(&run_id)?.ok_or_else(|| "Run could not be loaded.".to_string().into())
}

pub(super) fn mark_run_failed_if_active(run_id: Option<&str>, error: &str) {
    let Some(run_id) = run_id else {
        return;
    };
    let error_type = crate::run_error::classify_run_error(error);
    // Compare-and-set: only fails a run that isn't already terminal, atomically,
    // so a concurrent `abort_run` (which sets `cancelled`) is never overwritten.
    if let Err(update_error) = store::fail_run_if_active(run_id, error, error_type) {
        eprintln!("FutureOS run failure status update failed: {update_error}");
    }
}

/// CAS a run to `completed` — the success twin of [`mark_run_failed_if_active`].
///
/// The frontend pipeline also writes `completed` once its `agent_prompt` invoke
/// resolves, but that write is only reachable while the webview processes IPC:
/// a hidden/occluded window suspends the webview (macOS), and the invoke
/// response may never be applied. This backend write is the authoritative
/// settle — the row alone gates the sidebar spinner and the composer lock, so
/// the run must not depend on a possibly-suspended frontend to reach terminal.
/// Compare-and-set: a concurrent user abort (`cancelled`) wins and survives.
pub(super) fn mark_run_completed_if_active(run_id: Option<&str>) {
    let Some(run_id) = run_id else {
        return;
    };
    if let Err(update_error) = store::update_run_status_if_active(store::UpdateRunStatusInput {
        run_id: run_id.to_string(),
        status: "completed".to_string(),
        error_message: None,
        error_type: None,
    }) {
        eprintln!("FutureOS run completion status update failed: {update_error}");
    }
}

/// Poll the Agent's `get_state.isStreaming` until it reports idle (or a short
/// timeout / the agent disappears). Best-effort confirmation that the Agent has
/// stopped writing files before the after snapshot (§6.2).
pub(crate) async fn wait_for_agent_idle(session_id: &str) {
    let Ok(mut client) = connect_agent().await else {
        return;
    };
    // ~5s budget at 200ms intervals.
    for _ in 0..25 {
        match client
            .execute_command(get_state_command(session_id.to_string()))
            .await
        {
            Ok(response) => {
                let data = response.into_inner().data;
                let streaming = serde_json::from_str::<serde_json::Value>(&data)
                    .ok()
                    .and_then(|value| value.get("isStreaming").and_then(|s| s.as_bool()))
                    .unwrap_or(false);
                if !streaming {
                    return;
                }
            }
            // Agent unreachable → treat as idle; nothing more we can confirm.
            Err(_) => return,
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

fn is_agent_unavailable_error(error: &crate::AppError) -> bool {
    matches!(error, crate::AppError::AgentUnavailable(_))
}

#[cfg(test)]
mod tests {
    use super::super::replica::AGENT_REPLICAS;
    use super::super::test_support::{
        mock_agent, seed_run, seed_thread, seed_workspace, Reply, TestHome,
    };
    use super::*;

    #[tokio::test]
    async fn abort_session_resolves_active_run_and_aborts() {
        let _home = TestHome::new("rc-abort-session");
        let mock = mock_agent();

        mock.push_data(
            "get_state",
            serde_json::json!({"activeRun": {"runId": "run-canonical"}}),
        );
        mock.push("abort", Reply::Data("{}".to_string()));
        abort_session("sess-1").await.expect("abort");
        let abort = &mock.requests_of("abort")[0];
        assert_eq!(abort.run_id, "run-canonical");
        assert_eq!(abort.session_id, "sess-1");
    }

    #[tokio::test]
    async fn abort_session_without_active_run_aborts_with_empty_run_id() {
        let _home = TestHome::new("rc-abort-no-run");
        let mock = mock_agent();

        mock.push_data("get_state", serde_json::json!({"isStreaming": false}));
        mock.push("abort", Reply::Data("{}".to_string()));
        abort_session("sess-1").await.expect("abort");
        assert_eq!(mock.requests_of("abort")[0].run_id, "");
    }

    #[tokio::test]
    async fn abort_session_error_paths() {
        let _home = TestHome::new("rc-abort-errors");
        let mock = mock_agent();

        // get_state rejected at app level.
        mock.push("get_state", Reply::Reject("no session".to_string()));
        let error = abort_session("sess-1").await.expect_err("state reject");
        assert_eq!(error.to_string(), "no session");

        // get_state transport failure.
        mock.push("get_state", Reply::Status(tonic::Code::Unavailable, "gone"));
        let error = abort_session("sess-1").await.expect_err("state transport");
        assert!(
            error
                .to_string()
                .contains("Unable to read Future Agent run state"),
            "{error}"
        );

        // abort transport failure.
        mock.push_data("get_state", serde_json::json!({}));
        mock.push("abort", Reply::Status(tonic::Code::Internal, "boom"));
        let error = abort_session("sess-1").await.expect_err("abort transport");
        assert!(
            error
                .to_string()
                .contains("Unable to abort Future Agent session"),
            "{error}"
        );

        // abort rejected at app level.
        mock.push_data("get_state", serde_json::json!({}));
        mock.push("abort", Reply::Reject("not running".to_string()));
        let error = abort_session("sess-1").await.expect_err("abort reject");
        assert_eq!(error.to_string(), "not running");
    }

    #[tokio::test]
    async fn abort_agent_thread_uses_local_run_id_fallback_and_replica_mapping() {
        let home = TestHome::new("rc-abort-thread");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        // No replica mapping: the local run id doubles as the canonical id.
        mock.push("abort", Reply::Data("{}".to_string()));
        abort_agent_thread(&thread.id, Some("run-local"))
            .await
            .expect("abort");
        assert_eq!(mock.requests_of("abort")[0].run_id, "run-local");

        // A replica binding redirects the abort to the canonical run id.
        let lease = AGENT_REPLICAS
            .acquire("run-canonical-2")
            .expect("lease")
            .bind_local(Some("run-local-2"))
            .expect("bind");
        mock.push("abort", Reply::Data("{}".to_string()));
        abort_agent_thread(&thread.id, Some("run-local-2"))
            .await
            .expect("abort");
        assert_eq!(mock.requests_of("abort")[1].run_id, "run-canonical-2");
        drop(lease);

        // An empty local run id falls back to the canonical active-run probe.
        mock.push_data(
            "get_state",
            serde_json::json!({"activeRun": {"runId": "run-probed"}}),
        );
        mock.push("abort", Reply::Data("{}".to_string()));
        abort_agent_thread(&thread.id, Some(""))
            .await
            .expect("abort");
        assert_eq!(mock.requests_of("abort")[2].run_id, "run-probed");

        // Thread without a session id: the thread id doubles as the session.
        let no_session = seed_thread(&workspace.id, None);
        mock.push_data("get_state", serde_json::json!({}));
        mock.push("abort", Reply::Data("{}".to_string()));
        abort_agent_thread(&no_session.id, None)
            .await
            .expect("abort");
        assert_eq!(mock.requests_of("abort")[3].session_id, no_session.id);

        // Unknown thread.
        let error = abort_agent_thread("no-such-thread", None)
            .await
            .expect_err("missing thread");
        assert_eq!(error.to_string(), "Thread could not be loaded.");

        // Abort rejection propagates.
        mock.push("abort", Reply::Reject("nope".to_string()));
        let error = abort_agent_thread(&thread.id, Some("run-local"))
            .await
            .expect_err("abort reject");
        assert_eq!(error.to_string(), "nope");
    }

    #[tokio::test]
    async fn abort_run_cancels_locally_and_tolerates_a_down_agent() {
        let home = TestHome::new("rc-abort-run");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);

        // Healthy abort: the run is cancelled and returned.
        mock.push("abort", Reply::Data("{}".to_string()));
        let record = abort_run(thread.id.clone(), run.id.clone())
            .await
            .expect("abort run");
        assert_eq!(record.status, "cancelled");
        assert_eq!(record.error_type.as_deref(), Some("abort_requested"));

        // Agent down (Unavailable transport): still cancelled locally.
        let run2 = seed_run(&thread.id);
        mock.push(
            "abort",
            Reply::Status(tonic::Code::Unavailable, "agent dead"),
        );
        let record = abort_run(thread.id.clone(), run2.id.clone())
            .await
            .expect("abort tolerated");
        assert_eq!(record.status, "cancelled");

        // A non-Unavailable failure propagates and the run stays untouched.
        let run3 = seed_run(&thread.id);
        mock.push("abort", Reply::Status(tonic::Code::Internal, "boom"));
        let error = abort_run(thread.id.clone(), run3.id.clone())
            .await
            .expect_err("internal error propagates");
        assert!(error
            .to_string()
            .contains("Unable to abort Future Agent run"));
        assert_eq!(
            store::get_run(&run3.id).expect("run").expect("some").status,
            "running",
            "a failed abort leaves the run active"
        );
    }

    #[tokio::test]
    async fn abort_run_preserves_a_terminal_run_state() {
        let home = TestHome::new("rc-abort-terminal");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);
        store::update_run_status_if_active(store::UpdateRunStatusInput {
            run_id: run.id.clone(),
            status: "completed".to_string(),
            error_message: None,
            error_type: None,
        })
        .expect("complete");

        mock.push("abort", Reply::Data("{}".to_string()));
        let record = abort_run(thread.id.clone(), run.id.clone())
            .await
            .expect("abort returns current state");
        assert_eq!(
            record.status, "completed",
            "the CAS writer never rewrites a terminal state"
        );
    }

    #[test]
    fn mark_run_status_helpers_noop_on_none_and_report_store_errors() {
        let home = TestHome::new("rc-mark");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);

        mark_run_failed_if_active(None, "ignored");
        mark_run_completed_if_active(None);

        mark_run_failed_if_active(Some(&run.id), "boom");
        let record = store::get_run(&run.id).expect("run").expect("some");
        assert_eq!(record.status, "failed");
        assert_eq!(record.error_message.as_deref(), Some("boom"));

        let run2 = seed_run(&thread.id);
        mark_run_completed_if_active(Some(&run2.id));
        let record = store::get_run(&run2.id).expect("run").expect("some");
        assert_eq!(record.status, "completed");

        // Store failures are logged, never propagated.
        let prev = super::super::test_support::break_home();
        mark_run_failed_if_active(Some(&run.id), "boom again");
        mark_run_completed_if_active(Some(&run.id));
        super::super::test_support::restore_home(prev);
        assert_eq!(
            store::get_run(&run.id).expect("run").expect("some").status,
            "failed",
            "the broken-home writes changed nothing"
        );
    }

    #[tokio::test]
    async fn wait_for_agent_idle_returns_when_not_streaming() {
        let _home = TestHome::new("rc-idle");
        let mock = mock_agent();

        mock.push_data("get_state", serde_json::json!({"isStreaming": false}));
        wait_for_agent_idle("sess-1").await;
        assert_eq!(mock.requests_of("get_state").len(), 1);

        // Streaming twice, then idle: polls until the agent confirms quiet.
        mock.push_data("get_state", serde_json::json!({"isStreaming": true}));
        mock.push_data("get_state", serde_json::json!({"isStreaming": true}));
        mock.push_data("get_state", serde_json::json!({"isStreaming": false}));
        wait_for_agent_idle("sess-1").await;
        assert_eq!(mock.requests_of("get_state").len(), 4);

        // Transport failure mid-poll → treated as idle.
        mock.push("get_state", Reply::Status(tonic::Code::Unavailable, "gone"));
        wait_for_agent_idle("sess-1").await;
        assert_eq!(mock.requests_of("get_state").len(), 5);

        // Unparseable state payload → treated as not streaming.
        mock.push("get_state", Reply::Data("not json".to_string()));
        wait_for_agent_idle("sess-1").await;
        assert_eq!(mock.requests_of("get_state").len(), 6);
    }

    #[tokio::test]
    async fn wait_for_agent_idle_returns_immediately_when_agent_unreachable() {
        let _home = TestHome::new("rc-idle-down");
        let _mock = mock_agent();
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").ok();
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        wait_for_agent_idle("sess-1").await;
        if let Some(prev) = prev {
            std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        }
    }
}
