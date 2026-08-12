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

/// Root of the per-thread attachment tree (`~/.future/app/images`). Holds image
/// thumbnails plus originals that have no stable desktop path (pastes and
/// mobile uploads) — a persistent location, unlike the OS app cache dir which
/// macOS may purge. Reclaimed by `reconcile_orphan_images` and `clear_all_data`.
pub fn app_images_root() -> Result<PathBuf, crate::AppError> {
    Ok(app_dir()?.join("images"))
}

/// Per-thread attachment directory: `~/.future/app/images/<thread_id>` (with
/// `thumb/` and `origin/` subdirs).
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

/// Maximum idle connections kept warm in the pool. WAL allows one writer plus
/// concurrent readers, and the store's queries are all small, so a handful of
/// connections covers every poll path without queueing.
const POOL_MAX_IDLE: usize = 4;

/// Process-wide connection pool. Previously every store call opened a fresh
/// SQLite connection (file open + 3 PRAGMAs + WAL handshake) — several times
/// per second on the 1.5s poll paths, and 3× per artifact write. Connections
/// are keyed to the current db path: tests that override HOME get a fresh pool
/// instead of stale connections into the previous HOME's database.
static POOL: std::sync::LazyLock<std::sync::Mutex<(std::path::PathBuf, Vec<Connection>)>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new((std::path::PathBuf::new(), Vec::new())));

/// A connection checked out from [`POOL`]; returns to the pool on drop.
/// Derefs to [`Connection`], so existing call sites (queries, prepared
/// statements, `transaction()` via `DerefMut`) work unchanged.
pub(super) struct PooledConnection {
    conn: Option<Connection>,
}

impl std::ops::Deref for PooledConnection {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn
            .as_ref()
            .expect("pooled connection live until drop")
    }
}

impl std::ops::DerefMut for PooledConnection {
    fn deref_mut(&mut self) -> &mut Connection {
        self.conn
            .as_mut()
            .expect("pooled connection live until drop")
    }
}

impl Drop for PooledConnection {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else {
            return;
        };
        let Ok(path) = db_path() else {
            return;
        };
        if let Ok(mut pool) = POOL.lock() {
            // Return only if the pool still points at this database (a HOME
            // override mid-process swaps the path) and it has room.
            if pool.0 == path && pool.1.len() < POOL_MAX_IDLE {
                pool.1.push(conn);
            }
        }
    }
}

/// PRAGMAs applied to every fresh connection. Hoisted to a const so the
/// `execute_batch` call stays a single (single-edge) line.
const CONNECTION_PRAGMAS: &str = "PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA journal_mode = WAL;";

pub(super) fn connect() -> Result<PooledConnection, crate::AppError> {
    let path = db_path()?;
    if let Ok(mut pool) = POOL.lock() {
        if pool.0 != path {
            // HOME changed (test override) — drop connections into the old
            // database and re-key the pool.
            pool.1.clear();
            pool.0 = path.clone();
        }
        if let Some(conn) = pool.1.pop() {
            // Directory creation is skipped on the pooled path — the dirs
            // provably existed when the pooled connection was first opened.
            return Ok(PooledConnection { conn: Some(conn) });
        }
    }
    ensure_app_dirs()?;
    let conn = Connection::open(path)?;
    conn.execute_batch(CONNECTION_PRAGMAS)?;
    Ok(PooledConnection { conn: Some(conn) })
}

