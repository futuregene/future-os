//! Explicit-rewrite ownership specifications, authored for final validation.
use super::*;
use crate::session::{compaction_ops::CompactionClaim, sqlite_store::SqliteStore, Manager};
use serde_json::json;

fn fixture() -> (tempfile::TempDir, Manager) {
    let root = tempfile::tempdir().unwrap();
    let manager = Manager::new(root.path().join("sessions"));
    let store = manager.storage().unwrap();
    store.reserve_segmented("s").unwrap();
    store.replace("s",vec![json!({"id":"u","type":"user","role":"user","timestamp":"2026-10-26T00:00:00Z","content":"shared error text🙂".repeat(1000)})]).unwrap();
    (root, manager)
}
fn claim(manager: &Manager, session: &str, policy: i64) -> (String, String) {
    match manager
        .claim_compaction(session, json!({"policy":policy}), "original")
        .unwrap()
    {
        CompactionClaim::New { key, input } => (key, input),
        _ => panic!("expected new checkpoint receipt"),
    }
}
fn finish_cleanup(store: &SqliteStore) {
    store
        .db
        .call(|db| {
            for _ in 0..128 {
                if reclaim(db)? {
                    return Ok(());
                }
            }
            anyhow::bail!("checkpoint retirement failed to finish")
        })
        .unwrap();
}

#[test]
fn rewrite_retires_old_receipts_and_error_owners_but_preserves_current_legacy_and_shared_bytes() {
    let (_root, manager) = fixture();
    let store = manager.storage().unwrap();
    store.replace("legacy",vec![json!({"id":"l","type":"user","role":"user","timestamp":"2026-10-26T00:00:00Z","content":"legacy original"})]).unwrap();
    let (old, _) = claim(&manager, "s", 1);
    manager
        .fail_compaction("s", &old, &"shared error text🙂".repeat(1000))
        .unwrap();
    let (complete, input) = claim(&manager, "s", 2);
    manager
        .finish_compaction(
            "s",
            &complete,
            &input,
            None,
            json!({"checkpoint":null,"result":"unchanged"}),
        )
        .unwrap();
    let (_started, _) = claim(&manager, "s", 3);
    let (legacy, _) = claim(&manager, "legacy", 1);
    manager
        .fail_compaction("legacy", &legacy, "legacy error")
        .unwrap();
    let rows = store.entries("s").unwrap();
    store.db.call(|db| {
        db.execute_batch("CREATE TRIGGER hold_old_receipts BEFORE DELETE ON compaction_operations WHEN OLD.session_id='s' BEGIN SELECT RAISE(ABORT,'injected cleanup retry'); END;")?;Ok(())
    }).unwrap();
    store.replace("s", rows.clone()).unwrap();
    let (current, _) = claim(&manager, "s", 1);
    assert_ne!(current, old);
    manager
        .fail_compaction("s", &current, "current generation failure")
        .unwrap();
    assert!(store.db.call(reclaim).is_err());
    let (old_copy, current_copy) = (old.clone(), current.clone());
    store.db.read(move |db| {
        for key in [&old_copy,&current_copy] {
            assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM content_references WHERE session_id='s' AND owner_id=?1)",[format!("compaction-error:{key}")],|row|row.get::<_,bool>(0))?);
        }
        Ok(())
    }).unwrap();
    store
        .db
        .call(|db| {
            db.execute_batch("DROP TRIGGER hold_old_receipts")?;
            Ok(())
        })
        .unwrap();
    finish_cleanup(store);
    assert_eq!(store.entries("s").unwrap(), rows);
    let current_copy = current.clone();
    store
        .db
        .read(move |db| {
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM compaction_operations WHERE session_id='s'",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                1
            );
            assert_eq!(
                db.query_row(
                    "SELECT input_key FROM compaction_operations WHERE session_id='s'",
                    [],
                    |row| row.get::<_, String>(0)
                )?,
                current_copy
            );
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM compaction_operations WHERE session_id='legacy'",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                1
            );
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM checkpoint_generations WHERE session_id='legacy'",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                0
            );
            assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);
            Ok(())
        })
        .unwrap();
    assert!(manager
        .claim_compaction("s", json!({"policy":1}), "retry")
        .err()
        .unwrap()
        .to_string()
        .contains("current generation failure"));
}

#[test]
fn one_retirement_slice_removes_at_most_64_owned_fields_and_never_scans_body_segments() {
    let (_root, manager) = fixture();
    let (key, _) = claim(&manager, "s", 1);
    manager
        .fail_compaction("s", &key, "original failure")
        .unwrap();
    manager.storage().unwrap().db.call(move |db| {
        let tx=database::begin_write_transaction(db)?;
        let owner=format!("compaction-error:{key}");
        for i in 0..128 {
            tx.execute("INSERT INTO content_references(session_id,owner_kind,owner_id,field,encoding,content_id,byte_offset,byte_length) SELECT session_id,owner_kind,owner_id,?2,encoding,content_id,byte_offset,byte_length FROM content_references WHERE session_id='s' AND owner_id=?1 AND field='/error'",params![owner,format!("/synthetic/{i}")])?;
        }
        advance(&tx,"s")?;
        tx.commit()?;
        let count=|db:&Connection|->Result<i64> {Ok(db.query_row("SELECT count(*) FROM content_references WHERE session_id='s' AND owner_id=?1",[&owner],|row|row.get(0))?)};
        assert_eq!(count(db)?,129);
        reclaim(db)?;assert_eq!(count(db)?,65);
        reclaim(db)?;assert_eq!(count(db)?,1);
        assert!(reclaim(db)?);assert_eq!(count(db)?,0);
        assert!(!db.query_row("SELECT EXISTS(SELECT 1 FROM compaction_operations WHERE session_id='s' AND input_key=?1)",[key],|row|row.get::<_,bool>(0))?);
        Ok(())
    }).unwrap();
}

#[test]
fn stale_failure_cannot_recreate_error_ownership_after_an_explicit_rewrite() {
    let (_root, manager) = fixture();
    let (key, _) = claim(&manager, "s", 1);
    let store = manager.storage().unwrap();
    store.replace("s", store.entries("s").unwrap()).unwrap();
    manager
        .fail_compaction("s", &key, "late provider failure")
        .unwrap();
    finish_cleanup(store);
    let owner = format!("compaction-error:{key}");
    assert!(!store
        .db
        .read(move |db| Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM content_references WHERE session_id='s' AND owner_id=?1)",
            [owner],
            |row| row.get::<_, bool>(0)
        )?))
        .unwrap());
}
