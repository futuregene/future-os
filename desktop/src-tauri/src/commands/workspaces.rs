//! Workspace Tauri commands. FutureOS no longer auto-`git init`s Workspace
//! directories (§14.3) — `git_review` only *detects* real git repos, and the
//! shadow review pipeline supplies "last-run changes" for non-git Workspaces.

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
    // Hard-delete the workspace, its threads, and all their child rows.
    let workspace = store::delete_workspace(&workspace_id)?;
    crate::agent_bridge::reconcile_delete_outbox().await;
    // Physically reclaim the now-orphaned GUI dirs: the workspace's shadow-review
    // repo and each thread's image/chat-scratch dir. These key off DB presence,
    // which we just cleared. The user's own workspace files (at `workspace.path`,
    // never under ~/.future/app) are NEVER touched.
    let _ = store::reconcile_orphan_review_repos();
    let _ = store::reconcile_orphan_images();
    let _ = store::reconcile_orphan_chat_workspaces();
    Ok(workspace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    fn init(label: &str) -> HomeGuard {
        let home = HomeGuard::new(label);
        crate::store::initialize_app_store().expect("init store");
        home
    }

    fn create_input() -> store::CreateWorkspaceInput {
        store::CreateWorkspaceInput {
            name: Some("Project".into()),
            path: std::env::temp_dir()
                .join(format!("futureos-cmd-ws-{}", std::process::id()))
                .display()
                .to_string(),
            description: Some("a project".into()),
            create_directory: Some(true),
        }
    }

    #[test]
    fn async_command_wrapper_rejects_malformed_body() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![delete_workspace],
            &["delete_workspace"],
        );
    }

    #[tokio::test]
    async fn workspace_commands_round_trip() {
        let _home = init("cmd_workspaces");
        assert!(list_workspaces().expect("list empty").is_empty());

        let created = create_workspace(create_input()).expect("create");
        assert_eq!(created.name, "Project");
        assert_eq!(list_workspaces().expect("list").len(), 1);

        let renamed = rename_workspace(store::RenameWorkspaceInput {
            workspace_id: created.id.clone(),
            name: "Renamed".into(),
        })
        .expect("rename");
        assert_eq!(renamed.name, "Renamed");

        let deleted = delete_workspace(created.id.clone()).await.expect("delete");
        assert_eq!(deleted.id, created.id);
        assert!(list_workspaces().expect("list after delete").is_empty());
    }

    #[test]
    fn ensure_workspace_git_is_false_for_a_non_git_path() {
        let _home = init("cmd_ws_git");
        let created = create_workspace(create_input()).expect("create");
        // The freshly created dir is not a git repo, so the report is false.
        assert!(!ensure_workspace_git(created.id).expect("check git"));
    }

    #[test]
    fn ensure_workspace_git_skips_non_user_workspaces() {
        let _home = init("cmd_ws_git_kind");
        let chat = get_or_create_chat_workspace("thread_x".into(), Some("Chat".into()))
            .expect("chat workspace");
        assert_eq!(chat.kind, "temporary");
        // Non-user workspaces never probe git.
        assert!(!ensure_workspace_git(chat.id).expect("check kind"));
    }

    #[test]
    fn get_or_create_chat_workspace_is_idempotent() {
        let _home = init("cmd_ws_chat");
        let first =
            get_or_create_chat_workspace("thread_x".into(), Some("Chat".into())).expect("first");
        let second = get_or_create_chat_workspace("thread_x".into(), None).expect("second");
        assert_eq!(first.id, second.id);
    }
}
