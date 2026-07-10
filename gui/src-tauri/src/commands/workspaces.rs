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
pub async fn delete_workspace(
    workspace_id: String,
) -> Result<store::WorkspaceRecord, crate::AppError> {
    // Resolve every thread's agent session BEFORE the rows are gone.
    let session_ids = store::workspace_agent_session_ids(&workspace_id)?;
    // Hard-delete the workspace, its threads, and all their child rows.
    let workspace = store::delete_workspace(&workspace_id)?;
    // Best-effort: delete each thread's agent JSONL (the source of truth).
    if let Ok(mut client) = crate::agent_bridge::connect_agent().await {
        for session_id in session_ids {
            let trimmed = session_id.trim();
            if trimmed.is_empty() {
                continue;
            }
            let cmd = crate::agent_bridge::delete_session_command(trimmed.to_string());
            let _ = client.execute_command(cmd).await;
        }
    }
    // Physically reclaim the now-orphaned GUI dirs: the workspace's shadow-review
    // repo and each thread's image/chat-scratch dir. These key off DB presence,
    // which we just cleared. The user's own workspace files (at `workspace.path`,
    // never under ~/.future/app) are NEVER touched.
    let _ = store::reconcile_orphan_review_repos();
    let _ = store::reconcile_orphan_images();
    let _ = store::reconcile_orphan_chat_workspaces();
    Ok(workspace)
}
