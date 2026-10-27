use super::*;

/// The compactor's two guarantees, in one test: a start folds the backlog that no
/// completion notification covered, and a fold that fails is *not* retried by the
/// same process (a retry loop on a database that refuses would be a busy-loop;
/// the next start picks it up instead).
#[test]
fn a_start_folds_the_backlog_and_a_failed_fold_is_not_retried() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 1000);
    let compactor = CompletedRunCompactor::start(store.clone()).unwrap();
    compactor.flush();
    // The startup catch-up folded the settled run without any notification.
    assert!(store
        .replay_page("s", "run", -1, false, None)
        .unwrap()
        .snapshot
        .is_some());
    assert_eq!(
        store.events("s", "run").unwrap().len(),
        3,
        "start, one coalesced text segment, end"
    );

    // A second settled run whose retirement fails: published once, never retried
    // in-process.
    store
        .db
        .call(|db| {
            db.execute(
                "INSERT INTO runs(session_id,run_id,status,completed_at_ms) VALUES ('s','boom','completed',1)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    for (idx, kind) in [(0, "agent_start"), (1, "agent_end")] {
        let event = SseEvent {
            run_id: "boom".into(),
            idx,
            ..SseEvent::new(kind, json!({}))
        };
        store
            .append_event("s", serde_json::to_value(event).unwrap())
            .unwrap();
    }
    store.db.call(|db| {
        db.execute_batch("CREATE TRIGGER reject_cleanup BEFORE DELETE ON run_events WHEN OLD.idx=1 BEGIN SELECT RAISE(ABORT,'injected'); END;")?;
        Ok(())
    }).unwrap();
    compactor.request("s", "boom");
    compactor.flush();
    assert!(store
        .replay_page("s", "boom", -1, false, None)
        .unwrap()
        .snapshot
        .is_none());
    store
        .db
        .call(|db| {
            db.execute_batch("DROP TRIGGER reject_cleanup")?;
            Ok(())
        })
        .unwrap();
    // Still not retried: nothing in the process re-queues a failed fold.
    compactor.flush();
    assert!(store
        .replay_page("s", "boom", -1, false, None)
        .unwrap()
        .snapshot
        .is_none());
    assert_eq!(store.events("s", "boom").unwrap().len(), 2);
    drop(compactor);
    store.delete("s").unwrap();
    assert!(!store.has_events("s", "boom").unwrap());
}
