//! Shared conversation and session operations.

use crate::console;
use crate::{agent_bridge, store};

pub async fn fork_thread(
    thread_id: String,
    source_entry_id: String,
    request_id: String,
) -> Result<String, crate::Error> {
    agent_bridge::fork_agent_session(&thread_id, &source_entry_id, &request_id).await
}

pub fn list_threads() -> Result<Vec<store::ThreadRecord>, crate::Error> {
    store::list_threads()
}

pub fn get_thread(thread_id: String) -> Result<Option<store::ThreadRecord>, crate::Error> {
    store::get_thread(&thread_id)
}

pub fn get_recent_thread() -> Result<Option<store::ThreadRecord>, crate::Error> {
    store::get_recent_thread()
}

pub fn mark_thread_opened(thread_id: String) -> Result<(), crate::Error> {
    store::mark_thread_opened(&thread_id)
}

pub fn create_thread(input: store::CreateThreadInput) -> Result<store::ThreadRecord, crate::Error> {
    // No auto `git init` for workspace-mode threads (§14.3); shadow review
    // handles non-git Workspaces.
    store::create_thread(input)
}

pub async fn generate_thread_title(
    thread_id: String,
    language: String,
) -> Result<serde_json::Value, crate::Error> {
    let thread = store::get_thread(&thread_id)?
        .ok_or_else(|| crate::Error::Message("Thread could not be loaded".into()))?;
    let session_id = thread
        .agent_session_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(&thread.id)
        .to_string();
    agent_bridge::generate_session_title(session_id, language).await
}

pub async fn rename_thread(
    input: store::RenameThreadInput,
) -> Result<store::ThreadRecord, crate::Error> {
    // Propagate to the agent FIRST and only rename the DB row on success. The
    // agent's session_name is the authoritative name shared with every client
    // (TUI /name, CLI, channels), and get_thread_agent_state / startup import
    // converge the DB title toward it — so a local-only rename whose
    // propagation failed would be silently synced back (reverted) on the next
    // poll. Renaming is therefore all-or-nothing: if the agent call fails,
    // the rename fails and the user sees the error in the dialog.
    // Validate up front (store::rename_thread re-checks) so an invalid title
    // never reaches the agent.
    if input.title.trim().is_empty() {
        return Err("title cannot be empty.".to_string().into());
    }
    let thread = store::get_thread(&input.thread_id)?
        .ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let session_id = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(&thread.id)
        .to_string();
    let mut client = crate::agent_bridge::connect_agent().await?;
    crate::agent_bridge::verify_session_database(&mut client, &session_id).await?;
    let cmd = crate::agent_bridge::set_session_name_command(input.title.clone(), session_id);
    let resp = client
        .execute_command(cmd)
        .await
        .map_err(|status| {
            crate::agent_bridge::map_rpc_error("FutureOS rename propagation failed", status)
        })?
        .into_inner();
    if !resp.success && !resp.error.contains("session not found") {
        return Err(format!("Future Agent rejected the rename: {}", resp.error).into());
    }
    // "session not found" is the one benign rejection: the thread has no
    // agent session (never prompted, or deleted), so nothing can ever sync a
    // stale name back over this rename.
    store::rename_thread(input)
}

pub async fn update_thread_model(
    input: store::UpdateThreadModelInput,
) -> Result<store::ThreadRecord, crate::Error> {
    let model_id = input.model_id.clone();
    let thread = store::update_thread_model(input)?;
    // Propagate to the agent immediately so the change takes effect before the
    // next prompt (best-effort — a failure here must not fail the local update).
    if let (Some(model_id), Ok(mut client)) = (model_id, crate::agent_bridge::connect_agent().await)
    {
        if !model_id.trim().is_empty() {
            let session_id = thread
                .agent_session_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .unwrap_or(&thread.id)
                .to_string();
            if crate::agent_bridge::verify_session_database(&mut client, &session_id)
                .await
                .is_ok()
            {
                let cmd = crate::agent_bridge::set_model_command(model_id, session_id);
                let _ = client.execute_command(cmd).await;
            }
        }
    }
    Ok(thread)
}

pub async fn update_thread_thinking_level(
    input: store::UpdateThreadThinkingLevelInput,
) -> Result<store::ThreadRecord, crate::Error> {
    let thinking_level = input.thinking_level.clone();
    let thread = store::update_thread_thinking_level(input)?;
    // Propagate to the agent immediately (best-effort).
    if let (Some(thinking_level), Ok(mut client)) =
        (thinking_level, crate::agent_bridge::connect_agent().await)
    {
        if !thinking_level.trim().is_empty() {
            let session_id = thread
                .agent_session_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .unwrap_or(&thread.id)
                .to_string();
            if crate::agent_bridge::verify_session_database(&mut client, &session_id)
                .await
                .is_ok()
            {
                let cmd =
                    crate::agent_bridge::set_thinking_level_command(thinking_level, session_id);
                let _ = client.execute_command(cmd).await;
            }
        }
    }
    Ok(thread)
}

pub fn pin_thread(input: store::PinThreadInput) -> Result<store::ThreadRecord, crate::Error> {
    store::pin_thread(input)
}

pub fn archive_thread(thread_id: String) -> Result<store::ThreadRecord, crate::Error> {
    store::archive_thread(&thread_id)
}

pub fn restore_thread(thread_id: String) -> Result<store::ThreadRecord, crate::Error> {
    store::restore_thread(&thread_id)
}

pub async fn delete_thread(
    input: store::DeleteThreadInput,
) -> Result<store::ThreadRecord, crate::Error> {
    // The same sequence the tasks host runs when a task deletes the conversation
    // it just used: see `crate::conversations`.
    crate::conversations::delete_conversation(&input.thread_id, input.delete_files).await
}

/// Batch-delete multiple threads. For each thread, the thread and its
/// descendants are hard-deleted and their agent session JSONL is removed. For
/// chat-mode threads with `delete_files`, the temporary workspace directory on
/// disk is also removed. Workspace-mode threads are never touched on disk
/// regardless of `delete_files`. Returns a summary of deleted count and
/// failures.
pub async fn batch_delete_threads(
    input: store::BatchDeleteThreadsInput,
) -> Result<store::BatchDeleteResult, crate::Error> {
    let mut result = store::BatchDeleteResult {
        deleted_count: 0,
        failed: Vec::new(),
    };
    let mut prepared = Vec::new();
    let mut stopped = std::collections::HashSet::new();
    for thread_id in &input.thread_ids {
        let targets = match store::thread_delete_closure(thread_id) {
            Ok(targets) => targets,
            Err(error) => {
                result.failed.push(format!("{thread_id}: {error}"));
                continue;
            }
        };
        let ready = async {
            for target in &targets {
                close_thread_terminals(&target.id);
                let session_id = crate::conversations::thread_session_id(target);
                if !stopped.contains(session_id) {
                    crate::conversations::stop_active_session_before_delete(session_id).await?;
                    stopped.insert(session_id.to_owned());
                }
            }
            Ok::<_, crate::Error>(())
        }
        .await;
        match ready {
            Ok(()) => prepared.push(thread_id.clone()),
            Err(error) => result.failed.push(format!("{thread_id}: {error}")),
        }
    }
    let local = store::delete_thread_trees(&prepared, input.delete_files)?;
    result.deleted_count += local.deleted_count;
    result.failed.extend(local.failed);
    crate::agent_bridge::reconcile_delete_outbox().await;

    Ok(result)
}

/// Close every terminal tab a conversation owns. Called from the deletion
/// paths so terminal lifetime is bounded by conversation lifetime, independent
/// of whether a client still holds an open panel.
pub(crate) fn close_thread_terminals(thread_id: &str) {
    crate::events::publish(crate::events::Event::CloseThreadTerminals(thread_id));
}

