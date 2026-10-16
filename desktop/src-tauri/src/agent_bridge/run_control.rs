//! Run control: abort an in-flight agent run, mark a run failed, and wait for
//! the agent to confirm idle before snapshotting. These back the abort command
//! and the parent module's prompt finalization.

use super::client::{
    compact_command, connect_agent, get_state_command, is_session_not_found, map_rpc_error,
    run_control_command, RpcResponseExt,
};
use super::replica::AGENT_REPLICAS;
use crate::store;

/// What the Agent knows about a session when asked for its live run.
enum SessionProbe {
    /// The Agent has no such session. Definitive, not transient: the prompt
    /// never reached it or the session was deleted, so no session-scoped
    /// command can succeed for that id and there is nothing to abort. A local
    /// row still claiming this session is running is stale, and its owner has
    /// to stop treating it as live (see `conversations::delete_conversation`
    /// and the active-run watchdog).
    Unknown,
    /// The Agent resolved the session; its executing run id when one is live.
    ActiveRun(Option<String>),
}

async fn probe_session(
    client: &mut super::client::AgentClient,
    session_id: &str,
) -> Result<SessionProbe, crate::AppError> {
    let response = client
        .execute_command(get_state_command(session_id.to_string()))
        .await
        .map_err(|status| map_rpc_error("Unable to read Future Agent run state", status))?
        .into_inner();
    if !response.success {
        if is_session_not_found(&response) {
            return Ok(SessionProbe::Unknown);
        }
        // Surfaces the Agent's own message, like `ok_or_rpc_error`.
        return Err(if response.error.is_empty() {
            "Future Agent rejected the state request."
                .to_string()
                .into()
        } else {
            response.error.into()
        });
    }
    let state: serde_json::Value = future_rpc::decode::response_data(&response);
    Ok(SessionProbe::ActiveRun(
        state
            .get("activeRun")
            .and_then(|run| run.get("runId"))
            .and_then(|run_id| run_id.as_str())
            .filter(|run_id| !run_id.is_empty())
            .map(str::to_string),
    ))
}

/// Ask the Agent to abort `canonical_run_id` in `session_id`.
///
/// A session the Agent has no record of has nothing in flight, so the abort is
/// a no-op rather than a failure: the session can disappear between a probe and
/// this call, and a caller holding only a local run id never probed at all. The
/// caller still settles its own row, which is the point of aborting.
async fn abort_run_on_agent(
    client: &mut super::client::AgentClient,
    context: &str,
    session_id: &str,
    canonical_run_id: Option<String>,
) -> Result<(), crate::AppError> {
    let response = client
        .execute_command(run_control_command(
            "abort",
            session_id.to_string(),
            canonical_run_id,
        ))
        .await
        // Transport-level Unavailable → AgentUnavailable, so `abort_run`
        // still cancels the run locally when the agent died after the shared
        // channel was established.
        .map_err(|status| map_rpc_error(context, status))?
        .into_inner();
    if response.success || is_session_not_found(&response) {
        return Ok(());
    }
    Err(if response.error.is_empty() {
        "Future Agent rejected the abort request."
            .to_string()
            .into()
    } else {
        response.error.into()
    })
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
        None => match probe_session(&mut client, &session_id).await? {
            SessionProbe::Unknown => return Ok(()),
            SessionProbe::ActiveRun(run_id) => run_id,
        },
    };
    abort_run_on_agent(
        &mut client,
        "Unable to abort Future Agent run",
        &session_id,
        canonical_run_id,
    )
    .await
}

