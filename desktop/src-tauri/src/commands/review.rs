//! Review changeset and git-diff Tauri commands.

use std::path::Path;

use serde::Serialize;

use crate::shadow_review::{self, LastRunReviewData, VolumeRedline, VolumeVerdict};
use crate::{agent_bridge, git_review, store};

/// Workspace review capabilities for the frontend (§10.1). `changePreview`
/// flips to `unsupported_too_large` for oversized non-git Workspaces (§6.7).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceReviewCapabilities {
    is_git_workspace: bool,
    views: Vec<String>,
    default_view: String,
    change_preview: String,
}

#[tauri::command]
pub fn get_workspace_review_capabilities(
    workspace_id: String,
) -> Result<WorkspaceReviewCapabilities, crate::AppError> {
    let workspace = store::get_workspace(&workspace_id)?
        .ok_or_else(|| "Workspace could not be loaded.".to_string())?;
    let path = Path::new(&workspace.path);
    let is_git = git_review::is_git_workspace(path);

    let change_preview = if !is_git
        && shadow_review::evaluate_volume(path, &VolumeRedline::default())
            == VolumeVerdict::TooLarge
    {
        "unsupported_too_large"
    } else {
        "ready"
    };

    let (views, default_view) = if is_git {
        (
            vec!["git_changes".to_string(), "last_run".to_string()],
            "git_changes".to_string(),
        )
    } else {
        (vec!["last_run".to_string()], "last_run".to_string())
    };

    Ok(WorkspaceReviewCapabilities {
        is_git_workspace: is_git,
        views,
        default_view,
        change_preview: change_preview.to_string(),
    })
}

#[tauri::command]
pub fn get_last_run_review(
    thread_id: String,
) -> Result<Option<LastRunReviewData>, crate::AppError> {
    let Some(changeset) = store::get_last_run_changeset(&thread_id)? else {
        return Ok(None);
    };
    Ok(Some(shadow_review::build_last_run_review(changeset)?))
}

#[tauri::command]
pub fn retry_run_review(run_id: String) -> Result<Option<LastRunReviewData>, crate::AppError> {
    agent_bridge::retry_run_review(&run_id)?;
    // `retry` materializes a fresh changeset before returning Ok (otherwise it
    // errors), so the changeset is always present here — the old `else {
    // return Ok(None) }` arm was unreachable.
    let changeset = store::get_run_changeset(&run_id)?
        .expect("retry_run_review: retry always materializes a run changeset");
    Ok(Some(shadow_review::build_last_run_review(changeset)?))
}

