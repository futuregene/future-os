use super::*;
use crate::session::{Manager, SessionEntry};
use serde_json::json;

fn fixture() -> (tempfile::TempDir, SqliteStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store
        .replace(
            "old",
            vec![json!({
                "id":"6bd914ec-6c8c-4165-bd30-a103988aae69","type":"user","role":"user",
                "content":[{"type":"text","text":"retained text"}],
                "meta":{"run_id":"70b0a379-8653-413e-bb83-9ad240216c4d"},
                "timestamp":"2026-01-01T00:00:00Z","extension":{"keep":true}
            })],
        )
        .unwrap();
    store
        .db
        .call(|db| {
            db.execute(
                "UPDATE runs SET status='completed',input_tokens=123,cache_read_baseline=7",
                [],
            )?;
            db.execute("UPDATE sessions SET created_at_ms=1,updated_at_ms=2", [])?;
            Ok(())
        })
        .unwrap();
    (dir, store)
}

fn scalar(store: &SqliteStore, sql: &'static str) -> i64 {
    store
        .db
        .read(move |db| Ok(db.query_row(sql, [], |r| r.get(0))?))
        .unwrap()
}

#[test]
fn open_migrates_once_without_changing_messages_identities_usage_or_times() {
    let (dir, store) = fixture();
    let before = store.entries("old").unwrap();
    let manager = Manager::new(dir.path().to_path_buf());
    manager.load("old").unwrap();
    assert_eq!(store.storage_format("old").unwrap(), 2);
    assert_eq!(store.entries("old").unwrap(), before);
    assert_eq!(
        scalar(
            &store,
            "SELECT count(*) FROM entries WHERE typeof(entry_id)='text'"
        ),
        1
    );
    assert_eq!(scalar(&store,"SELECT count(*) FROM runs WHERE typeof(run_id)='text' AND input_tokens=123 AND cache_read_baseline=7"),1);
    assert_eq!(
        scalar(
            &store,
            "SELECT count(*) FROM sessions WHERE created_at_ms=1 AND updated_at_ms=2"
        ),
        1
    );
    let revision = store.revision("old").unwrap();
    manager.load("old").unwrap();
    assert_eq!(store.revision("old").unwrap(), revision);
    manager
        .append_entries(
            "old",
            &[SessionEntry::new_user("user", json!("new format"))],
        )
        .unwrap();
    assert_eq!(scalar(&store, "SELECT count(*) FROM run_events"), 0);
}

#[test]
fn listing_and_deleting_legacy_sessions_do_not_start_conversion() {
    let (dir, store) = fixture();
    store.db.call(|db| {db.execute_batch("CREATE TRIGGER forbid_conversion BEFORE INSERT ON legacy_conversions BEGIN SELECT RAISE(ABORT,'must not migrate'); END")?;Ok(())}).unwrap();
    let manager = Manager::new(dir.path().to_path_buf());
    assert_eq!(manager.list_all().unwrap().len(), 1);
    assert_eq!(store.storage_format("old").unwrap(), 1);
    manager.delete("old").unwrap();
    assert!(!store.contains("old").unwrap());
    assert_eq!(scalar(&store, "SELECT count(*) FROM legacy_conversions"), 0);
    assert_eq!(scalar(&store, "SELECT count(*) FROM session_tombstones"), 1);
}

#[test]
fn failed_publication_keeps_old_rows_and_retry_recovers_staged_bodies() {
    let (_dir, store) = fixture();
    let before = store.entries("old").unwrap();
    store.db.call(|db| {db.execute_batch("CREATE TRIGGER fail_publication BEFORE INSERT ON entries BEGIN SELECT RAISE(ABORT,'injected publication failure'); END")?;Ok(())}).unwrap();
    assert!(store.ensure_canonical("old").is_err());
    assert_eq!(store.storage_format("old").unwrap(), 1);
    assert_eq!(store.entries("old").unwrap(), before);
    assert_eq!(scalar(&store, "SELECT count(*) FROM legacy_conversions"), 1);
    store
        .db
        .call(|db| {
            db.execute_batch("DROP TRIGGER fail_publication")?;
            Ok(())
        })
        .unwrap();
    assert!(store.ensure_canonical("old").unwrap());
    assert_eq!(store.entries("old").unwrap(), before);
    assert_eq!(scalar(&store, "SELECT count(*) FROM legacy_conversions"), 0);
    assert_eq!(
        scalar(
            &store,
            "SELECT count(*) FROM content_references WHERE owner_kind='checkpoint'"
        ),
        0
    );
}