/// Abort an in-flight run for an already-resolved agent session id. Unlike
/// [`abort_agent_thread`] it takes the session id directly (the quit guard reads
/// it from `store::active_run_sessions`, so there is no thread to reload) and
/// does not touch the store — on force-quit we only need the agent to stop
/// streaming before the process exits; startup convergence settles the run rows
/// on the next launch. Best-effort at the call site: aborting a session that
/// already finished is a harmless no-op on the agent side.
///
/// The same goes for a session the Agent has no record of. This is called with
/// the session id a *local* run row resolves to, and such a row can outlive the
/// session it names (a prompt that never reached the Agent, or an agent-side
/// delete). Reporting "nothing to abort" for it lets the caller that only wants
/// the session stopped — deleting the conversation — go on with its work.
pub(crate) async fn abort_session(session_id: &str) -> Result<(), crate::AppError> {
    let mut client = connect_agent().await?;
    let canonical_run_id = match probe_session(&mut client, session_id).await? {
        SessionProbe::Unknown => return Ok(()),
        SessionProbe::ActiveRun(run_id) => run_id,
    };
    abort_run_on_agent(
        &mut client,
        "Unable to abort Future Agent session",
        session_id,
        canonical_run_id,
    )
    .await
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
pub(super) fn mark_run_completed_if_active(
    run_id: Option<&str>,
) -> Option<tokio::task::JoinHandle<()>> {
    let run_id = run_id?;
    match store::update_run_status_if_active(store::UpdateRunStatusInput {
        run_id: run_id.to_string(),
        status: "completed".to_string(),
        error_message: None,
        error_type: None,
    }) {
        // Only the CAS winner may generate a title. Replayed terminal events,
        // reattachment and concurrent completion paths must not trigger twice.
        Ok(true) => {
            if !store::get_app_settings().is_ok_and(|settings| settings.auto_title_first_turn) {
                return None;
            }
            let run_id = run_id.to_string();
            // A title request may take tens of seconds. Never hold up run
            // finalization, the observer's event stream or the next user send.
            Some(tokio::spawn(async move {
                if let Err(error) = title_settled_thread_if_enabled(&run_id).await {
                    eprintln!("FutureOS title generation skipped or failed: {error}");
                }
            }))
        }
        Ok(false) => None,
        Err(update_error) => {
            eprintln!("FutureOS run completion status update failed: {update_error}");
            None
        }
    }
}

/// Name a conversation once it has settled, if nobody has named it.
///
/// Eligibility is the thread's own state, not a run count: the thread must
/// still wear the title it was *minted* with (`title_seed`), and no turn may be
/// owed — neither a run row that has not settled (a queued follow-up, a goal
/// plane steering the worker the moment its previous reply landed) nor a
/// streaming Agent. Counting runs instead — the previous rule — made this task
/// a no-op for exactly those conversations: a steered worker's first reply is
/// followed by the next run immediately, so the "exactly one run" and "exactly
/// one user entry" gates were already false by the time the job ran, and the
/// conversation kept the placeholder title its creator gave it forever.
///
/// The gate therefore fires on the completion *after* the last queued turn, and
/// the title it saves changes the thread's title away from its seed, so it can
/// never fire twice for a conversation or overwrite a name someone chose.
async fn title_settled_thread_if_enabled(run_id: &str) -> Result<(), crate::AppError> {
    if !store::get_app_settings()?.auto_title_first_turn {
        return Ok(());
    }
    let Some(run) = store::get_run(run_id)? else {
        return Ok(());
    };
    let Some(thread) = store::get_thread(&run.thread_id)? else {
        return Ok(());
    };
    // Only a thread still wearing its minted title is unnamed: every other one
    // carries a name this task must not replace — the rename dialog, a TUI
    // `/name` the importer synced back, a phone rename, a previous generation,
    // or an imported/forked conversation that arrived with a name of its own
    // (`title_seed` is `None`).
    if thread.title_seed.as_deref() != Some(thread.title.as_str()) {
        return Ok(());
    }
    // A turn is already owed: the steering or follow-up prompt was queued
    // before this completion job got to run. Its own completion will retry.
    if store::has_active_run(&run.thread_id)? {
        return Ok(());
    }
    let Some(session_id) = thread.agent_session_id.as_deref() else {
        return Ok(());
    };
    // agent_end may arrive just before the Agent releases its streaming flag.
    if !wait_for_agent_idle(session_id).await {
        return Ok(());
    }
    // The idle poll is the only await in here: a prompt accepted during it is
    // streaming now and has its run row, so re-check that nothing is owed
    // before spending a model call on a conversation that is still working.
    if store::has_active_run(&run.thread_id)? {
        return Ok(());
    }
    let settings = store::get_app_settings()?;
    if !settings.auto_title_first_turn {
        return Ok(());
    }
    // Exactly the same tool-free title generator used by the rename dialog.
    // Neither generation nor saving appends messages or compacts the context.
    let suggestion =
        super::generate_session_title(session_id.to_string(), settings.title_language).await?;
    // The user may have renamed/deleted the thread, sent another turn or
    // disabled the setting while the model was working. Never apply stale work.
    let Some(current) = store::get_thread(&thread.id)? else {
        return Ok(());
    };
    if current.title != thread.title
        || current.title_seed.as_deref() != Some(current.title.as_str())
        || current.agent_session_id != thread.agent_session_id
        || store::has_active_run(&thread.id)?
        || !store::get_app_settings()?.auto_title_first_turn
    {
        return Ok(());
    }
    let title = suggestion["title"]
        .as_str()
        .ok_or("Model returned no title")?;
    super::rename_session(session_id.to_string(), title.to_string()).await?;
    crate::emit_threads_updated();
    Ok(())
}

/// Compact a thread's Agent context only on an explicit manual request.
pub async fn compact_thread_context(
    thread_id: String,
) -> Result<serde_json::Value, crate::AppError> {
    let thread = store::get_thread(&thread_id)?.ok_or_else(|| "Thread not found.".to_string())?;
    let session_id = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "This conversation has no context to compact.".to_string())?;
    compact_agent_session(session_id.to_string()).await
}

