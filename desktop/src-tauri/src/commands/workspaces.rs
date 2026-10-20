//! Workspace IPC adaptation for the shared application services.

use crate::store;

#[cfg_attr(feature = "gui", tauri::command)]
pub fn list_workspaces() -> Result<Vec<store::WorkspaceRecord>, crate::AppError> {
    future_host::workspaces::list_workspaces()
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn create_workspace(
    input: store::CreateWorkspaceInput,
) -> Result<store::WorkspaceRecord, crate::AppError> {
    future_host::workspaces::create_workspace(input)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn ensure_workspace_git(workspace_id: String) -> Result<bool, crate::AppError> {
    future_host::workspaces::ensure_workspace_git(workspace_id)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn get_or_create_chat_workspace(
    thread_id: String,
    title: Option<String>,
) -> Result<store::WorkspaceRecord, crate::AppError> {
    future_host::workspaces::get_or_create_chat_workspace(thread_id, title)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn rename_workspace(
    input: store::RenameWorkspaceInput,
) -> Result<store::WorkspaceRecord, crate::AppError> {
    future_host::workspaces::rename_workspace(input)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn pin_workspace(
    input: store::PinWorkspaceInput,
) -> Result<store::WorkspaceRecord, crate::AppError> {
    future_host::workspaces::pin_workspace(input)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn delete_workspace(
    workspace_id: String,
) -> Result<store::WorkspaceRecord, crate::AppError> {
    future_host::workspaces::delete_workspace(workspace_id).await
}

#[cfg(all(test, feature = "gui"))]
mod tests {
    use super::*;

    #[test]
    fn async_command_wrapper_rejects_malformed_body() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![delete_workspace],
            &["delete_workspace"],
        );
    }
}
