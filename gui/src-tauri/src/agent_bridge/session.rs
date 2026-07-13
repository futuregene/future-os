//! Agent session lifecycle: ensure/create a session for a thread, set its
//! permission level, and resolve a thread's workspace path and prior-message
//! count. These back the per-prompt setup in the parent module.

use tonic::transport::Channel;

use super::client::{
    fork_command, get_session_entries_command, get_state_command, new_session_command,
    set_cwd_command, set_permission_level_command, set_sandbox_policy_command, RpcResponseExt,
};
use crate::{agent_proto::FutureAgentClient, store};

/// Ensure an agent session exists for the given thread. Returns the session
/// id (the existing one, or the newly-created one if the agent generated it).
/// `model_id` and `thinking_level` are applied to newly-created sessions so
/// the agent starts with the user's selection immediately.
pub(super) async fn ensure_agent_session(
    client: &mut FutureAgentClient<Channel>,
    session_id: &str,
    cwd: &str,
    model_id: Option<&str>,
    thinking_level: Option<&str>,
) -> Result<String, crate::AppError> {
    // If the thread already has a stored session id, check if it's still valid.
    if !session_id.is_empty() {
        let response = client
            .execute_command(get_state_command(session_id.to_string()))
            .await
            .map_err(|error| format!("Unable to inspect Future Agent session: {error}"))?
            .into_inner();

        if response.success {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.data) {
                let active_id = value
                    .get("sessionId")
                    .and_then(|id| id.as_str())
                    .unwrap_or_default();
                let active_cwd = value
                    .get("cwd")
                    .and_then(|cwd| cwd.as_str())
                    .unwrap_or_default();
                if active_id == session_id && active_cwd == cwd {
                    return Ok(session_id.to_string());
                }
            }
        }
    }

    // Create a new session. Pass empty session_id to let the agent generate it.
    let resp = client
        .execute_command(new_session_command(
            String::new(),
            cwd.to_string(),
            "gui",
            serde_json::Value::Null,
            model_id.map(str::to_string),
            thinking_level.map(str::to_string),
        ))
        .await
        .map_err(|error| format!("Unable to create Future Agent session: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the session initialization.")?;

    let new_id = serde_json::from_str::<serde_json::Value>(&resp.data)
        .ok()
        .and_then(|v| v.get("sessionId").cloned())
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();

    Ok(new_id)
}

