mod app_settings;
mod approval_config;
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
mod threads;
mod util;
mod workspaces;

use db::*;

pub use app_settings::{
    get_app_settings, update_app_settings, AppSettings, UpdateAppSettingsInput,
};
pub use approvals::{
    decide_approval_request, ensure_approval_request, list_approval_requests,
    list_review_file_changes,
};
pub use artifacts::{
    artifact_type_from_path, create_artifact, delete_artifact, ensure_artifact,
    import_attachment_artifact, list_artifacts,
};
pub use cleanup::{
    cancel_stale_approval_requests, clear_finished_runs, get_thread_cleanup_summary,
};
pub use db::{get_approval_request, get_run};
pub use markdown_refs::{resolve_markdown_references, search_reference_targets};
pub use messages::{append_message, list_messages};
pub use records::*;
pub use research::{list_research_resources, promote_artifact_to_research};
pub use review_snapshots::{
    create_review_snapshot, get_last_run_changeset, get_review_snapshot, get_run_changeset,
    list_interrupted_runs, list_snapshots_with_commits, mark_run_overlapped, mark_snapshot_failed,
    prune_thread_changesets, upsert_run_changeset,
};
pub use runs::{
    append_run_event, complete_tool_call, create_run, list_run_events, list_runs, list_tool_calls,
    list_tool_outputs, update_run_status, upsert_tool_call,
};
pub use threads::{
    archive_thread, create_thread, delete_thread, get_recent_thread, get_thread, list_threads,
    pin_thread, rename_thread, restore_thread, update_thread_model,
};
pub use workspaces::{
    create_workspace, get_or_create_chat_workspace, get_workspace, list_workspaces,
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
    Ok(())
}

/// Wipe all GUI-local data: empty every table (keeping the file + schema, which
/// avoids Windows file-lock issues) and remove the temp chat workspaces and the
/// shadow-review repos. Agent config (`~/.future/agent`: auth.json / models.json)
/// is untouched, so login and providers survive. Used by Settings ▸ 调试 ▸ 重置.
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
    for table in &tables {
        conn.execute(&format!("DELETE FROM \"{table}\""), [])?;
    }
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    drop(conn);

    // Best effort: remove GUI-managed file trees; they're recreated on demand.
    let app = app_dir()?;
    let _ = std::fs::remove_dir_all(app.join("workspaces"));
    let _ = std::fs::remove_dir_all(app.join("review"));
    Ok(())
}
