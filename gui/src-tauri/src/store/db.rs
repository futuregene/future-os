//! Database connection plumbing — app directory layout, the SQLite connection
//! factory, schema application — plus a handful of small cross-domain row
//! lookups shared by several store modules.

use rusqlite::{params, Connection, OptionalExtension};
use std::{fs, path::PathBuf};

use super::records::*;
use super::schema::SCHEMA;
use super::util::{create_id, now_millis};
use super::{get_thread, get_workspace};

pub(super) fn app_dir() -> Result<PathBuf, crate::AppError> {
    let home = std::env::var("HOME").map_err(|_| "HOME environment variable is not set.")?;
    Ok(PathBuf::from(home).join(".future").join("app"))
}

pub(super) fn db_path() -> Result<PathBuf, crate::AppError> {
    Ok(app_dir()?.join("app.db"))
}

pub(super) fn chat_workspace_path(thread_id: &str) -> Result<PathBuf, crate::AppError> {
    Ok(app_dir()?.join("workspaces").join("chat").join(thread_id))
}

pub(super) fn ensure_app_dirs() -> Result<(), crate::AppError> {
    fs::create_dir_all(app_dir()?.join("workspaces").join("chat")).map_err(crate::AppError::from)
}

pub(super) fn connect() -> Result<Connection, crate::AppError> {
    ensure_app_dirs()?;
    let conn = Connection::open(db_path()?)?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         PRAGMA journal_mode = WAL;",
    )?;
    Ok(conn)
}

pub(super) fn apply_schema(conn: &Connection) -> Result<(), crate::AppError> {
    conn.execute_batch(SCHEMA).map_err(crate::AppError::from)
}

pub(super) fn get_message(id: &str) -> Result<Option<MessageRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        "SELECT id, thread_id, run_id, role, content_type, content, status, created_at, updated_at
         FROM messages
         WHERE id = ?1",
        params![id],
        message_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

pub fn get_run(id: &str) -> Result<Option<RunRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        "SELECT id, thread_id, trigger_message_id, status, model_provider, model_id,
                started_at, ended_at, error_message, error_type, created_at, updated_at
         FROM runs
         WHERE id = ?1",
        params![id],
        run_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

pub(super) fn get_run_event(id: &str) -> Result<Option<RunEventRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        "SELECT id, run_id, type, payload, sequence, created_at
         FROM run_events
         WHERE id = ?1",
        params![id],
        run_event_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

pub(super) fn run_thread_id(conn: &Connection, run_id: &str) -> Result<String, crate::AppError> {
    conn.query_row(
        "SELECT thread_id FROM runs WHERE id = ?1",
        params![run_id],
        |row| row.get(0),
    )
    .map_err(crate::AppError::from)
}

pub fn get_approval_request(id: &str) -> Result<Option<ApprovalRequestRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        "SELECT id, thread_id, run_id, tool_call_id, kind, status, title, summary,
                risk_level, requested_action, decision_note, decided_at, created_at, updated_at,
                action_category, action_payload, sandbox_boundary, reviewer, decision_scope, decision_source
         FROM approval_requests
         WHERE id = ?1",
        params![id],
        approval_request_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

pub(super) fn get_or_create_user_workspace(
    name: String,
    path: PathBuf,
    description: Option<String>,
) -> Result<WorkspaceRecord, crate::AppError> {
    let normalized_path = path.display().to_string();
    let conn = connect()?;
    let existing = conn
        .query_row(
            "SELECT id, name, kind, path, description, cleanup_status,
                    cleanup_requested_at, cleaned_at, last_opened_at,
                    created_at, updated_at, deleted_at
             FROM workspaces
             WHERE kind = 'user' AND path = ?1 AND deleted_at IS NULL
             LIMIT 1",
            params![normalized_path],
            workspace_from_row,
        )
        .optional()?;

    if let Some(workspace) = existing {
        return Ok(workspace);
    }

    let now = now_millis();
    let workspace_id = create_id("ws");
    conn.execute(
        "INSERT INTO workspaces (
             id, name, kind, path, description, cleanup_status, last_opened_at,
             created_at, updated_at
         ) VALUES (?1, ?2, 'user', ?3, ?4, 'active', ?5, ?5, ?5)",
        params![workspace_id, name, normalized_path, description, now],
    )?;

    get_workspace(&workspace_id)?
        .ok_or_else(|| "Created workspace could not be loaded.".to_string().into())
}

pub(super) fn update_thread_status(
    thread_id: &str,
    status: &str,
) -> Result<ThreadRecord, crate::AppError> {
    let now = now_millis();
    let archived_at = if status == "archived" {
        Some(now)
    } else {
        None
    };
    let conn = connect()?;
    conn.execute(
        "UPDATE threads
         SET status = ?1, archived_at = ?2, updated_at = ?3
         WHERE id = ?4 AND status != 'deleted'",
        params![status, archived_at, now, thread_id],
    )?;

    get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string().into())
}