pub(super) fn apply_schema(conn: &Connection) -> Result<(), crate::AppError> {
    conn.execute_batch(SCHEMA)?;
    // Rename columns on databases created before the N-3 rename. `CREATE TABLE
    // IF NOT EXISTS` can't do it, and without this the store reads/writes
    // `artifact_type` against a table that still has the old `type` column —
    // silently dropping artifacts. Idempotent: skip when already migrated.
    for (table, old, new) in RENAMED_COLUMNS {
        if column_exists(conn, table, old)? && !column_exists(conn, table, new)? {
            let sql = format!("ALTER TABLE {table} RENAME COLUMN {old} TO {new}");
            conn.execute(&sql, [])?;
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
    let _ = conn.execute_batch("PRAGMA foreign_keys = OFF;");
    for table in DROPPED_TABLES {
        if let Err(e) = conn.execute(&format!("DROP TABLE IF EXISTS {table}"), []) {
            eprintln!("FutureOS migration: DROP TABLE {table} failed: {e}");
        }
    }
    let _ = conn.execute_batch("PRAGMA foreign_keys = ON;");
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
/// Legacy compatibility cleanup for databases written before the release
/// baseline. Do not extend this startup pass for new schema changes: post-release
/// changes require a dedicated, versioned migration (see `desktop/CLAUDE.md`). Its
/// removal or replacement must itself be handled as a migration after confirming
/// that all supported upgrade paths have crossed this historical boundary.
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

    let update_sql = format!(
        "UPDATE artifacts
         SET created_at = (
             SELECT MIN(d.created_at)
             FROM artifacts d
             WHERE d.thread_id = artifacts.thread_id
               AND d.path = artifacts.path
               AND d.deleted_at IS NULL
         )
         WHERE {SCOPE} AND id = ({SURVIVOR})"
    );
    conn.execute(&update_sql, [])?;
    let delete_sql =
        format!("DELETE FROM artifacts WHERE {SCOPE} AND id <> ({SURVIVOR})");
    conn.execute(&delete_sql, [])?;
    Ok(())
}

fn is_duplicate_column_error(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(_, Some(message)) if message.contains("duplicate column name"))
}

/// Whether `table` has a column named `column`. `table`/`column` come from the
/// `RENAMED_COLUMNS` constant (never user input), so interpolation is safe.
fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool, crate::AppError> {
    let sql = format!(
        "SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = '{column}'"
    );
    let count: i64 = conn.query_row(&sql, [], |row| row.get(0))?;
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
pub(crate) mod test_support {
    //! Shared fixtures for store tests: an in-memory database with the full
    //! schema applied, and a fake-HOME-guarded `connect()` (serialized on the
    //! process-wide TEST_HOME_LOCK via HomeGuard).
    use rusqlite::Connection;

    /// In-memory database with the full schema applied (no HOME needed).
    pub(crate) fn memory_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        super::apply_schema(&conn).expect("apply schema");
        conn
    }

    /// `connect()` against a fresh fake HOME, schema applied. The returned
    /// guard must outlive all use of the connection pool for this database.
    pub(crate) fn guarded_conn(
        label: &str,
    ) -> (
        crate::auth_store::test_support::HomeGuard,
        super::PooledConnection,
    ) {
        let home = crate::auth_store::test_support::HomeGuard::new(label);
        let conn = super::connect().expect("connect");
        super::apply_schema(&conn).expect("apply schema");
        (home, conn)
    }
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

    #[test]
    fn apply_schema_renames_legacy_type_column() {
        // A database from before the `type` → `artifact_type` rename. The
        // legacy table must carry every column the migration touches (the
        // dedupe pass and the added indexes read them).
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE artifacts (
                 id TEXT PRIMARY KEY,
                 workspace_id TEXT,
                 thread_id TEXT,
                 path TEXT,
                 type TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 deleted_at INTEGER
             );
             INSERT INTO artifacts (id, type, created_at, updated_at)
             VALUES ('a1', 'document', 1, 1);",
        )
        .unwrap();

        apply_schema(&conn).unwrap();

        assert!(!column_exists(&conn, "artifacts", "type").unwrap());
        assert!(column_exists(&conn, "artifacts", "artifact_type").unwrap());
        let artifact_type: String = conn
            .query_row("SELECT artifact_type FROM artifacts WHERE id = 'a1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(artifact_type, "document");
    }

    #[test]
    fn apply_schema_propagates_non_duplicate_alter_errors() {
        // A table at SQLite's column ceiling (2000): ADD COLUMN fails with
        // "too many columns" — not a duplicate-column error — so the
        // migration must surface it instead of swallowing it.
        let conn = Connection::open_in_memory().unwrap();
        let mut columns = vec![
            "id TEXT PRIMARY KEY".to_string(),
            // Columns the schema's indexes reference.
            "thread_id TEXT".to_string(),
            "run_id TEXT".to_string(),
            "status TEXT".to_string(),
        ];
        for index in 0..1996 {
            columns.push(format!("c{index} TEXT"));
        }
        conn.execute_batch(&format!(
            "CREATE TABLE approval_requests ({})",
            columns.join(", ")
        ))
        .unwrap();

        let result = apply_schema(&conn);
        assert!(result.is_err(), "ALTER past the column ceiling must abort");
    }

    #[test]
    fn apply_schema_logs_and_continues_past_undroppable_objects() {
        let conn = Connection::open_in_memory().unwrap();
        // A legacy `threads` table whose dropped column is pinned by an index
        // (DROP COLUMN refuses indexed columns). It must also carry every
        // column the schema's own indexes reference, or the SCHEMA batch
        // itself fails before the migration steps run.
        conn.execute_batch(
            "CREATE TABLE threads (
                 id TEXT PRIMARY KEY,
                 workspace_id TEXT,
                 status TEXT,
                 pinned INTEGER,
                 last_message_at INTEGER,
                 updated_at INTEGER,
                 model_provider TEXT,
                 model_id TEXT,
                 thinking_level TEXT
             );
             CREATE INDEX idx_legacy_provider ON threads(model_provider);",
        )
        .unwrap();
        // …and a view squatting on a dropped table's name (DROP TABLE refuses
        // views). Both failures are logged, neither aborts the migration.
        conn.execute_batch("CREATE VIEW skills AS SELECT 1 AS id;").unwrap();

        apply_schema(&conn).unwrap();

        assert!(
            column_exists(&conn, "threads", "model_provider").unwrap(),
            "the indexed column survives the best-effort drop"
        );
        assert!(
            !column_exists(&conn, "threads", "thinking_level").unwrap(),
            "unindexed dropped columns are still dropped"
        );
    }

    #[test]
    fn run_thread_id_reads_the_owning_thread() {
        let conn = test_support::memory_conn();
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, created_at, updated_at
             ) VALUES ('t1', 'ws1', 'chat', 'T', 1, 1);
             INSERT INTO runs (id, thread_id, status, created_at, updated_at)
             VALUES ('r1', 't1', 'running', 1, 1);",
        )
        .unwrap();

        assert_eq!(run_thread_id(&conn, "r1").unwrap(), "t1");
        assert!(run_thread_id(&conn, "ghost").is_err());
    }

    #[test]
    fn path_helpers_resolve_under_the_fake_home() {
        let home = crate::auth_store::test_support::HomeGuard::new("db_paths");
        let root = std::env::var("HOME").unwrap();
        let root = std::path::Path::new(&root);
        assert_eq!(
            chat_workspace_path("sess1").unwrap(),
            root.join(".future/workspaces/chat/sess1")
        );
        assert_eq!(
            thread_images_dir("t1").unwrap(),
            root.join(".future/app/images/t1")
        );
        drop(home);
    }

    #[test]
    fn get_approval_request_round_trips() {
        let (_home, conn) = test_support::guarded_conn("db_get_approval");
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, created_at, updated_at
             ) VALUES ('t1', 'ws1', 'chat', 'T', 1, 1);
             INSERT INTO approval_requests (
                 id, thread_id, kind, status, title, created_at, updated_at
             ) VALUES ('a1', 't1', 'shell', 'pending', 'Deploy', 1, 1);",
        )
        .unwrap();
        drop(conn);

        let found = get_approval_request("a1").unwrap().expect("present");
        assert_eq!(found.title, "Deploy");
        assert!(get_approval_request("ghost").unwrap().is_none());
    }

    #[test]
    fn dropping_a_connection_without_a_home_or_pool_just_drops() {
        use std::sync::MutexGuard;

        /// Removes HOME for the guard's lifetime (serialized against other
        /// HOME-mutating tests) so `db_path()` fails inside `Drop`.
        struct NoHomeGuard {
            previous: Option<String>,
            _lock: MutexGuard<'static, ()>,
        }
        impl NoHomeGuard {
            fn new() -> Self {
                let lock = crate::TEST_HOME_LOCK
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                let previous = std::env::var("HOME").ok();
                std::env::remove_var("HOME");
                NoHomeGuard {
                    previous,
                    _lock: lock,
                }
            }
        }
        impl Drop for NoHomeGuard {
            fn drop(&mut self) {
                if let Some(value) = &self.previous {
                    std::env::set_var("HOME", value);
                }
            }
        }

        let _no_home = NoHomeGuard::new();
        // The `conn: None` and `db_path()`-fails drop arms.
        drop(PooledConnection { conn: None });
        drop(PooledConnection {
            conn: Some(Connection::open_in_memory().unwrap()),
        });
    }

    #[test]
    fn poisoned_pool_lock_degrades_gracefully() {
        // Poison the pool mutex so both `POOL.lock()` Err edges (connect and
        // Drop) fire; then clear the poison so later tests pool normally.
        let _ = std::panic::catch_unwind(|| {
            let _guard = POOL.lock().expect("lock pool");
            panic!("intentional: poison the connection pool for the Err-edge test");
        });
        assert!(POOL.lock().is_err(), "pool is poisoned");

        let home = crate::auth_store::test_support::HomeGuard::new("db_poisoned_pool");
        // connect() falls back to a fresh open…
        let conn = connect().expect("connect despite the poisoned pool");
        // …and dropping it skips the pool return.
        drop(conn);

        POOL.clear_poison();
        drop(home);
    }
}
