#[macro_use]
mod record_macro;

mod app_settings;
mod approvals;
mod artifacts;
mod cleanup;
mod db;
mod deletions;
mod markdown_refs;
mod records;
mod review_snapshots;
mod runs;
mod schema;
mod status;
mod threads;
mod util;
mod workspace_files;
mod workspaces;

use db::*;

pub use app_settings::{
    get_app_settings, update_app_settings, AppSettings, UpdateAppSettingsInput,
};
pub use approvals::{
    decide_approval_request, ensure_approval_request, list_approval_requests,
    list_pending_approval_requests, list_review_file_changes, ApprovalRequestRecord,
};
pub use artifacts::{
    artifact_type_from_path, create_artifact, delete_artifact, ensure_artifact,
    import_attachment_artifact, list_artifacts, ArtifactRecord,
};
pub use cleanup::{
    clear_finished_runs, get_thread_cleanup_summary, list_active_runs, list_interrupted_runs,
    reanimate_run, reconcile_orphan_chat_workspaces, reconcile_orphan_images,
    reconcile_orphan_review_repos, reconcile_orphan_sessions, settle_interrupted_run_from_agent,
    ActiveRun,
};
pub use db::{app_images_root, future_dir, get_approval_request, get_run, thread_images_dir};
pub use deletions::{
    acknowledge_agent_session_delete, is_agent_session_tombstoned,
    note_agent_session_delete_failure, pending_agent_session_deletes,
};
pub use markdown_refs::resolve_markdown_references;
pub use records::*;
pub use review_snapshots::{
    create_review_snapshot, get_last_run_changeset, get_review_snapshot, get_run_changeset,
    list_snapshots_with_commits, list_unmaterialized_runs, mark_run_overlapped,
    mark_snapshot_failed, prune_thread_changesets, upsert_run_changeset, ReviewChangesetRecord,
    ReviewFileChangeRecord, ReviewSnapshotRecord,
};
pub use runs::{
    active_run_sessions, advance_tool_projection, clear_all_run_events_files,
    clear_run_event_buffer, create_run, delete_run_events_file, fail_run_if_active,
    find_run_by_trigger_message_id, get_tool_call_input, latest_run, latest_run_infos,
    list_run_events, list_run_events_since, list_runs, project_tool_outputs,
    tool_projection_cursor, update_run_status_if_active, LatestRunInfo, RunEventRecord, RunRecord,
    ToolCallRecord, ToolOutputRecord,
};
#[cfg(test)]
pub(crate) use runs::{append_run_event, flush_run_event_log_for_test};
pub use threads::{
    archive_thread, batch_delete_threads, create_thread, delete_thread, delete_thread_with_files,
    find_thread_by_agent_session, get_recent_thread, get_thread, list_threads,
    move_thread_to_workspace, pin_thread, purge_soft_deleted_threads, rename_thread,
    restore_thread, sync_thread_title, update_thread_model, update_thread_session_id,
    update_thread_thinking_level, ThreadRecord,
};
pub use util::{create_id, now_millis, take_catalog_dirty};
pub use workspace_files::{search_workspace_files, WorkspaceFileResult, WorkspaceFileSearchInput};
pub use workspaces::{
    create_workspace, delete_workspace, get_or_create_chat_workspace, get_workspace,
    list_workspaces, purge_soft_deleted_workspaces, rename_workspace, update_chat_workspace_path,
    WorkspaceRecord,
};

pub fn app_data_path() -> Result<AppDataPath, crate::AppError> {
    Ok(AppDataPath {
        app_dir: app_dir()?.display().to_string(),
        db_path: db_path()?.display().to_string(),
    })
}

pub fn initialize_app_store() -> Result<(), crate::AppError> {
    ensure_app_dirs()?;
    let conn = connect()?;
    apply_schema(&conn)?;
    drop(conn);
    // Reconcile GUI threads against the agent's session list (orphan-session
    // cleanup) runs later, off the launch path, once the agent is reachable —
    // see store::reconcile_orphan_sessions.
    // Hard-delete any threads left in the legacy soft-deleted state (and their
    // orphaned child rows). delete_thread now hard-deletes, so this only clears
    // pre-existing rows. Best-effort — never block startup.
    log_purge(
        purge_soft_deleted_threads(),
        "soft-deleted thread(s)",
        "purge_soft_deleted_threads",
    );
    // Likewise hard-delete any legacy soft-deleted workspaces (and their scoped
    // rows). Runs after the thread purge so both are converged before the dir
    // reconcilers below reclaim the now-orphaned review/image/chat dirs.
    log_purge(
        purge_soft_deleted_workspaces(),
        "soft-deleted workspace(s)",
        "purge_soft_deleted_workspaces",
    );
    // Reclaim per-thread image dirs (thumbnails + workspace-mode originals) whose
    // thread is gone — including threads deleted out-of-band via the TUI/CLI.
    log_reconcile(
        cleanup::reconcile_orphan_images(),
        "reconcile_orphan_images",
    );
    // Reclaim per-thread temp chat-workspace scratch dirs whose thread is gone.
    log_reconcile(
        cleanup::reconcile_orphan_chat_workspaces(),
        "reconcile_orphan_chat_workspaces",
    );
    // Reclaim per-workspace shadow-review repos whose workspace is gone/deleted.
    log_reconcile(
        cleanup::reconcile_orphan_review_repos(),
        "reconcile_orphan_review_repos",
    );
    Ok(())
}

