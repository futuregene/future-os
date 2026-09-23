//! Agent-owned SQLite worker. Every operation runs on one bounded, ordered
//! connection; a successful write returns only after its transaction commits.

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// A database is worth a full rewrite when more than a quarter of it is free
/// pages and that is at least this many bytes (over this, incremental
/// reclamation would take tens of seconds).
const COMPACT_MIN_FREE_BYTES: i64 = 64 * 1024 * 1024;

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

#[derive(Clone)]
pub(crate) struct Database {
    inner: Arc<Worker>,
}

struct Worker {
    sender: Option<mpsc::SyncSender<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            if thread.thread().id() != std::thread::current().id() {
                let _ = thread.join();
            }
        }
    }
}

fn busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.code == rusqlite::ErrorCode::DatabaseBusy
                || inner.code == rusqlite::ErrorCode::DatabaseLocked
    )
}

/// Open a write transaction that owns the write lock from the start.
///
/// A deferred transaction that reads before it writes fails *immediately* when
/// another connection commits in between — SQLite returns `SQLITE_BUSY_SNAPSHOT`
/// instead of waiting, because waiting would break the snapshot those reads were
/// served from (measured: 0.00 s even with a 5 s `busy_timeout`). That is the
/// "database is locked" seen when the skill registry wrote this same file.
/// Taking the lock at `BEGIN` lets `busy_timeout` apply, and this retry covers a
/// writer that holds it longer than that.
///
/// Retrying here rather than around the whole body is deliberate: once the
/// transaction is open this connection holds the write lock, so no other writer
/// can interfere, and the body keeps ownership of its (possibly huge) payload
/// instead of cloning it once per attempt.
pub(crate) fn begin_immediate(connection: &Connection) -> Result<rusqlite::Transaction<'_>> {
    const ATTEMPTS: usize = 4;
    let mut delay = Duration::from_millis(20);
    let mut attempt = 0;
    loop {
        attempt += 1;
        // `new_unchecked` takes `&Connection` (rusqlite's own API for retry
        // loops); the checked constructor needs `&mut`, which cannot be
        // re-borrowed while the returned transaction borrows it.
        match rusqlite::Transaction::new_unchecked(
            connection,
            rusqlite::TransactionBehavior::Immediate,
        ) {
            Ok(transaction) => return Ok(transaction),
            Err(error) if attempt < ATTEMPTS && busy(&error) => {
                std::thread::sleep(delay);
                delay *= 2;
            }
            Err(error) => return Err(error).context("begin immediate transaction"),
        }
    }
}

/// Deleting a session (or pruning a run) frees pages, and without incremental
/// auto-vacuum those pages stay in the file forever: `agent.db` had reached
/// 7.4 GB holding 1.5 GB of live rows, 79% of it free pages, which is also what
/// makes writes slow enough to lose the write-lock race above.
///
/// `auto_vacuum` only takes effect after a `VACUUM`, so the migration runs once
/// per database. It is deliberately not fatal: a machine without room to
/// rewrite the file keeps working exactly as before and retries next start.
fn ensure_incremental_auto_vacuum(connection: &Connection) {
    const INCREMENTAL: i64 = 2;
    let mode: i64 = match connection.pragma_query_value(None, "auto_vacuum", |row| row.get(0)) {
        Ok(mode) => mode,
        Err(error) => {
            tracing::warn!(%error, "could not read auto_vacuum");
            return;
        }
    };
    if mode == INCREMENTAL {
        return;
    }
    let started = std::time::Instant::now();
    let file_pages = |connection: &Connection| -> i64 {
        connection
            .query_row(
                "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
                [],
                |row| row.get(0),
            )
            .unwrap_or_default()
    };
    let before = file_pages(connection);
    // `VACUUM` cannot run inside a transaction, and this connection has none.
    if let Err(error) = connection
        .execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM;")
        .context("enable incremental auto-vacuum")
    {
        tracing::warn!(
            %error,
            "could not compact agent.db; deleted sessions will keep their space until this succeeds"
        );
        return;
    }
    let after = file_pages(connection);
    tracing::info!(
        before_mb = before / 1_048_576,
        after_mb = after / 1_048_576,
        took_ms = started.elapsed().as_millis() as u64,
        "compacted agent.db once and enabled incremental reclamation"
    );
}