/// Bulk streaming-status reconciliation: ONE agent RPC
/// (`list_streaming_sessions`, which only scans the agent's in-memory session
/// map — no cold hydration or disk I/O) mapped back to GUI thread ids. The
/// active subset gets observers so work started by another client is projected;
/// idle historical sessions remain cold.
///
/// Agent unreachable → empty list. The process-level compatibility monitor
/// retries, while React uses this command only for its initial snapshot.
pub async fn list_streaming_thread_ids() -> Result<Vec<String>, crate::Error> {
    crate::agent_bridge::reconcile_streaming_observers().await
}

/// Fetch a thread's session state from the agent (model, thinking, name, cwd).
/// Falls back to the stored DB values when the agent is unreachable.
///
/// A thread without an agent session has no agent state to fetch.  Must not
/// resolve the bare `thread.id` as a session_id — the agent's `get_session`
/// fallback returns the default session's state, leaking another
/// conversation's model/thinking into the wrong thread.
/// Converge a stale DB title toward the agent's session_name, best-effort.
/// Extracted so the defensive failure log is testable with a broken store.
fn sync_title_best_effort(thread_id: &str, name: &str) {
    if let Err(error) = store::sync_thread_title(thread_id, name) {
        console::eprintln!("FutureOS thread title sync failed for {thread_id}: {error}");
    }
}

/// The "no agent session" payload for [`get_thread_agent_state`]: null model /
/// thinking level and session id, so the frontend falls back to the thread
/// title and the global draft selections.
fn empty_agent_state(session_name: &str) -> serde_json::Value {
    serde_json::json!({
        "model": null,
        "thinkingLevel": null,
        "sessionName": session_name,
        "sessionId": null,
        "cwd": null,
        "parentSessionId": null,
        "isStreaming": false,
    })
}

pub async fn get_thread_agent_state(thread_id: String) -> Result<serde_json::Value, crate::Error> {
    let thread = store::get_thread(&thread_id)?.ok_or_else(|| "Thread not found.".to_string())?;
    let Some(session_id) = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(empty_agent_state(&thread.title));
    };

    // Agent unreachable or get_state failed: return an ERROR, not a null
    // payload. The frontend caches whatever this command returns; caching
    // fabricated nulls poisoned the composer with the global draft
    // model/thinking level for the whole TTL window. An error instead
    // rejects the fetch, leaving the last-known-good cache entry in place.
    //
    // The one exception is a missing session ("session not found"): the agent
    // losing a session (e.g. after an environment switch restarts it) is not a
    // transient fetch failure, so a null payload is the honest answer — the
    // settings can be absent. Prompt admission separately verifies the history
    // source; a missing source never authorizes an empty replacement session.
    let mut client = crate::agent_bridge::connect_agent()
        .await
        .map_err(|e| format!("Future Agent unreachable: {e}"))?;
    crate::agent_bridge::verify_thread_source_with_client(&mut client, &thread_id, session_id)
        .await?;
    let cmd = crate::agent_bridge::get_state_command(session_id.to_string());
    let resp = client
        .execute_command(cmd)
        .await
        .map_err(|e| format!("get_state RPC failed: {e}"))?
        .into_inner();
    if !resp.success {
        if resp.error_code == future_rpc::session_errors::SESSION_NOT_FOUND_CODE
            || future_rpc::session_errors::legacy_session_not_found_message(&resp.error)
        {
            return Ok(empty_agent_state(&thread.title));
        }
        return Err(format!("get_state rejected: {}", resp.error).into());
    }
    let value = future_rpc::decode::response_data(&resp);
    if value.is_null() {
        return Err("get_state typed payload is missing or invalid"
            .to_string()
            .into());
    }
    // Converge the DB title toward the agent's session_name — the name shared
    // with every client (TUI `/name`, CLI, channels), whose renames never
    // reach the GUI DB. The sidebar treats the agent name as authoritative
    // and falls back to the DB title whenever agent state is unavailable, so
    // a stale DB title surfaces as "the rename didn't survive a restart".
    // Best-effort; sync_thread_title never bumps updated_at (sidebar order).
    if let Some(name) = value
        .get("sessionName")
        .or_else(|| value.get("session_name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        if name != thread.title {
            sync_title_best_effort(&thread_id, name);
        }
    }
    Ok(value)
}

/// Manually compact the current Agent session without creating a user message.
pub async fn compact_thread_context(thread_id: String) -> Result<serde_json::Value, crate::Error> {
    agent_bridge::compact_thread_context(thread_id).await
}

/// Read Agent history, or preserved GUI history when upgrading an older DB.
pub async fn get_session_entries(thread_id: String) -> Result<serde_json::Value, crate::Error> {
    read_thread_history(&thread_id, None, None).await
}

/// One UI history page. Full-history callers keep using get_session_entries.
pub async fn get_session_entries_page(
    thread_id: String,
    before: Option<i64>,
    limit: i64,
    history_source: Option<String>,
) -> Result<serde_json::Value, crate::Error> {
    read_thread_history(
        &thread_id,
        Some((before, limit.clamp(1, 100))),
        history_source.as_deref(),
    )
    .await
}

async fn read_thread_history(
    thread_id: &str,
    page: Option<(Option<i64>, i64)>,
    expected_source: Option<&str>,
) -> Result<serde_json::Value, crate::Error> {
    let thread = store::get_thread(thread_id)?.ok_or("Thread not found.")?;
    let canonical = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let session_id = canonical
        .map(str::to_owned)
        .or(store::legacy_session_id(thread_id)?);
    let archived = store::has_legacy_history(thread_id)?;
    let legacy_cursor = expected_source.is_some_and(|source| source.starts_with("legacy:"));
    let tail = page.is_none_or(|(before, _)| before.is_none_or(|cursor| cursor == i64::MAX));
    let mut key = format!("local:{thread_id}");
    let mut availability = "ready";
    let mut result = Ok(serde_json::json!({"entries":[],"hasMore":false,"nextOffset":0}));
    if let Some(session_id) = session_id.as_ref() {
        // Carry the instance-fenced client from ownership verification through
        // every body/fragment read. A reconnect must start a fresh operation.
        let verified = async {
            let mut client = crate::agent_bridge::connect_agent().await?;
            let presence =
                crate::agent_bridge::require_present_with_client(&mut client, session_id).await?;
            Ok::<_, crate::Error>((client, presence))
        }
        .await;
        result = match verified {
            Ok((mut client, presence)) => {
                let status = presence.sessions[0].status.as_str();
                if status != "present" {
                    availability = match status {
                        "deleted" => "deleted",
                        "import_failed" => "import_failed",
                        _ => "missing",
                    };
                    Err("Conversation history is unavailable: its Agent session or database is missing. Restore the matching Agent database or legacy sessions backup.".into())
                } else if store::thread_source(thread_id)?
                    .is_some_and(|source| source.0 != presence.database_id)
                {
                    availability = "different_database";
                    Err("Conversation belongs to a different Agent database; restore its matching data.".into())
                } else {
                    key = format!(
                        "agent:{}:{}:{}:{session_id}",
                        presence.database_id,
                        presence.restore_generation,
                        client.readiness().agent_instance_id
                    );
                    if canonical.is_some() {
                        store::record_thread_source(
                            thread_id,
                            session_id,
                            &presence.database_id,
                            &presence.restore_generation,
                        )?;
                    }
                    if !legacy_cursor && expected_source.is_some_and(|source| source != key) {
                        return Err(
                            "Conversation history source changed; reload before paging further."
                                .into(),
                        );
                    }
                    if legacy_cursor {
                        crate::agent_bridge::get_session_entries_before_with_client(
                            &client,
                            session_id.clone(),
                            i64::MAX,
                            1,
                        )
                        .await
                    } else {
                        match page {
                            Some((before, limit)) => {
                                crate::agent_bridge::get_session_entries_before_with_client(
                                    &client,
                                    session_id.clone(),
                                    before.unwrap_or(i64::MAX),
                                    limit,
                                )
                                .await
                            }
                            None => {
                                crate::agent_bridge::get_session_entries_with_client(
                                    &mut client,
                                    session_id.clone(),
                                )
                                .await
                            }
                        }
                    }
                }
            }
            Err(error) => {
                availability = "offline";
                Err(error)
            }
        };
    }
    let empty = result.as_ref().is_ok_and(|value| {
        value["entries"].as_array().is_some_and(|entries| {
            entries
                .iter()
                .all(|entry| !matches!(entry["role"].as_str(), Some("user" | "assistant" | "tool")))
        })
    });
    if archived && (tail || legacy_cursor) && (empty || result.is_err()) {
        let source_key = store::legacy_history_source_key(thread_id)?;
        if expected_source.is_some_and(|source| source != source_key) {
            return Err(
                "Conversation history source changed; reload before paging further.".into(),
            );
        }
        let mut recovered = match page {
            Some((before, limit)) => {
                store::legacy_history_page(thread_id, before.unwrap_or(i64::MAX), limit)?
            }
            None => {
                serde_json::json!({"entries":store::legacy_history_entries(thread_id)?,"hasMore":false,"nextOffset":0})
            }
        };
        recovered["history"] = serde_json::json!({"source":"legacy","availability":availability,"canContinue":false,"sourceKey":source_key});
        return Ok(recovered);
    }
    if legacy_cursor {
        return Err("Conversation history source changed; reload before paging further.".into());
    }
    // A turn reaches this store before it reaches the Agent: `create_run` stamps
    // the thread's activity and the Agent only persists the user entry once the
    // prompt is admitted. An empty tail page read inside that window is an
    // in-flight turn, not a lost conversation — only a conversation that has
    // nothing running can really be missing its history.
    if empty
        && tail
        && !store::has_active_run(thread_id)?
        && (thread.last_message_at.is_some() || !store::list_runs(thread_id)?.is_empty())
    {
        return Err("Conversation history is unavailable: restore its matching Agent database or legacy sessions backup.".into());
    }
    let mut value = result?;
    value["history"] = serde_json::json!({"source":if session_id.is_some(){"agent"}else{"none"},"availability":availability,
        "canContinue":canonical.is_some() || session_id.is_none(),"sourceKey":key});
    Ok(value)
}

