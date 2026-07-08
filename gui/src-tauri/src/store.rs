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
mod research;
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
    list_review_file_changes, ApprovalRequestRecord,
};
pub use artifacts::{
    artifact_type_from_path, create_artifact, delete_artifact, ensure_artifact,
    import_attachment_artifact, list_artifacts, ArtifactRecord,
};
pub use cleanup::{
    cancel_stale_approval_requests, clear_finished_runs, get_thread_cleanup_summary,
};
pub use db::{get_approval_request, get_run};
pub use markdown_refs::resolve_markdown_references;
pub use messages::{append_message, list_messages, MessageRecord};
pub use records::*;
pub use research::{list_research_resources, promote_artifact_to_research, ResearchResourceRecord};
pub use review_snapshots::{
    create_review_snapshot, get_last_run_changeset, get_review_snapshot, get_run_changeset,
    list_snapshots_with_commits, list_unmaterialized_runs, mark_run_overlapped,
    mark_snapshot_failed, prune_thread_changesets, upsert_run_changeset, ReviewChangesetRecord,
    ReviewFileChangeRecord, ReviewSnapshotRecord,
};
pub use runs::{
    active_run_sessions, append_run_event, complete_tool_call, create_run, fail_run_if_active,
    get_tool_call_input, list_run_events, list_runs, list_tool_calls, list_tool_outputs,
    update_run_status_if_active, upsert_tool_call, RunEventRecord, RunRecord, ToolCallRecord,
    ToolOutputRecord,
};
pub use threads::{
    archive_thread, create_thread, delete_thread, find_thread_by_agent_session, get_recent_thread,
    get_thread, list_threads, pin_thread, rename_thread, restore_thread, update_thread_model,
    update_thread_thinking_level, ThreadRecord,
};
pub use workspace_files::{search_workspace_files, WorkspaceFileResult, WorkspaceFileSearchInput};
pub use workspaces::{
    create_workspace, delete_workspace, get_or_create_chat_workspace, get_workspace,
    list_workspaces, rename_workspace, WorkspaceRecord,
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
    Ok(())
}
