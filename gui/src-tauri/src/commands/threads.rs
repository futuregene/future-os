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
    let title = input.title.clone();
    let thread = store::rename_thread(input)?;
    // Propagate to the agent immediately so the session name stays in sync
    // (best-effort — a failure here must not fail the local rename).
    let session_id = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(&thread.id)
        .to_string();
    if let Ok(mut client) = crate::agent_bridge::connect_agent().await {
        let cmd = crate::agent_bridge::set_session_name_command(title, session_id);
        let _ = client.execute_command(cmd).await;
    }
    Ok(thread)
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
pub async fn delete_thread(thread_id: String) -> Result<store::ThreadRecord, crate::AppError> {
    let thread = store::delete_thread(&thread_id)?;
    // Also delete the agent session JSONL (the source of truth) so the mirror and
    // the agent stay consistent. The session id mirrors the GUI's own resolution:
    // agent_session_id when set, else the thread id. Best-effort — a failure here
    // must not fail the local delete.
    let session_id = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(&thread.id)
        .to_string();
    if let Ok(mut client) = crate::agent_bridge::connect_agent().await {
        let cmd = crate::agent_bridge::delete_session_command(session_id);
        let _ = client.execute_command(cmd).await;
    }
    Ok(thread)
}

/// Bulk streaming-status query: ONE agent RPC (`list_streaming_sessions`,
/// which only scans the agent's in-memory session map — no hydration, no
/// disk I/O) mapped back to GUI thread ids via the stored agent_session_id.
/// Replaces the old per-thread get_state fan-out, which hydrated every
/// polled session on the agent at startup.
///
/// Agent unreachable → empty list (nothing shows as streaming); callers
/// self-heal on the next poll tick.
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
    serde_json::from_str::<serde_json::Value>(&resp.data)
        .map_err(|e| format!("get_state parse error: {e}").into())
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
pub fn clear_finished_runs(thread_id: String) -> Result<usize, crate::AppError> {
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
pub async fn attach_remote_stream(
    thread_id: String,
) -> Result<serde_json::Value, crate::AppError> {
    let run_id = crate::agent_bridge::attach_remote_stream(&thread_id).await?;
    Ok(serde_json::json!({ "runId": run_id }))
}

/// Start observing a session's settings changes in the background.  The agent
/// broadcasts model_changed, thinking_level_changed, etc. via StreamEvents;
/// this command subscribes to those events and forwards them to the frontend.
/// Call on every thread switch — old observation is automatically cancelled.
#[tauri::command]
pub fn observe_session(session_id: String) {
    crate::agent_bridge::start_observing_session(session_id);
}

/// Move a thread to the workspace matching a new cwd (e.g. after TUI /cwd).
#[tauri::command]
pub fn reconcile_thread_workspace(
    session_id: String,
    cwd: String,
) -> Result<(), crate::AppError> {
    crate::agent_bridge::reconcile_thread_workspace(&session_id, &cwd)
        .map_err(crate::AppError::from)
}
