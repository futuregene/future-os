//! Thread lifecycle Tauri commands plus thread-scoped cleanup queries.

use crate::{agent_bridge, store};

#[tauri::command]
pub async fn fork_thread(
    thread_id: String,
    user_message_content: String,
    user_message_index: i64,
) -> Result<String, crate::AppError> {
    agent_bridge::fork_agent_session(&thread_id, &user_message_content, user_message_index).await
}

#[tauri::command]
pub fn list_threads() -> Result<Vec<store::ThreadRecord>, crate::AppError> {
    store::list_threads()
}

#[tauri::command]
pub fn get_thread(thread_id: String) -> Result<Option<store::ThreadRecord>, crate::AppError> {
    store::get_thread(&thread_id)
}

#[tauri::command]
pub fn get_recent_thread() -> Result<Option<store::ThreadRecord>, crate::AppError> {
    store::get_recent_thread()
}

#[tauri::command]
pub fn create_thread(
    input: store::CreateThreadInput,
) -> Result<store::ThreadRecord, crate::AppError> {
    // No auto `git init` for workspace-mode threads (§14.3); shadow review
    // handles non-git Workspaces.
    store::create_thread(input)
}

#[tauri::command]
pub async fn rename_thread(
    input: store::RenameThreadInput,
) -> Result<store::ThreadRecord, crate::AppError> {
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

#[tauri::command]
pub async fn update_thread_model(
    input: store::UpdateThreadModelInput,
) -> Result<store::ThreadRecord, crate::AppError> {
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
            let cmd = crate::agent_bridge::set_model_command(model_id, session_id);
            let _ = client.execute_command(cmd).await;
        }
    }
    Ok(thread)
}

#[tauri::command]
pub async fn update_thread_thinking_level(
    input: store::UpdateThreadThinkingLevelInput,
) -> Result<store::ThreadRecord, crate::AppError> {
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
            let cmd = crate::agent_bridge::set_thinking_level_command(thinking_level, session_id);
            let _ = client.execute_command(cmd).await;
        }
    }
    Ok(thread)
}

#[tauri::command]
pub fn pin_thread(input: store::PinThreadInput) -> Result<store::ThreadRecord, crate::AppError> {
    store::pin_thread(input)
}

#[tauri::command]
pub fn archive_thread(thread_id: String) -> Result<store::ThreadRecord, crate::AppError> {
    store::archive_thread(&thread_id)
}

#[tauri::command]
pub fn restore_thread(thread_id: String) -> Result<store::ThreadRecord, crate::AppError> {
    store::restore_thread(&thread_id)
}

#[tauri::command]
pub async fn delete_thread(
    input: store::DeleteThreadInput,
) -> Result<store::ThreadRecord, crate::AppError> {
    let session_id = store::get_thread(&input.thread_id)?
        .map(|thread| thread.agent_session_id.unwrap_or(thread.id));
    let thread = store::delete_thread_with_files(&input.thread_id, input.delete_files)?;
    if let Some(session_id) = session_id {
        if store::is_agent_session_tombstoned(&session_id)? {
            agent_bridge::drop_observer(&session_id);
        }
    }
    crate::agent_bridge::reconcile_delete_outbox().await;
    Ok(thread)
}

/// Batch-delete multiple threads. For each thread, the DB row + children are
/// hard-deleted and the agent session JSONL is removed. For chat-mode threads
/// with `delete_files`, the temporary workspace directory on disk is also
/// removed. Workspace-mode threads are never touched on disk regardless of
/// `delete_files`. Returns a summary of deleted count and failures.
#[tauri::command]
pub async fn batch_delete_threads(
    input: store::BatchDeleteThreadsInput,
) -> Result<store::BatchDeleteResult, crate::AppError> {
    let result = store::batch_delete_threads(&input)?;
    crate::agent_bridge::reconcile_delete_outbox().await;

    Ok(result)
}