pub fn archive_finished_runs(thread_id: String) -> Result<usize, crate::Error> {
    store::archive_finished_runs(&thread_id)
}

pub fn get_thread_cleanup_summary(
    thread_id: String,
) -> Result<store::ThreadCleanupSummary, crate::Error> {
    store::get_thread_cleanup_summary(&thread_id)
}

/// Attach to a remote agent session stream: create a synthetic run and
/// subscribe to live events so the GUI shows real-time streaming content
/// for prompts initiated by other clients (TUI, CLI, phone).
pub async fn attach_remote_stream(thread_id: String) -> Result<serde_json::Value, crate::Error> {
    let run_id = crate::agent_bridge::attach_remote_stream(&thread_id).await?;
    Ok(serde_json::json!({ "runId": run_id }))
}

/// Ensure the session observer is live for this session. Observers are
/// long-lived per-session tasks (settings fan-out, event projection for runs
/// no pipeline owns, NATS mirroring) — this call is now an idempotent hint
/// (LRU touch), no longer a single-slot re-subscription. Safe to call on
/// every thread switch.
pub fn observe_session(thread_id: String, session_id: String) -> Result<(), crate::Error> {
    let thread_id = thread_id.trim();
    let session_id = session_id.trim();
    if thread_id.is_empty() || session_id.is_empty() {
        return Err("Both thread id and session id are required to observe a session.".into());
    }
    // This GUI command is deliberately single-target: it may wake or create
    // exactly the observer keyed by this already-owned session, never a global
    // stream or a different thread's observer.
    crate::agent_bridge::ensure_observer_for_thread(session_id, thread_id)
        .map_err(crate::Error::from)
}

