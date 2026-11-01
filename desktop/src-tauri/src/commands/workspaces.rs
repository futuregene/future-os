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
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::store::{CreateThreadInput, CreateWorkspaceInput, RenameWorkspaceInput};

    #[test]
    fn async_command_wrapper_rejects_malformed_body() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![delete_workspace],
            &["delete_workspace"],
        );
    }

    /// One round trip per command: the value each wrapper returns is the shared
    /// service's own record, so a forwarded argument that got dropped (or a
    /// record read from the wrong row) fails the assertion, and the two
    /// non-default answers — a non-git user workspace, and a chat workspace that
    /// already exists — are what proves the argument reached the service rather
    /// than a default being handed back.
    #[tokio::test]
    async fn every_workspace_command_forwards_its_arguments() {
        let _lock = crate::commands::agent_mock::mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd_workspaces_wave");
        crate::store::initialize_app_store().expect("init store");
        // `delete_workspace` drains the agent delete outbox; without a mock the
        // drain settles its rows against an unreachable agent and returns.
        crate::commands::agent_mock::ensure_mock_agent();

        let root = std::env::temp_dir().join(format!("futureos-ws-{}", std::process::id()));
        let created = create_workspace(CreateWorkspaceInput {
            name: Some("Wave".into()),
            path: root.join("wave-ws").display().to_string(),
            description: Some("described".into()),
            create_directory: Some(true),
        })
        .expect("create workspace");
        assert_eq!(created.name, "Wave");
        assert_eq!(created.description.as_deref(), Some("described"));

        let listed = list_workspaces().expect("list");
        let ids: Vec<&str> = listed.iter().map(|ws| ws.id.as_str()).collect();
        assert!(ids.contains(&created.id.as_str()), "{listed:?}");

        // A plain directory is not a git repository, so the answer is the
        // *negative* one the wrapper must pass straight through.
        assert!(
            !ensure_workspace_git(created.id.clone()).expect("git probe"),
            "a fresh temp directory is not a git workspace"
        );

        let renamed = rename_workspace(RenameWorkspaceInput {
            workspace_id: created.id.clone(),
            name: "Renamed".into(),
        })
        .expect("rename");
        assert_eq!(renamed.id, created.id);
        assert_eq!(renamed.name, "Renamed");

        let pinned = pin_workspace(crate::store::PinWorkspaceInput {
            workspace_id: created.id.clone(),
            pinned: true,
        })
        .expect("pin");
        assert!(pinned.pinned);

        // The chat workspace is keyed by thread id: asking twice with a
        // different title must answer the same row, which is only true if the
        // thread id is forwarded rather than the title.
        let thread = crate::store::create_thread(CreateThreadInput {
            mode: "chat".into(),
            title: Some("Chat".into()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create thread");
        let chat = get_or_create_chat_workspace(thread.id.clone(), Some("First".into()))
            .expect("chat workspace");
        let again = get_or_create_chat_workspace(thread.id.clone(), Some("Second".into()))
            .expect("existing chat workspace");
        assert_eq!(chat.id, again.id);
        assert_eq!(
            chat.kind, "temporary",
            "a chat workspace is the per-conversation scratch workspace"
        );

        let deleted = delete_workspace(created.id.clone())
            .await
            .expect("delete workspace");
        assert_eq!(deleted.id, created.id);
        assert!(
            !list_workspaces()
                .expect("list after delete")
                .iter()
                .any(|ws| ws.id == created.id),
            "the deleted workspace is gone from the list"
        );
    }
}
