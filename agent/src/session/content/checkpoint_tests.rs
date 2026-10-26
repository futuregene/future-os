//! Canonical checkpoint receipt specifications, run only in final validation.
use super::*;
use crate::session::{checkpoint_to_entry, Manager};

fn fixture() -> (tempfile::TempDir, Manager) {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let store = manager.storage().unwrap();
    store.reserve_segmented("s").unwrap();
    store.replace("s", vec![
        json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"model":"mock"}}),
        json!({"id":"u","type":"user","role":"user","timestamp":"2026-10-26T00:00:01Z","content":"original requirement🙂".repeat(5000)}),
        json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:02Z","content":"original answer".repeat(5000)}),
    ]).unwrap();
    (dir, manager)
}
fn checkpoint() -> crate::compaction::ContextCheckpoint {
    serde_json::from_value(json!({"entry_id":"cp-entry","checkpoint_id":"cp","covered_from_entry_id":"u","cutoff_entry_id":"a",
        "summary":[{"type":"text","text":"checkpoint summary🙂".repeat(12000)}],"protected_entry_ids":["u"],"tokens_before":100000,"tokens_after":10000,
        "trigger":"manual","algorithm_version":"deterministic-evidence-v1","model":"mock","context_window":128000,"created_at":"2026-10-26T00:00:03Z"})).unwrap()
}
fn claim(manager: &Manager, policy: Value) -> (String, String) {
    match manager
        .claim_compaction("s", policy, "original-operation")
        .unwrap()
    {
        CompactionClaim::New { key, input } => (key, input),
        _ => panic!("expected new checkpoint admission"),
    }
}
fn finish(manager: &Manager, key: &str, input: &str) -> Result<()> {
    manager.finish_compaction(
        "s",
        key,
        input,
        Some(checkpoint_to_entry(&checkpoint())),
        json!({"checkpoint":null,"result":{"checkpointId":"cp","tokensAfter":10000}}),
    )
}
fn cached(manager: &Manager, policy: Value) -> Value {
    match manager.claim_compaction("s", policy, "retry").unwrap() {
        CompactionClaim::Cached {
            value,
            operation_id,
        } => {
            assert_eq!(operation_id, "original-operation");
            value
        }
        _ => panic!("expected cached checkpoint receipt"),
    }
}
fn assert_no_staging(store: &SqliteStore) {
    store
        .db
        .read(|db| {
            for sql in [
                "SELECT count(*) FROM content_operations",
                "SELECT count(*) FROM content_entry_stages",
                "SELECT count(*) FROM content_references WHERE owner_kind='checkpoint'",
            ] {
                assert_eq!(
                    db.query_row(sql, [], |row| row.get::<_, i64>(0))?,
                    0,
                    "{sql}"
                );
            }
            Ok(())
        })
        .unwrap();
}

#[test]
fn completed_receipt_is_thin_and_reconstructs_the_exact_checkpoint_after_reopen() {
    let (dir, manager) = fixture();
    let policy = json!({"model":"mock","policy":1});
    let (key, input) = claim(&manager, policy.clone());
    finish(&manager, &key, &input).unwrap();
    let store = manager.storage().unwrap();
    let expected =
        json!({"checkpoint":checkpoint(),"result":{"checkpointId":"cp","tokensAfter":10000}});
    assert_eq!(cached(&manager, policy.clone()), expected);
    let bodies=store.db.read(|db| {
        let raw:String=db.query_row("SELECT result_json FROM compaction_operations WHERE session_id='s'",[],|row|row.get(0))?;
        assert!(raw.len()<2048 && !raw.contains("checkpoint summary"));
        assert_eq!(db.query_row("SELECT count(*) FROM content_references WHERE session_id='s' AND owner_kind='entry' AND owner_id='cp-entry' AND field='/content/summary'",[],|row|row.get::<_,i64>(0))?,1);
        Ok(db.query_row("SELECT count(*) FROM content_objects",[],|row|row.get::<_,i64>(0))?)
    }).unwrap();
    finish(&manager, &key, &input).unwrap();
    assert_no_staging(store);
    assert_eq!(
        store
            .db
            .read(|db| Ok(db
                .query_row("SELECT count(*) FROM content_objects", [], |row| row
                    .get::<_, i64>(0))?))
            .unwrap(),
        bodies
    );
    drop(manager);
    let manager = Manager::new(dir.path().join("sessions"));
    assert_eq!(cached(&manager, policy), expected);
    assert_eq!(
        manager
            .load("s")
            .unwrap()
            .entries
            .iter()
            .filter(|entry| entry.entry_type == "compaction")
            .count(),
        1
    );
}

