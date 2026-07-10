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
pub fn delete_thread(thread_id: String) -> Result<store::ThreadRecord, crate::AppError> {
    store::delete_thread(&thread_id)
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