/// Log a best-effort purge result; never propagate (startup must not block on
/// purge failures). Split out so all three arms are directly testable.
fn log_purge(result: Result<usize, crate::AppError>, noun: &str, op: &str) {
    match result {
        Ok(0) => {}
        Ok(count) => eprintln!("purged {count} {noun}"),
        Err(error) => eprintln!("{op} failed: {error}"),
    }
}

/// Log a best-effort reconcile result; never propagate (startup must not block
/// on reclaim failures). Split out so the `Ok`/`Err` arms are directly testable.
fn log_reconcile(result: Result<usize, crate::AppError>, op: &str) {
    if let Err(error) = result {
        eprintln!("{op} failed: {error}");
    }
}

/// Wipe all GUI-local data and rebuild a pristine DB from the latest schema:
/// drop every table, then re-apply [`apply_schema`] (so a reset matches the
/// current schema even if the old DB predates a change — not just emptied rows
/// on a stale structure). Dropping in place avoids the Windows file-lock risk of
/// deleting the db file while a connection is open. Also removes the temp chat
/// workspaces and shadow-review repos. Agent config (`~/.future/agent`:
/// auth.json / models.json) is untouched, so login and providers survive. Used
/// by Settings ▸ Debug ▸ Reset.
pub fn clear_all_data() -> Result<(), crate::AppError> {
    let conn = connect()?;
    // Retain a durable deletion intent even though all display/projection data
    // is being reset. The table is deliberately control-plane state, not user
    // history, and startup will retry it against the Agent.
    deletions::enqueue_all_agent_session_deletes_in(&conn)?;
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let tables: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    // DROP TABLE also removes the table's indexes and triggers.
    for table in &tables {
        if table == "agent_delete_outbox" {
            continue;
        }
        conn.execute(&format!("DROP TABLE IF EXISTS \"{table}\""), [])?;
    }
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    apply_schema(&conn)?;
    drop(conn);

    // Best effort: remove GUI-managed file trees; they're recreated on demand.
    let app = app_dir()?;
    let _ = std::fs::remove_dir_all(app.join("workspaces"));
    let _ = std::fs::remove_dir_all(app.join("review"));
    let _ = std::fs::remove_dir_all(app.join("images"));
    // Per-run event logs + their in-memory buffer.
    clear_all_run_events_files();
    // New chat workspace root (~/.future/workspaces/chat/), outside app_dir.
    let _ = std::fs::remove_dir_all(future_dir()?.join("workspaces").join("chat"));
    Ok(())
}

/// Test fixture: a fake HOME whose pooled connection already has the schema
/// applied (the connection is dropped back into the pool), so subsequent
/// `connect()`-backed store calls from other modules observe the tables.
#[cfg(test)]
pub(crate) fn test_schema_home(label: &str) -> crate::auth_store::test_support::HomeGuard {
    let (home, conn) = db::test_support::guarded_conn(label);
    drop(conn);
    home
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    fn err() -> crate::AppError {
        "boom".to_string().into()
    }

    #[test]
    fn log_purge_covers_all_arms() {
        log_purge(Ok(0), "thing(s)", "op");
        log_purge(Ok(3), "thing(s)", "op");
        log_purge(Err(err()), "thing(s)", "op");
    }

    #[test]
    fn log_reconcile_covers_ok_and_err() {
        log_reconcile(Ok(0), "op");
        log_reconcile(Err(err()), "op");
    }

    #[test]
    fn initialize_app_store_purges_soft_deleted_rows() {
        let _home = HomeGuard::new("store-init-purge");
        initialize_app_store().unwrap();
        // Seed a legacy soft-deleted thread and workspace, then re-initialize:
        // the purge branches (Ok(count)) must run and hard-delete them.
        {
            let conn = connect().unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, name, kind, path, cleanup_status, created_at, updated_at, deleted_at)
                 VALUES ('ws-soft', 'w', 'user', '/tmp/w', 'active', 1, 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO threads (id, workspace_id, mode, title, status, created_at, updated_at, deleted_at)
                 VALUES ('th-soft', 'ws-soft', 'chat', 't', 'deleted', 1, 1, 1)",
                [],
            )
            .unwrap();
        }
        initialize_app_store().unwrap();
        let conn = connect().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE id = 'th-soft'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        let ws_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM workspaces WHERE id = 'ws-soft'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ws_count, 0);
    }

    #[test]
    fn clear_all_data_rebuilds_pristine_schema() {
        let _home = HomeGuard::new("store-clear");
        initialize_app_store().unwrap();
        clear_all_data().unwrap();
        // The reset re-applies the schema: a fresh query against the core table
        // must succeed and be empty.
        let conn = connect().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM workspaces", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