#[test]
fn admission_and_cached_receipts_do_not_require_full_history_views_or_dialogue_bodies() {
    let (_dir, manager) = fixture();
    let store = manager.storage().unwrap();
    store.db.call(|db| {
        db.execute_batch("DROP VIEW entry_records; DROP VIEW block_records; DROP VIEW resolved_message_blocks")?;
        db.execute("DELETE FROM content_segments WHERE content_id IN (SELECT content_id FROM content_references WHERE session_id='s' AND owner_id IN ('u','a'))",[])?;
        Ok(())
    }).unwrap();
    let (key, input) = claim(&manager, json!({"policy":1}));
    finish(&manager, &key, &input).unwrap();
    assert_eq!(
        cached(&manager, json!({"policy":1}))["checkpoint"],
        serde_json::to_value(checkpoint()).unwrap()
    );
    assert!(
        store.entries("s").is_err(),
        "the fixture must actually have unreadable dialogue bodies"
    );
}

#[test]
fn input_identity_ignores_metadata_markers_and_checkpoints_but_fences_dialogue_and_rewrites() {
    let (_dir, manager) = fixture();
    let store = manager.storage().unwrap();
    let identity = || store.db.read(|db| input_identity(db, "s")).unwrap();
    let initial = identity();
    manager
        .update_session_info("s", "session_name", json!("changed title"))
        .unwrap();
    store.append("s",vec![json!({"id":"start","type":"run_started","timestamp":"2026-10-26T00:00:04Z","content":{"run_id":"r","epoch":1}})]).unwrap();
    assert_eq!(identity(), initial);
    let (key, input) = claim(&manager, json!({"policy":1}));
    finish(&manager, &key, &input).unwrap();
    assert_eq!(identity(), initial);
    store.append("s",vec![json!({"id":"u2","type":"user","role":"user","timestamp":"2026-10-26T00:00:05Z","content":"new directive"})]).unwrap();
    let appended = identity();
    assert_ne!(appended, initial);
    let rows = store.entries("s").unwrap();
    store.replace("s", rows).unwrap();
    assert_ne!(
        identity(),
        appended,
        "an explicit rewrite must fence even identical retained bytes"
    );
    store.db.read(|db| {
        for kind in ["user","assistant","tool","system"] {
            let plan:String=db.query_row("EXPLAIN QUERY PLAN SELECT max(position) FROM entries WHERE session_id='s' AND entry_type=?1",[kind],|row|row.get(3))?;
            assert!(plan.contains("entries_kind"),"{plan}");
        }
        Ok(())
    }).unwrap();
}

#[test]
fn changed_input_or_failed_atomic_publication_keeps_original_history_and_allows_explicit_retry() {
    let (_dir, manager) = fixture();
    let store = manager.storage().unwrap();
    let (key, input) = claim(&manager, json!({"policy":1}));
    store.db.call(|db| {
        db.execute_batch("CREATE TRIGGER refuse_checkpoint BEFORE UPDATE OF state ON compaction_operations WHEN NEW.state='completed' BEGIN SELECT RAISE(ABORT,'injected'); END;")?;Ok(())
    }).unwrap();
    assert!(finish(&manager, &key, &input).is_err());
    assert_no_staging(store);
    assert!(store
        .entries("s")
        .unwrap()
        .iter()
        .all(|entry| entry["id"] != "cp-entry"));
    store
        .db
        .call(|db| {
            db.execute_batch("DROP TRIGGER refuse_checkpoint")?;
            Ok(())
        })
        .unwrap();
    finish(&manager, &key, &input).unwrap();
    let (changed_key, changed_input) = claim(&manager, json!({"policy":2}));
    store.append("s",vec![json!({"id":"u2","type":"user","role":"user","timestamp":"2026-10-26T00:00:06Z","content":"new directive"})]).unwrap();
    assert!(finish(&manager, &changed_key, &changed_input)
        .unwrap_err()
        .to_string()
        .contains("compaction_input_changed"));
    assert_no_staging(store);
    assert!(manager
        .claim_compaction("s", json!({"policy":1}), "new-input")
        .is_ok());
}

