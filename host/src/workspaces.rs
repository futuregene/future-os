//! Shared workspace application services. FutureOS no longer auto-`git init`s Workspace
//! directories (§14.3) — `git_review` only *detects* real git repos, and the
//! shadow review pipeline supplies "last-run changes" for non-git Workspaces.

use crate::{git_review, store};

pub fn list_workspaces() -> Result<Vec<store::WorkspaceRecord>, crate::Error> {
    store::list_workspaces()
}

pub fn create_workspace(
    input: store::CreateWorkspaceInput,
) -> Result<store::WorkspaceRecord, crate::Error> {
    store::create_workspace(input)
}

/// Reports whether a user Workspace directory is a real git repo. Kept for the
/// existing frontend call site; it no longer initialises anything (§14.3).
pub fn ensure_workspace_git(workspace_id: String) -> Result<bool, crate::Error> {
    let workspace = store::get_workspace(&workspace_id)?
        .ok_or_else(|| "Workspace could not be loaded.".to_string())?;
    if workspace.kind != "user" {
        return Ok(false);
    }
    Ok(git_review::is_git_workspace(std::path::Path::new(
        &workspace.path,
    )))
}

pub fn get_or_create_chat_workspace(
    thread_id: String,
    title: Option<String>,
) -> Result<store::WorkspaceRecord, crate::Error> {
    store::get_or_create_chat_workspace(&thread_id, title)
}

/// One workspace by id, whatever its kind.
///
/// The rail's list holds the user's own directories only (`kind = 'user'`), so a
/// chat conversation's per-thread scratch workspace is never in it. A reader
/// that has a workspace id — the open conversation's own `workspaceId` — asks
/// here instead of searching that list, which is what kept a chat's Files tab
/// pointing at somebody else's folder.
pub fn get_workspace(workspace_id: String) -> Result<Option<store::WorkspaceRecord>, crate::Error> {
    store::get_workspace(&workspace_id)
}

pub fn rename_workspace(
    input: store::RenameWorkspaceInput,
) -> Result<store::WorkspaceRecord, crate::Error> {
    store::rename_workspace(input)
}

/// Pin a Workspace group above the unpinned ones (same ordering flag, and the
/// same rule, as pinning a conversation).
pub fn pin_workspace(
    input: store::PinWorkspaceInput,
) -> Result<store::WorkspaceRecord, crate::Error> {
    store::pin_workspace(input)
}

/// Shelf a user workspace: it and every conversation it holds leave the default
/// lists without any conversation changing its own status. Refused while any
/// conversation inside is still running.
pub async fn archive_workspace(
    workspace_id: String,
) -> Result<store::WorkspaceRecord, crate::Error> {
    // A run another client started has no local `runs` row; the Agent's
    // resident streaming set is what makes the refusal complete. Unreachable
    // agent answers empty, matching the sidebar's own tolerance.
    let streaming = crate::agent_bridge::reconcile_streaming_observers()
        .await
        .unwrap_or_default();
    store::archive_workspace(&workspace_id, &streaming)
}

/// Bring a shelved workspace (and the conversations it was hiding) back.
pub fn restore_workspace(workspace_id: String) -> Result<store::WorkspaceRecord, crate::Error> {
    store::restore_workspace(&workspace_id)
}

/// One page of the workspace list: `archived = false` is the default list
/// (pinned excluded — they lead, see `list_pinned_workspaces`), `true` is the
/// archived view.
pub fn list_workspaces_page(
    archived: bool,
    cursor: Option<String>,
    limit: i64,
) -> Result<store::WorkspacesPage, crate::Error> {
    store::list_workspaces_page(archived, cursor.as_deref(), limit)
}

/// The pinned user workspaces, whole.
pub fn list_pinned_workspaces() -> Result<Vec<store::WorkspaceRecord>, crate::Error> {
    store::list_pinned_workspaces()
}

pub async fn delete_workspace(
    workspace_id: String,
) -> Result<store::WorkspaceRecord, crate::Error> {
    // Resolve terminal ownership before deleting its conversation rows. The
    // optional entry presentation sink closes embedded GUI terminals after
    // the database deletion succeeds; headless leaves that sink unbound.
    let thread_ids: Vec<String> = if crate::events::has_presentation() {
        store::list_threads()?
            .into_iter()
            .filter(|thread| thread.workspace_id == workspace_id)
            .map(|thread| thread.id)
            .collect()
    } else {
        Vec::new()
    };
    // Hard-delete the workspace, its threads, and all their child rows.
    let workspace = store::delete_workspace(&workspace_id)?;
    for thread_id in &thread_ids {
        crate::events::publish(crate::events::Event::CloseThreadTerminals(thread_id));
    }
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

/// What deleting this workspace would remove, and what it would keep.
///
/// The delete is not an archive delete; the confirmation names both numbers
/// before the reader commits to it.
pub fn delete_summary(workspace_id: String) -> Result<store::WorkspaceDeleteSummary, crate::Error> {
    store::delete_summary(&workspace_id)
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

        let pinned = pin_workspace(store::PinWorkspaceInput {
            workspace_id: created.id.clone(),
            pinned: true,
        })
        .expect("pin");
        assert!(pinned.pinned);
        assert!(list_workspaces().expect("list")[0].pinned);

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

    /// A chat's scratch workspace is `temporary`, which the rail's user-only list
    /// never holds — so a reader that has the id must be able to read it directly.
    #[test]
    fn get_workspace_reads_a_temporary_chat_workspace_by_id() {
        let _home = init("cmd_ws_get");
        let chat = get_or_create_chat_workspace("thread_y".into(), Some("Chat".into()))
            .expect("chat workspace");
        let read = get_workspace(chat.id.clone())
            .expect("read")
            .expect("the chat workspace exists");
        assert_eq!(read.id, chat.id);
        assert_eq!(read.kind, "temporary");
        assert_eq!(
            read.path, chat.path,
            "the path is what the Files tab and the @ picker root at"
        );
        assert!(list_pinned_workspaces().expect("pinned").is_empty());
        assert!(get_workspace("ws-missing".into()).expect("read").is_none());
    }
}
