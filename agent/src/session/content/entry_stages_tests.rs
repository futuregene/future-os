//! Entry-stage crash/ownership specifications, authored for final validation.
use super::*;
use crate::session::{Manager, SessionEntry};
use serde_json::json;

fn fixture() -> (tempfile::TempDir, Manager) {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let store = manager.storage().unwrap();
    store.reserve_segmented("s").unwrap();
    store.replace("s",vec![json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"model":"mock"}})]).unwrap();
    (dir, manager)
}
fn input(id: &str) -> Value {
    json!({"id":id,"type":"user","role":"user","timestamp":"2026-10-26T00:00:01Z","meta":{"run_id":"queued"},"content":"durable input🙂".repeat(18000)})
}
fn owners(store: &SqliteStore, id: &str) -> i64 {
    let id = id.to_owned();
    store.db.read(move |db|Ok(db.query_row("SELECT count(*) FROM content_references WHERE session_id='s' AND owner_kind='entry' AND owner_id=?1",[id],|row|row.get(0))?)).unwrap()
}
fn stages(store: &SqliteStore) -> i64 {
    store
        .db
        .read(|db| {
            Ok(db.query_row(
                "SELECT count(*) FROM content_operations WHERE kind='entry'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap()
}

#[test]
fn unsuccessful_publication_and_dropped_preparation_release_only_unpublished_owners() {
    let (_dir, manager) = fixture();
    let store = manager.storage().unwrap();
    store.append("s", vec![input("published")]).unwrap();
    store.db.call(|db| {
        db.execute_batch("CREATE TRIGGER refuse_new_entry BEFORE INSERT ON entries WHEN NEW.entry_id='refused' BEGIN SELECT RAISE(ABORT,'injected publication failure'); END;")?;Ok(())
    }).unwrap();
    assert!(store.append("s", vec![input("refused")]).is_err());
    assert_eq!(owners(store, "refused"), 0);
    assert!(owners(store, "published") > 0);
    let prepared = PreparedEntries::stage(store, "s", vec![input("abandoned")]).unwrap();
    assert!(owners(store, "abandoned") > 0);
    assert_eq!(stages(store), 1);
    drop(prepared);
    assert_eq!(owners(store, "abandoned"), 0);
    assert_eq!(stages(store), 0);
}

#[test]
fn concurrent_identical_preparations_keep_ownership_until_the_last_lease_finishes() {
    let (_dir, manager) = fixture();
    let store = manager.storage().unwrap();
    let first = PreparedEntries::stage(store, "s", vec![input("shared")]).unwrap();
    let mut second = PreparedEntries::stage(store, "s", vec![input("shared")]).unwrap();
    drop(first);
    assert_eq!(stages(store), 1);
    assert!(owners(store, "shared") > 0);
    let headers = second.take();
    store
        .db
        .call(move |db| {
            let tx = database::begin_write_transaction(db)?;
            crate::session::sqlite_store::insert_transcript_entries(&tx, "s", headers)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    drop(second);
    assert_eq!(stages(store), 0);
    assert!(owners(store, "shared") > 0);
    assert_eq!(
        store.entries("s").unwrap()[1]["content"][0]["text"],
        input("shared")["content"]
    );
}

#[test]
fn live_stages_survive_another_manager_open_but_a_crashed_receipt_is_reclaimed() {
    let (_dir, manager) = fixture();
    let store = manager.storage().unwrap();
    let prepared = PreparedEntries::stage(store, "s", vec![input("live")]).unwrap();
    let another = Manager::new(manager.dir.clone());
    another.storage().unwrap();
    assert_eq!(stages(store), 1);
    assert!(owners(store, "live") > 0);
    drop(prepared);
    let operation = uuid::Uuid::new_v4().to_string();
    store.db.call(move |db| {
        let tx=database::begin_write_transaction(db)?;
        tx.execute("INSERT INTO content_operations(operation_id,session_id,owner_prefix,expected_revision,kind) VALUES (?1,'s',?2,0,'entry')",params![operation,format!("entry:{operation}:")])?;
        tx.execute("INSERT INTO content_entry_stages(operation_id,session_id,entry_id,run_id) VALUES (?1,'s','crashed','queued')",[operation])?;
        tx.commit()?;Ok(())
    }).unwrap();
    super::super::entries::stage_named(
        store,
        "s",
        "entry",
        "crashed",
        "/content/0/text",
        "unpublished crash tail",
        false,
    )
    .unwrap();
    store
        .recover_content_operations(&crate::service::StartupCancellation::default())
        .unwrap();
    assert_eq!(owners(store, "crashed"), 0);
    assert_eq!(stages(store), 0);
    assert_eq!(store.entries("s").unwrap().len(), 1);
}

#[test]
fn queued_receipt_protects_accepted_body_and_a_failed_second_acceptance_leaks_nothing() {
    let (dir, manager) = fixture();
    let user: SessionEntry = serde_json::from_value(input("accepted")).unwrap();
    let ack = crate::runtime::RunAck::queued("queued".into(), 1, 1);
    manager
        .record_queued_receipt(
            "s",
            "request",
            &ack,
            "fingerprint",
            &json!({"ignored":"payload"}),
            &user,
        )
        .unwrap();
    let store = manager.storage().unwrap();
    assert_eq!(stages(store), 0);
    assert!(owners(store, "accepted") > 0);
    let other: SessionEntry = serde_json::from_value(input("refused")).unwrap();
    assert!(manager
        .record_queued_receipt("s", "request", &ack, "different", &json!({}), &other)
        .is_err());
    assert_eq!(owners(store, "refused"), 0);
    assert!(owners(store, "accepted") > 0);
    drop(manager);
    let reopened = Manager::new(dir.path().join("sessions"));
    let entries = reopened.storage().unwrap().entries("s").unwrap();
    let restored = entries
        .iter()
        .find(|entry| entry["id"] == "accepted")
        .unwrap();
    assert_eq!(restored["content"][0]["text"], input("accepted")["content"]);
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["type"] == "run_terminal")
            .count(),
        1
    );
    assert_eq!(stages(reopened.storage().unwrap()), 0);
}

#[test]
fn a_canonical_run_collision_lookup_does_not_read_historical_content() {
    let (_dir, manager) = fixture();
    let store = manager.storage().unwrap();
    store.append("s", vec![input("published")]).unwrap();
    store.db.call(|db| {
        db.execute("DELETE FROM content_segments WHERE content_id IN (SELECT content_id FROM content_references WHERE owner_id='published')",[])?;Ok(())
    }).unwrap();
    assert!(store.has_transcript_run("s", "queued").unwrap());
    assert!(!store.has_transcript_run("s", "other").unwrap());
    assert!(store.entries("s").is_err());
}
