use super::*;

#[test]
fn active_incomplete_and_gapped_journals_are_kept() {
    for status in ["running", "completed"] {
        let dir = tempfile::tempdir().unwrap();
        let store = fixture(&dir.path().join("agent.db"), status, 10);
        if status == "completed" {
            store
                .db
                .call(|db| {
                    db.execute("DELETE FROM run_events WHERE idx=11", [])?;
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(store.compact_run("s", "run").unwrap(), 0);
        assert!(store
            .replay_page("s", "run", -1, false, None)
            .unwrap()
            .snapshot
            .is_none());
    }
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("agent.db"), "completed", 10);
    store
        .db
        .call(|db| {
            db.execute("DELETE FROM run_events WHERE idx=5", [])?;
            Ok(())
        })
        .unwrap();
    assert!(store.compact_run("s", "run").is_err());
    assert_eq!(store.events("s", "run").unwrap().len(), 11);
}

#[test]
fn oversize_snapshot_keeps_raw_paged_replay() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("agent.db"), "completed", 1);
    let body = json!({"text":"x".repeat(SNAPSHOT_BYTE_BUDGET)}).to_string();
    store
        .db
        .call(move |db| {
            db.execute(
                "UPDATE run_events SET payload=json_set(payload,'$.data',?1) WHERE idx=1",
                [body],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(store.compact_run("s", "run").unwrap(), 0);
    let page = store.replay_page("s", "run", 0, false, Some(1)).unwrap();
    assert_eq!(page.events.len(), 1);
    assert!(page.snapshot.is_none());
}

#[test]
fn oversized_raw_journal_keeps_paging_without_an_atomic_bulk_delete() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(
        &dir.path().join("agent.db"),
        "completed",
        MAX_ATOMIC_RETIRE_EVENTS as usize,
    );
    assert_eq!(store.compact_run("s", "run").unwrap(), 0);
    let page = store.replay_page("s", "run", 0, false, Some(1)).unwrap();
    assert_eq!(page.events.len(), 1);
    assert!(page.snapshot.is_none());
    assert_eq!(
        store
            .db
            .call(|db| Ok(db.query_row(
                "SELECT count(*) FROM run_events WHERE run_id='run'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
        MAX_ATOMIC_RETIRE_EVENTS + 2
    );
}

#[test]
fn duplicate_epoch_at_page_boundary_is_not_silently_retired() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("agent.db"), "completed", 300);
    let event = SseEvent {
        run_id: "run".into(),
        epoch: 1,
        idx: 255,
        ..SseEvent::new("text_chunk", json!({"text":"another epoch"}))
    };
    store
        .append_event("s", serde_json::to_value(event).unwrap())
        .unwrap();
    assert!(store.compact_run("s", "run").is_err());
    assert_eq!(store.events("s", "run").unwrap().len(), 303);
}

#[test]
fn unknown_envelope_and_non_duplicate_provider_text_keep_raw_journal() {
    for unknown in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = fixture(&dir.path().join("agent.db"), "completed", 1);
        store.db.call(move |db| {
            if unknown {
                db.execute("UPDATE run_events SET payload=json_set(payload,'$.future_field','must survive') WHERE idx=1",[])?;
            } else {
                db.execute("UPDATE run_events SET payload=json_set(payload,'$.event_type','text_delta') WHERE idx=1",[])?;
            }
            Ok(())
        }).unwrap();
        if unknown {
            assert!(store.compact_run("s", "run").is_err());
        } else {
            assert_eq!(store.compact_run("s", "run").unwrap(), 0);
        }
        assert_eq!(store.events("s", "run").unwrap().len(), 3);
        assert!(store
            .replay_page("s", "run", -1, false, None)
            .unwrap()
            .snapshot
            .is_none());
    }
}