#[tauri::command]
pub fn get_git_review(
    workspace_id: String,
    base: Option<String>,
    custom_base: Option<String>,
) -> Result<git_review::GitReview, crate::AppError> {
    git_review::get_git_review(workspace_id, base, custom_base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::store::{CreateThreadInput, CreateWorkspaceInput};
    use std::path::PathBuf;

    fn init(label: &str) -> HomeGuard {
        let home = HomeGuard::new(label);
        crate::store::initialize_app_store().expect("init store");
        home
    }

    fn workspace(label: &str) -> crate::store::WorkspaceRecord {
        let path = PathBuf::from(std::env::var("HOME").expect("test home"))
            .join(label)
            .display()
            .to_string();
        crate::store::create_workspace(CreateWorkspaceInput {
            name: Some(label.to_string()),
            path,
            description: None,
            create_directory: Some(true),
        })
        .expect("create workspace")
    }

    fn thread_in(workspace: &crate::store::WorkspaceRecord) -> crate::store::ThreadRecord {
        crate::store::create_thread(CreateThreadInput {
            mode: "workspace".to_string(),
            title: Some("Review".to_string()),
            workspace_id: Some(workspace.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create thread")
    }

    fn git_repo(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "futureos-cmd-review-{}-{}",
            std::process::id(),
            label
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        run_git(&dir, &["init", "-q", "-b", "main"]);
        run_git(&dir, &["config", "user.email", "t@example.com"]);
        run_git(&dir, &["config", "user.name", "T"]);
        std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
        run_git(&dir, &["add", "a.txt"]);
        run_git(&dir, &["commit", "-qm", "init"]);
        dir
    }

    fn run_git(dir: &std::path::Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "git {args:?} failed: {stderr}");
    }

    #[test]
    fn get_workspace_review_capabilities_errors_for_unknown_workspace() {
        let _home = init("cmd_review_caps_ghost");
        assert!(get_workspace_review_capabilities("ghost".into()).is_err());
    }

    #[test]
    fn get_workspace_review_capabilities_reports_a_non_git_workspace() {
        let _home = init("cmd_review_caps_nongit");
        let ws = workspace("plain_ws");
        let caps = get_workspace_review_capabilities(ws.id).expect("caps");
        assert!(!caps.is_git_workspace);
        assert_eq!(caps.views, vec!["last_run".to_string()]);
        assert_eq!(caps.default_view, "last_run");
        assert_eq!(caps.change_preview, "ready");
    }

    #[test]
    fn get_workspace_review_capabilities_marks_oversized_non_git_workspaces() {
        let _home = init("cmd_review_caps_too_large");
        let ws = workspace("too_large_ws");
        let big = std::path::Path::new(&ws.path).join("huge.bin");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(512 * 1024 * 1024 + 1)
            .unwrap();
        let caps = get_workspace_review_capabilities(ws.id).expect("caps");
        assert_eq!(caps.change_preview, "unsupported_too_large");
    }

    #[test]
    fn get_workspace_review_capabilities_reports_a_git_workspace() {
        let _home = init("cmd_review_caps_git");
        let repo = git_repo("caps-git");
        let ws = crate::store::create_workspace(CreateWorkspaceInput {
            name: Some("git_ws".to_string()),
            path: repo.display().to_string(),
            description: None,
            create_directory: None,
        })
        .expect("create git workspace");
        let caps = get_workspace_review_capabilities(ws.id).expect("caps");
        assert!(caps.is_git_workspace);
        assert_eq!(
            caps.views,
            vec!["git_changes".to_string(), "last_run".to_string()]
        );
        assert_eq!(caps.default_view, "git_changes");
    }

    #[test]
    fn get_last_run_review_is_none_without_a_changeset() {
        let _home = init("cmd_review_last_run");
        let ws = workspace("last_run_ws");
        let thread = thread_in(&ws);
        let review = get_last_run_review(thread.id).expect("review");
        assert!(review.is_none());
    }

    #[test]
    fn retry_run_review_errors_for_an_unknown_run() {
        let _home = init("cmd_review_retry");
        assert!(retry_run_review("ghost_run".into()).is_err());
    }

    #[test]
    fn retry_run_review_returns_the_changeset_after_a_capture() {
        let _home = init("cmd_review_retry_ok");
        let ws = workspace("retry_ws");
        let thread = thread_in(&ws);
        let run = crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("run");
        // Real captures produce real shadow commits — the only seeding the
        // retry path accepts.
        std::fs::write(std::path::Path::new(&ws.path).join("a.txt"), b"hello").unwrap();
        crate::agent_bridge::capture_before(&thread.id, &run.id);
        crate::agent_bridge::finalize_after(&thread.id, &run.id);
        let review = retry_run_review(run.id).expect("retry");
        assert!(review.is_some());
    }

    #[test]
    fn get_git_review_errors_for_unknown_workspace() {
        let _home = init("cmd_review_git_ghost");
        assert!(get_git_review("ghost".into(), None, None).is_err());
    }

    #[test]
    fn get_git_review_reports_a_non_git_workspace() {
        let _home = init("cmd_review_git_nongit");
        let ws = workspace("nongit_ws");
        let review = get_git_review(ws.id, None, None).expect("review");
        let value = serde_json::to_value(&review).expect("serialize");
        assert_eq!(value["isGitWorkspace"], serde_json::json!(false));
        assert_eq!(value["files"], serde_json::json!([]));
    }

    #[test]
    fn get_git_review_reports_a_git_workspace() {
        let _home = init("cmd_review_git_ok");
        let repo = git_repo("git-ok");
        let ws = crate::store::create_workspace(CreateWorkspaceInput {
            name: Some("git_ok_ws".to_string()),
            path: repo.display().to_string(),
            description: None,
            create_directory: None,
        })
        .expect("create git workspace");
        let review = get_git_review(ws.id, None, None).expect("review");
        let value = serde_json::to_value(&review).expect("serialize");
        assert_eq!(value["isGitWorkspace"], serde_json::json!(true));
        assert_eq!(value["branch"], serde_json::json!("main"));
    }

    #[test]
    fn get_last_run_review_errors_when_changeset_lookup_fails() {
        let _home = init("cmd_review_last_run_db");
        let home = std::env::var("HOME").expect("test home");
        let conn =
            rusqlite::Connection::open(std::path::Path::new(&home).join(".future/app/app.db"))
                .expect("open db");
        conn.execute_batch("DROP TABLE review_changesets;").unwrap();
        drop(conn);
        assert!(get_last_run_review("any".into()).is_err());
    }

    #[test]
    fn get_last_run_review_errors_when_file_changes_lookup_fails() {
        let _home = init("cmd_review_last_run_files");
        let ws = workspace("last_run_files_ws");
        let thread = thread_in(&ws);
        let run = crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("run");
        std::fs::write(std::path::Path::new(&ws.path).join("a.txt"), b"hello").unwrap();
        crate::agent_bridge::capture_before(&thread.id, &run.id);
        crate::agent_bridge::finalize_after(&thread.id, &run.id);
        // Drop the file-changes table so `build_last_run_review`'s first lookup
        // fails after a real changeset has been materialized.
        let home = std::env::var("HOME").expect("test home");
        let conn =
            rusqlite::Connection::open(std::path::Path::new(&home).join(".future/app/app.db"))
                .expect("open db");
        conn.execute_batch("DROP TABLE review_file_changes;")
            .unwrap();
        drop(conn);
        assert!(get_last_run_review(thread.id).is_err());
    }
}
