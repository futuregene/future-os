use super::*;

#[test]
fn a_startup_token_cannot_interrupt_the_ready_database_writer() {
    let root = tempfile::tempdir().unwrap();
    let cancellation = crate::service::StartupCancellation::default();
    let database =
        Database::open_cancellable(&root.path().join("agent.db"), cancellation.clone()).unwrap();
    cancellation.cancel();
    database
        .call(|db| {
            db.execute("INSERT INTO sessions(id) VALUES ('ordinary-write')", [])?;
            Ok(())
        })
        .unwrap();
}

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
fn a_database_that_answers_to_our_id_without_a_schema_version_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("half-identified.db");
    let connection = Connection::open(&path).unwrap();
    // It carries the Agent's application id but no schema version, and a table
    // the Agent schema does not own. Adopting it would run `CREATE IF NOT EXISTS`
    // over somebody's data and then report an empty history for it.
    connection
        .execute_batch(&format!(
            "PRAGMA application_id = {}; CREATE TABLE entries(id INTEGER);",
            0x46555452
        ))
        .unwrap();
    drop(connection);

    let error = match Database::open(&path) {
        Ok(_) => panic!("a half-identified database must not be adopted"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("incomplete schema identity"), "{error}");

    // The refusal is a preservation act: the table is still there, unmodified.
    let preserved = Connection::open(&path).unwrap();
    let entries: i64 = preserved
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name='entries'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(entries, 1, "the refusal must not rewrite the file");
}

/// Pages a startup may move in or out of the freelist when the schema it opens is
/// already current.
///
/// A startup does write, even with nothing to migrate: `initialize_schema`
/// re-creates the derived views (`entries::RESOLVED_BLOCKS_SQL`,
/// `entry_records::entry_record_views_sql`, `entries::header_views_sql`) so a code
/// change to them takes effect, and every `DROP VIEW IF EXISTS` + `CREATE VIEW`
/// pair deletes and re-inserts that view's `sqlite_schema` row. The re-insert
/// lands where the cell fits rather than in the hole it just left, so the
/// `sqlite_schema` btree comes out fragmented: 12 pages as the file is created,
/// 13 after the first reopen, and that extra leaf is allocated from the freelist
/// (measured: 20500 free pages before the reopen, 20499 after, `page_count`
/// unchanged at 20642).
///
/// Whether the fragmented btree still fits is a boundary decided by the whole
/// schema, not by anything the reopen does differently: take `run_snapshot_cache`
/// — the table the replay cache added — out of `schema.sql` and this reopen leaves
/// the freelist exactly as it found it (`dbstat`: `sqlite_schema` stays at 12
/// pages); with the table there it needs one more leaf. So the exact freelist
/// equality this test used to assert was describing the schema's size, not the
/// startup's behaviour, and it stopped holding the moment the schema grew past
/// it. `page_count` below stays exact, and a second opening must reuse the holes
/// the first one left, so what the startup may cost is one page, once per file,
/// rather than one per start.
const REOPEN_SCHEMA_PAGES: i64 = 1;

