//! Agent model listing and prompting Tauri commands.

use crate::agent_bridge;

#[tauri::command]
pub async fn list_agent_models() -> Result<Vec<agent_bridge::AgentModelOption>, crate::AppError> {
    agent_bridge::list_agent_models().await
}

#[tauri::command]
pub async fn agent_prompt(
    message: String,
    image_paths: Option<Vec<String>>,
    thread_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<agent_bridge::AgentPromptResponse, crate::AppError> {
    agent_bridge::agent_prompt(
        message,
        image_paths,
        thread_id,
        session_id,
        run_id,
        model_id,
        thinking_level,
    )
    .await
}