pub(super) async fn set_agent_permission_level(
    client: &mut FutureAgentClient<Channel>,
    session_id: &str,
    level: &str,
) -> Result<(), crate::AppError> {
    client
        .execute_command(set_permission_level_command(
            level.to_string(),
            session_id.to_string(),
        ))
        .await
        .map_err(|error| format!("Unable to set Future Agent permission level: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the permission level selection.")?;
    Ok(())
}

/// Push the session's approval tier to the agent. The agent reads the rule
/// files (`${WS}/.future/approval_rule.json`, `~/.future/approval_rule.json`)
/// directly — only the tier travels over the wire (APPROVAL_PLAN.md):
/// `"manual"` (ask), `"sandbox"` (macOS Seatbelt wraps bash), or `"off"`
/// (fully open). The tier is a global app preference, defaulting to `"manual"`.
pub(super) async fn set_agent_sandbox_policy(
    client: &mut FutureAgentClient<Channel>,
    session_id: &str,
    _thread_id: &str,
) -> Result<(), crate::AppError> {
    let tier = store::get_app_settings()
        .map(|settings| settings.approval_tier)
        .unwrap_or_else(|_| "off".to_string());
    let policy = crate::agent_proto::SandboxPolicy { tier };
    client
        .execute_command(set_sandbox_policy_command(policy, session_id.to_string()))
        .await
        .map_err(|error| format!("Unable to set Future Agent sandbox policy: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the sandbox policy.")?;
    Ok(())
}

pub(super) fn workspace_path_for_thread(thread_id: &str) -> Result<String, crate::AppError> {
    let thread =
        store::get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let workspace = store::get_workspace(&thread.workspace_id)?
        .ok_or_else(|| "Thread workspace could not be loaded.".to_string())?;
    Ok(workspace.path)
}

/// Returns `true` when the thread is a chat-mode thread (not workspace-bound).
pub(super) fn is_chat_thread(thread_id: &str) -> bool {
    store::get_thread(thread_id)
        .ok()
        .flatten()
        .map(|t| t.mode == "chat")
        .unwrap_or(true)
}

/// Fork a session at the given user message. Returns the new GUI thread id.
///
/// Creates a dedicated chat workspace named after the forked session id, copies
/// thread metadata from the parent, and creates per-turn completed run records
/// so the right panel is populated immediately.  Messages are served from the
/// agent JSONL (no SQLite `messages` table), so no message import is needed.
pub async fn fork_agent_session(
    thread_id: &str,
    user_message_content: &str,
    // 0-based ordinal of the user message among all user messages. The GUI
    // renders exactly one message per user entry in order, so the Nth user
    // message maps to the Nth user entry — matching by ordinal instead of
    // content means two identical prompts ("continue", "run the tests") fork the
    // intended turn, not the first occurrence. `< 0` (unknown) falls back to
    // content matching.
    user_message_index: i64,
) -> Result<String, crate::AppError> {
    let thread =
        store::get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let session_id = thread
        .agent_session_id
        .ok_or_else(|| "No agent session for this thread.".to_string())?;

    let mut client = super::client::connect_agent().await?;

    // ── find the fork point ────────────────────────────────────────────

    let response = client
        .execute_command(get_session_entries_command(session_id.clone()))
        .await
        .map_err(|error| format!("Unable to list session entries: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the session-entries request.")?;

    let entries: Vec<serde_json::Value> = serde_json::from_str::<serde_json::Value>(&response.data)
        .ok()
        .and_then(|v| v.get("entries").cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let is_user = |e: &serde_json::Value| e.get("role").and_then(|r| r.as_str()) == Some("user");

    // Prefer the user-message ordinal; fall back to content when it's unknown
    // (< 0) or out of range.
    let match_idx = usize::try_from(user_message_index)
        .ok()
        .and_then(|nth| {
            entries
                .iter()
                .enumerate()
                .filter(|(_, e)| is_user(e))
                .nth(nth)
                .map(|(i, _)| i)
        })
        .or_else(|| {
            entries.iter().position(|e| {
                is_user(e)
                    && e.get("content")
                        .and_then(|c| c.as_str())
                        .is_some_and(|c| c.trim() == user_message_content.trim())
            })
        })
        .ok_or_else(|| "No matching user message found in agent session.".to_string())?;

    let mut fork_idx = match_idx;
    for (i, entry) in entries.iter().enumerate().skip(match_idx + 1) {
        let role = entry.get("role").and_then(|r| r.as_str()).unwrap_or("");
        fork_idx = i;
        if role == "user" {
            fork_idx = i - 1;
            break;
        }
    }
    let entry_id = entries[fork_idx]
        .get("id")
        .and_then(|id| id.as_str())
        .ok_or_else(|| "No fork entry found.".to_string())?;

    // ── call agent fork RPC ────────────────────────────────────────────

    let fork_response = client
        .execute_command(fork_command(
            session_id.clone(),
            entry_id.to_string(),
            session_id.clone(),
        ))
        .await
        .map_err(|error| format!("Unable to fork session: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the fork request.")?;

    let new_session_id = serde_json::from_str::<serde_json::Value>(&fork_response.data)
        .ok()
        .and_then(|v| v.get("sessionId").cloned())
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();

    if new_session_id.is_empty() {
        return Err("Fork did not return a session.".into());
    }

    // ── read forked entries for metadata ───────────────────────────────

    let entries_response = client
        .execute_command(get_session_entries_command(new_session_id.clone()))
        .await
        .map_err(|error| format!("Unable to list fork session entries: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the fork-session entries request.")?;

    let fork_entries: Vec<serde_json::Value> =
        serde_json::from_str::<serde_json::Value>(&entries_response.data)
            .ok()
            .and_then(|v| v.get("entries").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

    // The agent's fork_session sets these in the session_info entry.
    // The agent's fork_session writes metadata into a session_info entry
    // (role = "system").  Find it — get_session_entries now includes it.
    let session_info = fork_entries
        .iter()
        .find(|e| e.get("role").and_then(|r| r.as_str()) == Some("system"));
    let session_name = session_info
        .and_then(|e| e.get("content"))
        .and_then(|c| c.get("session_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("(fork)")
        .to_string();
    let session_model = session_info
        .and_then(|e| e.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();

    let assistant_count = fork_entries
        .iter()
        .filter(|e| e.get("role").and_then(|r| r.as_str()) == Some("assistant"))
        .count();

    // ── create workspace + thread ──────────────────────────────────────

    let (workspace_id, _cwd) = if thread.mode == "chat" {
        let ws = store::get_or_create_chat_workspace(&new_session_id, Some(session_name.clone()))?;
        let dir = store::chat_workspace_path(&new_session_id).map(|p| p.display().to_string())?;
        std::fs::create_dir_all(&dir)?;
        // Tell the agent the correct cwd (the forked session inherited the
        // parent's cwd from its session_info).
        if let Err(e) = client
            .execute_command(set_cwd_command(dir.clone(), new_session_id.clone()))
            .await
        {
            eprintln!("FutureOS: fork set_cwd failed: {e}");
        }
        (Some(ws.id), dir)
    } else {
        // Workspace thread — keep the parent's project directory as cwd.
        (Some(thread.workspace_id.clone()), String::new())
    };

    let new_thread = store::create_thread(store::CreateThreadInput {
        mode: thread.mode.clone(),
        title: Some(session_name),
        workspace_id,
        workspace_path: None,
        workspace_name: None,
        agent_session_id: Some(new_session_id.clone()),
    })?;

    let (provider, model_id) = split_model(&session_model);
    for _ in 0..assistant_count.max(1) {
        let run = store::create_run(store::CreateRunInput {
            thread_id: new_thread.id.clone(),
            trigger_message_id: None,
            model_provider: provider.clone(),
            model_id: model_id.clone(),
        })?;
        let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
            run_id: run.id,
            status: "completed".to_string(),
            error_message: None,
            error_type: None,
        });
    }

    Ok(new_thread.id)
}

pub(super) fn split_model(model: &str) -> (Option<String>, Option<String>) {
    if model.is_empty() {
        return (None, None);
    }
    if let Some((provider, id)) = model.split_once('/') {
        (Some(provider.to_string()), Some(id.to_string()))
    } else {
        (None, Some(model.to_string()))
    }
}