#[test]
fn opening_bloated_incremental_database_does_not_rewrite_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let connection = open_connection(&path).unwrap();
    connection.execute_batch("CREATE TABLE bloat(payload BLOB); INSERT INTO bloat VALUES(zeroblob(80*1024*1024)); DELETE FROM bloat;").unwrap();
    let layout = |connection: &Connection| -> (i64, i64) {
        (
            connection
                .pragma_query_value(None, "page_count", |r| r.get(0))
                .unwrap(),
            connection
                .pragma_query_value(None, "freelist_count", |r| r.get(0))
                .unwrap(),
        )
    };
    let (pages, free) = layout(&connection);
    assert!(free * 4096 > 64 * 1024 * 1024);
    drop(connection);
    // No idle worker is started here: this isolates the startup path.
    let reopened = open_connection(&path).unwrap();
    let (reopened_pages, reopened_free) = layout(&reopened);
    // The rewrite this guards against — a `VACUUM`, or the whole auto-vacuum
    // pass `PRAGMA incremental_vacuum` would run — shows up on both counters: it
    // rewrites the file down to the ~116 pages the schema occupies and empties
    // the freelist. Nothing here may shrink the file or reclaim its free pages.
    assert_eq!(reopened_pages, pages, "opening must not resize the file");
    assert!(
        (free - reopened_free).abs() <= REOPEN_SCHEMA_PAGES,
        "opening must not claim the freelist: {free} free pages before, {reopened_free} after"
    );
    drop(reopened);
    // The schema bootstrap converges, so it is a one-time cost: opening again
    // has to land on exactly the layout the previous open left behind. A page
    // per start would be unbounded growth of the file.
    let again = open_connection(&path).unwrap();
    assert_eq!(
        layout(&again),
        (reopened_pages, reopened_free),
        "a second opening must leave the layout where the first one put it"
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
        let transaction = super::begin_write_transaction(connection)?;
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

/// Opening installs the schema inside a transaction that reads the projection
/// identity and only then writes the DDL. In WAL mode that read-then-write
/// upgrade cannot wait: while another connection holds the write lock, SQLite
/// fails it immediately with SQLITE_BUSY_SNAPSHOT — "database is locked" —
/// without ever invoking `busy_timeout`, so no timeout setting can rescue it.
///
/// A second connection of this same process is the ordinary case, not an
/// exotic one: the writer lane reclaims space on its own connection in short
/// slices while readers and checkpoints run, and a CLI command can open
/// agent.db beside the running Agent. The window is short, which is exactly why
/// it shows up as a rare, load-dependent failure (Windows CI hit it once in
/// 3033 tests, on a head that changed no Agent code) instead of a reproducible
/// error — and why the open has to wait for the lock rather than fail.
#[test]
fn opening_waits_for_a_connection_that_holds_the_write_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    // A current database, so the open under test has no upgrade to perform and
    // only runs the schema reinstall every ordinary reopen performs.
    drop(open_connection(&path).unwrap());

    let blocker = Connection::open(&path).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE;").unwrap();
    // Release while the open is waiting: the holder cannot run on this thread.
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        blocker.execute_batch("COMMIT;").unwrap();
    });
    let opened = open_connection(&path).expect("opening waits for a concurrent writer");
    release.join().unwrap();

    // Waiting must not have been bought by skipping the schema install.
    let version: i64 = opened
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
}

/// Falsification attempt for the idle-reclamation error arm, kept as the record
/// of *why* that arm is unreachable here. `reclaim_space_when_idle` runs on the
/// worker's own connection, so reaching its `Err` arm needs a live connection
/// whose pragmas fail; damaging the file underneath it does not do that, because
/// SQLite serves the cached page 1 to an already-open connection — `SELECT 1`
/// never touches the file at all, and `freelist_count` on a truncated file still
/// answers from the cache. Two damage routes were tried (truncate, and truncate
/// with the WAL removed first); both leave the connection answering normally.
#[test]
fn a_replaced_database_file_is_invisible_to_an_already_open_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let db = Database::open(&path).unwrap();
    db.call::<()>(|connection| {
        connection.execute_batch("CREATE TABLE filler(id INTEGER PRIMARY KEY, body BLOB);")?;
        connection.execute(
            "INSERT INTO filler(id, body) VALUES (1, zeroblob(8192))",
            [],
        )?;
        connection.execute("DELETE FROM filler", [])?;
        Ok(())
    })
    .unwrap();

    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    std::fs::write(&path, b"this is not a database").unwrap();

    // The live connection still answers both, from its own cache.
    let free: i64 = db
        .call(|connection| {
            Ok(connection.pragma_query_value(None, "freelist_count", |row| row.get(0))?)
        })
        .unwrap();
    assert!(free >= 0, "the cached header still answers: {free}");
    assert!(db.shutdown().is_ok(), "the worker still closes cleanly");
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
            let first = super::begin_write_transaction(connection)?;
            let error = match super::begin_write_transaction(connection) {
                Ok(_) => panic!("a second BEGIN IMMEDIATE cannot succeed"),
                Err(error) => error,
            };
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