/// The `compact` RPC for a session the caller already owns — the shared path
/// behind the GUI command and the remote (phone) bridge, so both request the
/// same operation and validate the same acknowledgement. The Agent answers an
/// acceptance carrying the operation id; the checkpoint itself is reported
/// later on the compacting session's event stream.
pub async fn compact_agent_session(
    session_id: String,
) -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent()
        .await
        .map_err(|error| format!("Future Agent unreachable: {error}"))?;
    let response = client
        .execute_command(compact_command(session_id, String::new()))
        .await
        .map_err(|error| format!("compact RPC failed: {error}"))?
        .into_inner()
        .ok_or_rpc_error("compact returned an error")?;
    let value = future_rpc::decode::response_data(&response);
    let valid_ack = value.get("accepted").and_then(serde_json::Value::as_bool) == Some(true)
        && value
            .get("operationId")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|operation_id| !operation_id.is_empty());
    if !valid_ack {
        return Err("compact returned an invalid acknowledgement".into());
    }
    Ok(value)
}

/// Poll the Agent's `get_state.isStreaming` until it explicitly reports idle.
/// A timeout, transport failure, or malformed response is not confirmation and
/// therefore returns `false`.
///
/// A session the Agent has no record of *is* idle — nothing can be streaming for
/// it, and no later poll would answer differently — so that definitive answer
/// counts as confirmation rather than a timeout.
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
                if is_session_not_found(&response) {
                    return true;
                }
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
        get_state_payload, mock_agent, seed_run, seed_thread, seed_titleable_thread,
        seed_workspace, Reply, TestHome,
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

    /// A session the Agent has no record of has nothing to abort. Callers that
    /// only need the session stopped — deleting the conversation waits for
    /// exactly that — must not be told the stop failed, and no abort is worth
    /// sending for a session the Agent cannot resolve.
    #[tokio::test]
    async fn abort_session_is_a_no_op_for_a_session_the_agent_never_had() {
        let _home = TestHome::new("rc-abort-unknown-session");
        let mock = mock_agent();

        mock.push_state_for_session(
            "thread-1",
            Reply::RejectCode(
                future_rpc::session_errors::SESSION_NOT_FOUND_CODE,
                "session not found — pass a valid session_id (new_session creates one)".to_string(),
            ),
        );
        abort_session("thread-1").await.expect("nothing to abort");
        assert!(
            mock.requests_of("abort").is_empty(),
            "no abort is sent for a session that does not exist"
        );

        // An older agent reports it in the message only: same outcome.
        mock.push_state_for_session("thread-2", Reply::Reject("session not found".to_string()));
        abort_session("thread-2").await.expect("nothing to abort");
        assert!(mock.requests_of("abort").is_empty());

        // The agent can also lose the session between the probe and the abort.
        mock.push_state_for_session(
            "thread-3",
            Reply::TypedData(get_state_payload("thread-3", true)),
        );
        mock.push(
            "abort",
            Reply::RejectCode(
                future_rpc::session_errors::SESSION_NOT_FOUND_CODE,
                "session not found — pass a valid session_id (new_session creates one)".to_string(),
            ),
        );
        abort_session("thread-3").await.expect("gone mid-abort");
        assert_eq!(mock.requests_of("abort").len(), 1);
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

        // ...unless the rejection is the agent saying it has no such session. A
        // local row can outlive the session it names (the prompt never reached
        // the agent), and `abort_run` still has to cancel that row locally
        // instead of failing the user's stop.
        mock.push(
            "abort",
            Reply::RejectCode(
                future_rpc::session_errors::SESSION_NOT_FOUND_CODE,
                "session not found — pass a valid session_id (new_session creates one)".to_string(),
            ),
        );
        abort_agent_thread(&thread.id, Some("run-local"))
            .await
            .expect("nothing to abort");

        // A thread whose session the agent never had, probed with no local run
        // id: no abort is sent at all.
        let unbound = seed_thread(&workspace.id, None);
        mock.push_state_for_session(
            &unbound.id,
            Reply::RejectCode(
                future_rpc::session_errors::SESSION_NOT_FOUND_CODE,
                "session not found — pass a valid session_id (new_session creates one)".to_string(),
            ),
        );
        let aborts_before = mock.requests_of("abort").len();
        abort_agent_thread(&unbound.id, None)
            .await
            .expect("nothing to abort");
        assert_eq!(mock.requests_of("abort").len(), aborts_before);
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
        store::update_app_settings(store::UpdateAppSettingsInput {
            auto_title_first_turn: Some(false),
            ..Default::default()
        })
        .expect("disable background titles for status-only test");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);

        mark_run_failed_if_active(None, "ignored");
        assert!(mark_run_completed_if_active(None).is_none());

        mark_run_failed_if_active(Some(&run.id), "boom");
        let record = store::get_run(&run.id).expect("run").expect("some");
        assert_eq!(record.status, "failed");
        assert_eq!(record.error_message.as_deref(), Some("boom"));

        let run2 = seed_run(&thread.id);
        assert!(mark_run_completed_if_active(Some(&run2.id)).is_none());
        let record = store::get_run(&run2.id).expect("run").expect("some");
        assert_eq!(record.status, "completed");

        // Store failures are logged, never propagated.
        let prev = super::super::test_support::break_home();
        mark_run_failed_if_active(Some(&run.id), "boom again");
        assert!(mark_run_completed_if_active(Some(&run.id)).is_none());
        super::super::test_support::restore_home(prev);
        assert_eq!(
            store::get_run(&run.id).expect("run").expect("some").status,
            "failed",
            "the broken-home writes changed nothing"
        );
    }

    async fn complete_and_wait(run_id: &str) {
        if let Some(task) = mark_run_completed_if_active(Some(run_id)) {
            task.await.expect("title task");
        }
    }

    fn enable_title_generation(language: &str) {
        store::update_app_settings(store::UpdateAppSettingsInput {
            auto_title_first_turn: Some(true),
            title_language: Some(language.into()),
            ..Default::default()
        })
        .expect("enable titles");
    }

    #[tokio::test]
    async fn auto_title_names_a_conversation_once_the_last_queued_turn_settles() {
        let home = TestHome::new("rc-steered-settle-title");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        enable_title_generation("zh");
        // A worker conversation: the goal plane named it after the goal and
        // steered it again before the first reply landed, so a second run row
        // already exists — `isStreaming` never drops between the two turns.
        let thread = seed_titleable_thread(&workspace.id, "sess-worker");
        let steered = seed_run(&thread.id);
        let queued = seed_run(&thread.id);

        complete_and_wait(&steered.id).await;
        assert!(
            mock.requests_of("generate_session_title").is_empty(),
            "a conversation that still owes a turn is not named"
        );
        assert!(
            mock.requests_of("get_state").is_empty(),
            "the queued run is visible locally, so the Agent is not even probed"
        );

        // The steering turn finishes and nothing is owed: name it now, not
        // never (what the run-count gate did for every steered conversation).
        mock.push_state_for_session(
            "sess-worker",
            Reply::TypedData(get_state_payload("sess-worker", false)),
        );
        mock.push_data(
            "generate_session_title",
            serde_json::json!({"title":"覆盖率 worker 收尾", "model":"test"}),
        );
        mock.push_data("set_session_name", serde_json::json!({}));
        complete_and_wait(&queued.id).await;
        assert_eq!(
            store::get_thread(&thread.id).unwrap().unwrap().title,
            "覆盖率 worker 收尾"
        );

        // Named once: a later turn cannot regenerate it.
        let later = seed_run(&thread.id);
        complete_and_wait(&later.id).await;
        assert_eq!(mock.requests_of("generate_session_title").len(), 1);
        assert_eq!(mock.requests_of("set_session_name").len(), 1);
    }

    #[tokio::test]
    async fn auto_title_never_replaces_a_name_someone_chose() {
        let home = TestHome::new("rc-renamed-not-titled");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        enable_title_generation("zh");
        let thread = seed_titleable_thread(&workspace.id, "sess-named");
        store::rename_thread(store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "人工命名".into(),
        })
        .expect("rename");
        // A completed first turn leaves a one-exchange history and an idle
        // Agent behind. The superseded rule needed both to call this
        // conversation a new one — and with them scripted it renamed a name
        // someone chose, which is the failure this test pins. The title alone
        // decides now, so neither is asked for.
        mock.push_typed_data("get_session_entries", first_pair());
        mock.push_state_for_session(
            "sess-named",
            Reply::TypedData(get_state_payload("sess-named", false)),
        );

        let run = seed_run(&thread.id);
        complete_and_wait(&run.id).await;
        assert!(
            mock.requests_of("generate_session_title").is_empty(),
            "a rename is permanent: the name must not be generated away"
        );
        assert!(
            mock.requests_of("get_state").is_empty(),
            "an already-false gate must not even probe the Agent"
        );
        assert_eq!(
            store::get_thread(&thread.id).unwrap().unwrap().title,
            "人工命名"
        );
    }

    fn first_pair() -> serde_json::Value {
        serde_json::json!({"entries": [
            {"id":"u1", "kind":"user", "role":"user", "createdAtMs":1000, "blocks":[]},
            {"id":"a1", "kind":"assistant", "role":"assistant", "createdAtMs":1001, "blocks":[]}
        ]})
    }

    #[tokio::test]
    async fn first_turn_generates_and_saves_title_once_without_compacting() {
        let home = TestHome::new("rc-first-turn-title");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        for language in ["zh", "en"] {
            let thread = seed_titleable_thread(&workspace.id, language);
            let run = seed_run(&thread.id);
            let settings = store::update_app_settings(store::UpdateAppSettingsInput {
                title_language: Some(language.into()),
                ..Default::default()
            })
            .expect("set title language");
            assert!(
                settings.auto_title_first_turn,
                "title generation is enabled by default"
            );
            mock.push_state_for_session(
                language,
                Reply::TypedData(get_state_payload(language, false)),
            );
            mock.push_data(
                "generate_session_title",
                serde_json::json!({"title":"Generated title", "model":"test"}),
            );
            mock.push_data("set_session_name", serde_json::json!({}));

            complete_and_wait(&run.id).await;
            let request = mock.requests_of("generate_session_title").pop().unwrap();
            assert_eq!(request.session_id, language);
            assert_eq!(request.mode, language);
            let rename = mock.requests_of("set_session_name").pop().unwrap();
            assert_eq!(rename.session_id, language);
            assert_eq!(rename.name, "Generated title");
            assert_eq!(
                store::get_thread(&thread.id).unwrap().unwrap().title,
                "Generated title"
            );
            // Replayed completion/reattachment and later turns cannot generate again.
            complete_and_wait(&run.id).await;
            let second = seed_run(&thread.id);
            complete_and_wait(&second.id).await;
        }
        assert_eq!(mock.requests_of("generate_session_title").len(), 2);
        assert_eq!(mock.requests_of("set_session_name").len(), 2);
        assert!(mock.requests_of("compact").is_empty());
        assert!(mock.requests_of("prompt").is_empty());
    }

    #[tokio::test]
    async fn a_disabled_setting_leaves_a_conversation_unnamed_until_the_next_turn_settles() {
        let home = TestHome::new("rc-first-turn-opt-out");
        store::update_app_settings(store::UpdateAppSettingsInput {
            auto_title_first_turn: Some(false),
            ..Default::default()
        })
        .expect("explicitly disable titles");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_titleable_thread(&workspace.id, "sess-1");
        let first = seed_run(&thread.id);
        complete_and_wait(&first.id).await;
        assert!(mock.requests_of("get_state").is_empty());
        enable_title_generation("en");
        // Replaying the settled run generates nothing, and turning the setting
        // on never names a conversation by itself.
        complete_and_wait(&first.id).await;
        assert!(mock.requests_of("generate_session_title").is_empty());
        assert!(mock.requests_of("get_state").is_empty());
        // The next turn that settles does name it.
        mock.push_state_for_session(
            "sess-1",
            Reply::TypedData(get_state_payload("sess-1", false)),
        );
        mock.push_data(
            "generate_session_title",
            serde_json::json!({"title":"Named later", "model":"test"}),
        );
        mock.push_data("set_session_name", serde_json::json!({}));
        let second = seed_run(&thread.id);
        complete_and_wait(&second.id).await;
        assert_eq!(
            store::get_thread(&thread.id).unwrap().unwrap().title,
            "Named later"
        );
        assert!(mock.requests_of("compact").is_empty());
    }

    #[tokio::test]
    async fn an_unsuccessful_turn_is_not_named_but_the_next_settled_turn_is() {
        let home = TestHome::new("rc-first-turn-unsuccessful");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        enable_title_generation("en");
        for (named, status) in ["failed", "cancelled"].into_iter().enumerate() {
            let thread = seed_titleable_thread(&workspace.id, status);
            let first = seed_run(&thread.id);
            store::update_run_status_if_active(store::UpdateRunStatusInput {
                run_id: first.id.clone(),
                status: status.into(),
                error_message: None,
                error_type: None,
            })
            .expect("settle unsuccessfully");
            complete_and_wait(&first.id).await;
            assert_eq!(
                mock.requests_of("generate_session_title").len(),
                named,
                "a turn that failed cannot name the conversation"
            );
            mock.push_state_for_session(status, Reply::TypedData(get_state_payload(status, false)));
            mock.push_data(
                "generate_session_title",
                serde_json::json!({"title":"Named on success", "model":"test"}),
            );
            mock.push_data("set_session_name", serde_json::json!({}));
            let second = seed_run(&thread.id);
            complete_and_wait(&second.id).await;
            assert_eq!(store::get_run(&first.id).unwrap().unwrap().status, status);
        }
        assert_eq!(mock.requests_of("generate_session_title").len(), 2);
        assert!(mock.requests_of("compact").is_empty());
    }

    #[tokio::test]
    async fn a_conversation_that_arrived_with_a_name_is_not_titled() {
        let home = TestHome::new("rc-imported-name");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        enable_title_generation("en");
        // An imported conversation (or a fork): the Desktop did not mint its
        // title, so a generated one could undo a name chosen where the
        // conversation came from.
        let thread = seed_thread(&workspace.id, Some("imported-session"));
        assert!(
            thread.title_seed.is_none(),
            "a thread created around an existing session carries no seed"
        );
        let run = seed_run(&thread.id);
        complete_and_wait(&run.id).await;
        assert!(mock.requests_of("generate_session_title").is_empty());
        assert!(mock.requests_of("get_state").is_empty());
        assert_eq!(
            store::get_thread(&thread.id).unwrap().unwrap().title,
            thread.title
        );
    }

    #[tokio::test]
    async fn title_failure_leaves_answer_and_title_unchanged_without_retry() {
        let home = TestHome::new("rc-first-turn-title-failure");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        enable_title_generation("en");
        for failure in ["generate", "empty", "save"] {
            let thread = seed_titleable_thread(&workspace.id, failure);
            let run = seed_run(&thread.id);
            mock.push_state_for_session(
                failure,
                Reply::TypedData(get_state_payload(failure, false)),
            );
            match failure {
                "generate" => mock.push(
                    "generate_session_title",
                    Reply::Reject("title failed".into()),
                ),
                "empty" => {
                    mock.push_data("generate_session_title", serde_json::json!({"title":" "}))
                }
                _ => {
                    mock.push_data(
                        "generate_session_title",
                        serde_json::json!({"title":"Suggested title"}),
                    );
                    mock.push("set_session_name", Reply::Reject("rename failed".into()));
                }
            }
            complete_and_wait(&run.id).await;
            complete_and_wait(&run.id).await;
            assert_eq!(
                store::get_run(&run.id).unwrap().unwrap().status,
                "completed"
            );
            assert_eq!(
                store::get_thread(&thread.id).unwrap().unwrap().title,
                thread.title
            );
        }
        assert_eq!(mock.requests_of("generate_session_title").len(), 3);
        assert_eq!(mock.requests_of("set_session_name").len(), 1);
        assert!(mock.requests_of("compact").is_empty());
    }

    #[tokio::test]
    async fn background_title_does_not_block_completion_or_overwrite_newer_work() {
        let home = TestHome::new("rc-first-turn-title-race");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        for change in ["rename", "disable", "next-turn"] {
            enable_title_generation("en");
            let thread = seed_titleable_thread(&workspace.id, change);
            let run = seed_run(&thread.id);
            mock.push_state_for_session(change, Reply::TypedData(get_state_payload(change, false)));
            let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            mock.push(
                "generate_session_title",
                Reply::Deferred {
                    entered: entered_tx,
                    data: reply_rx,
                },
            );
            let task = mark_run_completed_if_active(Some(&run.id)).expect("background task");
            tokio::time::timeout(std::time::Duration::from_secs(5), entered_rx)
                .await
                .unwrap()
                .unwrap();
            assert!(!task.is_finished());
            assert_eq!(
                store::get_run(&run.id).unwrap().unwrap().status,
                "completed"
            );
            let expected = match change {
                "rename" => {
                    store::rename_thread(store::RenameThreadInput {
                        thread_id: thread.id.clone(),
                        title: "User title".into(),
                    })
                    .unwrap();
                    "User title".to_string()
                }
                "disable" => {
                    store::update_app_settings(store::UpdateAppSettingsInput {
                        auto_title_first_turn: Some(false),
                        ..Default::default()
                    })
                    .unwrap();
                    thread.title.clone()
                }
                _ => {
                    seed_run(&thread.id);
                    thread.title.clone()
                }
            };
            reply_tx
                .send(serde_json::json!({"title":"Stale title"}))
                .unwrap();
            task.await.unwrap();
            assert_eq!(
                store::get_thread(&thread.id).unwrap().unwrap().title,
                expected
            );
        }
        assert!(mock.requests_of("set_session_name").is_empty());
        assert!(mock.requests_of("compact").is_empty());
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

        // A session the Agent has no record of cannot be streaming, and no
        // later poll would answer differently: that is confirmation, not a
        // timeout. The local row naming it is stale, and its owner has to stop
        // waiting on a session that does not exist.
        mock.push_state_for_session(
            "sess-gone",
            Reply::RejectCode(
                future_rpc::session_errors::SESSION_NOT_FOUND_CODE,
                "session not found — pass a valid session_id (new_session creates one)".to_string(),
            ),
        );
        assert!(wait_for_agent_idle("sess-gone").await);
        assert_eq!(mock.requests_of("get_state").len(), 7, "one poll is enough");
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
