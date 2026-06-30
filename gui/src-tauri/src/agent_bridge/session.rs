//! Agent session lifecycle: ensure/create a session for a thread, set its
//! permission level, and resolve a thread's workspace path and prior-message
//! count. These back the per-prompt setup in the parent module.

use tonic::transport::Channel;

use super::client::{
    get_state_command, new_session_command, set_permission_level_command, RpcResponseExt,
};
use crate::{agent_proto::FutureAgentClient, store};

pub(super) async fn ensure_agent_session(
    client: &mut FutureAgentClient<Channel>,
    session_id: &str,
    cwd: &str,
    force_reset: bool,
) -> Result<(), crate::AppError> {
    if force_reset {
        return create_agent_session(client, session_id, cwd).await;
    }

    let response = client
        .execute_command(get_state_command(session_id.to_string()))
        .await
        .map_err(|error| format!("Unable to inspect Future Agent session: {error}"))?
        .into_inner();

    if response.success {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.data) {
            let active_session_id = value
                .get("sessionId")
                .and_then(|session_id| session_id.as_str())
                .unwrap_or_default();
            let active_cwd = value
                .get("cwd")
                .and_then(|cwd| cwd.as_str())
                .unwrap_or_default();

            if active_session_id == session_id && active_cwd == cwd {
                return Ok(());
            }
        }
    }

    create_agent_session(client, session_id, cwd).await
}

async fn create_agent_session(
    client: &mut FutureAgentClient<Channel>,
    session_id: &str,
    cwd: &str,
) -> Result<(), crate::AppError> {
    client
        .execute_command(new_session_command(session_id.to_string(), cwd.to_string()))
        .await
        .map_err(|error| format!("Unable to create Future Agent session: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the session initialization.")?;
    Ok(())
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

pub(super) fn prior_user_message_count(thread_id: &str) -> Result<usize, crate::AppError> {
    let messages = store::list_messages(thread_id)?;
    let user_message_count = messages
        .iter()
        .filter(|message| message.role == "user")
        .count();
    Ok(user_message_count.saturating_sub(1))
}

pub(super) fn workspace_path_for_thread(thread_id: &str) -> Result<String, crate::AppError> {
    let thread =
        store::get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let workspace = store::get_workspace(&thread.workspace_id)?
        .ok_or_else(|| "Thread workspace could not be loaded.".to_string())?;
    Ok(workspace.path)
}
