//! Authored during implementation, for the final validation phase only.
use super::*;
use crate::session::Manager;
use serde_json::json;

fn configuration() -> Value {
    json!({"model":"mock","cwd":"/synthetic","session_name":"before","parent_session_id":"parent",
        "system_prompt":"persistent system rules🙂".repeat(2500),
        "source_meta":{"opaque":"provenance".repeat(3000)},
        "vendor/key~name":{"state":"state".repeat(3000)},"tokens_out":0})
}
fn setup() -> (tempfile::TempDir, Manager) {
    let root = tempfile::tempdir().unwrap();
    let manager = Manager::new(root.path().join("sessions"));
    let store = manager.storage().unwrap();
    store.reserve_segmented("s").unwrap();
    store.replace("s",vec![json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":configuration()})]).unwrap();
    (root, manager)
}
fn config(store: &SqliteStore) -> Value {
    store
        .entries("s")
        .unwrap()
        .into_iter()
        .find(|entry| entry["type"] == "session_info")
        .unwrap()["content"]
        .clone()
}
fn count_bodies(store: &SqliteStore) -> i64 {
    store
        .db
        .read(|db| Ok(db.query_row("SELECT count(*) FROM content_objects", [], |row| row.get(0))?))
        .unwrap()
}
fn boundary(id: &str, run: &str, tokens: i64, error: &str) -> Value {
    json!({"id":id,"type":"run_terminal","timestamp":"2026-10-26T00:00:02Z",
        "content":{"run_id":run,"state":"error","run_tokens":tokens,"run_duration_ms":2,
            "input_tokens":0,"cache_read_tokens":0,"error":error}})
}
fn info(tokens: i64) -> Value {
    json!({"id":format!("info-{tokens}"),"type":"session_info","timestamp":"2026-10-26T00:00:02Z",
        "content":{"tokens_out":tokens,"session_name":"stale frozen name","model":"stale frozen model",
            "system_prompt":"stale frozen rules","source_meta":{"must_not_replace":true}}})
}
fn start(store: &SqliteStore, run: &str) {
    store.append("s",vec![json!({"id":format!("start-{run}"),"type":"run_started","timestamp":"2026-10-26T00:00:01Z","content":{"run_id":run,"epoch":1}})]).unwrap();
}

#[test]
fn patching_a_title_reuses_large_configuration_and_retires_the_previous_owner() {
    let (_root, manager) = setup();
    let store = manager.storage().unwrap();
    let bodies = count_bodies(store);
    manager
        .update_session_info("s", "session_name", json!("renamed"))
        .unwrap();
    let actual = config(store);
    let mut expected = configuration();
    expected["session_name"] = json!("renamed");
    assert_eq!(actual, expected);
    assert_eq!(count_bodies(store), bodies);
    store.db.read(|db| {
        let bytes:i64=db.query_row("SELECT length(CAST(current_metadata_json AS BLOB)) FROM sessions WHERE id='s'",[],|row|row.get(0))?;
        assert!(bytes<4096);
        let stale:i64=db.query_row("SELECT count(*) FROM content_references WHERE session_id='s' AND owner_kind='entry' AND owner_id='info'",[],|row|row.get(0))?;
        assert_eq!(stale,0);
        let active:i64=db.query_row("SELECT count(*) FROM content_references WHERE session_id='s' AND owner_kind='entry' AND owner_id=(SELECT entry_id FROM entries WHERE session_id='s' AND entry_type='session_info')",[],|row|row.get(0))?;
        assert_eq!(active,3);
        assert_eq!(db.query_row("SELECT count(*) FROM content_operations",[],|row|row.get::<_,i64>(0))?,0);
        assert_eq!(db.query_row("SELECT count(*) FROM content_references WHERE owner_kind='checkpoint'",[],|row|row.get::<_,i64>(0))?,0);
        Ok(())
    }).unwrap();
    manager
        .update_session_info("s", "source_meta", Value::Null)
        .unwrap();
    assert!(config(store)["source_meta"].is_null());
    assert_eq!(
        config(store)["vendor/key~name"],
        configuration()["vendor/key~name"]
    );
}

#[test]
fn concurrent_independent_patches_merge_against_the_latest_committed_metadata() {
    let (_root, manager) = setup();
    let store = manager.storage().unwrap();
    let gate = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut workers = Vec::new();
    for (key, value) in [
        ("one", json!({"data":"一".repeat(5000)})),
        ("two", json!(["二".repeat(5000)])),
    ] {
        let (store, gate) = (store.clone(), gate.clone());
        workers.push(std::thread::spawn(move || {
            gate.wait();
            store.patch_segmented_metadata("s", [(key.to_owned(), value)].into_iter().collect())
        }));
    }
    gate.wait();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    let actual = config(store);
    assert_eq!(actual["one"], json!({"data":"一".repeat(5000)}));
    assert_eq!(actual["two"], json!(["二".repeat(5000)]));
    assert_eq!(actual["source_meta"], configuration()["source_meta"]);
}

#[test]
fn abandoned_metadata_staging_cleans_only_its_receipt_while_a_live_operation_is_protected() {
    let (_root, manager) = setup();
    let store = manager.storage().unwrap();
    let operation = uuid::Uuid::new_v4().to_string();
    let prefix = format!("metadata:{operation}:");
    let (op, p) = (operation.clone(), prefix.clone());
    store.db.call(move |db| {
        db.execute("INSERT INTO content_operations(operation_id,session_id,owner_prefix,expected_revision,kind) SELECT ?1,'s',?2,revision,'metadata' FROM sessions WHERE id='s'",params![op,p])?;Ok(())
    }).unwrap();
    super::super::entries::stage_named(
        store,
        "s",
        "checkpoint",
        &format!("{prefix}unpublished"),
        "/content/source_meta",
        "unacknowledged value",
        false,
    )
    .unwrap();
    let active = ActiveOperation::claim(&operation);
    let other = SqliteStore::open(&manager.database_path()).unwrap();
    other
        .recover_content_operations(&crate::service::StartupCancellation::default())
        .unwrap();
    assert_eq!(
        other
            .db
            .read(|db| Ok(db
                .query_row("SELECT count(*) FROM content_operations", [], |row| row
                    .get::<_, i64>(0))?))
            .unwrap(),
        1
    );
    drop(active);
    other
        .recover_content_operations(&crate::service::StartupCancellation::default())
        .unwrap();
    assert_eq!(config(&other), configuration());
    assert_eq!(
        other
            .db
            .read(|db| Ok(db.query_row(
                "SELECT count(*) FROM content_references WHERE owner_kind='checkpoint'",
                [],
                |row| row.get::<_, i64>(0)
            )?))
            .unwrap(),
        0
    );
}

#[test]
fn metadata_and_terminal_publish_atomically_and_retry_never_rolls_back_a_newer_run() {
    let (_root, manager) = setup();
    let store = manager.storage().unwrap();
    start(store, "one");
    let first = boundary("end-one", "one", 10, &"first error".repeat(1000));
    store
        .commit_terminal_entries("s", vec![info(10), first.clone()])
        .unwrap();
    assert_eq!(config(store)["tokens_out"], 10);
    assert_eq!(config(store)["model"], "mock");
    assert_eq!(config(store)["session_name"], "before");
    assert_eq!(
        config(store)["system_prompt"],
        configuration()["system_prompt"]
    );
    assert_eq!(
        store
            .segmented_replay("s", "one", -1, false, Some(10))
            .unwrap()
            .events
            .last()
            .unwrap()
            .event_type,
        "agent_end"
    );

    // Another terminal identity for the closed run rejects AFTER preparation,
    // rolling back its metadata and derived run state in the same transaction.
    let wrong = boundary("different-end", "one", 999, "wrong");
    assert!(store
        .commit_terminal_entries("s", vec![info(999), wrong])
        .is_err());
    assert_eq!(config(store)["tokens_out"], 10);
    assert!(store
        .entries("s")
        .unwrap()
        .iter()
        .all(|entry| entry["id"] != "different-end"));

    start(store, "two");
    store
        .commit_terminal_entries(
            "s",
            vec![info(50), boundary("end-two", "two", 40, "second")],
        )
        .unwrap();
    let revision = store.revision("s").unwrap();
    store
        .commit_terminal_entries("s", vec![info(10), first.clone()])
        .unwrap();
    assert_eq!(config(store)["tokens_out"], 50);
    assert_eq!(store.revision("s").unwrap(), revision);
    let mut changed = first;
    changed["content"]["error"] = json!("");
    assert!(store
        .commit_terminal_entries("s", vec![info(10), changed])
        .is_err());
    assert_eq!(config(store)["tokens_out"], 50);
}

#[test]
fn runtime_metadata_and_run_finalization_do_not_load_historical_message_bodies() {
    let (_root, manager) = setup();
    let store = manager.storage().unwrap();
    store.append("s",vec![json!({"id":"user","type":"user","role":"user","timestamp":"2026-10-26T00:00:01Z","content":"historical body".repeat(3000)})]).unwrap();
    store.db.call(|db| {
        db.execute("DELETE FROM content_segments WHERE content_id IN (SELECT content_id FROM content_references WHERE session_id='s' AND owner_id='user')",[])?;Ok(())
    }).unwrap();
    assert_eq!(manager.state_metadata("s").unwrap(), (1, "parent".into()));
    let loaded = manager.load_metadata("s").unwrap();
    assert_eq!(
        loaded.get_session_info().unwrap()["system_prompt"],
        configuration()["system_prompt"]
    );
    start(store, "one");
    let persistence =
        crate::session::SessionPersistence::new(std::sync::Arc::new(manager), "s".into());
    let metadata =
        SessionEntry::session_info(json!({"tokens_out":1}), String::new(), String::new());
    persistence
        .commit_run(vec![
            metadata,
            serde_json::from_value(boundary("end", "one", 1, "failed")).unwrap(),
        ])
        .unwrap();
}
