//! Thread lifecycle Tauri commands plus thread-scoped cleanup queries.

use crate::store;

use super::workspaces::ensure_workspace_git_repo;

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
    let thread = store::create_thread(input)?;
    if thread.mode == "workspace" {
        if let Ok(Some(workspace)) = store::get_workspace(&thread.workspace_id) {
            ensure_workspace_git_repo(&workspace);
        }
    }
    Ok(thread)
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
