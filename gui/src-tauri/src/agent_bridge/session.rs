//! Agent session lifecycle: ensure/create a session for a thread, set its
//! permission level, and resolve a thread's workspace path and prior-message
//! count. These back the per-prompt setup in the parent module.

use tonic::transport::Channel;

use super::client::{
    fork_command, get_session_entries_command, get_state_command, new_session_command,
    set_permission_level_command, set_sandbox_policy_command, RpcResponseExt,
};
use crate::{agent_proto::FutureAgentClient, store};

/// Ensure an agent session exists for the given thread. Returns the session
/// id (the existing one, or the newly-created one if the agent generated it).
pub(super) async fn ensure_agent_session(
    client: &mut FutureAgentClient<Channel>,
    session_id: &str,
    cwd: &str,
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
        .unwrap_or_else(|_| "manual".to_string());
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

/// Fork a session at the given user message. Returns the new agent session id.
/// Also imports the forked session history as GUI messages so the new thread
/// displays the preserved conversation immediately.
/// The user message is matched by content text against agent entries.
pub async fn fork_agent_session(
    thread_id: &str,
    user_message_content: &str,
) -> Result<String, crate::AppError> {
    let thread = store::get_thread(thread_id)?
        .ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let session_id = thread
        .agent_session_id
        .ok_or_else(|| "No agent session for this thread.".to_string())?;

    let mut client = super::client::connect_agent().await?;

    // Get all session entries to find the entry matching the user message
    // and determine the fork point (the entry right after the matched one,
    // so the AI response is included in the preserved history).
    let response = client
        .execute_command(get_session_entries_command(session_id.clone()))
        .await
        .map_err(|error| format!("Unable to list session entries: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the session-entries request.")?;

    let entries: Vec<serde_json::Value> =
        serde_json::from_str::<serde_json::Value>(&response.data)
            .ok()
            .and_then(|v| v.get("entries").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

    // Find the user message entry matching the content, then fork from the
    // *next* entry after it (so the AI response following this user message
    // is included in the forked history).
    let match_idx = entries
        .iter()
        .position(|e| {
            e.get("content")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.trim() == user_message_content.trim())
                && e.get("role").and_then(|r| r.as_str()) == Some("user")
        })
        .ok_or_else(|| "No matching user message found in agent session.".to_string())?;

    // Walk forward past all entries of this turn (assistant + tool) until
    // the next user message to include the full response in the preserved
    // history. A single user turn may produce many assistant→tool→assistant
    // cycles before the final text response.
    let mut fork_idx = match_idx;
    for i in (match_idx + 1)..entries.len() {
        let role = entries[i].get("role").and_then(|r| r.as_str()).unwrap_or("");
        fork_idx = i;
        // Stop when we hit the next user message — fork just before it.
        if role == "user" {
            fork_idx = i - 1;
            break;
        }
    }
    let entry_id = entries[fork_idx]
        .get("id")
        .and_then(|id| id.as_str())
        .ok_or_else(|| "No fork entry found.".to_string())?;

    // Fork the session at that entry.
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
        .unwrap_or_else(|| String::new());

    if new_session_id.is_empty() {
        return Err("Fork did not return a session.".into());
    }

    // Read the forked session's entries and import them as GUI messages
    // so the new thread shows the preserved conversation immediately.
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

    // Create the GUI thread bound to the forked agent session.
    let new_thread = store::create_thread(store::CreateThreadInput {
        mode: thread.mode.clone(),
        title: Some(format!("{} (fork)", thread.title)),
        workspace_id: Some(thread.workspace_id.clone()),
        workspace_path: None,
        workspace_name: None,
        model_provider: thread.model_provider.clone(),
        model_id: thread.model_id.clone(),
        thinking_level: thread.thinking_level.clone(),
        agent_session_id: Some(new_session_id.clone()),
    })?;

    // Import entries grouped by user turn. Each turn produces one user
    // message and one assistant message (merging tool calls inline), so the
    // forked thread looks like the original session.
    let mut turn_user: Option<&serde_json::Value> = None;
    let mut turn_tools: Vec<String> = Vec::new();
    let mut turn_final_text = String::new();

    for entry in &fork_entries {
        let role = entry
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("");
        let content = entry
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("");

        if role == "user" {
            // Flush the previous turn.
            if let Some(user) = turn_user {
                let user_text = user
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                if !user_text.trim().is_empty() {
                    let _ = store::append_message(store::AppendMessageInput {
                        thread_id: new_thread.id.clone(),
                        run_id: None,
                        role: "user".to_string(),
                        content_type: Some("markdown".to_string()),
                        content: user_text.trim().to_string(),
                        status: Some("complete".to_string()),
                    });
                }
                if !turn_final_text.trim().is_empty() || !turn_tools.is_empty() {
                    let mut merged = turn_final_text.trim().to_string();
                    if !turn_tools.is_empty() {
                        if !merged.is_empty() {
                            merged.push_str("\n\n");
                        }
                        merged.push_str(&format!(
                            "<details><summary>🔧 tool calls</summary>\n\n{}\n</details>",
                            turn_tools.join("\n")
                        ));
                    }
                    if !merged.trim().is_empty() {
                        let _ = store::append_message(store::AppendMessageInput {
                            thread_id: new_thread.id.clone(),
                            run_id: None,
                            role: "assistant".to_string(),
                            content_type: Some("markdown".to_string()),
                            content: merged,
                            status: Some("complete".to_string()),
                        });
                    }
                }
            }
            turn_user = Some(entry);
            turn_tools.clear();
            turn_final_text.clear();
        } else if role == "tool" {
            let name = entry
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("tool");
            let args = entry
                .get("tool_args")
                .and_then(|a| a.as_str())
                .unwrap_or("");
            let result_preview = if content.len() > 200 {
                format!("{}…", &content[..200])
            } else {
                content.to_string()
            };
            turn_tools.push(format!("- `{}` {} → {}", name, args, result_preview));
        } else if role == "assistant" {
            if !content.trim().is_empty() {
                turn_final_text = content.to_string();
            }
        }
    }

    // Flush the last turn.
    if let Some(user) = turn_user {
        let user_text = user
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("");
        if !user_text.trim().is_empty() {
            let _ = store::append_message(store::AppendMessageInput {
                thread_id: new_thread.id.clone(),
                run_id: None,
                role: "user".to_string(),
                content_type: Some("markdown".to_string()),
                content: user_text.trim().to_string(),
                status: Some("complete".to_string()),
            });
        }
        if !turn_final_text.trim().is_empty() || !turn_tools.is_empty() {
            let mut merged = turn_final_text.trim().to_string();
            if !turn_tools.is_empty() {
                if !merged.is_empty() {
                    merged.push_str("\n\n");
                }
                merged.push_str(&format!(
                    "<details><summary>🔧 tool calls</summary>\n\n{}\n</details>",
                    turn_tools.join("\n")
                ));
            }
            if !merged.trim().is_empty() {
                let _ = store::append_message(store::AppendMessageInput {
                    thread_id: new_thread.id.clone(),
                    run_id: None,
                    role: "assistant".to_string(),
                    content_type: Some("markdown".to_string()),
                    content: merged,
                    status: Some("complete".to_string()),
                });
            }
        }
    }

    Ok(new_thread.id)
}