/// Bulk streaming-status query: ONE agent RPC (`list_streaming_sessions`,
/// which only scans the agent's in-memory session map — no hydration, no
/// disk I/O) mapped back to GUI thread ids via the stored agent_session_id.
/// Replaces the old per-thread get_state fan-out, which hydrated every
/// polled session on the agent at startup.
///
/// Agent unreachable → empty list. The process-level compatibility monitor
/// retries, while React uses this command only for its initial snapshot.
#[tauri::command]
pub async fn list_streaming_thread_ids() -> Result<Vec<String>, crate::AppError> {
    let mut client = match crate::agent_bridge::connect_agent().await {
        Ok(client) => client,
        Err(_) => return Ok(vec![]),
    };
    let resp = client
        .execute_command(crate::agent_bridge::list_streaming_sessions_command())
        .await;
    let streaming_session_ids: std::collections::HashSet<String> = match resp {
        Ok(resp) => {
            let inner = resp.into_inner();
            if !inner.success {
                return Ok(vec![]);
            }
            serde_json::from_str::<serde_json::Value>(&inner.data)
                .ok()
                .and_then(|v| v.get("sessionIds")?.as_array().cloned())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        }
        Err(_) => return Ok(vec![]),
    };
    if streaming_session_ids.is_empty() {
        return Ok(vec![]);
    }
    let threads = store::list_threads()?;
    Ok(threads
        .into_iter()
        .filter(|t| {
            t.agent_session_id
                .as_deref()
                .is_some_and(|sid| streaming_session_ids.contains(sid))
        })
        .map(|t| t.id)
        .collect())
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
        eprintln!("FutureOS thread title sync failed for {thread_id}: {error}");
    }
}

#[tauri::command]
pub async fn get_thread_agent_state(
    thread_id: String,
) -> Result<serde_json::Value, crate::AppError> {
    let thread = store::get_thread(&thread_id)?.ok_or_else(|| "Thread not found.".to_string())?;
    let Some(session_id) = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(serde_json::json!({
            "model": null,
            "thinkingLevel": null,
            "session_name": thread.title,
            "sessionId": null,
            "cwd": null,
            "parentSessionId": null,
            "isStreaming": false,
        }));
    };

    // Agent unreachable or get_state failed: return an ERROR, not a null
    // payload. The frontend caches whatever this command returns; caching
    // fabricated nulls poisoned the composer with the global draft
    // model/thinking level for the whole TTL window. An error instead
    // rejects the fetch, leaving the last-known-good cache entry in place.
    let mut client = crate::agent_bridge::connect_agent()
        .await
        .map_err(|e| format!("Future Agent unreachable: {e}"))?;
    let cmd = crate::agent_bridge::get_state_command(session_id.to_string());
    let resp = client
        .execute_command(cmd)
        .await
        .map_err(|e| format!("get_state RPC failed: {e}"))?
        .into_inner();
    if !resp.success {
        return Err(format!("get_state rejected: {}", resp.error).into());
    }
    let value = serde_json::from_str::<serde_json::Value>(&resp.data)
        .map_err(|e| format!("get_state parse error: {e}"))?;
    // Converge the DB title toward the agent's session_name — the name shared
    // with every client (TUI `/name`, CLI, channels), whose renames never
    // reach the GUI DB. The sidebar treats the agent name as authoritative
    // and falls back to the DB title whenever agent state is unavailable, so
    // a stale DB title surfaces as "the rename didn't survive a restart".
    // Best-effort; sync_thread_title never bumps updated_at (sidebar order).
    if let Some(name) = value
        .get("session_name")
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

/// Fetch session entries from the agent (user, assistant, tool messages).
/// Used as the primary message source — SQLite messages are a fallback.
///
/// A thread without an agent session (no `agent_session_id`) has no entries yet.
/// Must not query the agent with the bare `thread_id`: the agent's
/// `get_session` fallback leaks the default session's entries into an
/// unrelated thread, cross-contaminating conversations.
#[tauri::command]
pub async fn get_session_entries(thread_id: String) -> Result<serde_json::Value, crate::AppError> {
    let thread = store::get_thread(&thread_id)?.ok_or_else(|| "Thread not found.".to_string())?;
    let session_id = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());

    // A thread with no agent session has no entries.  Never fall back to
    // `thread.id` as a session_id — the agent resolves unrecognised ids to
    // its default session, leaking another conversation's history.
    let Some(session_id) = session_id else {
        return Ok(serde_json::json!({ "entries": [] }));
    };

    let mut client = crate::agent_bridge::connect_agent()
        .await
        .map_err(|e| format!("Agent unavailable: {e}"))?;
    let cmd = crate::agent_bridge::get_session_entries_command(session_id.to_string());
    let resp = client
        .execute_command(cmd)
        .await
        .map_err(|e| format!("get_session_entries failed: {e}"))?
        .into_inner();
    if !resp.success {
        return Err(resp.error.into());
    }
    serde_json::from_str(&resp.data).map_err(|e| format!("Parse error: {e}").into())
}

