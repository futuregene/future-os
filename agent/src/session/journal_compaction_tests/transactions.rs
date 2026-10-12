use super::*;
#[test]
fn competing_writer_does_not_block_the_worker_or_retire_raw_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 10);
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (sender, receiver) = mpsc::channel();
    let worker_store = store.clone();
    let worker = std::thread::spawn(move || {
        let result = worker_store.compact_run("s", "run");
        let query = worker_store.db.call(|db| {
            Ok((
                db.query_row("SELECT count(*) FROM run_events", [], |r| {
                    r.get::<_, i64>(0)
                })?,
                db.query_row("PRAGMA busy_timeout", [], |r| r.get::<_, i64>(0))?,
                db.query_row("SELECT count(*) FROM run_snapshots", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            ))
        });
        sender.send((result, query)).unwrap();
    });
    // Keep the external write lock held through the reply. Foreground reads
    // must finish while it is still held, rather than after busy_timeout.
    let reply = receiver.recv_timeout(std::time::Duration::from_secs(2));
    writer.execute_batch("ROLLBACK").unwrap();
    worker.join().unwrap();
    let (result, query) = reply.expect("background compaction monopolized the database worker");
    assert!(result.is_err());
    assert_eq!(query.unwrap(), (12, 5000, 0));
    assert!(store.has_events("s", "run").unwrap());
}

#[test]
fn transaction_failure_retains_every_raw_row_and_explicit_calls_are_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("agent.db"), "completed", 1000);
    store.db.call(|db| {
        db.execute_batch("CREATE TRIGGER reject_cleanup BEFORE DELETE ON run_events WHEN OLD.idx=500 BEGIN SELECT RAISE(ABORT,'injected'); END;")?;
        Ok(())
    }).unwrap();
    assert!(store.compact_run("s", "run").is_err());
    let page = store.replay_page("s", "run", -1, false, None).unwrap();
    assert!(page.snapshot.is_none());
    assert_eq!(page.events.len(), 1002);
    store
        .db
        .call(|db| {
            db.execute_batch("DROP TRIGGER reject_cleanup")?;
            Ok(())
        })
        .unwrap();
    assert_eq!(store.compact_run("s", "run").unwrap(), 1002);
    assert_eq!(store.compact_run("s", "run").unwrap(), 0);
}

#[test]
fn space_reclamation_never_deletes_existing_raw_journals() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 3);
    store.compact_run("s", "run").unwrap();
    // Simulate a previously published snapshot with a redundant raw tail.
    store.db.call(|db| {
        db.execute("INSERT INTO run_events(session_id,run_id,epoch,idx,payload) VALUES ('s','run',0,0,'{}')",[])?;
        Ok(())
    }).unwrap();
    store
        .db
        .call(crate::session::database::reclaim_space_when_idle)
        .unwrap();
    drop(store);
    let store = SqliteStore::open(&path).unwrap();
    store
        .db
        .call(crate::session::database::reclaim_space_when_idle)
        .unwrap();
    assert_eq!(
        store
            .db
            .call(|db| Ok(db.query_row(
                "SELECT count(*) FROM run_events WHERE run_id='run'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
        1
    );
    assert!(store
        .replay_page("s", "run", -1, false, None)
        .unwrap()
        .snapshot
        .is_some());
}

#[test]
fn atomic_retirement_preserves_pricing_sequence_and_retires_raw_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 1000);
    store.db.call(|db| {
        db.execute("UPDATE run_events SET payload=json_set(payload,'$.event_type','usage','$.data',?1) WHERE idx=500",[json!({"output_tokens":7}).to_string()])?;
        Ok(())
    }).unwrap();
    let pricing = store.pricing_event_payloads("s").unwrap();
    store.compact_run("s", "run").unwrap();
    let raw: i64 = store
        .db
        .call(|db| {
            Ok(db.query_row(
                "SELECT count(*) FROM run_events WHERE run_id='run'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(raw, 0);
    assert!(store.events("s", "run").unwrap().len() < 10);
    assert_eq!(store.pricing_event_payloads("s").unwrap(), pricing);
    let event = SseEvent {
        run_id: "next".into(),
        idx: 0,
        ..SseEvent::new("usage", json!({"output_tokens":8}))
    };
    store
        .append_event("s", serde_json::to_value(event).unwrap())
        .unwrap();
    let ordered = store.pricing_event_payloads("s").unwrap();
    assert_eq!(ordered[0], pricing[0]);
    assert_eq!(ordered.len(), 2);
    let sequence: i64 = store
        .db
        .call(|db| {
            Ok(db.query_row(
                "SELECT sequence FROM run_events WHERE run_id='next'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert!(sequence > 1002);
    assert_eq!(
        text(
            &store
                .replay_page("s", "run", -1, false, None)
                .unwrap()
                .snapshot
                .unwrap()
        ),
        "中文𠮷".repeat(499) + &"中文𠮷".repeat(500)
    );
}

#[test]
fn pinned_reader_does_not_block_foreground_write_or_destroy_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 1000);
    let reader = rusqlite::Connection::open(&path).unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM run_events;")
        .unwrap();
    store.compact_run("s", "run").unwrap();
    assert!(!store
        .db
        .call(crate::session::database::reclaim_space_when_idle)
        .unwrap());
    let event = SseEvent {
        run_id: "next".into(),
        idx: 0,
        ..SseEvent::new("text_chunk", json!({"text":"new"}))
    };
    store
        .append_event("s", serde_json::to_value(event).unwrap())
        .unwrap();
    assert!(store
        .replay_page("s", "run", 123, false, None)
        .unwrap()
        .snapshot
        .is_some());
    reader.execute_batch("ROLLBACK").unwrap();
}
