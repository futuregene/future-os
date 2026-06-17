use rusqlite::params;

use super::models::ThreadCleanupSummary;
use super::support::{connect, count_workspace_files, now_millis};
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

pub fn cancel_stale_approval_requests() -> Result<usize, String> {
    initialize_app_store()?;
    let now = now_millis();
    let mut conn = connect()?;
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    tx.execute(
        "UPDATE runs
         SET status = 'cancelled',
             error_message = 'Pending approval was cancelled because FutureOS restarted.',
             ended_at = COALESCE(ended_at, ?1),
             updated_at = ?1
         WHERE status = 'waiting_approval'
           AND id IN (
             SELECT run_id
             FROM approval_requests
             WHERE status = 'pending'
               AND run_id IS NOT NULL
           )",
        params![now],
    )
    .map_err(|error| error.to_string())?;
    let changed = tx
        .execute(
            "UPDATE approval_requests
             SET status = 'cancelled',
                 decision_note = 'Cancelled because FutureOS restarted.',
                 decided_at = ?1,
                 updated_at = ?1
             WHERE status = 'pending'",
            params![now],
        )
        .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(changed)
}

pub fn clear_finished_runs(thread_id: &str) -> Result<usize, String> {
    initialize_app_store()?;
    let mut conn = connect()?;
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    let changed = tx
        .execute(
            "UPDATE messages
             SET run_id = NULL
             WHERE thread_id = ?1
               AND run_id IN (
                 SELECT id FROM runs
                 WHERE thread_id = ?1
                   AND status IN ('completed', 'failed', 'cancelled')
               )",
            params![thread_id],
        )
        .map_err(|error| error.to_string())?;
    tx.execute(
        "UPDATE artifacts
         SET run_id = NULL
         WHERE thread_id = ?1
           AND run_id IN (
             SELECT id FROM runs
             WHERE thread_id = ?1
               AND status IN ('completed', 'failed', 'cancelled')
           )",
        params![thread_id],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "DELETE FROM tool_outputs
         WHERE tool_call_id IN (
           SELECT tc.id
           FROM tool_calls tc
           JOIN runs r ON r.id = tc.run_id
           WHERE r.thread_id = ?1
             AND r.status IN ('completed', 'failed', 'cancelled')
         )",
        params![thread_id],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "DELETE FROM run_events
         WHERE run_id IN (
           SELECT id FROM runs
           WHERE thread_id = ?1
             AND status IN ('completed', 'failed', 'cancelled')
         )",
        params![thread_id],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "DELETE FROM approval_requests
         WHERE thread_id = ?1
           AND run_id IN (
             SELECT id FROM runs
             WHERE thread_id = ?1
               AND status IN ('completed', 'failed', 'cancelled')
           )",
        params![thread_id],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "DELETE FROM review_file_changes
         WHERE changeset_id IN (
           SELECT c.id
           FROM review_changesets c
           JOIN runs r ON r.id = c.run_id
           WHERE r.thread_id = ?1
             AND r.status IN ('completed', 'failed', 'cancelled')
         )",
        params![thread_id],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "DELETE FROM review_changesets
         WHERE thread_id = ?1
           AND run_id IN (
             SELECT id FROM runs
             WHERE thread_id = ?1
               AND status IN ('completed', 'failed', 'cancelled')
           )",
        params![thread_id],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "DELETE FROM tool_calls
         WHERE run_id IN (
           SELECT id FROM runs
           WHERE thread_id = ?1
             AND status IN ('completed', 'failed', 'cancelled')
         )",
        params![thread_id],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "DELETE FROM runs
         WHERE thread_id = ?1
           AND status IN ('completed', 'failed', 'cancelled')",
        params![thread_id],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(changed)
}