#[tauri::command]
pub async fn clear_finished_runs(thread_id: String) -> Result<usize, crate::AppError> {
    let thread =
        store::get_thread(&thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let session_id = thread.agent_session_id.unwrap_or(thread.id);
    let terminal_runs = store::list_runs(&thread_id)?
        .into_iter()
        .filter(|run| matches!(run.status.as_str(), "completed" | "failed" | "cancelled"))
        .map(|run| run.id)
        .collect::<Vec<_>>();
    if !terminal_runs.is_empty() {
        let mut client = agent_bridge::connect_agent().await?;
        for run_id in terminal_runs {
            let response = client
                .execute_command(agent_bridge::prune_run_events_command(
                    session_id.clone(),
                    run_id,
                ))
                .await
                .map_err(|status| agent_bridge::map_rpc_error("Agent event prune failed", status))?
                .into_inner();
            if !response.success {
                return Err(format!("Agent event prune failed: {}", response.error).into());
            }
        }
    }
    store::clear_finished_runs(&thread_id)
}

#[tauri::command]
pub fn get_thread_cleanup_summary(
    thread_id: String,
) -> Result<store::ThreadCleanupSummary, crate::AppError> {
    store::get_thread_cleanup_summary(&thread_id)
}

/// Attach to a remote agent session stream: create a synthetic run and
/// subscribe to live events so the GUI shows real-time streaming content
/// for prompts initiated by other clients (TUI, CLI, phone).
#[tauri::command]
pub async fn attach_remote_stream(thread_id: String) -> Result<serde_json::Value, crate::AppError> {
    let run_id = crate::agent_bridge::attach_remote_stream(&thread_id).await?;
    Ok(serde_json::json!({ "runId": run_id }))
}

/// Ensure the session observer is live for this session. Observers are
/// long-lived per-session tasks (settings fan-out, event projection for runs
/// no pipeline owns, NATS mirroring) — this call is now an idempotent hint
/// (LRU touch), no longer a single-slot re-subscription. Safe to call on
/// every thread switch.
#[tauri::command]
pub fn observe_session(thread_id: String, session_id: String) -> Result<(), crate::AppError> {
    let thread_id = thread_id.trim();
    let session_id = session_id.trim();
    if thread_id.is_empty() || session_id.is_empty() {
        return Err("Both thread id and session id are required to observe a session.".into());
    }
    // This GUI command is deliberately single-target: it may wake or create
    // exactly the observer keyed by this already-owned session, never a global
    // stream or a different thread's observer.
    crate::agent_bridge::ensure_observer_for_thread(session_id, thread_id)
        .map_err(crate::AppError::from)
}

/// Move a thread to the workspace matching a new cwd (e.g. after TUI /cwd).
#[tauri::command]
pub fn reconcile_thread_workspace(session_id: String, cwd: String) -> Result<(), crate::AppError> {
    crate::agent_bridge::reconcile_thread_workspace(&session_id, &cwd)
        .map_err(crate::AppError::from)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
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

    #[test]
    fn async_command_wrappers_reject_malformed_bodies() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![
                fork_thread,
                rename_thread,
                update_thread_model,
                update_thread_thinking_level,
                delete_thread,
                batch_delete_threads,
                get_thread_agent_state,
                get_session_entries,
                clear_finished_runs,
                attach_remote_stream
            ],
            &[
                "fork_thread",
                "rename_thread",
                "update_thread_model",
                "update_thread_thinking_level",
                "delete_thread",
                "batch_delete_threads",
                "get_thread_agent_state",
                "get_session_entries",
                "clear_finished_runs",
                "attach_remote_stream",
            ],
        );
        // `fork_thread` takes three arguments, so the empty-body rejection only
        // exercises its *first* argument's error arm (attributed to the signature
        // line). Fail the *last* argument instead to hit the error arm attributed
        // to the `#[tauri::command]` attribute line.
        crate::commands::ipc_harness::assert_all_reject_bodies(
            tauri::generate_handler![fork_thread],
            &[(
                "fork_thread",
                serde_json::json!({ "threadId": "t", "userMessageContent": "c" }),
            )],
        );
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
        assert!(fork_thread("no-such-thread".into(), "x".into(), 0)
            .await
            .is_err());
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
        crate::commands::agent_mock::ensure_mock_agent();
        crate::commands::agent_mock::with_broken_endpoint(list_streaming_thread_ids)
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
    fn thread_read_and_pin_commands_round_trip() {
        let _home = init("cmd_threads");
        assert!(list_threads().expect("list empty").is_empty());
        assert!(get_recent_thread().expect("recent empty").is_none());

        let thread = make_thread(&_home, None);
        assert_eq!(list_threads().expect("list").len(), 1);
        assert_eq!(
            get_thread(thread.id.clone()).expect("get").map(|t| t.id),
            Some(thread.id.clone())
        );
        assert!(get_thread("ghost".into()).expect("ghost").is_none());
        assert_eq!(
            get_recent_thread().expect("recent").map(|t| t.id),
            Some(thread.id.clone())
        );

        let pinned = pin_thread(store::PinThreadInput {
            thread_id: thread.id.clone(),
            pinned: true,
        })
        .expect("pin");
        assert!(pinned.pinned);

        let archived = archive_thread(thread.id.clone()).expect("archive");
        assert_eq!(archived.status, "archived");

        let restored = restore_thread(thread.id.clone()).expect("restore");
        assert_eq!(restored.status, "active");
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
        crate::commands::agent_mock::ensure_mock_agent();
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
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            streaming_ids: vec!["sess_live".to_string()],
            ..Default::default()
        });
        let ids = list_streaming_thread_ids().await.expect("streaming");
        assert_eq!(ids, vec![thread.id]);
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
        assert_eq!(value["session_name"], serde_json::json!(thread.title));
    }

    #[tokio::test]
    async fn get_thread_agent_state_errors_for_unknown_thread() {
        let _home = init("cmd_agent_state_ghost");
        assert!(get_thread_agent_state("ghost".into()).await.is_err());
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
    async fn rename_thread_maps_transport_errors() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_rename_xport");
        let thread = make_thread(&_home, Some("sess_rename_x"));
        crate::commands::agent_mock::ensure_mock_agent();
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
        crate::commands::agent_mock::ensure_mock_agent();
        // The agent never acknowledges the delete, so the tombstone row
        // survives and the observer-drop arm runs.
        script_mock_agent(MockScript {
            transport_fail: ["delete_session".to_string()].into_iter().collect(),
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
        crate::commands::agent_mock::ensure_mock_agent();
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
    async fn rename_thread_propagates_to_the_agent_first() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_rename");
        let thread = make_thread(&_home, Some("sess_rename"));
        crate::commands::agent_mock::ensure_mock_agent();
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
        crate::commands::agent_mock::ensure_mock_agent();
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

    #[tokio::test]
    async fn clear_finished_runs_without_terminal_runs_skips_the_agent() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_clear_runs");
        let thread = make_thread(&_home, None);
        let cleared = clear_finished_runs(thread.id.clone()).await.expect("clear");
        assert_eq!(cleared, 0);
    }

    #[tokio::test]
    async fn rename_thread_tolerates_a_session_not_found_rejection() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_rename_notfound");
        let thread = make_thread(&_home, Some("sess_nf"));
        crate::commands::agent_mock::ensure_mock_agent();
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
        crate::commands::agent_mock::ensure_mock_agent();
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
        crate::commands::agent_mock::ensure_mock_agent();
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
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("delete_session".to_string(), "{}".to_string())]),
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
    async fn batch_delete_threads_delegates() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_batch_delete");
        let thread = make_thread(&_home, None);
        crate::commands::agent_mock::ensure_mock_agent();
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
    async fn get_thread_agent_state_reads_and_syncs_the_agent_title() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_agent_state_ok");
        let thread = make_thread(&_home, Some("sess_state"));
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "get_state".to_string(),
                "{\"session_name\":\"Agent Name\",\"model\":\"m\"}".to_string(),
            )]),
            ..Default::default()
        });
        let value = get_thread_agent_state(thread.id.clone())
            .await
            .expect("state");
        assert_eq!(value["session_name"], serde_json::json!("Agent Name"));
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
        crate::commands::agent_mock::ensure_mock_agent();
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
    async fn clear_finished_runs_prunes_terminal_runs_via_the_agent() {
        let _lock = mock_agent_lock();
        let _home = init("cmd_clear_runs_terminal");
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

        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("prune_run_events".to_string(), "{}".to_string())]),
            ..Default::default()
        });
        let cleared = clear_finished_runs(thread.id.clone()).await.expect("clear");
        assert_eq!(cleared, 1);
        script_mock_agent(MockScript::default());
    }
}
