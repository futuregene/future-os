use super::*;
use anyhow::bail;

#[test]
fn bounded_queue_rejects_overload_and_recovers_after_drain() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("agent.db")).unwrap();
    let gate = Arc::new(std::sync::Barrier::new(2));
    let worker_gate = gate.clone();
    db.inner
        .sender
        .lock()
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
            .lock()
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
        .lock()
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
fn connection_moves_checkpoints_off_commit_and_retains_the_reuse_limit() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("agent.db")).unwrap();
    let settings = db
        .call(|connection| {
            Ok((
                connection
                    .pragma_query_value(None, "wal_autocheckpoint", |row| row.get::<_, i64>(0))?,
                connection
                    .pragma_query_value(None, "journal_size_limit", |row| row.get::<_, i64>(0))?,
            ))
        })
        .unwrap();
    assert_eq!(settings, (0, 8 * 1024 * 1024));
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
fn v2_database_upgrades_to_current_schema_with_the_skills_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    // Actual schema from the first SQLite release, before skills/snapshots.
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../../../tests/fixtures/database/v2.sql"))
        .unwrap();
    drop(connection);

    let db = Database::open(&path).unwrap();
    db.call(|connection| {
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let skills: i64 =
            connection.query_row("SELECT count(*) FROM skills", [], |row| row.get(0))?;
        assert_eq!((version, skills), (CURRENT_SCHEMA_VERSION, 0));
        Ok(())
    })
    .unwrap();
}

#[test]
fn damaged_current_history_is_not_recreated_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    drop(Database::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.execute("INSERT INTO sessions(id) VALUES ('keep')", [])
        .unwrap();
    conn.execute_batch("DROP TABLE run_events").unwrap();
    drop(conn);
    let error = match Database::open(&path) {
        Ok(_) => panic!("missing authoritative history must not be recreated"),
        Err(error) => error,
    };
    assert!(error
        .to_string()
        .contains("history structure is incomplete"));
    let conn = Connection::open(&path).unwrap();
    let missing: bool = conn
        .query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='run_events')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(missing);
    let preserved: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id='keep')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(preserved);
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

/// A job that panics takes the worker thread with it. Callers are then told the
/// worker is *unavailable* — a distinct, retryable condition — instead of
/// waiting on a reply that can never come, and the shutdown path still reports
/// the panic rather than claiming a clean close of a connection that was never
/// closed.
///
/// There are two moments here and the test names both: while the worker is still
/// unwinding, a queued job is accepted and then dropped (the caller sees
/// "interrupted"); once the receiver is gone, the same call is refused up front
/// ("unavailable"). The assertion is therefore a bounded liveness wait — it can
/// only fail if the product never reaches the refused state — not a sleep that
/// hopes a race goes one way.
#[test]
fn a_panicking_job_leaves_later_callers_with_a_disconnected_worker() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("agent.db")).unwrap();

    // A well-behaved call first, so the worker is provably alive.
    let ok: i64 = db
        .call(|connection| Ok(connection.query_row("SELECT 1", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(ok, 1);

    // The worker unwinds inside this job; its receiver leaves with the thread.
    let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = db.call(|_| -> Result<()> { panic!("injected worker panic") });
    }));
    assert!(first.is_ok(), "the panic happens on the worker thread");

    let probe = |db: &Database| {
        db.call(|connection| Ok(connection.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))?))
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut refusal = None;
    let mut interrupted = 0u32;
    while std::time::Instant::now() < deadline {
        match probe(&db) {
            Ok(_) => break,
            Err(error) if error.to_string().contains("SQLite worker is unavailable") => {
                refusal = Some(error.to_string());
                break;
            }
            Err(_) => {
                // Still unwinding: the job was accepted and then dropped. Bounded
                // by the deadline, so this cannot loop forever.
                interrupted += 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
    assert!(
        refusal.is_some(),
        "a dead worker must eventually refuse work up front          (observed {interrupted} interrupted probe(s) and no refusal)"
    );

    // The shutdown path must report the panic rather than claim a clean close.
    let closed = db.shutdown();
    assert!(
        closed.is_err(),
        "shutdown after a worker panic must report it, not report success"
    );
}
