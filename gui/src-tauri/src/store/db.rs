//! Database connection plumbing — app directory layout, the SQLite connection
//! factory, schema application — plus a handful of small cross-domain row
//! lookups shared by several store modules.

use rusqlite::{params, Connection, OptionalExtension};
use std::{fs, path::PathBuf};

use super::approvals::{
    approval_request_from_row, ApprovalRequestRecord, APPROVAL_REQUEST_COLUMNS,
};
use super::get_thread;
use super::messages::{message_from_row, MessageRecord, MESSAGE_COLUMNS};
use super::runs::{
    run_event_from_row, run_from_row, RunEventRecord, RunRecord, RUN_COLUMNS, RUN_EVENT_COLUMNS,
};
use super::schema::{ADDED_COLUMNS, ADDED_INDEXES, SCHEMA};
use super::threads::ThreadRecord;
use super::util::{create_id, loaded, now_millis};
use super::workspaces::{get_workspace_in, workspace_from_row, WorkspaceRecord, WORKSPACE_COLUMNS};

pub(super) fn app_dir() -> Result<PathBuf, crate::AppError> {
    let home = crate::home_dir().ok_or("HOME/USERPROFILE environment variable is not set.")?;
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
    conn.execute_batch(SCHEMA)?;
    // Add columns introduced after a table's initial creation. `CREATE TABLE
    // IF NOT EXISTS` is a no-op on existing tables, so these run separately and
    // tolerate the "duplicate column name" error on already-migrated DBs.
    for (table, column) in ADDED_COLUMNS {
        let sql = format!("ALTER TABLE {table} ADD COLUMN {column}");
        if let Err(error) = conn.execute(&sql, []) {
            if !is_duplicate_column_error(&error) {
                return Err(error.into());
            }
        }
    }
    // Indexes over added columns run last, once those columns are guaranteed.
    for statement in ADDED_INDEXES {
        conn.execute(statement, [])?;
    }
    Ok(())
}

fn is_duplicate_column_error(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(_, Some(message)) if message.contains("duplicate column name"))
}

pub(super) fn get_message(id: &str) -> Result<Option<MessageRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"),
        params![id],
        message_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

pub fn get_run(id: &str) -> Result<Option<RunRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        &format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?1"),
        params![id],
        run_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

pub(super) fn get_run_event(id: &str) -> Result<Option<RunEventRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        &format!("SELECT {RUN_EVENT_COLUMNS} FROM run_events WHERE id = ?1"),
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
        &format!("SELECT {APPROVAL_REQUEST_COLUMNS} FROM approval_requests WHERE id = ?1"),
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
    let conn = connect()?;
    get_or_create_user_workspace_in(&conn, name, path, description)
}

/// Connection-injecting variant so a composite write (e.g. `create_thread`) can
/// resolve/create the workspace and insert its own row in one transaction.
pub(super) fn get_or_create_user_workspace_in(
    conn: &Connection,
    name: String,
    path: PathBuf,
    description: Option<String>,
) -> Result<WorkspaceRecord, crate::AppError> {
    let normalized_path = path.display().to_string();
    let existing = conn
        .query_row(
            &format!(
                "SELECT {WORKSPACE_COLUMNS}
             FROM workspaces
             WHERE kind = 'user' AND path = ?1 AND deleted_at IS NULL
             LIMIT 1"
            ),
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

    loaded(get_workspace_in(conn, &workspace_id)?, "Created workspace")
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

    loaded(get_thread(thread_id)?, "Thread")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_schema_on_fresh_db_succeeds() {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
    }

    #[test]
    fn apply_schema_migrates_pre_source_kind_db() {
        // Reproduces the startup failure: an existing `review_changesets` that
        // predates the `source_kind` column. The migration must add the column
        // and only then create the index that references it.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE review_changesets (
                 id TEXT PRIMARY KEY,
                 thread_id TEXT NOT NULL,
                 run_id TEXT,
                 tool_call_id TEXT,
                 title TEXT NOT NULL,
                 summary TEXT,
                 status TEXT NOT NULL,
                 files_changed INTEGER NOT NULL DEFAULT 0,
                 additions INTEGER NOT NULL DEFAULT 0,
                 deletions INTEGER NOT NULL DEFAULT 0,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );",
        )
        .unwrap();

        apply_schema(&conn).unwrap();

        // Idempotent: applying twice must not fail either.
        apply_schema(&conn).unwrap();

        let has_source_kind: bool = conn
            .prepare(
                "SELECT 1 FROM pragma_table_info('review_changesets') WHERE name = 'source_kind'",
            )
            .unwrap()
            .query_row([], |_| Ok(true))
            .unwrap_or(false);
        assert!(has_source_kind);
    }
}