#[test]
fn deletion_during_staging_cannot_resurrect_a_session() {
    let (_dir, store) = fixture();
    let conversion = capture(&store, "old").unwrap().unwrap();
    super::super::entries::prepare_staged(
        &store,
        "old",
        conversion.entries.clone(),
        &conversion.prefix,
    )
    .unwrap();
    store.delete("old").unwrap();
    assert!(publish(&store, "old", conversion).is_err());
    assert_eq!(recover(&store, 16).unwrap(), 0);
    assert!(!store.contains("old").unwrap());
    assert_eq!(scalar(&store, "SELECT count(*) FROM entries"), 0);
}

#[test]
fn concurrent_handles_share_one_conversion() {
    let (dir, store) = fixture();
    let other = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let gate = barrier.clone();
    let a = std::thread::spawn(move || {
        gate.wait();
        store.ensure_canonical("old").unwrap()
    });
    let b = std::thread::spawn(move || {
        barrier.wait();
        other.ensure_canonical("old").unwrap()
    });
    assert_ne!(a.join().unwrap(), b.join().unwrap());
}

#[test]
fn migration_retains_pricing_from_raw_events_and_snapshots() {
    let (_dir, store) = fixture();
    store.append_event("old",json!({"run_id":"70b0a379-8653-413e-bb83-9ad240216c4d","epoch":0,"idx":0,"event_type":"usage","data":"{\"input_tokens\":123}"})).unwrap();
    store.db.call(|db| {
        db.execute("INSERT INTO run_snapshots(session_id,run_id,version,cursor,payload,pricing_payloads) VALUES ('old','folded',1,0,'{}',?1)",[json!([{"sequence":99,"payload":json!({"event_type":"usage","data":"{\"output_tokens\":7}"}).to_string()}]).to_string()])?;
        Ok(())
    }).unwrap();
    let before = store.pricing_event_payloads("old").unwrap();
    store.ensure_canonical("old").unwrap();
    assert_eq!(store.pricing_event_payloads("old").unwrap(), before);
    assert_eq!(scalar(&store, "SELECT count(*) FROM run_events"), 0);
    assert_eq!(scalar(&store, "SELECT count(*) FROM run_snapshots"), 0);
    assert_eq!(scalar(&store, "SELECT count(*) FROM run_pricing"), 2);
}

#[test]
fn active_runs_are_not_converted() {
    let (_dir, store) = fixture();
    store
        .db
        .call(|db| {
            db.execute("UPDATE runs SET status='running'", [])?;
            Ok(())
        })
        .unwrap();
    assert!(store.ensure_canonical("old").is_err());
    assert_eq!(store.storage_format("old").unwrap(), 1);
    assert_eq!(scalar(&store, "SELECT count(*) FROM legacy_conversions"), 0);
}

#[test]
fn writing_without_opening_converts_only_the_target_session() {
    let (dir, store) = fixture();
    let untouched = store.entries("old").unwrap();
    store.replace("untouched", untouched.clone()).unwrap();
    let manager = Manager::new(dir.path().to_path_buf());
    manager
        .append_entries(
            "old",
            &[SessionEntry::new_user("user", json!("accepted directly"))],
        )
        .unwrap();
    assert_eq!(store.storage_format("old").unwrap(), 2);
    assert_eq!(store.storage_format("untouched").unwrap(), 1);
    assert_eq!(store.entries("untouched").unwrap(), untouched);
    assert_eq!(store.entries("old").unwrap().len(), 2);
}

#[test]
fn message_page_opens_legacy_history_through_conversion() {
    let (dir, store) = fixture();
    let manager = Manager::new(dir.path().to_path_buf());
    let (entries, total) = manager.message_page("old", 0, 40).unwrap();
    assert_eq!(total, 1);
    assert_eq!(entries.len(), 1);
    assert_eq!(store.storage_format("old").unwrap(), 2);
}
