//! No NFS mount is needed to verify policy, transaction ownership and recovery.
use crate::session::sqlite_store::SqliteStore;
use serde_json::json;

fn info(id: &str) -> serde_json::Value {
    json!({"id": id, "type":"session_info", "timestamp":"2026-11-04T00:00:00Z", "content":{"cwd":"/work","model":"test/model","session_name":"NFS conversation"}})
}

#[test]
fn nfs_create_delete_and_mode_switch_preserve_committed_history() {
    if crate::test_support::reexec_env_test() {
        return;
    }
    future_rpc::storage::StorageMode::Nfs.apply();
    let root = tempfile::tempdir().unwrap();
    future_rpc::storage::StorageMode::record_selection(root.path()).unwrap();
    assert_eq!(
        future_rpc::storage::StorageMode::read_home(root.path()).unwrap(),
        future_rpc::storage::StorageMode::Nfs
    );
    let path = root.path().join("agent.db");
    let store = SqliteStore::open(&path).unwrap();
    assert!(store.db.inner.wal.is_none());
    let settings = store
        .db
        .call(|db| {
            Ok((
                db.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))?,
                db.pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))?,
            ))
        })
        .unwrap();
    assert_eq!(settings, ("persist".into(), 2));
    let id = uuid::Uuid::new_v4().to_string();
    store.create_initial(&id, info("initial")).unwrap();
    assert!(store.contains(&id).unwrap());
    assert_eq!(
        store.entries(&id).unwrap()[0]["content"]["session_name"],
        "NFS conversation"
    );
    // Holding a page lease does not block another connection's durable write.
    let lease = store.db.read_snapshot().unwrap();
    assert!(!lease.pins_transaction());
    lease
        .read(|db| {
            let _: i64 = db.query_row("SELECT count(*) FROM sessions", [], |row| row.get(0))?;
            Ok(())
        })
        .unwrap();
    store.create_initial("second", info("second-info")).unwrap();
    let scope = crate::session::content::reader::Snapshot::capture(&store, &id).unwrap();
    store
        .delete_many(vec![id.clone(), "second".into()])
        .unwrap();
    assert!(!store.contains(&id).unwrap());
    assert!(scope.verify(&store).is_err());
    assert!(store.create_initial(&id, info("resurrect")).is_err());
    drop(lease);
    store.db.shutdown().unwrap();
    drop(store);
    let reopened = SqliteStore::open(&path).unwrap();
    assert!(!reopened.contains(&id).unwrap());
    reopened.create_initial("kept", info("kept-info")).unwrap();
    reopened.db.shutdown().unwrap();
    drop(reopened);
    future_rpc::storage::StorageMode::Wal.apply();
    let local = SqliteStore::open(&path).unwrap();
    assert!(local.db.inner.wal.is_some());
    assert!(local.contains("kept").unwrap());
    local.db.shutdown().unwrap();
}

#[test]
fn nfs_delete_failure_rolls_back_all_tombstones_and_large_metadata_roundtrips() {
    if crate::test_support::reexec_env_test() {
        return;
    }
    future_rpc::storage::StorageMode::Nfs.apply();
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&root.path().join("agent.db")).unwrap();
    store.db.call(|db| {
        db.execute_batch("CREATE TRIGGER fail_create BEFORE INSERT ON entries WHEN NEW.session_id='failed' BEGIN SELECT RAISE(ABORT,'injected creation failure'); END;")?;
        Ok(())
    }).unwrap();
    assert!(store.create_initial("failed", info("failed-info")).is_err());
    assert!(!store.contains("failed").unwrap());
    let pending: i64 = store
        .db
        .read(|db| {
            Ok(db.query_row(
                "SELECT count(*) FROM sessions WHERE id='failed'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(
        pending, 0,
        "failed atomic creation leaves no reserved identity"
    );
    store
        .db
        .call(|db| {
            db.execute_batch("DROP TRIGGER fail_create")?;
            Ok(())
        })
        .unwrap();
    let mut large = info("large-info");
    large["content"]["append_system_prompt"] = "instructions ".repeat(1000).into();
    store.create_initial("large", large.clone()).unwrap();
    assert_eq!(
        store.entries("large").unwrap()[0]["content"]["append_system_prompt"],
        large["content"]["append_system_prompt"]
    );
    store
        .create_initial("blocked", info("blocked-info"))
        .unwrap();
    store.db.call(|db| {
        db.execute_batch("CREATE TRIGGER fail_delete BEFORE DELETE ON sessions WHEN OLD.id='blocked' BEGIN SELECT RAISE(ABORT,'injected deletion failure'); END;")?;
        Ok(())
    }).unwrap();
    assert!(store
        .delete_many(vec!["large".into(), "blocked".into()])
        .is_err());
    assert!(store.contains("large").unwrap());
    let tombstones: i64 = store
        .db
        .read(|db| {
            Ok(
                db.query_row("SELECT count(*) FROM session_tombstones", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(tombstones, 0);
    store.db.shutdown().unwrap();
}
