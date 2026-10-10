//! Agent-owned SQLite worker. Every operation runs on one bounded, ordered
//! connection; a successful write returns only after its transaction commits.

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

#[derive(Clone)]
pub(crate) struct Database {
    inner: Arc<Worker>,
}

struct Worker {
    reclaim_requested: Arc<AtomicBool>,
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

/// Enable incremental reclamation only when no existing data needs rewriting.
/// Existing NONE-mode databases keep freed pages available for SQLite reuse;
/// startup never performs a full VACUUM or changes their storage layout.
fn configure_incremental_auto_vacuum(connection: &Connection) -> Result<()> {
    let mode: i64 = connection.pragma_query_value(None, "auto_vacuum", |row| row.get(0))?;
    let empty: bool = connection.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%')",
        [],
        |row| row.get(0),
    )?;
    // FULL and INCREMENTAL share the pointer-map layout. Switching between
    // them needs no rebuild; a NONE-mode populated database is left alone.
    if empty || mode == 1 {
        connection.pragma_update(None, "auto_vacuum", 2)?;
    }
    Ok(())
}

impl Database {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let path = path.to_path_buf();
        let (sender, receiver) = mpsc::sync_channel::<Job>(256);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let reclaim_requested = Arc::new(AtomicBool::new(true));
        let worker_reclaim = reclaim_requested.clone();
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
                loop {
                    match receiver.recv_timeout(Duration::from_millis(50)) {
                        Ok(job) => job(&mut connection),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if worker_reclaim.load(Ordering::Acquire) {
                                match reclaim_idle_step(&mut connection) {
                                    Ok(done) => worker_reclaim.store(!done, Ordering::Release),
                                    Err(error) => tracing::debug!(%error, "background space maintenance will retry"),
                                }
                            }
                        }
                    }
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
                reclaim_requested,
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

    /// Non-blocking hint. Idle worker slices give queued foreground writes
    /// priority; startup may resume freed-page reclamation, never compaction
    /// or deletion of journal records.
    pub(crate) fn request_reclaim(&self) {
        self.inner.reclaim_requested.store(true, Ordering::Release);
    }

