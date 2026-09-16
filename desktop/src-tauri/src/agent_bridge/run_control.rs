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
pub(super) async fn mark_run_completed_if_active(run_id: Option<&str>) {
    let Some(run_id) = run_id else {
        return;
    };
    match store::update_run_status_if_active(store::UpdateRunStatusInput {
        run_id: run_id.to_string(),
        status: "completed".to_string(),
        error_message: None,
        error_type: None,
    }) {
        // Only the CAS winner may request compaction. Replayed terminal events,
        // reattachment and concurrent completion paths must not trigger twice.
        Ok(true) => {
            if let Err(error) = compact_first_turn_if_enabled(run_id).await {
                // A summary failure must never turn a successful answer into a
                // failed run. The existing compaction lifecycle reports errors.
                eprintln!("FutureOS first-turn compaction skipped or failed: {error}");
            }
        }
        Ok(false) => {}
        Err(update_error) => {
            eprintln!("FutureOS run completion status update failed: {update_error}");
        }
    }
}

async fn compact_first_turn_if_enabled(run_id: &str) -> Result<(), crate::AppError> {
    if !store::get_app_settings()?.auto_compact_first_turn {
        return Ok(());
    }
    let Some(run) = store::get_run(run_id)? else {
        return Ok(());
    };
    // Include failed, cancelled and archived runs: this is the first turn,
    // not the first successful turn or the first currently visible answer.
    if store::list_runs(&run.thread_id)?.len() != 1 {
        return Ok(());
    }
    let Some(thread) = store::get_thread(&run.thread_id)? else {
        return Ok(());
    };
    let Some(session_id) = thread.agent_session_id.as_deref() else {
        return Ok(());
    };
    // agent_end may arrive just before the Agent releases its streaming flag.
    if !wait_for_agent_idle(session_id).await {
        return Ok(());
    }
    let history = super::get_session_entries(session_id.to_string()).await?;
    let Some(entries) = history.get("entries").and_then(serde_json::Value::as_array) else {
        return Ok(());
    };
    // A locally new run can belong to an existing/imported Agent session.
    // Check canonical history as well, rather than summarizing an old chat.
    if entries
        .iter()
        .filter(|entry| entry["role"] == "user")
        .count()
        != 1
    {
        return Ok(());
    }
    // A new send or a settings change may have landed during the idle/history
    // awaits. Never defer this work into a later turn or silently retry it.
    if store::list_runs(&run.thread_id)?.len() == 1
        && store::get_app_settings()?.auto_compact_first_turn
    {
        crate::commands::compact_thread_context(run.thread_id).await?;
    }
    Ok(())
}