#[test]
fn conflicting_receipts_and_cleared_externalized_summary_never_reuse_a_checkpoint_identity() {
    let (_dir, manager) = fixture();
    let (key, input) = claim(&manager, json!({"policy":1}));
    finish(&manager, &key, &input).unwrap();
    assert!(manager
        .finish_compaction(
            "s",
            &key,
            &input,
            Some(checkpoint_to_entry(&checkpoint())),
            json!({"checkpoint":null,"result":"different"})
        )
        .is_err());
    let (key, input) = claim(&manager, json!({"policy":2}));
    let mut empty = checkpoint();
    empty.summary.clear();
    assert!(manager
        .finish_compaction(
            "s",
            &key,
            &input,
            Some(checkpoint_to_entry(&empty)),
            json!({"checkpoint":null,"result":null})
        )
        .is_err());
    assert_eq!(
        cached(&manager, json!({"policy":1}))["checkpoint"],
        serde_json::to_value(checkpoint()).unwrap()
    );
    assert_no_staging(manager.storage().unwrap());
}

#[test]
fn failure_bodies_are_referenced_and_digest_checked_without_a_duplicate_receipt_copy() {
    let (_dir, manager) = fixture();
    let (key, _) = claim(&manager, json!({"policy":1}));
    let error = "checkpoint failure🙂".repeat(25000);
    manager.fail_compaction("s", &key, &error).unwrap();
    let store = manager.storage().unwrap();
    store
        .db
        .read(|db| {
            let raw: String = db.query_row(
                "SELECT result_json FROM compaction_operations WHERE session_id='s'",
                [],
                |row| row.get(0),
            )?;
            assert!(raw.len() < 1024 && !raw.contains("checkpoint failure"));
            Ok(())
        })
        .unwrap();
    let replayed = manager
        .claim_compaction("s", json!({"policy":1}), "retry")
        .err()
        .unwrap()
        .to_string();
    assert_eq!(replayed, format!("compaction_previous_failed: {error}"));
    assert_no_staging(store);
    store.db.call(|db| {
        db.execute("UPDATE content_segments SET body='damaged' WHERE content_id IN (SELECT content_id FROM content_references WHERE session_id='s' AND owner_kind='tool' AND field='/error') AND byte_offset=0",[])?;Ok(())
    }).unwrap();
    assert!(manager
        .claim_compaction("s", json!({"policy":1}), "retry")
        .is_err());
    manager.delete("s").unwrap();
    assert_eq!(
        store
            .db
            .read(|db| Ok(db.query_row(
                "SELECT count(*) FROM content_references WHERE owner_kind='tool'",
                [],
                |row| row.get::<_, i64>(0)
            )?))
            .unwrap(),
        0
    );
}

#[test]
fn modified_checkpoint_body_is_refused_and_a_noop_receipt_remains_replayable() {
    let (_dir, manager) = fixture();
    let (key, input) = claim(&manager, json!({"policy":0}));
    manager
        .finish_compaction(
            "s",
            &key,
            &input,
            None,
            json!({"checkpoint":null,"result":{"unchanged":true}}),
        )
        .unwrap();
    assert_eq!(
        cached(&manager, json!({"policy":0})),
        json!({"checkpoint":null,"result":{"unchanged":true}})
    );
    let (key, input) = claim(&manager, json!({"policy":1}));
    finish(&manager, &key, &input).unwrap();
    manager.storage().unwrap().db.call(|db| {
        db.execute("UPDATE entries SET content_json=json_set(content_json,'$.tokens_after',999) WHERE session_id='s' AND entry_id='cp-entry'",[])?;Ok(())
    }).unwrap();
    assert!(manager
        .claim_compaction("s", json!({"policy":1}), "retry")
        .err()
        .unwrap()
        .to_string()
        .contains("modified"));
}
