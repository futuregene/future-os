//! Thread lifecycle Tauri commands plus thread-scoped cleanup queries.

use crate::{agent_bridge, store};

#[tauri::command]
pub async fn fork_thread(
    thread_id: String,
    user_message_content: String,
) -> Result<String, crate::AppError> {
    agent_bridge::fork_agent_session(&thread_id, &user_message_content).await
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
pub fn rename_thread(
    input: store::RenameThreadInput,
) -> Result<store::ThreadRecord, crate::AppError> {
    store::rename_thread(input)
}

#[tauri::command]
pub fn update_thread_model(
    input: store::UpdateThreadModelInput,
) -> Result<store::ThreadRecord, crate::AppError> {
    store::update_thread_model(input)
}

#[tauri::command]
pub fn update_thread_thinking_level(
    input: store::UpdateThreadThinkingLevelInput,
) -> Result<store::ThreadRecord, crate::AppError> {
    store::update_thread_thinking_level(input)
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

/// Fetch a thread's session state from the agent (model, thinking, name, cwd).
/// Falls back to the stored DB values when the agent is unreachable.
#[tauri::command]
pub async fn get_thread_agent_state(
    thread_id: String,
) -> Result<serde_json::Value, crate::AppError> {
    let thread = store::get_thread(&thread_id)?
        .ok_or_else(|| "Thread not found.".to_string())?;
    let session_id = thread.agent_session_id.as_deref().unwrap_or(&thread.id);

    if let Ok(mut client) = crate::agent_bridge::connect_agent().await {
        let cmd = crate::agent_bridge::get_state_command(session_id.to_string());
        if let Ok(resp) = client.execute_command(cmd).await {
            let inner = resp.into_inner();
            if inner.success {
                if let Ok(data) =
                    serde_json::from_str::<serde_json::Value>(&inner.data)
                {
                    return Ok(data);
                }
            }
        }
    }

    // Fallback: agent unreachable — return null for model/thinking.
    Ok(serde_json::json!({
        "model": null,
        "thinkingLevel": null,
        "sessionName": thread.title,
        "cwd": null,
        "parentSessionId": null,
    }))
}

/// Fetch session entries from the agent (user, assistant, tool messages).
/// Used as the primary message source — SQLite messages are a fallback.
#[tauri::command]
pub async fn get_session_entries(
    thread_id: String,
) -> Result<serde_json::Value, crate::AppError> {
    let thread = store::get_thread(&thread_id)?
        .ok_or_else(|| "Thread not found.".to_string())?;
    let session_id = thread.agent_session_id.as_deref().unwrap_or(&thread.id);

    let mut client = crate::agent_bridge::connect_agent().await
        .map_err(|e| format!("Agent unavailable: {e}"))?;
    let cmd = crate::agent_bridge::get_session_entries_command(session_id.to_string());
    let resp = client.execute_command(cmd).await
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