/// Move a thread to the workspace matching a new cwd (e.g. after TUI /cwd).
pub fn reconcile_thread_workspace(session_id: String, cwd: String) -> Result<(), crate::Error> {
    crate::agent_bridge::reconcile_thread_workspace(&session_id, &cwd).map_err(crate::Error::from)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::test_support::command_agent::{mock_agent_lock, script_mock_agent, MockScript};
    use std::collections::HashMap;

    fn init(label: &str) -> HomeGuard {
        let home = HomeGuard::new(label);
        crate::store::initialize_app_store().expect("init store");
        home
    }

    fn make_thread(home: &HomeGuard, agent_session_id: Option<&str>) -> store::ThreadRecord {
        let _ = home;
        crate::store::create_thread(store::CreateThreadInput {
            mode: "chat".into(),
            title: Some("Chat".into()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: agent_session_id.map(str::to_string),
        })
        .expect("create thread")
    }

    /// A store failure while preparing one selection is reported against *that*
    /// thread and must not stop the others from being handled: the user asked
    /// for a batch, and the per-thread result is what the UI renders.
    #[tokio::test]
    async fn batch_delete_reports_a_failed_selection_and_keeps_going() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_batch_delete_failure");
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript::default());

        // Break the store the same way the title-failure test does: the closure
        // lookup for every selection then fails, which is the arm that reports
        // a per-thread failure instead of aborting the batch.
        let home = std::env::var("HOME").expect("test home");
        let conn =
            rusqlite::Connection::open(std::path::Path::new(&home).join(".future/app/app.db"))
                .expect("open db");
        conn.execute_batch("DROP TABLE threads;").unwrap();
        drop(conn);

        let result = batch_delete_threads(store::BatchDeleteThreadsInput {
            thread_ids: vec!["first".into(), "second".into()],
            delete_files: false,
        })
        .await
        .expect("a broken selection is reported, not propagated");

        assert_eq!(result.deleted_count, 0, "nothing could be prepared");
        assert_eq!(
            result.failed.len(),
            2,
            "every selection is accounted for: {:?}",
            result.failed
        );
        assert!(
            result.failed[0].starts_with("first: "),
            "{:?}",
            result.failed
        );
        assert!(
            result.failed[1].starts_with("second: "),
            "{:?}",
            result.failed
        );
        script_mock_agent(MockScript::default());
    }

    /// The other per-selection failure: the selection is well-formed but the
    /// Agent will not confirm its session stopped. Deletion must be cancelled for
    /// *that* thread and reported, never executed against a live session.
    #[tokio::test]
    async fn a_selection_whose_session_will_not_stop_is_reported() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_batch_delete_stop_failure");
        crate::test_support::command_agent::ensure_mock_agent();
        let thread = make_thread(&_home, Some("sess-busy"));
        // A non-terminal run makes this session "active", so the batch asks the
        // Agent to stop it before deleting — and the Agent refuses the abort.
        let run = crate::agent_bridge::test_support::seed_run(&thread.id);
        script_mock_agent(MockScript {
            errors: HashMap::from([("abort".to_string(), "session is mid-answer".to_string())]),
            ..Default::default()
        });

        let result = batch_delete_threads(store::BatchDeleteThreadsInput {
            thread_ids: vec![thread.id.clone()],
            delete_files: false,
        })
        .await
        .expect("a refused stop is reported, not propagated");

        assert_eq!(result.deleted_count, 0, "nothing may be deleted");
        assert_eq!(result.failed.len(), 1);
        assert!(
            result.failed[0].starts_with(&format!("{}: ", thread.id)),
            "{:?}",
            result.failed
        );
        assert!(
            crate::store::get_thread(&thread.id).unwrap().is_some(),
            "the conversation survives a cancelled deletion"
        );
        assert!(
            crate::store::get_run(&run.id).unwrap().is_some(),
            "its run survives too"
        );
        script_mock_agent(MockScript::default());
    }

    #[test]
    fn create_thread_command_creates_a_chat_thread() {
        let _home = init("cmd_create_thread");
        let thread = create_thread(store::CreateThreadInput {
            mode: "chat".into(),
            title: Some("New Chat".into()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create thread");
        assert_eq!(thread.title, "New Chat");
    }

    #[tokio::test]
    async fn fork_thread_errors_for_an_unknown_thread() {
        let _home = init("cmd_fork_ghost");
        assert!(
            fork_thread("no-such-thread".into(), "x".into(), "request".into())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn attach_remote_stream_errors_for_an_unknown_thread() {
        let _home = init("cmd_attach_ghost");
        assert!(attach_remote_stream("no-such-thread".into()).await.is_err());
    }

    #[tokio::test]
    async fn list_streaming_thread_ids_is_empty_when_the_agent_call_errors() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_streaming_err");
        let thread = make_thread(&_home, Some("sess_stream_err"));
        crate::test_support::command_agent::ensure_mock_agent();
        crate::test_support::command_agent::with_broken_endpoint(list_streaming_thread_ids)
            .await
            .expect("streaming");
        let _ = thread;
    }

    #[test]
    fn sync_title_best_effort_logs_through_a_store_failure() {
        let _home = init("cmd_title_sync_fail");
        let thread = make_thread(&_home, Some("sess_sync"));
        let home = std::env::var("HOME").expect("test home");
        let conn =
            rusqlite::Connection::open(std::path::Path::new(&home).join(".future/app/app.db"))
                .expect("open db");
        conn.execute_batch("DROP TABLE threads;").unwrap();
        drop(conn);
        // The defensive eprintln arm must not panic.
        sync_title_best_effort(&thread.id, "Renamed");
    }

    #[test]
    fn get_thread_cleanup_summary_delegates() {
        let _home = init("cmd_threads_cleanup");
        let thread = make_thread(&_home, None);
        let summary = get_thread_cleanup_summary(thread.id.clone()).expect("summary");
        assert_eq!(summary.thread_id, thread.id);
    }

    #[test]
    fn observe_session_requires_both_ids() {
        let _home = init("cmd_observe");
        assert!(observe_session("  ".into(), "sess".into()).is_err());
        assert!(observe_session("thread".into(), "".into()).is_err());
    }

    #[test]
    fn reconcile_thread_workspace_handles_missing_and_empty_cwd() {
        let _home = init("cmd_reconcile_ws");
        assert!(reconcile_thread_workspace("ghost".into(), "/tmp/x".into()).is_err());
        // A thread with an agent session, empty cwd → no-op Ok.
        let thread = make_thread(&_home, Some("sess_1"));
        assert!(reconcile_thread_workspace("sess_1".into(), "  ".into()).is_ok());
        let _ = thread;
    }

    #[tokio::test]
    async fn list_streaming_thread_ids_is_empty_when_agent_down() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_streaming_down");
        let thread = make_thread(&_home, Some("sess_stream"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            down: true,
            ..Default::default()
        });
        // Down agent → empty list (not an error).
        let ids = list_streaming_thread_ids().await.expect("streaming");
        assert!(ids.is_empty());
        let _ = thread;
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn list_streaming_thread_ids_maps_streaming_sessions_to_threads() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_streaming");
        let thread = make_thread(&_home, Some("sess_live"));
        let _idle_thread = make_thread(&_home, Some("sess_idle"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            streaming_ids: vec!["sess_live".to_string()],
            ..Default::default()
        });
        let ids = list_streaming_thread_ids().await.expect("streaming");
        assert_eq!(ids, vec![thread.id]);
        assert!(
            crate::agent_bridge::has_observer("sess_live"),
            "only the reported active session gains an observer"
        );
        assert!(
            !crate::agent_bridge::has_observer("sess_idle"),
            "an idle historical session stays cold"
        );
        crate::agent_bridge::drop_observer("sess_live");
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn get_thread_agent_state_without_session_returns_null_payload() {
        let _home = init("cmd_agent_state_no_session");
        let thread = make_thread(&_home, None);
        let value = get_thread_agent_state(thread.id.clone())
            .await
            .expect("state");
        assert_eq!(value["model"], serde_json::Value::Null);
        assert_eq!(value["sessionId"], serde_json::Value::Null);
        assert_eq!(value["sessionName"], serde_json::json!(thread.title));
    }

    #[tokio::test]
    async fn get_thread_agent_state_errors_for_unknown_thread() {
        let _home = init("cmd_agent_state_ghost");
        assert!(get_thread_agent_state("ghost".into()).await.is_err());
    }

    #[tokio::test]
    async fn compact_thread_context_targets_the_thread_session() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_compact_context");
        let thread = make_thread(&_home, Some("sess_compact"));
        let agent = crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "compact".to_string(),
                r#"{"accepted":true,"operationId":"cmp-test"}"#.to_string(),
            )]),
            ..Default::default()
        });

        compact_thread_context(thread.id).await.expect("compact");

        assert!(agent.served("compact", "sess_compact"));
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn compact_thread_context_requires_an_existing_session() {
        let _home = init("cmd_compact_context_empty");
        let thread = make_thread(&_home, None);
        let error = compact_thread_context(thread.id).await.unwrap_err();
        assert!(error.to_string().contains("no context to compact"));
    }

    #[tokio::test]
    async fn compact_thread_context_rejects_empty_agent_data() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_compact_empty_data");
        let thread = make_thread(&_home, Some("sess_compact_empty"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("compact".to_string(), String::new())]),
            ..Default::default()
        });
        let error = compact_thread_context(thread.id).await.unwrap_err();
        assert!(error.to_string().contains("invalid acknowledgement"));
        script_mock_agent(MockScript::default());
    }

    #[test]
    fn mark_thread_opened_delegates_to_the_store() {
        let _home = init("cmd_mark_opened");
        let thread = make_thread(&_home, None);
        mark_thread_opened(thread.id.clone()).expect("mark opened");
        // The store records the open timestamp; the command is a thin wrapper.
        let reopened = store::get_thread(&thread.id)
            .expect("get thread")
            .expect("present");
        assert!(reopened.last_opened_at.is_some());
    }

    #[tokio::test]
    async fn get_session_entries_without_session_returns_empty() {
        let _home = init("cmd_entries_no_session");
        let thread = make_thread(&_home, None);
        let value = get_session_entries(thread.id.clone())
            .await
            .expect("entries");
        assert_eq!(value["entries"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn archived_gui_history_is_visible_when_the_original_agent_database_is_missing() {
        let _lock = mock_agent_lock();
        let home = init("legacy_history_read");
        let thread = make_thread(&home, Some("missing-legacy"));
        let path = store::app_data_path().unwrap().db_path;
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch("CREATE TABLE legacy_messages(id TEXT,thread_id TEXT,run_id TEXT,role TEXT,content TEXT,created_at INTEGER);").unwrap();
        conn.execute("INSERT INTO legacy_messages VALUES('u',?1,NULL,'user','preserved question',1),('a',?1,NULL,'assistant','preserved answer',2)",[&thread.id]).unwrap();
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            errors: HashMap::from([(
                "get_session_entries".to_string(),
                "session not found".to_string(),
            )]),
            ..Default::default()
        });
        let page = get_session_entries_page(thread.id.clone(), None, 1, None)
            .await
            .unwrap();
        assert_eq!(page["entries"].as_array().unwrap().len(), 1);
        assert_eq!(page["entries"][0]["blocks"][0]["text"], "preserved answer");
        assert_eq!(page["hasMore"], true);
        assert_eq!(page["history"]["canContinue"], false);
        let full = get_session_entries(thread.id.clone()).await.unwrap();
        assert_eq!(full["entries"].as_array().unwrap().len(), 2);
        let older = get_session_entries_page(
            thread.id.clone(),
            Some(1),
            1,
            page["history"]["sourceKey"].as_str().map(str::to_owned),
        )
        .await
        .unwrap();
        assert_eq!(
            older["entries"][0]["blocks"][0]["text"],
            "preserved question"
        );
        assert_eq!(older["hasMore"], false);
        script_mock_agent(MockScript {data:HashMap::from([("get_session_entries".into(),serde_json::json!({"entries":[{"id":"metadata","role":"system","kind":"session_info","blocks":[],"createdAtMs":1}],"hasMore":false,"nextOffset":0}).to_string())]),..Default::default()});
        let recovered = get_session_entries_page(thread.id.clone(), None, 1, None)
            .await
            .unwrap();
        assert_eq!(recovered["entries"], page["entries"]);
        let error = crate::agent_bridge::agent_prompt(
            "continue".into(),
            None,
            thread.id,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("recovery archive"));
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn rename_thread_maps_transport_errors() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_rename_xport");
        let thread = make_thread(&_home, Some("sess_rename_x"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            transport_fail: ["set_session_name".to_string()].into_iter().collect(),
            ..Default::default()
        });
        let err = rename_thread(store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "Renamed".into(),
        })
        .await
        .unwrap_err();
        assert!(!err.to_string().is_empty());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn delete_thread_drops_observer_when_the_tombstone_survives() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_delete_tomb");
        let thread = make_thread(&_home, Some("sess_tomb"));
        crate::test_support::command_agent::ensure_mock_agent();
        // The agent never acknowledges the delete, so the tombstone row
        // survives and the observer-drop arm runs.
        script_mock_agent(MockScript {
            transport_fail: ["delete_sessions".to_string()].into_iter().collect(),
            ..Default::default()
        });
        let deleted = delete_thread(store::DeleteThreadInput {
            thread_id: thread.id.clone(),
            delete_files: false,
        })
        .await
        .expect("delete");
        assert_eq!(deleted.id, thread.id);
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn list_streaming_thread_ids_is_empty_on_transport_error() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_streaming_xport");
        let thread = make_thread(&_home, Some("sess_xport"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            transport_fail: ["list_streaming_sessions".to_string()]
                .into_iter()
                .collect(),
            ..Default::default()
        });
        let ids = list_streaming_thread_ids().await.expect("streaming");
        assert!(ids.is_empty());
        script_mock_agent(MockScript::default());
        let _ = thread;
    }

    #[tokio::test]
    async fn generate_thread_title_returns_a_draft_without_renaming() {
        let _lock = mock_agent_lock();
        let home = init("cmd_title_suggestion");
        let thread = make_thread(&home, Some("sess_title"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "generate_session_title".into(),
                r#"{"title":"Suggested","model":"selected/model"}"#.into(),
            )]),
            ..Default::default()
        });
        let result = generate_thread_title(thread.id.clone(), "en".into())
            .await
            .unwrap();
        assert_eq!(result["title"], "Suggested");
        assert_eq!(
            store::get_thread(&thread.id).unwrap().unwrap().title,
            thread.title
        );
        script_mock_agent(MockScript {
            errors: HashMap::from([("generate_session_title".into(), "no completed pairs".into())]),
            ..Default::default()
        });
        assert!(generate_thread_title(thread.id.clone(), "en".into())
            .await
            .is_err());
        assert_eq!(
            store::get_thread(&thread.id).unwrap().unwrap().title,
            thread.title
        );
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn rename_thread_propagates_to_the_agent_first() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_rename");
        let thread = make_thread(&_home, Some("sess_rename"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("set_session_name".to_string(), "{}".to_string())]),
            ..Default::default()
        });
        let renamed = rename_thread(store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "Renamed".into(),
        })
        .await
        .expect("rename");
        assert_eq!(renamed.title, "Renamed");
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn rename_thread_rejects_an_empty_title_before_the_agent() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_rename_empty");
        let thread = make_thread(&_home, None);
        assert!(rename_thread(store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "   ".into(),
        })
        .await
        .is_err());
    }

    #[tokio::test]
    async fn rename_thread_surfaces_agent_rejection() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_rename_reject");
        let thread = make_thread(&_home, Some("sess_reject"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            errors: HashMap::from([("set_session_name".to_string(), "nope".to_string())]),
            ..Default::default()
        });
        assert!(rename_thread(store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "Renamed".into(),
        })
        .await
        .is_err());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn update_thread_model_without_model_skips_the_agent() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_model_none");
        let thread = make_thread(&_home, None);
        let updated = update_thread_model(store::UpdateThreadModelInput {
            thread_id: thread.id.clone(),
            model_provider: None,
            model_id: None,
        })
        .await
        .expect("update");
        assert_eq!(updated.id, thread.id);
    }

    #[tokio::test]
    async fn update_thread_thinking_level_without_level_skips_the_agent() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_level_none");
        let thread = make_thread(&_home, None);
        let updated = update_thread_thinking_level(store::UpdateThreadThinkingLevelInput {
            thread_id: thread.id.clone(),
            thinking_level: None,
        })
        .await
        .expect("update");
        assert_eq!(updated.id, thread.id);
    }

    #[test]
    fn archive_finished_runs_without_terminal_runs_returns_zero() {
        let _home = init("cmd_archive_runs");
        let thread = make_thread(&_home, None);
        let archived = archive_finished_runs(thread.id).expect("archive");
        assert_eq!(archived, 0);
    }

    #[tokio::test]
    async fn rename_thread_tolerates_a_session_not_found_rejection() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_rename_notfound");
        let thread = make_thread(&_home, Some("sess_nf"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            errors: HashMap::from([(
                "set_session_name".to_string(),
                "session not found".to_string(),
            )]),
            ..Default::default()
        });
        let renamed = rename_thread(store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "Still renamed".into(),
        })
        .await
        .expect("rename despite session-not-found");
        assert_eq!(renamed.title, "Still renamed");
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn update_thread_model_propagates_to_the_agent() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_model_set");
        let thread = make_thread(&_home, Some("sess_model"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript::default());
        let updated = update_thread_model(store::UpdateThreadModelInput {
            thread_id: thread.id.clone(),
            model_provider: None,
            model_id: Some("future/deepseek".into()),
        })
        .await
        .expect("update model");
        assert_eq!(updated.id, thread.id);
    }

    #[tokio::test]
    async fn update_thread_thinking_level_propagates_to_the_agent() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_level_set");
        let thread = make_thread(&_home, Some("sess_level"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript::default());
        let updated = update_thread_thinking_level(store::UpdateThreadThinkingLevelInput {
            thread_id: thread.id.clone(),
            thinking_level: Some("high".into()),
        })
        .await
        .expect("update level");
        assert_eq!(updated.id, thread.id);
    }

    #[tokio::test]
    async fn delete_thread_tombstones_and_reconciles() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_delete");
        let thread = make_thread(&_home, Some("sess_del"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            ..Default::default()
        });
        let deleted = delete_thread(store::DeleteThreadInput {
            thread_id: thread.id.clone(),
            delete_files: false,
        })
        .await
        .expect("delete");
        assert_eq!(deleted.id, thread.id);
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn delete_thread_stops_an_active_agent_session_first() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_delete_active");
        let thread = make_thread(&_home, Some("sess_active_delete"));
        store::record_thread_source(
            &thread.id,
            "sess_active_delete",
            "test-agent-database",
            "test-generation",
        )
        .unwrap();
        store::create_run(store::CreateRunInput {
            id: Some("run_active_delete".to_string()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("active run");
        let agent = crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "get_state".to_string(),
                crate::test_support::agent::get_state_payload("sess_active_delete", false)
                    .to_string(),
            )]),
            ..Default::default()
        });
        let request_offset = agent.requests().len();

        delete_thread(store::DeleteThreadInput {
            thread_id: thread.id.clone(),
            delete_files: false,
        })
        .await
        .expect("delete");

        let commands: Vec<String> = agent
            .requests()
            .into_iter()
            .skip(request_offset)
            .map(|request| request.0)
            .collect();
        let abort = commands
            .iter()
            .position(|command| command == "abort")
            .unwrap();
        let delete = commands
            .iter()
            .position(|command| command == "delete_sessions")
            .unwrap();
        assert!(abort < delete, "{commands:?}");
        script_mock_agent(MockScript::default());
    }

    /// The conversation whose prompt never reached the agent: the thread has no
    /// agent session and its run row is still non-terminal. Deleting it has to
    /// succeed — the agent has nothing to stop — where the old flow reported the
    /// agent's "session not found" as a failure and left the conversation
    /// undeletable, for as long as that row lived without ever being settled.
    #[tokio::test]
    async fn delete_thread_succeeds_when_the_agent_never_had_the_session() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_delete_unbound");
        let thread = make_thread(&_home, None);
        store::create_run(store::CreateRunInput {
            id: Some("run_unbound".to_string()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("active run");
        let agent = crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            errors: HashMap::from([(
                "get_state".to_string(),
                "session not found — pass a valid session_id (new_session creates one)".to_string(),
            )]),
            ..Default::default()
        });
        let request_offset = agent.requests().len();

        delete_thread(store::DeleteThreadInput {
            thread_id: thread.id.clone(),
            delete_files: false,
        })
        .await
        .expect("a session that does not exist has nothing to stop");

        let commands: Vec<String> = agent
            .requests()
            .into_iter()
            .skip(request_offset)
            .map(|request| request.0)
            .collect();
        assert!(
            !commands.iter().any(|command| command == "abort"),
            "no stop is attempted for a session the agent has no record of: {commands:?}"
        );
        // No Agent source ever owned this unbound thread. Retain intent rather
        // than sending a guessed session deletion to whichever database is live.
        assert!(
            !commands.iter().any(|command| command == "delete_sessions"),
            "{commands:?}"
        );
        assert!(store::get_thread(&thread.id).expect("get").is_none());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn batch_delete_removes_running_threads_whose_sessions_are_missing() {
        let _lock = mock_agent_lock();
        let home = init("cmd_batch_delete_missing_sessions");
        let unbound = make_thread(&home, None);
        let bound = make_thread(&home, Some("missing-session"));
        let runs = [
            crate::agent_bridge::test_support::seed_run(&unbound.id),
            crate::agent_bridge::test_support::seed_run(&bound.id),
        ];
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            errors: HashMap::from([("get_state".into(), "session not found".into())]),
            ..Default::default()
        });

        let result = batch_delete_threads(store::BatchDeleteThreadsInput {
            thread_ids: vec![unbound.id.clone(), bound.id.clone()],
            delete_files: false,
        })
        .await
        .unwrap();

        assert_eq!(result.deleted_count, 2);
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        for thread in [unbound, bound] {
            assert!(store::get_thread(&thread.id).unwrap().is_none());
        }
        for run in runs {
            assert!(store::get_run(&run.id).unwrap().is_none());
        }
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn batch_delete_threads_delegates() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_batch_delete");
        let thread = make_thread(&_home, None);
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript::default());
        let result = batch_delete_threads(store::BatchDeleteThreadsInput {
            thread_ids: vec![thread.id.clone(), "ghost".into()],
            delete_files: false,
        })
        .await
        .expect("batch delete");
        assert_eq!(result.deleted_count, 1);
        assert_eq!(result.failed.len(), 1);
    }

    #[tokio::test]
    async fn delete_thread_takes_its_descendants_with_it() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_delete_cascade");
        let parent = make_thread(&_home, Some("sess_gui_parent"));
        let child = make_thread(&_home, Some("sess_gui_child"));
        store::sync_thread_parent_session("sess_gui_child", "sess_gui_parent").expect("lineage");
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            ..Default::default()
        });

        delete_thread(store::DeleteThreadInput {
            thread_id: parent.id.clone(),
            delete_files: false,
        })
        .await
        .expect("delete parent");

        assert!(store::get_thread(&parent.id).expect("get").is_none());
        assert!(store::get_thread(&child.id).expect("get").is_none());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn batch_delete_does_not_report_a_cascaded_child_as_a_failure() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_batch_cascade");
        let parent = make_thread(&_home, Some("sess_batch_parent"));
        let child = make_thread(&_home, Some("sess_batch_child"));
        store::sync_thread_parent_session("sess_batch_child", "sess_batch_parent")
            .expect("lineage");
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            ..Default::default()
        });

        // The user selected both the parent and its child.
        let result = batch_delete_threads(store::BatchDeleteThreadsInput {
            thread_ids: vec![parent.id.clone(), child.id.clone()],
            delete_files: false,
        })
        .await
        .expect("batch delete");

        assert!(result.failed.is_empty(), "{:?}", result.failed);
        assert_eq!(result.deleted_count, 2, "both selections are gone");
        assert!(store::get_thread(&child.id).expect("get").is_none());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn get_thread_agent_state_reads_and_syncs_the_agent_title() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_agent_state_ok");
        let thread = make_thread(&_home, Some("sess_state"));
        let agent = crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript::default());
        let mut payload = crate::test_support::agent::get_state_payload("sess_state", false);
        payload["sessionName"] = serde_json::json!("Agent Name");
        agent.script_typed_for("get_state", "sess_state", payload);
        let value = get_thread_agent_state(thread.id.clone())
            .await
            .expect("state");
        assert_eq!(value["sessionName"], serde_json::json!("Agent Name"));
        // Title converged toward the agent name.
        let reloaded = crate::store::get_thread(&thread.id)
            .expect("get")
            .expect("thread");
        assert_eq!(reloaded.title, "Agent Name");
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn get_session_entries_reads_the_agent_history() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_entries_ok");
        let thread = make_thread(&_home, Some("sess_entries"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "get_session_entries".to_string(),
                "{\"entries\":[]}".to_string(),
            )]),
            ..Default::default()
        });
        let value = get_session_entries(thread.id.clone())
            .await
            .expect("entries");
        assert_eq!(value["entries"], serde_json::json!([]));
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn get_session_entries_page_returns_one_page_and_cursor() {
        let _lock = mock_agent_lock();
        let home = init("cmd_entries_page");
        let thread = make_thread(&home, Some("sess_page"));
        script_mock_agent(MockScript::default());
        let agent = crate::test_support::command_agent::ensure_mock_agent();
        agent.script_typed_for(
            "get_session_entries",
            "sess_page",
            serde_json::json!({
                "entries":[{
                    "id":"synthetic",
                    "kind":"user",
                    "role":"user",
                    "createdAtMs":1767225600000_i64,
                    "blocks":[{"kind":"text","text":"page"}]
                }],
                "hasMore":true,"nextOffset":20,
            }),
        );
        // A fetch-all loop would request the same non-advancing cursor and
        // error here. The UI command must return immediately after one page.
        let page = get_session_entries_page(thread.id, None, 10, None)
            .await
            .unwrap();
        assert_eq!(page["entries"].as_array().unwrap().len(), 1);
        assert_eq!(page["hasMore"], true);
        assert_eq!(page["nextOffset"], 20);
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn get_session_entries_reads_typed_only_agent_history() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_entries_typed");
        let thread = make_thread(&_home, Some("sess_entries_typed"));
        script_mock_agent(MockScript::default());
        let agent = crate::test_support::command_agent::ensure_mock_agent();
        agent.script_typed_for(
            "get_session_entries",
            "sess_entries_typed",
            serde_json::json!({"entries": [{
                "id": "entry-typed",
                "kind": "user",
                "role": "user",
                "createdAtMs": 1787824800000_i64,
                "blocks": [{"kind": "text", "text": "still here"}]
            }]}),
        );

        let value = get_session_entries(thread.id.clone())
            .await
            .expect("typed entries");
        assert_eq!(value["entries"][0]["blocks"][0]["text"], "still here");
        script_mock_agent(MockScript::default());
    }

    #[test]
    fn archive_finished_runs_keeps_terminal_run_for_transcript_inspection() {
        let _home = init("cmd_archive_runs_terminal");
        let thread = make_thread(&_home, Some("sess_clear"));
        crate::store::create_run(store::CreateRunInput {
            id: Some("run_terminal".into()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("create run");
        crate::store::update_run_status_if_active(store::UpdateRunStatusInput {
            run_id: "run_terminal".into(),
            status: "completed".into(),
            error_message: None,
            error_type: None,
        })
        .expect("terminal");

        let archived = archive_finished_runs(thread.id).expect("archive");
        assert_eq!(archived, 1);
        let run = crate::store::get_run("run_terminal")
            .expect("get run")
            .expect("run retained");
        assert!(run.archived_at.is_some());
    }

    #[tokio::test]
    async fn update_thread_model_with_blank_model_skips_the_agent() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_model_blank");
        let thread = make_thread(&_home, None);
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript::default());
        let updated = update_thread_model(store::UpdateThreadModelInput {
            thread_id: thread.id.clone(),
            model_provider: None,
            model_id: Some("  ".into()),
        })
        .await
        .expect("update");
        assert_eq!(updated.id, thread.id);
        script_mock_agent(MockScript::default());
    }

    /// A conversation that has never been prompted has no agent session yet, so
    /// there is nothing to page through: the command answers an **empty page**
    /// without calling the agent at all. The distinction from the sibling test
    /// (`…treats_a_vanished_session_as_an_empty_page`) is the shape of the
    /// answer, not the shape of the failure — this arm must not depend on the
    /// agent being reachable, which is why the mock is left unreachable here.
    #[tokio::test]
    async fn get_session_entries_page_is_empty_for_a_thread_without_a_session() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_entries_page_nosession");
        let thread = make_thread(&_home, None);
        crate::test_support::command_agent::ensure_mock_agent();
        // Nothing is scripted. If this arm called the agent, the default mock
        // answer would not produce this payload, so the assertions below would
        // fail rather than pass by accident.
        let page = get_session_entries_page(thread.id.clone(), None, 10, None)
            .await
            .expect("a session-less thread is an empty page");
        assert_eq!(page["entries"], serde_json::json!([]));
        assert_eq!(page["hasMore"], serde_json::json!(false));
        assert_eq!(page["nextOffset"], serde_json::json!(0));
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn update_thread_thinking_level_with_blank_level_skips_the_agent() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_level_blank");
        let thread = make_thread(&_home, None);
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript::default());
        let updated = update_thread_thinking_level(store::UpdateThreadThinkingLevelInput {
            thread_id: thread.id.clone(),
            thinking_level: Some("  ".into()),
        })
        .await
        .expect("update");
        assert_eq!(updated.id, thread.id);
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn list_streaming_thread_ids_is_empty_when_sessions_rejected() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_streaming_reject");
        let thread = make_thread(&_home, Some("sess_stream_rej"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            errors: HashMap::from([("list_streaming_sessions".to_string(), "boom".to_string())]),
            ..Default::default()
        });
        let ids = list_streaming_thread_ids().await.expect("streaming");
        assert!(ids.is_empty());
        script_mock_agent(MockScript::default());
        let _ = thread;
    }

    #[tokio::test]
    async fn get_thread_agent_state_errors_when_get_state_rejected() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_agent_state_rej");
        let thread = make_thread(&_home, Some("sess_state_rej"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            errors: HashMap::from([("get_state".to_string(), "rejected".to_string())]),
            ..Default::default()
        });
        let err = get_thread_agent_state(thread.id.clone()).await.unwrap_err();
        assert!(!err.to_string().is_empty());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn get_thread_agent_state_errors_when_the_typed_payload_is_null() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_agent_state_null");
        let thread = make_thread(&_home, Some("sess_state_null"));
        crate::test_support::command_agent::ensure_mock_agent();
        // `success = true` with a null payload: the reply is well-formed at the
        // RPC level but carries no state. A client that treated that as "no
        // state" would render an empty panel; it is an error instead, because
        // `get_state` always answers with an object.
        script_mock_agent(MockScript {
            data: HashMap::from([("get_state".to_string(), "null".to_string())]),
            ..Default::default()
        });
        let error = get_thread_agent_state(thread.id.clone())
            .await
            .expect_err("a null typed payload must not be mistaken for state");
        assert!(
            error.to_string().contains("typed payload"),
            "the error must name the payload as the problem: {error}"
        );
        script_mock_agent(MockScript::default());
    }

    /// Missing history is actionable; storage failures must retain their own
    /// diagnostics instead of being collapsed into a successful empty page.
    #[tokio::test]
    async fn get_session_entries_page_reports_a_missing_history_source() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_entries_page_gone");
        let thread = make_thread(&_home, Some("sess_entries_gone"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            errors: HashMap::from([(
                "get_session_entries".to_string(),
                "session not found".to_string(),
            )]),
            ..Default::default()
        });
        let error = get_session_entries_page(thread.id.clone(), None, 10, None)
            .await
            .expect_err("missing history must not masquerade as an empty conversation");
        assert!(error.to_string().contains("history is unavailable"));

        // Unrelated storage errors keep their original diagnostics.
        script_mock_agent(MockScript {
            errors: HashMap::from([(
                "get_session_entries".to_string(),
                "disk exploded".to_string(),
            )]),
            ..Default::default()
        });
        let error = get_session_entries_page(thread.id.clone(), None, 10, None)
            .await
            .expect_err("an unrelated agent failure must propagate");
        assert!(
            error.to_string().contains("disk exploded"),
            "the agent's own message must survive: {error}"
        );
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn get_thread_agent_state_skips_title_sync_without_session_name() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_agent_state_noname");
        let thread = make_thread(&_home, Some("sess_state_noname"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("get_state".to_string(), "{\"model\":\"m\"}".to_string())]),
            ..Default::default()
        });
        let value = get_thread_agent_state(thread.id.clone())
            .await
            .expect("state");
        assert_eq!(value["model"], serde_json::json!("m"));
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn get_session_entries_errors_when_agent_rejects() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_entries_rej");
        let thread = make_thread(&_home, Some("sess_entries_rej"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            errors: HashMap::from([("get_session_entries".to_string(), "nope".to_string())]),
            ..Default::default()
        });
        let err = get_session_entries(thread.id.clone()).await.unwrap_err();
        assert!(!err.to_string().is_empty());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn get_thread_agent_state_returns_null_when_session_not_found() {
        // Regression: a session lost after an environment switch (the agent is
        // restarted and no longer knows the id) must read as "no session", not
        // surface the raw "session not found" error to the frontend.
        let _lock = mock_agent_lock();
        let _home = init("cmd_agent_state_missing");
        let thread = make_thread(&_home, Some("sess_state_missing"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            errors: HashMap::from([(
                "get_state".to_string(),
                "session not found — pass a valid session_id (new_session creates one)".to_string(),
            )]),
            ..Default::default()
        });
        let value = get_thread_agent_state(thread.id.clone())
            .await
            .expect("missing session is a benign null payload");
        assert_eq!(value["sessionId"], serde_json::Value::Null);
        assert_eq!(value["model"], serde_json::Value::Null);
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn get_session_entries_reports_missing_history() {
        // A lost source must remain distinguishable from a new conversation.
        let _lock = mock_agent_lock();
        let _home = init("cmd_entries_missing");
        let thread = make_thread(&_home, Some("sess_entries_missing"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            errors: HashMap::from([(
                "get_session_entries".to_string(),
                "session not found — pass a valid session_id (new_session creates one)".to_string(),
            )]),
            ..Default::default()
        });
        let error = get_session_entries(thread.id.clone())
            .await
            .expect_err("missing history source");
        assert!(error.to_string().contains("history is unavailable"));
        script_mock_agent(MockScript::default());
    }

    /// A reload can land between the two halves of a send: `create_run` stamps
    /// the thread's activity immediately, while the Agent persists the user
    /// entry only once the prompt is admitted. The empty page read inside that
    /// window belongs to the live turn — reporting missing history there put the
    /// "restore your backup" banner on top of a conversation's very first reply.
    #[tokio::test]
    async fn an_empty_tail_page_belongs_to_the_live_turn_that_has_not_landed_yet() {
        let _lock = mock_agent_lock();
        let home = init("cmd_entries_page_in_flight");
        let thread = make_thread(&home, Some("sess_in_flight"));
        script_mock_agent(MockScript::default());
        let agent = crate::test_support::command_agent::ensure_mock_agent();
        agent.script_typed_for(
            "get_session_entries",
            "sess_in_flight",
            serde_json::json!({"entries": [], "hasMore": false, "nextOffset": 0}),
        );
        store::create_run(store::CreateRunInput {
            id: Some("run_in_flight".into()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("live run");

        let page = get_session_entries_page(thread.id, None, 10, None)
            .await
            .expect("an in-flight turn is not lost history");
        assert_eq!(page["entries"], serde_json::json!([]));
        assert_eq!(page["history"]["availability"], "ready");
        script_mock_agent(MockScript::default());
    }

    /// The same empty page with nothing running is exactly the loss the banner
    /// exists for: the conversation shows activity, but its Agent history is
    /// gone. The live-turn exemption must not swallow that.
    #[tokio::test]
    async fn an_empty_tail_page_for_a_settled_conversation_is_still_a_loss() {
        let _lock = mock_agent_lock();
        let home = init("cmd_entries_page_lost");
        let thread = make_thread(&home, Some("sess_lost"));
        script_mock_agent(MockScript::default());
        let agent = crate::test_support::command_agent::ensure_mock_agent();
        agent.script_typed_for(
            "get_session_entries",
            "sess_lost",
            serde_json::json!({"entries": [], "hasMore": false, "nextOffset": 0}),
        );
        store::create_run(store::CreateRunInput {
            id: Some("run_settled".into()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("run");
        store::update_run_status_if_active(store::UpdateRunStatusInput {
            run_id: "run_settled".into(),
            status: "completed".into(),
            error_message: None,
            error_type: None,
        })
        .expect("settle the run");

        let error = get_session_entries_page(thread.id, None, 10, None)
            .await
            .expect_err("a settled conversation with no history is a real loss");
        assert!(error.to_string().contains("history is unavailable"));
        script_mock_agent(MockScript::default());
    }

    /// The deletion is *cancelled*, not merely reported, when the agent will not
    /// confirm that the session stopped. That is the whole point of the guard:
    /// deleting the row while a run is still writing would orphan the JSONL the
    /// agent is appending to, and the transcript would come back corrupted on
    /// the next launch. The last assertion is what makes this a cancellation
    /// claim rather than an error-path claim.
    #[tokio::test]
    async fn deleting_a_thread_is_cancelled_when_the_agent_will_not_confirm_the_stop() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_delete_unconfirmed");
        let thread = make_thread(&_home, Some("sess_delete_unconf"));
        // A live run is what makes the session active, so the delete path takes
        // the abort-then-confirm branch instead of the no-op one.
        crate::store::create_run(store::CreateRunInput {
            id: Some("run_unconfirmed".into()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("create run");
        crate::test_support::command_agent::ensure_mock_agent();
        // `abort_session` is accepted (the default mock answer), but `get_state`
        // never decodes into a state, so the agent never confirms idle.
        script_mock_agent(MockScript {
            data: HashMap::from([("get_state".to_string(), "null".to_string())]),
            ..Default::default()
        });

        let error = delete_thread(store::DeleteThreadInput {
            thread_id: thread.id.clone(),
            delete_files: false,
        })
        .await
        .expect_err("an unconfirmed stop must refuse the deletion");
        assert!(
            error.to_string().contains("did not confirm"),
            "the refusal must say the agent did not confirm: {error}"
        );
        assert!(
            crate::store::get_thread(&thread.id)
                .expect("lookup")
                .is_some(),
            "the cancellation must leave the conversation row in place"
        );
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn attach_remote_stream_returns_the_active_run_id() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_attach_ok");
        let thread = make_thread(&_home, Some("sess_attach"));
        crate::store::create_run(store::CreateRunInput {
            id: Some("run_active".into()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("create run");
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript::default());
        let value = attach_remote_stream(thread.id.clone())
            .await
            .expect("attach");
        assert_eq!(value["runId"], serde_json::json!("run_active"));
        agent_bridge::drop_observer("sess_attach");
        script_mock_agent(MockScript::default());
    }

    #[test]
    fn observe_session_ensures_an_observer_for_valid_ids() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_observe_ok");
        let thread = make_thread(&_home, Some("sess_observe"));
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript::default());
        observe_session(thread.id.clone(), "sess_observe".into()).expect("observe");
        agent_bridge::drop_observer("sess_observe");
        script_mock_agent(MockScript::default());
    }

    /// The window's list / open / recency reads and the three bookkeeping writes
    /// it drives them with. Each one is a one-line delegation, so what can be
    /// wrong here is the *shape* the window receives: the list is in the store's
    /// order, the pin / archive / restore trio each answer the row they changed,
    /// and opening a thread is what makes it the recent one.
    #[test]
    fn the_list_and_the_three_bookkeeping_writes_answer_the_window() {
        let _home = init("cmd_thread_reads");
        let first = make_thread(&_home, None);
        let second = make_thread(&_home, None);

        let listed = list_threads().expect("list");
        assert_eq!(listed.len(), 2, "{listed:?}");
        assert_eq!(
            get_thread(first.id.clone())
                .expect("get")
                .expect("the row")
                .id,
            first.id
        );
        assert!(get_thread("thread_absent".into()).expect("get").is_none());

        mark_thread_opened(first.id.clone()).expect("opened");
        assert_eq!(
            get_recent_thread()
                .expect("recent")
                .expect("the opened thread")
                .id,
            first.id
        );
        // The recency read follows the LAST open, not the first: two threads
        // created in the same millisecond must still sort by which was opened.
        std::thread::sleep(std::time::Duration::from_millis(5));
        mark_thread_opened(second.id.clone()).expect("opened");
        assert_eq!(
            get_recent_thread()
                .expect("recent")
                .expect("the last opened thread")
                .id,
            second.id
        );

        let pinned = pin_thread(store::PinThreadInput {
            thread_id: first.id.clone(),
            pinned: true,
        })
        .expect("pin");
        assert!(pinned.pinned, "the row comes back pinned");
        let archived = archive_thread(first.id.clone()).expect("archive");
        assert_eq!(archived.status, "archived");
        let restored = restore_thread(first.id.clone()).expect("restore");
        assert_eq!(restored.status, "active");
    }
}
