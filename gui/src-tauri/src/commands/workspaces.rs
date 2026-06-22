//! Workspace Tauri commands. Creating a user workspace (or a workspace-mode
//! thread) initialises a git repo on disk so the review tooling has history to
//! diff against.

use crate::{git_review, store};

#[tauri::command]
pub fn list_workspaces() -> Result<Vec<store::WorkspaceRecord>, crate::AppError> {
    store::list_workspaces()
}

#[tauri::command]
pub fn create_workspace(
    input: store::CreateWorkspaceInput,
) -> Result<store::WorkspaceRecord, crate::AppError> {
    let workspace = store::create_workspace(input)?;
    ensure_workspace_git_repo(&workspace);
    Ok(workspace)
}

#[tauri::command]
pub fn ensure_workspace_git(workspace_id: String) -> Result<bool, crate::AppError> {
    let workspace = store::get_workspace(&workspace_id)?
        .ok_or_else(|| "Workspace could not be loaded.".to_string())?;
    if workspace.kind != "user" {
        return Ok(false);
    }
    Ok(git_review::ensure_git_init(std::path::Path::new(
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

/// Initialise git for a user workspace whose directory is not tracked yet.
/// No-op for temporary chat workspaces and when git is not installed.
pub(crate) fn ensure_workspace_git_repo(workspace: &store::WorkspaceRecord) {
    if workspace.kind != "user" {
        return;
    }
    git_review::ensure_git_init(std::path::Path::new(&workspace.path));
}
