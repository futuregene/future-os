use super::*;
use serde_json::Value;

#[test]
fn event_page_bounds_decoding_before_a_corrupt_tail() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&directory.path().join("agent.db")).unwrap();
    store.bind_events("synthetic").unwrap();
    for idx in 0..4 {
        store
            .append_event(
                "synthetic",
                serde_json::json!({
                    "run_id": "run", "epoch": 1, "idx": idx,
                }),
            )
            .unwrap();
    }
    store
        .db
        .call(|db| {
            db.execute(
                "UPDATE run_events SET payload='{\"idx\":\"invalid\"}' WHERE idx=3",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    #[derive(serde::Deserialize)]
    struct Event {
        idx: i64,
    }
    let (known, page) = store
        .typed_events_page::<Event>("synthetic", "run", -1, false, Some(2))
        .unwrap();
    assert!(known);
    assert_eq!(
        page.iter().map(|event| event.idx).collect::<Vec<_>>(),
        [0, 1]
    );
    let (_, next) = store
        .typed_events_page::<Event>("synthetic", "run", 1, false, Some(1))
        .unwrap();
    assert_eq!(next[0].idx, 2);
    assert!(store
        .typed_events_page::<Event>("synthetic", "run", 2, false, Some(1),)
        .is_err());
}

#[test]
fn event_batch_conflict_rolls_back_every_row() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&directory.path().join("agent.db")).unwrap();
    store.bind_events("synthetic").unwrap();
    let first = serde_json::json!({
        "session_id": "synthetic", "run_id": "run", "epoch": 1, "idx": 0,
        "event_type": "text_chunk", "data": "{\"text\":\"first\"}"
    });
    let mut conflicting = first.clone();
    conflicting["data"] = serde_json::json!("different");

    assert!(store
        .append_events("synthetic", vec![first, conflicting])
        .is_err());
    assert!(store.events("synthetic", "run").unwrap().is_empty());
}

#[test]
fn cursor_query_does_not_decode_old_payloads_and_uses_range_index() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&directory.path().join("sessions.sqlite")).unwrap();
    store.bind_events("synthetic").unwrap();
    for idx in 0..3 {
        store
            .append_event(
                "synthetic",
                serde_json::json!({
                    "run_id": "run", "epoch": 1, "idx": idx, "event_id": format!("e{idx}")
                }),
            )
            .unwrap();
    }
    store.db.call(|db| {
        db.execute("UPDATE run_events SET payload='{\"idx\":\"not-an-integer\"}' WHERE idx=0", [])?;
        let mut statement = db.prepare("EXPLAIN QUERY PLAN SELECT payload FROM run_events WHERE session_id='synthetic' AND run_id='run' AND idx>1 ORDER BY epoch,idx")?;
        let plans = statement.query_map([], |row| row.get::<_,String>(3))?.collect::<rusqlite::Result<Vec<_>>>()?;
        assert!(plans.iter().any(|plan| plan.contains("sqlite_autoindex_run_events_1") && plan.contains("idx>?")));
        assert!(!plans.iter().any(|plan| plan.contains("run_events_cursor")));
        Ok(())
    }).unwrap();
    #[derive(serde::Deserialize)]
    struct Event {
        idx: i64,
    }
    let (known, tail) = store
        .typed_events_since::<Event>("synthetic", "run", 1, false)
        .unwrap();
    assert!(known);
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].idx, 2);
    assert!(store.typed_events::<Event>("synthetic", "run").is_err());
    assert!(store
        .typed_events_since::<Event>("synthetic", "run", 2, false)
        .unwrap()
        .1
        .is_empty());
    assert!(
        !store
            .typed_events_since::<Event>("synthetic", "unknown", -1, false)
            .unwrap()
            .0
    );
}

#[test]
fn compact_event_storage_reconstructs_identity_and_unknown_fields() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&directory.path().join("agent.db")).unwrap();
    store.bind_events("synthetic").unwrap();
    let event = serde_json::json!({
        "event_type": "text_chunk",
        "data": "{\"text\":\"safe\"}",
        "session_id": "synthetic",
        "run_id": "run",
        "epoch": 1,
        "idx": 0,
        "session_idx": -1,
        "run_sequence": 3,
        "timestamp": "2026-01-01T00:00:00Z",
        "future_field": [1,2,3]
    });
    store.append_event("synthetic", event.clone()).unwrap();
    let mut expected = event;
    expected["event_id"] = serde_json::json!("synthetic:run:1:0");
    assert_eq!(store.events("synthetic", "run").unwrap(), [expected]);
    store
        .db
        .call(|db| {
            let (sequence, event_id, payload): (i64, Option<String>, String) = db.query_row(
                "SELECT sequence,event_id,payload FROM run_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            assert!(sequence > 0);
            assert!(event_id.is_none());
            let compact: Value = serde_json::from_str(&payload)?;
            assert!(compact.get("session_id").is_none());
            assert!(compact.get("run_id").is_none());
            assert!(compact.get("event_id").is_none());
            assert_eq!(compact["future_field"], serde_json::json!([1, 2, 3]));
            Ok(())
        })
        .unwrap();
}
