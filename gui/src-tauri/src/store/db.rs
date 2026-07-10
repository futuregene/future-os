//! Database connection plumbing — app directory layout, the SQLite connection
//! factory, schema application — plus a handful of small cross-domain row
//! lookups shared by several store modules.

use rusqlite::{params, Connection, OptionalExtension};
use std::{fs, path::PathBuf};

use super::approvals::{
    approval_request_from_row, ApprovalRequestRecord, APPROVAL_REQUEST_COLUMNS,
};
use super::runs::{
    run_event_from_row, run_from_row, RunEventRecord, RunRecord, RUN_COLUMNS, RUN_EVENT_COLUMNS,
};
use super::schema::{
    ADDED_COLUMNS, ADDED_INDEXES, DROPPED_COLUMNS, DROPPED_TABLES, RENAMED_COLUMNS, SCHEMA,
};

pub(super) fn app_dir() -> Result<PathBuf, crate::AppError> {
    let home = crate::home_dir().ok_or("HOME/USERPROFILE environment variable is not set.")?;
    Ok(PathBuf::from(home).join(".future").join("app"))
}

pub(super) fn db_path() -> Result<PathBuf, crate::AppError> {
    Ok(app_dir()?.join("app.db"))
}

pub(super) fn chat_workspace_path(thread_id: &str) -> Result<PathBuf, crate::AppError> {
    Ok(chat_workspaces_root()?.join(thread_id))
}

/// Root of the per-thread temporary chat workspaces (`~/.future/app/workspaces/
/// chat`). Each `<thread_id>` subdir is scratch space for one chat conversation;
/// reclaimed by `reconcile_orphan_chat_workspaces` and by `clear_all_data`.
/// User workspaces live at their own user-chosen paths (never under here), so
/// this reclamation can never touch them.
pub(super) fn chat_workspaces_root() -> Result<PathBuf, crate::AppError> {
    Ok(app_dir()?.join("workspaces").join("chat"))
}

/// Root of the per-workspace shadow-review git repos (`~/.future/app/review`).
/// Each `<workspace_id>` subdir is the shadow repo shared by that workspace's
/// runs; reclaimed by `reconcile_orphan_review_repos` and by `clear_all_data`.
pub(super) fn review_repos_root() -> Result<PathBuf, crate::AppError> {
    Ok(app_dir()?.join("review"))
}

/// Root of the per-thread image tree (`~/.future/app/images`). Holds attachment
/// thumbnails (both modes) and workspace-mode image originals — a persistent
/// location, unlike the OS app cache dir which macOS may purge. Reclaimed by
/// `reconcile_orphan_images` and by `clear_all_data`.
pub fn app_images_root() -> Result<PathBuf, crate::AppError> {
    Ok(app_dir()?.join("images"))
}

/// Per-thread image directory: `~/.future/app/images/<thread_id>` (with
/// `thumb/` and, for workspace conversations, `origin/` subdirs).
pub fn thread_images_dir(thread_id: &str) -> Result<PathBuf, crate::AppError> {
    Ok(app_images_root()?.join(thread_id))
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
    // Rename columns on databases created before the N-3 rename. `CREATE TABLE
    // IF NOT EXISTS` can't do it, and without this the store reads/writes
    // `event_type`/`artifact_type`/`resource_type` against tables that still
    // have the old `type` column — silently dropping run events, artifacts, and
    // research resources. Idempotent: skip when already migrated.
    for (table, old, new) in RENAMED_COLUMNS {
        if column_exists(conn, table, old)? && !column_exists(conn, table, new)? {
            conn.execute(
                &format!("ALTER TABLE {table} RENAME COLUMN {old} TO {new}"),
                [],
            )?;
        }
    }
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
    // Drop tables removed from the schema (see DROPPED_TABLES).
    // Disable FK enforcement to allow dropping tables referenced by other tables.
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
    for table in DROPPED_TABLES {
        conn.execute(&format!("DROP TABLE IF EXISTS {table}"), [])?;
    }
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    // Drop columns removed from the schema (see DROPPED_COLUMNS).
    for (table, column) in DROPPED_COLUMNS {
        if column_exists(conn, table, column)? {
            let sql = format!("ALTER TABLE {table} DROP COLUMN {column}");
            if let Err(error) = conn.execute(&sql, []) {
                // DROP COLUMN can fail if the column is referenced by an index
                // or is the last column — log and continue.
                eprintln!("FutureOS migration: failed to drop {table}.{column}: {error}");
            }
        }
    }
    Ok(())
}

fn is_duplicate_column_error(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(_, Some(message)) if message.contains("duplicate column name"))
}

/// Whether `table` has a column named `column`. `table`/`column` come from the
/// `RENAMED_COLUMNS` constant (never user input), so interpolation is safe.
fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool, crate::AppError> {
    let count: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = '{column}'"),
        [],
        |row| row.get(0),
    )?;
    Ok(count > 0)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_schema_on_fresh_db_succeeds() {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
    }

    #[test]
    fn apply_schema_drops_removed_tables() {
        // A database created by the old schema still has the four unused tables.
        // The migration must drop them (and stay idempotent when run again).
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE data_sources (id TEXT PRIMARY KEY);
             CREATE TABLE data_credentials (id TEXT PRIMARY KEY);
             CREATE TABLE skills (id TEXT PRIMARY KEY);
             CREATE TABLE skill_enablements (id TEXT PRIMARY KEY);",
        )
        .unwrap();

        apply_schema(&conn).unwrap();
        apply_schema(&conn).unwrap();

        for table in DROPPED_TABLES {
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |_| Ok(true),
                )
                .optional()
                .unwrap()
                .unwrap_or(false);
            assert!(!exists, "{table} should have been dropped");
        }
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

    #[test]
    fn apply_schema_renames_legacy_type_columns() {
        // A DB created before N-3 still has `run_events.type`. The store now
        // reads/writes `event_type`; without the rename migration, run events
        // are silently dropped (e.g. a Run's streaming activity never lands).
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE run_events (
                 id TEXT PRIMARY KEY,
                 run_id TEXT NOT NULL,
                 type TEXT NOT NULL,
                 payload TEXT,
                 sequence INTEGER NOT NULL,
                 created_at INTEGER NOT NULL
             );
             INSERT INTO run_events (id, run_id, type, payload, sequence, created_at)
             VALUES ('e1', 'r1', 'text_chunk', '{}', 0, 1);",
        )
        .unwrap();

        apply_schema(&conn).unwrap();
        // Idempotent: a fresh DB already has `event_type`, so re-running is a no-op.
        apply_schema(&conn).unwrap();

        assert!(
            column_exists(&conn, "run_events", "event_type").unwrap(),
            "type must be renamed to event_type"
        );
        assert!(
            !column_exists(&conn, "run_events", "type").unwrap(),
            "old type column must be gone"
        );
        // The pre-existing row survives the rename and is readable under the new name.
        let kind: String = conn
            .query_row(
                "SELECT event_type FROM run_events WHERE id = 'e1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kind, "text_chunk");
    }
}
