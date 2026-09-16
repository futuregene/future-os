use super::compaction_ops::*;
use super::*;
use serde_json::json;

fn fixture() -> (tempfile::TempDir, Manager) {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().to_owned());
    manager.storage().unwrap().replace("s",vec![
        json!({"id":"u","type":"user","role":"user","timestamp":"2026-01-01T00:00:00Z","content":"requirement"}),
        json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-01-01T00:00:01Z","content":"answer"}),
    ]).unwrap();
    (dir, manager)
}
fn claim(manager: &Manager, policy: serde_json::Value) -> (String, String) {
    match manager.claim_compaction("s", policy, "op").unwrap() {
        CompactionClaim::New { key, input } => (key, input),
        _ => panic!("new operation expected"),
    }
}
fn checkpoint() -> crate::compaction::ContextCheckpoint {
    serde_json::from_value(json!({"entry_id":"cp-entry","checkpoint_id":"cp","covered_from_entry_id":"u","cutoff_entry_id":"a",
        "summary":[{"type":"text","text":"summary"}],"protected_entry_ids":["u"],"tokens_before":100,"tokens_after":10,
        "trigger":"manual","algorithm_version":"semantic-s2-v1","model":"m","context_window":128000,"created_at":"2026-01-01T00:00:02Z"})).unwrap()
}
#[test]
fn checkpoint_and_receipt_are_atomic_and_survive_manager_restart() {
    let (dir, manager) = fixture();
    let policy = json!({"model":"m","instructions":"x"});
    let (key, input) = claim(&manager, policy.clone());
    let cp = checkpoint();
    let receipt =
        json!({"checkpoint":cp,"result":{"checkpointId":"cp","tokensBefore":100,"tokensAfter":10}});
    manager
        .finish_compaction(
            "s",
            &key,
            &input,
            Some(checkpoint_to_entry(&cp)),
            receipt.clone(),
        )
        .unwrap();
    manager
        .finish_compaction("s", &key, &input, Some(checkpoint_to_entry(&cp)), receipt)
        .unwrap();
    drop(manager);
    let manager = Manager::new(dir.path().to_owned());
    match manager
        .claim_compaction("s", json!({"instructions":"x","model":"m"}), "retry")
        .unwrap()
    {
        CompactionClaim::Cached {
            value,
            operation_id,
        } => {
            assert_eq!(operation_id, "op");
            assert_eq!(value["result"]["checkpointId"], "cp");
        }
        _ => panic!("cached result expected"),
    }
    assert_eq!(
        manager
            .load("s")
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.entry_type == ENTRY_TYPE_COMPACTION)
            .count(),
        1
    );
    // Session metadata and an additional checkpoint must not change input identity.
    manager
        .storage()
        .unwrap()
        .db
        .call(|db| {
            db.execute(
                "UPDATE sessions SET current_metadata_json=?1 WHERE id='s'",
                [r#"{"tokens_in":999}"#],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        manager.claim_compaction("s", policy, "again").unwrap(),
        CompactionClaim::Cached { .. }
    ));
}
#[test]
fn interrupted_claim_is_not_reexecuted_and_scope_is_exact() {
    let (dir, manager) = fixture();
    claim(&manager, json!({"policy":1}));
    drop(manager);
    let manager = Manager::new(dir.path().to_owned());
    assert!(manager
        .claim_compaction("s", json!({"policy":1}), "retry")
        .err()
        .unwrap()
        .to_string()
        .contains("compaction_indeterminate"));
    assert!(matches!(
        manager
            .claim_compaction("s", json!({"policy":2}), "changed")
            .unwrap(),
        CompactionClaim::New { .. }
    ));
}
#[test]
fn content_and_parameters_invalidate_but_failed_attempt_is_replayed() {
    let (_dir, manager) = fixture();
    let (key, _) = claim(&manager, json!({"model":"a"}));
    manager
        .fail_compaction("s", &key, "provider failed")
        .unwrap();
    assert!(manager
        .claim_compaction("s", json!({"model":"a"}), "retry")
        .err()
        .unwrap()
        .to_string()
        .contains("compaction_previous_failed"));
    manager
        .append_entries(
            "s",
            &[SessionEntry::new_user("user", json!("new directive"))],
        )
        .unwrap();
    assert!(matches!(
        manager
            .claim_compaction("s", json!({"model":"a"}), "new")
            .unwrap(),
        CompactionClaim::New { .. }
    ));
}
#[test]
fn input_change_prevents_checkpoint_commit_and_delete_cascades_receipts() {
    let (_dir, manager) = fixture();
    let (key, input) = claim(&manager, json!({}));
    manager
        .append_entries("s", &[SessionEntry::new_user("user", json!("new"))])
        .unwrap();
    assert!(manager
        .finish_compaction(
            "s",
            &key,
            &input,
            Some(checkpoint_to_entry(&checkpoint())),
            json!({})
        )
        .is_err());
    assert!(manager
        .load("s")
        .unwrap()
        .entries
        .iter()
        .all(|e| e.entry_type != ENTRY_TYPE_COMPACTION));
    manager.storage().unwrap().delete("s").unwrap();
    let count = manager
        .storage()
        .unwrap()
        .db
        .call(|db| {
            Ok(
                db.query_row("SELECT count(*) FROM compaction_operations", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(count, 0);
}