/// Compact a database whose free pages dominate the file.
///
/// Incremental reclamation drains a freelist a page at a time, so a file that
/// was already bloated when this shipped would take minutes to drain — and it
/// is exactly the case in the field (7.4 GB holding 1.5 GB of rows). `VACUUM`
/// rewrites it once instead, at the only moment nobody is waiting on the
/// database: startup. Guarded by a ratio so a healthy database never pays for
/// it, and non-fatal like the migration above.
fn compact_if_bloated(connection: &Connection) {
    let read = |name: &str| -> i64 {
        connection
            .pragma_query_value(None, name, |row| row.get(0))
            .unwrap_or_default()
    };
    let (pages, free, page_size) = (
        read("page_count"),
        read("freelist_count"),
        read("page_size"),
    );
    let free_bytes = free * page_size;
    let bloated = pages > 0 && free * 4 > pages && free_bytes > COMPACT_MIN_FREE_BYTES;
    if !bloated {
        return;
    }
    let started = std::time::Instant::now();
    match connection.execute_batch("VACUUM") {
        Ok(()) => {
            let after = read("page_count") * page_size;
            tracing::info!(
                before_mb = (pages * page_size) / 1_048_576,
                after_mb = after / 1_048_576,
                took_ms = started.elapsed().as_millis() as u64,
                "compacted agent.db; most of it was free pages"
            );
        }
        Err(error) => tracing::warn!(
            %error,
            free_mb = free_bytes / 1_048_576,
            "could not compact agent.db; incremental reclamation will drain it over time"
        ),
    }
}

impl Database {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let path = path.to_path_buf();
        let (sender, receiver) = mpsc::sync_channel::<Job>(256);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("agent-sqlite".into())
            .spawn(move || {
                let mut connection = match open_connection(&path) {
                    Ok(connection) => connection,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                for job in receiver {
                    job(&mut connection);
                }
            })
            .context("start SQLite worker")?;
        ready_rx
            .recv()
            .context("SQLite worker initialization stopped")??;
        Ok(Self {
            inner: Arc::new(Worker {
                sender: Some(sender),
                thread: Some(thread),
            }),
        })
    }

    pub(crate) fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.inner
            .sender
            .as_ref()
            .expect("live SQLite worker sender")
            .try_send(Box::new(move |connection| {
                let _ = reply_tx.send(operation(connection));
            }))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => anyhow!("SQLite operation queue is full"),
                mpsc::TrySendError::Disconnected(_) => anyhow!("SQLite worker is unavailable"),
            })?;
        reply_rx.recv().context("SQLite operation interrupted")?
    }

    /// Hand pages freed by a delete back to the filesystem.
    ///
    /// `incremental_vacuum` pops free pages **one at a time** off the end of the
    /// file and stops when the last page is in use — measured at 3291 calls to
    /// drain a 13 MB freelist (298 ms), while a 6 GB one would take minutes. So
    /// this loops up to `max_pages` to keep a delete's latency predictable, and
    /// a database that stays bloated is compacted when it is next opened
    /// ([`compact_if_bloated`]).
    ///
    /// `min_free_pages` keeps the hot paths cheap: below it there is nothing
    /// worth reclaiming, and the check is a header read.
    pub(crate) fn reclaim(&self, min_free_pages: i64, max_pages: i64) -> Result<()> {
        self.call(move |connection| {
            let free = |connection: &Connection| -> Result<i64> {
                Ok(connection.pragma_query_value(None, "freelist_count", |row| row.get(0))?)
            };
            let mut remaining = free(connection)?;
            if remaining < min_free_pages {
                return Ok(());
            }
            let page_size: i64 =
                connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
            let started = std::time::Instant::now();
            let mut reclaimed = 0;
            // One page comes back per call; the budget bounds a delete's worst
            // case, and the next delete picks up whatever is left.
            while remaining > 0 && reclaimed < max_pages {
                connection.execute_batch("PRAGMA incremental_vacuum")?;
                let now = free(connection)?;
                if now >= remaining {
                    break; // the last page is in use: no further progress
                }
                reclaimed += remaining - now;
                remaining = now;
            }
            if reclaimed > 0 {
                tracing::debug!(
                    reclaimed_mb = (reclaimed * page_size) / 1_048_576,
                    remaining_mb = (remaining * page_size) / 1_048_576,
                    took_ms = started.elapsed().as_millis() as u64,
                    "reclaimed freed pages"
                );
            }
            Ok(())
        })
    }
}

