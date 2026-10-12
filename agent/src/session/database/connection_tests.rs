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
            let error = super::begin_write_transaction(connection)
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
