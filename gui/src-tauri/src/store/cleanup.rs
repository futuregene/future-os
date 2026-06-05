use rusqlite::params;

use super::models::ThreadCleanupSummary;
use super::support::{connect, count_workspace_files};
use super::{get_thread, get_workspace, initialize_app_store};

pub fn get_thread_cleanup_summary(thread_id: &str) -> Result<ThreadCleanupSummary, String> {
    initialize_app_store()?;
    let thread = get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let workspace = get_workspace(&thread.workspace_id)?
        .ok_or_else(|| "Thread workspace could not be loaded.".to_string())?;
    let conn = connect()?;
    let artifact_count = conn
        .query_row(
            "SELECT COUNT(*)
             FROM artifacts
             WHERE workspace_id = ?1
               AND (thread_id = ?2 OR ?3 = 'workspace')
               AND deleted_at IS NULL",
            params![workspace.id, thread.id, thread.mode],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let workspace_file_count = if workspace.kind == "temporary" {
        count_workspace_files(&workspace.path)?
    } else {
        0
    };

    Ok(ThreadCleanupSummary {
        thread_id: thread.id,
        workspace_id: workspace.id,
        workspace_kind: workspace.kind,
        workspace_path: workspace.path,
        cleanup_status: workspace.cleanup_status,
        artifact_count,
        workspace_file_count,
    })
}