fn open_connection(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create SQLite directory")?;
    }
    let mut connection = Connection::open(path).context("open Agent SQLite database")?;
    connection.busy_timeout(Duration::from_secs(5))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let application: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    if version != 0 && version != 2 && version != 3 && version != 4 {
        bail!("unsupported Agent database schema version {version}");
    }
    if application != 0 && application != 0x46555452 {
        bail!("database belongs to a different application");
    }
    if application == 0 {
        let tables: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if version != 0 || tables != 0 {
            bail!("refusing to initialize an unrecognized database");
        }
    }
    if version == 2 {
        let current_layout: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('entries') WHERE name='metadata_json') AND EXISTS(SELECT 1 FROM pragma_table_info('runs') WHERE name='run_sequence') AND EXISTS(SELECT 1 FROM pragma_table_info('runs') WHERE name='status') AND NOT EXISTS(SELECT 1 FROM pragma_table_info('runs') WHERE name='payload')",
            [],
            |row| row.get(0),
        )?;
        if !current_layout {
            bail!("unsupported pre-release Agent database layout; recreate agent.db");
        }
    }
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "wal_autocheckpoint", 1_000)?;
    connection.pragma_update(None, "journal_size_limit", 8 * 1024 * 1024)?;
    // Before the schema batch: `VACUUM` needs to run with no transaction open.
    ensure_incremental_auto_vacuum(&connection);
    compact_if_bloated(&connection);
    let tx = connection.transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY NOT NULL,
            revision INTEGER NOT NULL DEFAULT 0,
            created_at_ms INTEGER,
            updated_at_ms INTEGER,
            current_metadata_json TEXT CHECK(current_metadata_json IS NULL OR json_valid(current_metadata_json)),
            title TEXT GENERATED ALWAYS AS (json_extract(current_metadata_json,'$.session_name')) VIRTUAL,
            cwd TEXT GENERATED ALWAYS AS (json_extract(current_metadata_json,'$.cwd')) VIRTUAL,
            model TEXT GENERATED ALWAYS AS (json_extract(current_metadata_json,'$.model')) VIRTUAL,
            thinking_level TEXT GENERATED ALWAYS AS (json_extract(current_metadata_json,'$.thinking_level')) VIRTUAL,
            parent_session_id TEXT GENERATED ALWAYS AS (nullif(json_extract(current_metadata_json,'$.parent_session_id'),'')) VIRTUAL
        );
        CREATE TABLE IF NOT EXISTS compaction_operations (
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            input_key TEXT NOT NULL,
            input_digest TEXT NOT NULL,
            operation_id TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('started','completed','failed')),
            result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json)),
            PRIMARY KEY(session_id,input_key)
        );
        CREATE TABLE IF NOT EXISTS fork_operations (
            request_id TEXT PRIMARY KEY NOT NULL,
            request_fingerprint TEXT NOT NULL,
            parent_session_id TEXT NOT NULL,
            child_session_id TEXT NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
            created_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS fork_operations_parent
            ON fork_operations(parent_session_id, created_at_ms);
        UPDATE sessions AS child
        SET created_at_ms = (
            SELECT operation.created_at_ms
            FROM fork_operations operation
            WHERE operation.child_session_id = child.id
        )
        WHERE EXISTS (
            SELECT 1
            FROM fork_operations operation
            WHERE operation.child_session_id = child.id
              AND (
                  child.created_at_ms IS NULL
                  OR child.created_at_ms != operation.created_at_ms
              )
        );
        CREATE TABLE IF NOT EXISTS entries (
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            position INTEGER NOT NULL,
            entry_id TEXT NOT NULL,
            entry_type TEXT NOT NULL,
            role TEXT,
            run_id TEXT,
            timestamp_ms INTEGER,
            metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
            content_json TEXT CHECK(content_json IS NULL OR json_valid(content_json)),
            PRIMARY KEY(session_id, position)
        );
        CREATE TABLE IF NOT EXISTS message_blocks (
            session_id TEXT NOT NULL,
            entry_position INTEGER NOT NULL,
            ordinal INTEGER NOT NULL CHECK(ordinal>=0),
            kind TEXT,
            text TEXT,
            tool_call_id TEXT,
            tool_name TEXT,
            arguments_json TEXT CHECK(arguments_json IS NULL OR json_valid(arguments_json)),
            is_error INTEGER CHECK(is_error IN (0,1)),
            metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
            PRIMARY KEY(session_id,entry_position,ordinal),
            FOREIGN KEY(session_id,entry_position) REFERENCES entries(session_id,position) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS message_blocks_tool ON message_blocks(session_id,tool_call_id,kind);
        CREATE UNIQUE INDEX IF NOT EXISTS entries_identity_unique ON entries(session_id, entry_id);
        DROP INDEX IF EXISTS entries_identity;
        CREATE INDEX IF NOT EXISTS entries_kind ON entries(session_id, entry_type, position);
        CREATE INDEX IF NOT EXISTS entries_run ON entries(session_id,run_id,position);
        CREATE TABLE IF NOT EXISTS runs (
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            run_id TEXT NOT NULL,
            status TEXT NOT NULL,
            run_sequence INTEGER,
            epoch INTEGER,
            started_at_ms INTEGER,
            completed_at_ms INTEGER,
            error TEXT,
            cache_write_tokens INTEGER,
            input_baseline INTEGER,
            cache_read_baseline INTEGER,
            input_tokens INTEGER,
            cache_read_tokens INTEGER,
            output_tokens INTEGER,
            duration_ms INTEGER,
            PRIMARY KEY(session_id,run_id)
        );
        CREATE INDEX IF NOT EXISTS runs_status ON runs(session_id,status,run_sequence);
        CREATE TABLE IF NOT EXISTS run_events (
            sequence INTEGER PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            run_id TEXT NOT NULL,
            epoch INTEGER NOT NULL,
            idx INTEGER NOT NULL,
            event_id TEXT,
            payload TEXT NOT NULL CHECK(json_valid(payload)),
            UNIQUE(session_id, run_id, idx, epoch)
        );
        CREATE TABLE IF NOT EXISTS legacy_imports (
            session_id TEXT PRIMARY KEY NOT NULL,
            status TEXT NOT NULL CHECK(status IN ('imported', 'skipped', 'deleted')),
            fingerprint TEXT NOT NULL,
            error_file TEXT,
            error_line INTEGER,
            error_kind TEXT,
            warnings INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS storage_meta (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS history_shapes (
            session_id TEXT NOT NULL,
            position INTEGER NOT NULL,
            payload TEXT NOT NULL,
            PRIMARY KEY(session_id,position),
            FOREIGN KEY(session_id,position) REFERENCES entries(session_id,position) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS history_display (
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            ordinal INTEGER NOT NULL,
            source_position INTEGER,
            is_user INTEGER NOT NULL,
            payload TEXT NOT NULL,
            PRIMARY KEY(session_id,ordinal)
        );
        CREATE INDEX IF NOT EXISTS history_users ON history_display(session_id,is_user,ordinal);
        PRAGMA application_id = 1179997266;
        PRAGMA user_version = 4;",
    )?;
    tx.execute_batch(crate::skills::registry::SKILLS_TABLE_SQL)?;
    tx.execute_batch(super::records::VIEWS)?;
    tx.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS run_events_custom_id
         ON run_events(session_id,event_id) WHERE event_id IS NOT NULL;",
    )?;
    tx.commit()?;
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_queue_rejects_overload_and_recovers_after_drain() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("agent.db")).unwrap();
        let gate = Arc::new(std::sync::Barrier::new(2));
        let worker_gate = gate.clone();
        db.inner
            .sender
            .as_ref()
            .unwrap()
            .try_send(Box::new(move |_| {
                worker_gate.wait();
                worker_gate.wait();
            }))
            .ok()
            .unwrap();
        gate.wait();
        for _ in 0..256 {
            db.inner
                .sender
                .as_ref()
                .unwrap()
                .try_send(Box::new(|_| {}))
                .ok()
                .unwrap();
        }
        assert!(db
            .call(|_| Ok(()))
            .unwrap_err()
            .to_string()
            .contains("queue is full"));
        gate.wait();
        // The barrier at the back of the queue confirms all accepted jobs ran.
        let (done, done_rx) = mpsc::sync_channel(1);
        db.inner
            .sender
            .as_ref()
            .unwrap()
            .send(Box::new(move |_| {
                done.send(()).unwrap();
            }))
            .ok()
            .unwrap();
        done_rx.recv().unwrap();
        db.call(|_| Ok(())).unwrap();
    }

    #[test]
    fn worker_failure_is_reported_to_current_and_future_callers() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("agent.db")).unwrap();
        assert!(db
            .call::<()>(|_| panic!("synthetic worker failure"))
            .is_err());
        assert!(db.call(|_| Ok(())).is_err());
    }

    #[test]
    fn committed_data_survives_reopen_and_failed_transaction_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        let db = Database::open(&path).unwrap();
        db.call(|connection| {
            connection.execute("INSERT INTO sessions(id) VALUES ('kept')", [])?;
            Ok(())
        })
        .unwrap();
        assert!(db
            .call::<()>(|connection| {
                let tx = connection.transaction()?;
                tx.execute("INSERT INTO sessions(id) VALUES ('rolled-back')", [])?;
                bail!("injected failure before commit")
            })
            .is_err());
        drop(db);
        let db = Database::open(&path).unwrap();
        assert_eq!(
            db.call(|connection| Ok(connection.query_row(
                "SELECT count(*) FROM sessions",
                [],
                |row| row.get::<_, i64>(0),
            )?))
            .unwrap(),
            1
        );
    }

    #[test]
    fn connection_bounds_wal_growth() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("agent.db")).unwrap();
        let settings = db
            .call(|connection| {
                Ok((
                    connection.pragma_query_value(None, "wal_autocheckpoint", |row| {
                        row.get::<_, i64>(0)
                    })?,
                    connection.pragma_query_value(None, "journal_size_limit", |row| {
                        row.get::<_, i64>(0)
                    })?,
                ))
            })
            .unwrap();
        assert_eq!(settings, (1_000, 8 * 1024 * 1024));
    }

    #[test]
    fn cascade_preserves_import_tombstone() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("agent.db")).unwrap();
        db.call(|connection| {
            connection.execute_batch("INSERT INTO sessions(id) VALUES ('s');
                INSERT INTO entries(session_id,position,entry_id,entry_type,metadata_json) VALUES ('s', 0, 'e', 'user', '{}');
                INSERT INTO legacy_imports(session_id,status,fingerprint) VALUES ('s','imported','hash');
                DELETE FROM sessions WHERE id='s';")?;
            let entries: i64 = connection.query_row("SELECT count(*) FROM entries", [], |r| r.get(0))?;
            let imports: i64 = connection.query_row("SELECT count(*) FROM legacy_imports", [], |r| r.get(0))?;
            assert_eq!((entries, imports), (0, 1));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn opening_repairs_fork_session_creation_time_from_the_operation_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        let db = Database::open(&path).unwrap();
        db.call(|connection| {
            connection.execute_batch(
                "INSERT INTO sessions(id,created_at_ms) VALUES ('parent',100),('child',100);
                 INSERT INTO fork_operations(
                    request_id,request_fingerprint,parent_session_id,
                    child_session_id,created_at_ms
                 ) VALUES ('request','{}','parent','child',200);",
            )?;
            Ok(())
        })
        .unwrap();
        drop(db);

        let reopened = Database::open(&path).unwrap();
        let created_at_ms = reopened
            .call(|connection| {
                Ok(connection.query_row(
                    "SELECT created_at_ms FROM sessions WHERE id='child'",
                    [],
                    |row| row.get::<_, i64>(0),
                )?)
            })
            .unwrap();
        assert_eq!(created_at_ms, 200);
    }

    #[test]
    fn v2_database_is_upgraded_to_v3_with_the_skills_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        // A v2 database from the previous release: recognized application_id,
        // current v2 layout, no skills table yet.
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE entries (
                    session_id TEXT NOT NULL,
                    position INTEGER NOT NULL,
                    entry_id TEXT NOT NULL,
                    entry_type TEXT NOT NULL,
                    run_id TEXT,
                    metadata_json TEXT NOT NULL,
                    PRIMARY KEY(session_id, position)
                );
                 CREATE TABLE runs (
                    session_id TEXT NOT NULL,
                    run_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    run_sequence INTEGER,
                    PRIMARY KEY(session_id,run_id)
                 );
                 PRAGMA application_id = 1179997266;
                 PRAGMA user_version = 2;",
            )
            .unwrap();
        drop(connection);

        let db = Database::open(&path).unwrap();
        db.call(|connection| {
            let version: i64 =
                connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
            let skills: i64 =
                connection.query_row("SELECT count(*) FROM skills", [], |row| row.get(0))?;
            assert_eq!((version, skills), (4, 0));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn future_schema_is_rejected_without_downgrade() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 99)
            .unwrap();
        assert!(Database::open(&path).is_err());
        assert_eq!(
            Connection::open(path)
                .unwrap()
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            99
        );
    }

    #[test]
    fn pre_release_layout_is_rejected_instead_of_upgraded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE run_events (
                    session_id TEXT NOT NULL,
                    run_id TEXT NOT NULL,
                    epoch INTEGER NOT NULL,
                    idx INTEGER NOT NULL,
                    event_id TEXT NOT NULL,
                    payload TEXT NOT NULL
                );
                PRAGMA application_id = 1179997266;
                PRAGMA user_version = 2;",
            )
            .unwrap();
        drop(connection);

        let error = match Database::open(&path) {
            Ok(_) => panic!("pre-release layout unexpectedly opened"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("pre-release Agent database layout"));
        let connection = Connection::open(path).unwrap();
        let still_old: bool = connection
            .query_row(
                "SELECT NOT EXISTS(
                    SELECT 1 FROM pragma_table_info('run_events') WHERE name='sequence'
                )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(still_old);
    }

    #[test]
    fn entry_identity_is_unique_within_a_session_not_globally() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("agent.db")).unwrap();
        db.call(|db| {
            db.execute_batch(
                "INSERT INTO sessions(id) VALUES ('one'),('two');
                INSERT INTO entries(session_id,position,entry_id,entry_type,metadata_json)
                VALUES ('one',0,'same','user','{}'),('two',0,'same','user','{}');",
            )?;
            let error = db
                .execute(
                    "INSERT INTO entries(session_id,position,entry_id,entry_type,metadata_json)
                 VALUES ('one',1,'same','user','{}')",
                    [],
                )
                .unwrap_err();
            assert_eq!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::ConstraintViolation)
            );
            let count: i64 = db.query_row("SELECT count(*) FROM entries", [], |row| row.get(0))?;
            assert_eq!(count, 2);
            Ok(())
        })
        .unwrap();
    }
}
