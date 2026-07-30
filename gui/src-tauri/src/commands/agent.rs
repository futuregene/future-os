//! Agent model listing and prompting Tauri commands.

use crate::agent_bridge;

#[tauri::command]
pub async fn list_agent_models() -> Result<Vec<agent_bridge::AgentModelOption>, crate::AppError> {
    agent_bridge::list_agent_models().await
}

/// Post-login init: synchronously fetch the Future provider's models in the
/// agent (warming its cache + rebuilding its registry) so the model list is
/// complete before the onboarding gate closes. See [`agent_bridge::sync_future_models`].
#[tauri::command]
pub async fn sync_future_models() -> Result<agent_bridge::SyncFutureModelsResult, crate::AppError> {
    agent_bridge::sync_future_models().await
}

#[tauri::command]
pub async fn agent_prompt(
    message: String,
    attachments: Option<Vec<agent_bridge::AttachmentInput>>,
    thread_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<agent_bridge::AgentPromptResponse, crate::AppError> {
    agent_bridge::agent_prompt(
        message,
        attachments,
        thread_id,
        session_id,
        run_id,
        model_id,
        thinking_level,
    )
    .await
}
