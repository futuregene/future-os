//! Run and tool-call Tauri commands. `abort_run` delegates its agent + store
//! orchestration to [`crate::agent_bridge`].

use crate::{agent_bridge, store};

#[tauri::command]
pub fn create_run(input: store::CreateRunInput) -> Result<store::RunRecord, crate::AppError> {
    store::create_run(input)
}

#[tauri::command]
pub fn list_runs(thread_id: String) -> Result<Vec<store::RunRecord>, crate::AppError> {
    store::list_runs(&thread_id)
}

#[tauri::command]
pub fn update_run_status(
    input: store::UpdateRunStatusInput,
) -> Result<store::RunRecord, crate::AppError> {
    store::update_run_status(input)
}

#[tauri::command]
pub async fn abort_run(
    thread_id: String,
    run_id: String,
) -> Result<store::RunRecord, crate::AppError> {
    agent_bridge::abort_run(thread_id, run_id).await
}

#[tauri::command]
pub fn list_run_events(run_id: String) -> Result<Vec<store::RunEventRecord>, crate::AppError> {
    store::list_run_events(&run_id)
}

#[tauri::command]
pub fn list_tool_calls(run_id: String) -> Result<Vec<store::ToolCallRecord>, crate::AppError> {
    store::list_tool_calls(&run_id)
}

#[tauri::command]
pub fn list_tool_outputs(
    tool_call_id: String,
) -> Result<Vec<store::ToolOutputRecord>, crate::AppError> {
    store::list_tool_outputs(&tool_call_id)
}