    /// Hand pages freed by a delete back to the filesystem.
    ///
    /// `incremental_vacuum` pops free pages **one at a time** off the end of the
    /// file and stops when the last page is in use — measured at 3291 calls to
    /// drain a 13 MB freelist (298 ms), while a 6 GB one would take minutes. So
    /// this loops up to `max_pages` to keep a delete's latency predictable, and
    /// later idle slices reclaim what remains; startup never rewrites the file.
    ///
    /// `min_free_pages` keeps the hot paths cheap: below it there is nothing
    /// worth reclaiming, and the check is a header read.
    #[cfg(test)]
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

/// Short idle slices, not a long job placed on the same ordered worker.
/// Busy checkpoints are retried without the foreground 5-second lock wait.
pub(super) fn reclaim_idle_step(connection: &mut Connection) -> Result<bool> {
    let started = std::time::Instant::now();
    connection.busy_timeout(Duration::ZERO)?;
    let result = (|| -> Result<bool> {
        // Space reclamation never changes journals. Snapshot construction
        // and raw deletion happen only in the run-completion transaction.
        let free = |connection: &Connection| -> Result<i64> {
            Ok(connection.pragma_query_value(None, "freelist_count", |r| r.get(0))?)
        };
        let mode: i64 = connection.pragma_query_value(None, "auto_vacuum", |r| r.get(0))?;
        let mut remaining = if mode == 2 { free(connection)? } else { 0 };
        for _ in 0..128 {
            if remaining == 0 || started.elapsed() >= Duration::from_millis(10) {
                break;
            }
            connection.execute_batch("PRAGMA incremental_vacuum(1)")?;
            let now = free(connection)?;
            if now >= remaining {
                break;
            }
            remaining = now;
        }
        // Reuse/truncate the WAL only when no reader needs it; never force a
        // long reader to end. Failure leaves the retry hint set.
        let (blocked, _, _): (i64, i64, i64) =
            connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
        Ok(remaining == 0 && blocked == 0)
    })();
    connection.busy_timeout(Duration::from_secs(5))?;
    result
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
    if version != 0 && version != 2 && version != 3 && version != 4 && version != 5 {
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
    configure_incremental_auto_vacuum(&connection)?;
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
        CREATE TABLE IF NOT EXISTS run_snapshots (
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            run_id TEXT NOT NULL,
            version INTEGER NOT NULL CHECK(version=1),
            cursor INTEGER NOT NULL CHECK(cursor>=0),
            payload TEXT NOT NULL CHECK(json_valid(payload)),
            pricing_payloads TEXT NOT NULL CHECK(json_valid(pricing_payloads)),
            raw_pending INTEGER NOT NULL DEFAULT 0 CHECK(raw_pending IN (0,1)),
            PRIMARY KEY(session_id,run_id)
        );
        CREATE INDEX IF NOT EXISTS run_snapshots_cleanup ON run_snapshots(session_id,run_id) WHERE raw_pending=1;
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
        PRAGMA user_version = 5;",
    )?;
    tx.execute("INSERT INTO storage_meta(key,value) VALUES ('event_sequence',CAST(coalesce((SELECT max(sequence) FROM run_events),0) AS TEXT))
        ON CONFLICT(key) DO UPDATE SET value=CAST(max(CAST(storage_meta.value AS INTEGER),CAST(excluded.value AS INTEGER)) AS TEXT)", [])?;
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
            assert_eq!((version, skills), (5, 0));
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

/// Opening a file that belongs to somebody else, and reclamation budgets. Both
/// are startup/latency guarantees rather than query behaviour, so they are
/// asserted against the connection's own pragmas.
#[cfg(test)]
mod open_and_reclaim_paths {
    use super::*;

    #[test]
    fn a_foreign_or_unrecognized_database_is_refused_instead_of_adopted() {
        let dir = tempfile::tempdir().unwrap();

        let foreign = dir.path().join("foreign.db");
        let connection = Connection::open(&foreign).unwrap();
        connection
            .execute_batch("PRAGMA application_id = 12345;")
            .unwrap();
        drop(connection);
        let error = match Database::open(&foreign) {
            Ok(_) => panic!("a foreign database must not be opened"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("different application"), "{error}");

        let unknown = dir.path().join("unknown.db");
        let connection = Connection::open(&unknown).unwrap();
        connection
            .execute_batch("CREATE TABLE unrelated(id INTEGER);")
            .unwrap();
        drop(connection);
        let error = match Database::open(&unknown) {
            Ok(_) => panic!("an unrecognized database must not be initialized"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("unrecognized database"), "{error}");
    }

    #[test]
    fn opening_bloated_incremental_database_does_not_rewrite_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        let connection = open_connection(&path).unwrap();
        connection.execute_batch("CREATE TABLE bloat(payload BLOB); INSERT INTO bloat VALUES(zeroblob(80*1024*1024)); DELETE FROM bloat;").unwrap();
        let pages: i64 = connection
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .unwrap();
        let free: i64 = connection
            .pragma_query_value(None, "freelist_count", |r| r.get(0))
            .unwrap();
        assert!(free * 4096 > 64 * 1024 * 1024);
        drop(connection);
        // No idle worker is started here: this isolates the startup path.
        let reopened = open_connection(&path).unwrap();
        assert_eq!(
            reopened
                .pragma_query_value(None, "page_count", |r| r.get::<_, i64>(0))
                .unwrap(),
            pages
        );
        assert_eq!(
            reopened
                .pragma_query_value(None, "freelist_count", |r| r.get::<_, i64>(0))
                .unwrap(),
            free
        );
    }

    #[test]
    fn a_locked_writer_is_retried_on_the_same_connection_until_it_is_free() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        let db = Database::open(&path).unwrap();
        // Fail fast, so the retry loop (not SQLite's busy handler) absorbs the
        // wait and the test does not depend on the 5 s busy timeout.
        db.call::<()>(|connection| {
            connection.busy_timeout(Duration::from_millis(0))?;
            Ok(())
        })
        .unwrap();

        let blocker = Connection::open(&path).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE;").unwrap();
        // The holder releases while the retry loop is sleeping. `db.call` blocks
        // its caller until the worker replies, so the release needs its own
        // thread that is already running before the write is attempted.
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            blocker.execute_batch("COMMIT;").unwrap();
        });
        let write = db.call::<()>(|connection| {
            // The retry loop under test wraps `BEGIN IMMEDIATE`, which is what
            // the store's transactional writes open through.
            let transaction = super::begin_immediate(connection)?;
            transaction.execute("INSERT INTO sessions(id) VALUES ('waiter')", [])?;
            transaction.commit()?;
            Ok(())
        });
        write.unwrap();
        release.join().unwrap();

        let ids: Vec<String> = db
            .call(|connection| {
                let mut statement = connection.prepare("SELECT id FROM sessions")?;
                let ids = statement
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                Ok(ids)
            })
            .unwrap();
        assert_eq!(ids, ["waiter"]);
    }

    #[test]
    fn reclaim_returns_freed_pages_and_honours_both_budgets() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("agent.db")).unwrap();
        db.call::<()>(|connection| {
            connection.execute_batch("CREATE TABLE filler(id INTEGER PRIMARY KEY, body BLOB);")?;
            for index in 0..500 {
                connection.execute(
                    "INSERT INTO filler(id, body) VALUES (?1, zeroblob(8192))",
                    [index],
                )?;
            }
            Ok(())
        })
        .unwrap();
        db.call::<()>(|connection| {
            connection.execute("DELETE FROM filler", [])?;
            Ok(())
        })
        .unwrap();
        let free_pages = |db: &Database| -> i64 {
            db.call(|connection| {
                Ok(connection.pragma_query_value(None, "freelist_count", |row| row.get(0))?)
            })
            .unwrap()
        };
        let before = free_pages(&db);
        assert!(before > 4, "the deletes must leave free pages: {before}");

        // The step budget caps how much work one call may do.
        db.reclaim(1, 3).unwrap();
        let after = free_pages(&db);
        assert!(after < before, "{after} < {before}");

        // Below the minimum there is nothing worth reclaiming.
        db.reclaim(before + 1, 3).unwrap();
        assert_eq!(free_pages(&db), after);
    }

    #[test]
    fn a_non_busy_transaction_failure_is_returned_instead_of_retried() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("agent.db")).unwrap();
        let started = std::time::Instant::now();
        let error = db
            .call(|connection| {
                // An open write transaction makes the second BEGIN fail with a
                // plain SQLite error, which is not a busy condition.
                let first = super::begin_immediate(connection)?;
                let error = super::begin_immediate(connection)
                    .expect_err("a second BEGIN IMMEDIATE cannot succeed");
                drop(first);
                Ok(error.to_string())
            })
            .unwrap();
        assert!(error.contains("begin immediate transaction"), "{error}");
        assert!(
            started.elapsed() < Duration::from_millis(20),
            "a non-busy failure must not go through the retry ladder"
        );
    }
}
