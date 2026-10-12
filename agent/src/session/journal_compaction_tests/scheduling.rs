use super::*;

#[test]
fn compactor_only_processes_notified_runs_and_never_retries_failures() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 1000);
    let compactor = CompletedRunCompactor::start(store.clone()).unwrap();
    compactor.flush();
    assert!(store
        .replay_page("s", "run", -1, false, None)
        .unwrap()
        .snapshot
        .is_none());
    store.db.call(|db| {
        db.execute_batch("CREATE TRIGGER reject_cleanup BEFORE DELETE ON run_events WHEN OLD.idx=500 BEGIN SELECT RAISE(ABORT,'injected'); END;")?;
        Ok(())
    }).unwrap();
    compactor.request("s", "run");
    compactor.flush();
    assert_eq!(store.events("s", "run").unwrap().len(), 1002);
    store
        .db
        .call(|db| {
            db.execute_batch("DROP TRIGGER reject_cleanup")?;
            Ok(())
        })
        .unwrap();
    compactor.flush();
    assert!(store
        .replay_page("s", "run", -1, false, None)
        .unwrap()
        .snapshot
        .is_none());
    drop(compactor);
    drop(store);
    let store = SqliteStore::open(&path).unwrap();
    let compactor = CompletedRunCompactor::start(store.clone()).unwrap();
    compactor.flush();
    assert_eq!(store.events("s", "run").unwrap().len(), 1002);
    assert!(store
        .replay_page("s", "run", -1, false, None)
        .unwrap()
        .snapshot
        .is_none());
    // A new completion notification works without touching the old journal.
    store.db.call(|db| {
        db.execute("INSERT INTO runs(session_id,run_id,status,completed_at_ms) VALUES ('s','new','completed',1)", [])?;
        Ok(())
    }).unwrap();
    for (idx, kind) in [(0, "agent_start"), (1, "agent_end")] {
        let event = SseEvent {
            run_id: "new".into(),
            idx,
            ..SseEvent::new(kind, json!({}))
        };
        store
            .append_event("s", serde_json::to_value(event).unwrap())
            .unwrap();
    }
    compactor.request("s", "new");
    compactor.flush();
    assert!(store
        .replay_page("s", "new", -1, false, None)
        .unwrap()
        .snapshot
        .is_some());
    assert_eq!(store.events("s", "run").unwrap().len(), 1002);
    store.delete("s").unwrap();
    assert!(!store.has_events("s", "new").unwrap());
}
