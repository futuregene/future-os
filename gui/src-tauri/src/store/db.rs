//! Database connection plumbing — app directory layout, the SQLite connection
//! factory, schema application — plus a handful of small cross-domain row
//! lookups shared by several store modules.

use rusqlite::{params, Connection, OptionalExtension};
use std::{fs, path::PathBuf};

use super::approvals::{
    approval_request_from_row, ApprovalRequestRecord, APPROVAL_REQUEST_COLUMNS,
};
use super::runs::{run_from_row, RunRecord, RUN_COLUMNS};
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

pub fn chat_workspace_path(id: &str) -> Result<PathBuf, crate::AppError> {
    Ok(chat_workspaces_root()?.join(id))
}

/// Root of the per-thread temporary chat workspaces
/// (`~/.future/workspaces/chat`).  Each subdir is named after the agent
/// session id (when known, e.g. from import) or the thread id (new GUI
/// threads).  Reclaimed by `reconcile_orphan_chat_workspaces` and by
/// `clear_all_data`.  User workspaces live at their own user-chosen paths
/// (never under here), so this reclamation can never touch them.
pub(super) fn chat_workspaces_root() -> Result<PathBuf, crate::AppError> {
    Ok(future_dir()?.join("workspaces").join("chat"))
}

/// `$HOME/.future/` — the FutureOS root on disk.
pub fn future_dir() -> Result<PathBuf, crate::AppError> {
    let home = crate::home_dir().ok_or("HOME/USERPROFILE environment variable is not set.")?;
    Ok(PathBuf::from(home).join(".future"))
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
    // `app_dir()` holds app.db itself; it must exist before `connect()` opens
    // the database. The chat-workspace root moved out from under `app/`, so
    // creating it no longer implicitly creates `app/` — create both explicitly
    // (a fresh install has neither).
    fs::create_dir_all(app_dir()?)?;
    fs::create_dir_all(chat_workspaces_root()?).map_err(crate::AppError::from)
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
    // `artifact_type` against a table that still has the old `type` column —
    // silently dropping artifacts. Idempotent: skip when already migrated.
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
    // Fold duplicate file artifacts before the unique index over them is
    // created; on DBs written by older builds it would otherwise fail.
    dedupe_file_artifacts(conn)?;
    // Indexes over added columns run last, once those columns are guaranteed.
    for statement in ADDED_INDEXES {
        conn.execute(statement, [])?;
    }
    // Drop tables removed from the schema (see DROPPED_TABLES).
    // Disable FK enforcement to allow dropping tables referenced by other tables.
    // Best-effort: a missing table (fresh DB) or FK conflict (stale DB) shouldn't block startup.
    if let Err(e) = conn.execute_batch("PRAGMA foreign_keys = OFF;") {
        eprintln!("FutureOS migration: PRAGMA foreign_keys=OFF failed: {e}");
    }
    for table in DROPPED_TABLES {
        if let Err(e) = conn.execute(&format!("DROP TABLE IF EXISTS {table}"), []) {
            eprintln!("FutureOS migration: DROP TABLE {table} failed: {e}");
        }
    }
    if let Err(e) = conn.execute_batch("PRAGMA foreign_keys = ON;") {
        eprintln!("FutureOS migration: PRAGMA foreign_keys=ON failed: {e}");
    }
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

/// Collapse the artifact rows older builds inserted one-per-write/edit of the
/// same file down to the one row per (thread_id, path) that `ensure_artifact`
/// now maintains and `idx_artifacts_thread_path` enforces.
///
/// PRE-RELEASE ONLY — delete this before release, with its tests, its call in
/// `apply_schema`, and the `ADDED_INDEXES` carve-out that exists to sequence it
/// (move `idx_artifacts_thread_path` into `SCHEMA` then). It exists solely to
/// fold the duplicates sitting in already-populated development databases; no
/// database that has only ever seen a post-change build can hold them, so once
/// none is in play this is dead weight that deletes user rows on every launch.
///
/// The survivor is the group's most recently touched row — it already carries
/// the latest run_id/summary/content — and it inherits the group's earliest
/// `created_at` so the Panel still shows when the file was first produced. The
/// rows it replaces are derived records of that same file, re-derivable from the
/// agent's tool events, so they're deleted outright rather than tombstoned.
///
/// Rows with a NULL `thread_id` or NULL `path` are left alone: neither has a file
/// identity to collapse, and the partial unique index excludes them as well.
fn dedupe_file_artifacts(conn: &Connection) -> Result<(), crate::AppError> {
    const SCOPE: &str = "deleted_at IS NULL AND thread_id IS NOT NULL AND path IS NOT NULL";
    const SURVIVOR: &str = "SELECT k.id
           FROM artifacts k
           WHERE k.thread_id = artifacts.thread_id
             AND k.path = artifacts.path
             AND k.deleted_at IS NULL
           ORDER BY k.updated_at DESC, k.rowid DESC
           LIMIT 1";

    conn.execute(
        &format!(
            "UPDATE artifacts
             SET created_at = (
                 SELECT MIN(d.created_at)
                 FROM artifacts d
                 WHERE d.thread_id = artifacts.thread_id
                   AND d.path = artifacts.path
                   AND d.deleted_at IS NULL
             )
             WHERE {SCOPE} AND id = ({SURVIVOR})"
        ),
        [],
    )?;
    conn.execute(
        &format!("DELETE FROM artifacts WHERE {SCOPE} AND id <> ({SURVIVOR})"),
        [],
    )?;
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

// get_run_event removed — run_events table dropped

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

    /// A migrated DB holding artifact rows, with FKs off — `dedupe_file_artifacts`
    /// only reads `artifacts`, so workspace/thread/run fixtures would be noise.
    fn artifact_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        conn
    }

    /// The same, minus the unique index — a database as an older build left it,
    /// free to hold one artifact row per write/edit of a file. Re-running
    /// `apply_schema` on it is what a user's first launch after this change does.
    fn legacy_artifact_db() -> Connection {
        let conn = artifact_db();
        conn.execute("DROP INDEX idx_artifacts_thread_path", [])
            .unwrap();
        conn
    }

    fn insert_artifact(
        conn: &Connection,
        id: &str,
        thread_id: Option<&str>,
        path: Option<&str>,
        created_at: i64,
        updated_at: i64,
        summary: &str,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO artifacts (
                 id, workspace_id, thread_id, run_id, title, artifact_type, path,
                 content, content_storage, summary, created_at, updated_at
             ) VALUES (?1, 'ws', ?2, ?3, 'report.md', 'document', ?4, NULL, 'file', ?5, ?6, ?7)",
            params![
                id,
                thread_id,
                format!("run_{id}"),
                path,
                summary,
                created_at,
                updated_at
            ],
        )
    }

    fn live_artifact_count(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM artifacts WHERE deleted_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn dedupe_folds_repeat_touches_of_one_file() {
        // One file written then edited twice — three rows, as older builds wrote.
        let conn = legacy_artifact_db();
        let file = Some("/ws/report.md");
        insert_artifact(&conn, "a1", Some("t1"), file, 100, 100, "Written by Agent.").unwrap();
        insert_artifact(&conn, "a2", Some("t1"), file, 200, 200, "Edited by Agent.").unwrap();
        insert_artifact(&conn, "a3", Some("t1"), file, 300, 300, "Edited by Agent.").unwrap();

        apply_schema(&conn).unwrap();

        assert_eq!(live_artifact_count(&conn), 1);
        let (id, created_at, updated_at): (String, i64, i64) = conn
            .query_row(
                "SELECT id, created_at, updated_at FROM artifacts",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(id, "a3", "the latest touch survives");
        assert_eq!(created_at, 100, "carrying the first sighting");
        assert_eq!(updated_at, 300);
    }

    #[test]
    fn dedupe_keeps_rows_with_no_shared_file_identity() {
        let conn = legacy_artifact_db();
        insert_artifact(&conn, "a1", Some("t1"), Some("/ws/a.md"), 100, 100, "").unwrap();
        insert_artifact(&conn, "a2", Some("t1"), Some("/ws/b.md"), 100, 100, "").unwrap();
        // Same file, but a different thread is a different work product.
        insert_artifact(&conn, "a3", Some("t2"), Some("/ws/a.md"), 100, 100, "").unwrap();
        // Path-less inline artifacts have no file identity to fold on.
        insert_artifact(&conn, "a4", Some("t1"), None, 100, 100, "").unwrap();
        insert_artifact(&conn, "a5", Some("t1"), None, 100, 100, "").unwrap();

        apply_schema(&conn).unwrap();

        assert_eq!(live_artifact_count(&conn), 5);
    }

    #[test]
    fn dedupe_and_index_ignore_tombstoned_rows() {
        // A user-deleted artifact must survive the fold, and must not block the
        // Agent from recording that same file again afterwards.
        let conn = legacy_artifact_db();
        let file = Some("/ws/report.md");
        insert_artifact(&conn, "a1", Some("t1"), file, 100, 100, "").unwrap();
        conn.execute("UPDATE artifacts SET deleted_at = 150 WHERE id = 'a1'", [])
            .unwrap();
        insert_artifact(&conn, "a2", Some("t1"), file, 200, 200, "").unwrap();

        apply_schema(&conn).unwrap();

        assert_eq!(live_artifact_count(&conn), 1);
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM artifacts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 2, "the tombstone is left alone");
    }

    #[test]
    fn unique_index_rejects_a_second_live_row_for_one_file() {
        let conn = artifact_db();
        let file = Some("/ws/report.md");
        insert_artifact(&conn, "a1", Some("t1"), file, 100, 100, "").unwrap();
        let duplicate = insert_artifact(&conn, "a2", Some("t1"), file, 200, 200, "");
        assert!(duplicate.is_err(), "idx_artifacts_thread_path must hold");
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
}
