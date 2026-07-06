//! Workspace Tauri commands. FutureOS no longer auto-`git init`s Workspace
//! directories (§14.3) — `git_review` only *detects* real git repos, and the
//! shadow review pipeline supplies "previous turn changes" for non-git Workspaces.

use crate::{git_review, store};

#[tauri::command]
pub fn list_workspaces() -> Result<Vec<store::WorkspaceRecord>, crate::AppError> {
    store::list_workspaces()
}

#[tauri::command]
pub fn create_workspace(
    input: store::CreateWorkspaceInput,
) -> Result<store::WorkspaceRecord, crate::AppError> {
    store::create_workspace(input)
}

/// Reports whether a user Workspace directory is a real git repo. Kept for the
/// existing frontend call site; it no longer initialises anything (§14.3).
#[tauri::command]
pub fn ensure_workspace_git(workspace_id: String) -> Result<bool, crate::AppError> {
    let workspace = store::get_workspace(&workspace_id)?
        .ok_or_else(|| "Workspace could not be loaded.".to_string())?;
    if workspace.kind != "user" {
        return Ok(false);
    }
    Ok(git_review::is_git_workspace(std::path::Path::new(
        &workspace.path,
    )))
}

#[tauri::command]
pub fn get_or_create_chat_workspace(
    thread_id: String,
    title: Option<String>,
) -> Result<store::WorkspaceRecord, crate::AppError> {
    store::get_or_create_chat_workspace(&thread_id, title)
}

#[tauri::command]
pub fn rename_workspace(
    input: store::RenameWorkspaceInput,
) -> Result<store::WorkspaceRecord, crate::AppError> {
    store::rename_workspace(input)
}

#[tauri::command]
pub fn delete_workspace(workspace_id: String) -> Result<store::WorkspaceRecord, crate::AppError> {
    store::delete_workspace(&workspace_id)
}
