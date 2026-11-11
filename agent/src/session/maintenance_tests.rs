use super::*;

fn fixture() -> (tempfile::TempDir, SqliteStore, Manager) {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&root.path().join("agent.db")).unwrap();
    for id in ["old", "busy", "broken", "failed"] {
        store
            .replace(
                id,
                vec![json!({
                    "id":format!("entry-{id}"), "type":"user", "role":"user",
                    "content":[{"type":"text", "text":format!("中文内容{id}").repeat(3500)}],
                    "timestamp":"2026-01-01T00:00:00Z", "extension":"metadata".repeat(500),
                    "meta":{"run_id":format!("run-{id}")}
                })],
            )
            .unwrap();
    }
    store
        .db
        .call(|db| {
            db.execute("UPDATE runs SET status='completed',input_tokens=123", [])?;
            Ok(())
        })
        .unwrap();
    let manager = Manager::new(root.path().to_path_buf());
    manager.initialize().unwrap();
    (root, store, manager)
}

#[test]
fn maintenance_preserves_messages_and_usage_and_is_repeatable() {
    let (_root, store, manager) = fixture();
    let before = store.entries("old").unwrap();
    let revision = store.revision("old").unwrap();
    let result = manager.organize_database_session("old").unwrap();
    assert_eq!(result["status"], "done");
    assert_eq!(result["migrated"], true);
    assert_eq!(result["hasMore"], false);
    assert!(result["payloadBytesSaved"].as_i64().unwrap() > 0);
    assert_eq!(store.entries("old").unwrap(), before);
    assert!(store.revision("old").unwrap() > revision);
    let converted_revision = store.revision("old").unwrap();
    assert_eq!(store.storage_format("old").unwrap(), 2);
    store
        .db
        .read(|db| {
            assert_eq!(
                db.query_row(
                    "SELECT input_tokens FROM runs WHERE session_id='old'",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                123
            );
            assert_eq!(
                db.query_row("SELECT count(*) FROM content_operations", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            Ok(())
        })
        .unwrap();
    let repeat = manager.organize_database_session("old").unwrap();
    assert_eq!(repeat["hasMore"], false);
    assert_eq!(repeat["migrated"], false);
    assert_eq!(store.revision("old").unwrap(), converted_revision);
    assert_eq!(repeat["compressedBodies"], 0);
    assert_eq!(store.entries("old").unwrap(), before);
    assert_eq!(
        manager.finish_database_organization().unwrap()["integrity"],
        "ok"
    );
}

#[test]
fn busy_broken_and_failed_sessions_do_not_prevent_the_next_session() {
    let (_root, store, manager) = fixture();
    store.ensure_canonical("broken").unwrap();
    let busy_before = store.entries("busy").unwrap();
    store.db.call(|db| {
        db.execute("UPDATE runs SET status='running' WHERE session_id='busy'", [])?;
        db.execute("DELETE FROM content_segments WHERE content_id IN (SELECT content_id FROM content_references WHERE session_id='broken' AND field='/content/0/text')", [])?;
        db.execute_batch("CREATE TRIGGER fail_maintenance BEFORE INSERT ON content_compressed WHEN NEW.content_id IN (SELECT content_id FROM content_references WHERE session_id='failed') BEGIN SELECT RAISE(ABORT,'injected compression failure'); END")?;
        Ok(())
    }).unwrap();
    assert_eq!(
        manager.organize_database_session("busy").unwrap()["status"],
        "skipped"
    );
    assert_eq!(store.storage_format("busy").unwrap(), 1);
    assert_eq!(store.entries("busy").unwrap(), busy_before);
    assert!(manager.organize_database_session("broken").is_err());
    // Give the failure target its own body, so the trigger does not target a
    // digest-deduplicated object shared with the good conversation.
    store.replace("failed", vec![json!({"id":"unique","type":"user","role":"user","content":[{"type":"text","text":"unique body".repeat(1000)}]})]).unwrap();
    assert!(manager.organize_database_session("failed").is_err());
    assert_eq!(
        manager.organize_database_session("old").unwrap()["status"],
        "done"
    );
    store
        .db
        .read(|db| {
            assert_eq!(
                db.query_row("SELECT count(*) FROM content_operations", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            Ok(())
        })
        .unwrap();
    let finish = manager.finish_database_organization().unwrap();
    assert_eq!(finish["compacted"], false);
    assert_eq!(finish["deferredReason"], "busy");
}
