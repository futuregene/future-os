#[macro_use]
mod record_macro;

mod app_settings;
mod approvals;
mod artifacts;
mod cleanup;
mod db;
mod markdown_refs;
mod messages;
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
    get_app_settings, is_builtin_skills_bootstrapped, mark_builtin_skills_bootstrapped,
    update_app_settings, AppSettings, UpdateAppSettingsInput,
};
pub use approvals::{
    decide_approval_request, ensure_approval_request, list_approval_requests,
    list_review_file_changes, ApprovalRequestRecord,
};
pub use artifacts::{
    artifact_type_from_path, create_artifact, delete_artifact, ensure_artifact,
    import_attachment_artifact, list_artifacts, ArtifactRecord,
};
pub use cleanup::{
    cancel_stale_approval_requests, clear_finished_runs, get_thread_cleanup_summary,
    list_interrupted_runs, reanimate_run, reconcile_orphan_chat_workspaces,
    reconcile_orphan_images, reconcile_orphan_review_repos, settle_interrupted_run,
};
pub use db::{
    app_images_root, chat_workspace_path, future_dir, get_approval_request, get_run,
    thread_images_dir,
};
pub use markdown_refs::resolve_markdown_references;
pub use messages::{append_message, list_messages, MessageRecord};
pub use records::*;
pub use review_snapshots::{
    create_review_snapshot, get_last_run_changeset, get_review_snapshot, get_run_changeset,
    list_snapshots_with_commits, list_unmaterialized_runs, mark_run_overlapped,
    mark_snapshot_failed, prune_thread_changesets, upsert_run_changeset, ReviewChangesetRecord,
    ReviewFileChangeRecord, ReviewSnapshotRecord,
};
pub use runs::{
    active_run_sessions, append_run_event, clear_all_run_events_files, clear_run_event_buffer,
    create_run, delete_run_events_file, fail_run_if_active, get_tool_call_input, list_run_events,
    list_run_events_bulk, list_runs, list_tool_calls, list_tool_outputs,
    update_run_status_if_active, RunEventRecord, RunRecord, ToolCallRecord, ToolOutputRecord,
};
pub use threads::{
    archive_thread, create_thread, delete_thread, find_thread_by_agent_session, get_recent_thread,
    get_thread, list_threads, pin_thread, purge_soft_deleted_threads, rename_thread,
    restore_thread, update_thread_model, update_thread_session_id, update_thread_thinking_level,
    ThreadRecord,
};
pub use workspace_files::{search_workspace_files, WorkspaceFileResult, WorkspaceFileSearchInput};
pub use workspaces::{
    create_workspace, delete_workspace, get_or_create_chat_workspace, get_workspace,
    list_workspaces, purge_soft_deleted_workspaces, rename_workspace, update_chat_workspace_path,
    workspace_agent_session_ids, WorkspaceRecord,
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
    // Reconcile GUI threads against the agent's base data: a thread whose
    // session JSONL was deleted externally (TUI/CLI/manual) is soft-deleted so
    // the UI can't show a conversation the model has silently lost. Best-effort
    // — a reconcile failure must never block app startup.
    if let Err(error) = cleanup::reconcile_orphan_sessions() {
        eprintln!("reconcile_orphan_sessions failed: {error}");
    }
    // Hard-delete any threads left in the legacy soft-deleted state (and their
    // orphaned child rows). delete_thread now hard-deletes, so this only clears
    // pre-existing rows. Best-effort — never block startup.
    match purge_soft_deleted_threads() {
        Ok(0) => {}
        Ok(count) => eprintln!("purged {count} soft-deleted thread(s)"),
        Err(error) => eprintln!("purge_soft_deleted_threads failed: {error}"),
    }
    // Likewise hard-delete any legacy soft-deleted workspaces (and their scoped
    // rows). Runs after the thread purge so both are converged before the dir
    // reconcilers below reclaim the now-orphaned review/image/chat dirs.
    match purge_soft_deleted_workspaces() {
        Ok(0) => {}
        Ok(count) => eprintln!("purged {count} soft-deleted workspace(s)"),
        Err(error) => eprintln!("purge_soft_deleted_workspaces failed: {error}"),
    }
    // Reclaim per-thread image dirs (thumbnails + workspace-mode originals) whose
    // thread is gone — including threads deleted out-of-band via the TUI/CLI.
    if let Err(error) = cleanup::reconcile_orphan_images() {
        eprintln!("reconcile_orphan_images failed: {error}");
    }
    // Reclaim per-thread temp chat-workspace scratch dirs whose thread is gone.
    if let Err(error) = cleanup::reconcile_orphan_chat_workspaces() {
        eprintln!("reconcile_orphan_chat_workspaces failed: {error}");
    }
    // Reclaim per-workspace shadow-review repos whose workspace is gone/deleted.
    if let Err(error) = cleanup::reconcile_orphan_review_repos() {
        eprintln!("reconcile_orphan_review_repos failed: {error}");
    }
    Ok(())
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
