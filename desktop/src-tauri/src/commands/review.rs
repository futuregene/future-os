//! Tauri IPC adapter for shared application operations.
use crate::git_review;
use crate::shadow_review::LastRunReviewData;
pub use future_host::application::review::WorkspaceReviewCapabilities;
#[tauri::command]
pub fn get_workspace_review_capabilities(
    workspace_id: String,
) -> Result<WorkspaceReviewCapabilities, crate::AppError> {
    future_host::application::review::get_workspace_review_capabilities(workspace_id)
}

#[tauri::command]
pub fn get_last_run_review(
    thread_id: String,
) -> Result<Option<LastRunReviewData>, crate::AppError> {
    future_host::application::review::get_last_run_review(thread_id)
}

#[tauri::command]
pub fn retry_run_review(run_id: String) -> Result<Option<LastRunReviewData>, crate::AppError> {
    future_host::application::review::retry_run_review(run_id)
}

#[tauri::command]
pub fn get_git_review(
    workspace_id: String,
    base: Option<String>,
    custom_base: Option<String>,
) -> Result<git_review::GitReview, crate::AppError> {
    future_host::application::review::get_git_review(workspace_id, base, custom_base)
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

    #[test]
    fn get_last_run_review_returns_the_changeset_after_a_capture() {
        let _home = init("cmd_review_last_run_ok");
        let ws = workspace("last_run_ok_ws");
        let thread = thread_in(&ws);
        let run = crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("run");
        // Real captures produce real shadow commits + a materialized changeset,
        // so the Some arm of get_last_run_review runs.
        std::fs::write(std::path::Path::new(&ws.path).join("a.txt"), b"hello").unwrap();
        crate::agent_bridge::capture_before(&thread.id, &run.id);
        crate::agent_bridge::finalize_after(&thread.id, &run.id);
        let review = get_last_run_review(thread.id).expect("review");
        assert!(review.is_some());
    }
}
