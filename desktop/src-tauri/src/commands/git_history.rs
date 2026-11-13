//! The Git log of one conversation's workspace, for this machine's own history
//! view.
//!
//! The read itself lives in `future_host::git_history` — the same module the
//! phone's sheet asks through `remote_host` — because one workspace has one
//! history and two readings of it would drift. What this file owns is only the
//! wiring: the path a thread's history is read in, and how the answer crosses
//! into the webview.
//!
//! Deliberately *not* a diff: `get_git_review` already reports the working tree
//! and the branch delta. This is the log — every commit, as the lanes that
//! produced them.
//!
//! Both commands take the workspace path rather than a thread id: the thread
//! knows its workspace, the caller already has the thread, and passing the path
//! keeps the read free of a second store lookup on every scroll.

/// The commit log of a workspace, as a lane graph.
#[tauri::command]
pub async fn get_git_history(
    path: String,
) -> Result<future_host::git_history::GraphListing, crate::AppError> {
    future_host::runtime::run_blocking(move || {
        let _request =
            future_host::request_scope::admit().map_err(|e| crate::AppError::from(e.message()))?;
        // The read answers with `future_host::Error`, which `AppError` carries
        // through `From`: the closure's return type converts it, so there is
        // nothing to map and nothing to wrap.
        future_host::git_history::list(&path)
    })
    .await?
}

/// One commit's stat and patch, bounded the way the phone's sheet bounds it.
#[tauri::command]
pub async fn get_git_commit(
    path: String,
    sha: String,
) -> Result<future_host::git_history::CommitDetail, crate::AppError> {
    future_host::runtime::run_blocking(move || {
        let _request =
            future_host::request_scope::admit().map_err(|e| crate::AppError::from(e.message()))?;
        future_host::git_history::detail(&path, &sha)
    })
    .await?
}
