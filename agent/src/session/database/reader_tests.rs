//! Run with the complete local suite after the storage refactor is integrated.
use super::*;

#[test]
fn an_open_writer_does_not_serialize_independent_history_reads() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("agent.db")).unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sessions(id) VALUES('committed')", [])?;
        Ok(())
    })
    .unwrap();
    let (arrived, started) = mpsc::sync_channel(1);
    let (release, resume) = mpsc::sync_channel(1);
    let writer = db.clone();
    let handle = std::thread::spawn(move || {
        writer.call(move |c| {
            let tx = c.transaction()?;
            tx.execute("INSERT INTO sessions(id) VALUES('pending')", [])?;
            arrived.send(()).unwrap();
            resume.recv().unwrap();
            tx.commit()?;
            Ok(())
        })
    });
    started.recv().unwrap();
    // Release the writer even if an assertion fails, so failure cannot deadlock
    // cleanup. This select must execute on a different read connection.
    let reader = db.clone();
    let (read_done, answer) = mpsc::sync_channel(1);
    let reading = std::thread::spawn(move || {
        let _ = read_done.send(reader.read(|c| {
            Ok(c.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?)
        }));
    });
    let count = answer.recv_timeout(std::time::Duration::from_secs(2));
    release.send(()).unwrap();
    handle.join().unwrap().unwrap();
    reading.join().unwrap();
    assert_eq!(count.unwrap().unwrap(), 1);
    assert_eq!(
        db.read(|c| Ok(c.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?))
            .unwrap(),
        2
    );
}

#[test]
fn readers_cannot_mutate_and_shutdown_closes_both_paths() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("agent.db")).unwrap();
    assert!(db
        .read(|c| {
            c.execute("INSERT INTO sessions(id) VALUES('forbidden')", [])?;
            Ok(())
        })
        .is_err());
    db.shutdown().unwrap();
    assert!(db.read(|_| Ok(())).is_err());
    assert!(db.call(|_| Ok(())).is_err());
}

#[test]
fn expired_queued_read_never_executes_and_fences_bypass_busy_readers() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(&root.path().join("agent.db")).unwrap();
    let (entered, arrived) = mpsc::channel();
    let mut releases = Vec::new();
    let mut workers = Vec::new();
    for _ in 0..2 {
        let reader = db.clone();
        let entered = entered.clone();
        let (release, wait) = mpsc::channel();
        releases.push(release);
        workers.push(std::thread::spawn(move || {
            reader.read(move |_| {
                entered.send(()).unwrap();
                wait.recv().unwrap();
                Ok(())
            })
        }));
    }
    arrived.recv().unwrap();
    arrived.recv().unwrap();
    assert_eq!(db.fence(|_| Ok(42)).unwrap(), 42);
    let store = crate::session::sqlite_store::SqliteStore { db: db.clone() };
    let (probe_done, probe_result) = mpsc::channel();
    let probing = std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<()> {
            assert!(!store.contains("new-session")?);
            store.storage_identity()?;
            assert_eq!(store.event_cursors("new-session")?, (0, 0));
            Ok(())
        })();
        probe_done.send(result).unwrap();
    });
    let probed = probe_result.recv_timeout(Duration::from_secs(2));
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    let request = request::Context::new(
        "list_sessions".into(),
        "s".into(),
        Some(Duration::from_millis(20)),
    );
    let scope = request.enter();
    let result = db.read(move |_| {
        flag.store(true, Ordering::Release);
        Ok(())
    });
    drop(scope);
    for release in releases {
        release.send(()).unwrap();
    }
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    probing.join().unwrap();
    probed
        .expect("new-session identity/cursor reads must bypass busy history readers")
        .unwrap();
    // Draining another job also proves the skipped queued closure released its slot.
    db.read(|_| Ok(())).unwrap();
    assert!(result.unwrap_err().to_string().contains("deadline"));
    assert!(!ran.load(Ordering::Acquire));
}

#[test]
fn cancellation_interrupts_sql_and_restores_the_connection() {
    let request = request::Context::new(
        "search_all_session_history".into(),
        String::new(),
        Some(Duration::from_secs(10)),
    );
    let cancel = request.cancel_on_drop();
    let (started, entered) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE interrupted_work(value INTEGER)")
            .unwrap();
        let result = request.run(&mut db, |db| {
            let tx = db.transaction()?;
            tx.execute("INSERT INTO interrupted_work VALUES (1)", [])?;
            started.send(()).unwrap();
            Ok(tx.query_row("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n", [], |row| row.get::<_,i64>(0))?)
        });
        assert!(result.is_err());
        assert!(db.is_autocommit());
        assert_eq!(
            db.query_row("SELECT count(*) FROM interrupted_work", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT 7", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            7
        );
    });
    entered.recv().unwrap();
    drop(cancel);
    worker.join().unwrap();
}

#[test]
fn first_prompt_preflight_does_not_queue_behind_unrelated_writes() {
    let root = tempfile::tempdir().unwrap();
    let manager = Arc::new(crate::session::Manager::new(root.path().join("sessions")));
    let store = manager.storage().unwrap();
    store.reserve_segmented("fresh-prompt").unwrap();
    store
        .replace(
            "fresh-prompt",
            vec![serde_json::json!({"id":"info","type":"session_info","content":{"model":"mock"}})],
        )
        .unwrap();
    let writer = store.db.clone();
    let (entered, started) = mpsc::channel();
    let (release, waiting) = mpsc::channel();
    let writing = std::thread::spawn(move || {
        writer.call(move |_| {
            entered.send(()).unwrap();
            waiting.recv().unwrap();
            Ok(())
        })
    });
    started.recv().unwrap();
    let reader = manager.clone();
    let (send, answer) = mpsc::channel();
    let reading = std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<()> {
            assert!(reader.queued_receipt("fresh-prompt", "request")?.is_none());
            assert!(!reader.has_queued_receipt_run("fresh-prompt", "run")?);
            let store = reader.storage()?;
            assert!(!store.has_events("fresh-prompt", "run")?);
            assert!(!store.has_transcript_run("fresh-prompt", "run")?);
            assert_eq!(store.storage_format("fresh-prompt")?, 2);
            reader.ensure_execution_allowed("fresh-prompt")?;
            Ok(())
        })();
        send.send(result).unwrap();
    });
    let result = answer.recv_timeout(Duration::from_secs(2));
    release.send(()).unwrap();
    writing.join().unwrap().unwrap();
    reading.join().unwrap();
    result
        .expect("first-prompt checks must not wait for the writer")
        .unwrap();
}