/// Poll the Agent's `get_state.isStreaming` until it explicitly reports idle.
/// A timeout, transport failure, or malformed response is not confirmation and
/// therefore returns `false`.
pub(crate) async fn wait_for_agent_idle(session_id: &str) -> bool {
    let Ok(mut client) = connect_agent().await else {
        return false;
    };
    // ~5s budget at 200ms intervals.
    for _ in 0..25 {
        match client
            .execute_command(get_state_command(session_id.to_string()))
            .await
        {
            Ok(response) => {
                let response = response.into_inner();
                if let Some(state) = future_rpc::decode::decode_get_state(&response) {
                    if !state.is_streaming {
                        return true;
                    }
                } else {
                    return false;
                }
            }
            Err(_) => return false,
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    false
}

fn is_agent_unavailable_error(error: &crate::AppError) -> bool {
    matches!(error, crate::AppError::AgentUnavailable(_))
}

#[cfg(test)]
mod tests {
    use super::super::replica::AGENT_REPLICAS;
    use super::super::test_support::{
        get_state_payload, mock_agent, seed_run, seed_thread, seed_workspace, Reply, TestHome,
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

        mock.push_state_for_session(
            "sess-1",
            Reply::TypedData(get_state_payload("sess-1", false)),
        );
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

    #[tokio::test]
    async fn mark_run_status_helpers_noop_on_none_and_report_store_errors() {
        let home = TestHome::new("rc-mark");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);

        mark_run_failed_if_active(None, "ignored");
        mark_run_completed_if_active(None).await;

        mark_run_failed_if_active(Some(&run.id), "boom");
        let record = store::get_run(&run.id).expect("run").expect("some");
        assert_eq!(record.status, "failed");
        assert_eq!(record.error_message.as_deref(), Some("boom"));

        let run2 = seed_run(&thread.id);
        mark_run_completed_if_active(Some(&run2.id)).await;
        let record = store::get_run(&run2.id).expect("run").expect("some");
        assert_eq!(record.status, "completed");

        // Store failures are logged, never propagated.
        let prev = super::super::test_support::break_home();
        mark_run_failed_if_active(Some(&run.id), "boom again");
        mark_run_completed_if_active(Some(&run.id)).await;
        super::super::test_support::restore_home(prev);
        assert_eq!(
            store::get_run(&run.id).expect("run").expect("some").status,
            "failed",
            "the broken-home writes changed nothing"
        );
    }

    #[tokio::test]
    async fn first_turn_compacts_once_and_never_on_later_turns() {
        let home = TestHome::new("rc-first-turn");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("first-turn-session"));
        let run = seed_run(&thread.id);
        store::update_app_settings(store::UpdateAppSettingsInput {
            auto_compact_first_turn: Some(true),
            ..Default::default()
        })
        .expect("enable");
        mock.push_state_for_session(
            "first-turn-session",
            Reply::TypedData(get_state_payload("first-turn-session", false)),
        );
        mock.push_typed_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"u1", "kind":"user", "role":"user", "createdAtMs":1000, "blocks":[]},
                {"id":"a1", "kind":"assistant", "role":"assistant", "createdAtMs":1001, "blocks":[]}
            ]}),
        );
        mock.push_data(
            "compact",
            serde_json::json!({"accepted":true,"operationId":"cmp-1"}),
        );

        mark_run_completed_if_active(Some(&run.id)).await;
        assert_eq!(mock.requests_of("compact").len(), 1);
        assert_eq!(
            mock.requests_of("compact")[0].session_id,
            "first-turn-session"
        );
        // Replayed completion/reattachment cannot request a second summary.
        mark_run_completed_if_active(Some(&run.id)).await;
        let second = seed_run(&thread.id);
        mark_run_completed_if_active(Some(&second.id)).await;
        assert_eq!(mock.requests_of("compact").len(), 1);
        assert_eq!(mock.requests_of("get_session_entries").len(), 1);
    }

    #[tokio::test]
    async fn first_turn_setting_is_opt_in_and_does_not_backfill_old_chats() {
        let home = TestHome::new("rc-first-turn-default");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let first = seed_run(&thread.id);
        mark_run_completed_if_active(Some(&first.id)).await;
        assert!(mock.requests_of("get_state").is_empty());
        store::update_app_settings(store::UpdateAppSettingsInput {
            auto_compact_first_turn: Some(true),
            ..Default::default()
        })
        .expect("enable after first turn");
        mark_run_completed_if_active(Some(&first.id)).await;
        let second = seed_run(&thread.id);
        mark_run_completed_if_active(Some(&second.id)).await;
        assert!(mock.requests_of("compact").is_empty());
        assert!(mock.requests_of("get_state").is_empty());
    }

    #[tokio::test]
    async fn unsuccessful_first_turns_do_not_compact_or_defer_to_next_success() {
        let home = TestHome::new("rc-first-turn-unsuccessful");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        store::update_app_settings(store::UpdateAppSettingsInput {
            auto_compact_first_turn: Some(true),
            ..Default::default()
        })
        .expect("enable");
        for status in ["failed", "cancelled"] {
            let thread = seed_thread(&workspace.id, Some(status));
            let first = seed_run(&thread.id);
            store::update_run_status_if_active(store::UpdateRunStatusInput {
                run_id: first.id.clone(),
                status: status.to_string(),
                error_message: None,
                error_type: None,
            })
            .expect("settle unsuccessfully");
            mark_run_completed_if_active(Some(&first.id)).await;
            let second = seed_run(&thread.id);
            mark_run_completed_if_active(Some(&second.id)).await;
            assert_eq!(store::get_run(&first.id).unwrap().unwrap().status, status);
        }
        assert!(mock.requests_of("compact").is_empty());
        assert!(mock.requests_of("get_state").is_empty());
    }

    #[tokio::test]
    async fn existing_agent_history_is_not_a_new_conversation() {
        let home = TestHome::new("rc-first-turn-old-history");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("old-session"));
        let run = seed_run(&thread.id);
        store::update_app_settings(store::UpdateAppSettingsInput {
            auto_compact_first_turn: Some(true),
            ..Default::default()
        })
        .expect("enable");
        mock.push_state_for_session(
            "old-session",
            Reply::TypedData(get_state_payload("old-session", false)),
        );
        mock.push_typed_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"u1", "kind":"user", "role":"user", "createdAtMs":1000, "blocks":[]},
                {"id":"u2", "kind":"user", "role":"user", "createdAtMs":1002, "blocks":[]}
            ]}),
        );
        mark_run_completed_if_active(Some(&run.id)).await;
        assert!(mock.requests_of("compact").is_empty());
    }

    #[tokio::test]
    async fn summary_rejection_does_not_fail_the_answer_or_retry() {
        let home = TestHome::new("rc-first-turn-rejected");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);
        store::update_app_settings(store::UpdateAppSettingsInput {
            auto_compact_first_turn: Some(true),
            ..Default::default()
        })
        .expect("enable");
        mock.push_state_for_session(
            "sess-1",
            Reply::TypedData(get_state_payload("sess-1", false)),
        );
        mock.push_typed_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"u1", "kind":"user", "role":"user", "createdAtMs":1000, "blocks":[]}
            ]}),
        );
        mock.push("compact", Reply::Reject("session busy".to_string()));
        mark_run_completed_if_active(Some(&run.id)).await;
        mark_run_completed_if_active(Some(&run.id)).await;
        assert_eq!(
            store::get_run(&run.id).unwrap().unwrap().status,
            "completed"
        );
        assert_eq!(mock.requests_of("compact").len(), 1);
    }

    #[tokio::test]
    async fn wait_for_agent_idle_returns_when_not_streaming() {
        let _home = TestHome::new("rc-idle");
        let mock = mock_agent();

        mock.push_state_for_session(
            "sess-1",
            Reply::TypedData(get_state_payload("sess-1", false)),
        );
        assert!(wait_for_agent_idle("sess-1").await);
        assert_eq!(mock.requests_of("get_state").len(), 1);

        // Streaming twice, then idle: polls until the agent confirms quiet.
        mock.push_state_for_session(
            "sess-1",
            Reply::TypedData(get_state_payload("sess-1", true)),
        );
        mock.push_state_for_session(
            "sess-1",
            Reply::TypedData(get_state_payload("sess-1", true)),
        );
        mock.push_state_for_session(
            "sess-1",
            Reply::TypedData(get_state_payload("sess-1", false)),
        );
        assert!(wait_for_agent_idle("sess-1").await);
        assert_eq!(mock.requests_of("get_state").len(), 4);

        // Transport failure cannot confirm that the Agent is idle.
        mock.push_state_for_session("sess-1", Reply::Status(tonic::Code::Unavailable, "gone"));
        assert!(!wait_for_agent_idle("sess-1").await);
        assert_eq!(mock.requests_of("get_state").len(), 5);

        // An unparseable state payload also cannot confirm idle.
        mock.push_state_for_session("sess-1", Reply::Data("not json".to_string()));
        assert!(!wait_for_agent_idle("sess-1").await);
        assert_eq!(mock.requests_of("get_state").len(), 6);
    }

    #[tokio::test]
    async fn wait_for_agent_idle_returns_immediately_when_agent_unreachable() {
        let _home = TestHome::new("rc-idle-down");
        let _mock = mock_agent();
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").ok();
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        assert!(!wait_for_agent_idle("sess-1").await);
        if let Some(prev) = prev {
            std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        }
    }
}
