use super::*;

#[test]
fn batch_delete_rolls_back_sessions_events_and_import_tombstones_together() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    for session in ["good", "bad"] {
        store
            .replace(
                session,
                vec![serde_json::json!({
                    "id": "user", "type": "user", "timestamp": "2026-01-01T00:00:00Z",
                    "content": "synthetic", "role": "user"
                })],
            )
            .unwrap();
        store
            .append_event(
                session,
                serde_json::json!({
                    "run_id": "run", "epoch": 1, "idx": 0,
                    "event_type": "text_chunk", "data": "{}"
                }),
            )
            .unwrap();
    }
    store.db.call(|db| {
        db.execute_batch("INSERT INTO legacy_imports(session_id,status,fingerprint) VALUES ('good','imported','synthetic');
            CREATE TRIGGER reject_batch_delete BEFORE DELETE ON sessions WHEN OLD.id='bad'
            BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;")?;
        Ok(())
    }).unwrap();
    assert!(store
        .delete_many(vec!["good".into(), "bad".into()])
        .is_err());
    assert!(store.contains("good").unwrap());
    assert_eq!(store.events("good", "run").unwrap().len(), 1);
    let status: String = store
        .db
        .call(|db| {
            Ok(db.query_row(
                "SELECT status FROM legacy_imports WHERE session_id='good'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(status, "imported");
    store
        .db
        .call(|db| {
            db.execute_batch("DROP TRIGGER reject_batch_delete")?;
            Ok(())
        })
        .unwrap();
    store
        .delete_many(vec!["good".into(), "bad".into(), "missing".into()])
        .unwrap();
    assert!(!store.contains("good").unwrap());
    assert!(!store.contains("bad").unwrap());
    assert!(store.events("good", "run").unwrap().is_empty());
    store
        .delete_many(vec!["good".into(), "bad".into()])
        .unwrap();
}
