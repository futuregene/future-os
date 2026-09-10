//! Fully synthetic fixtures. Never read the user's config/session directory.
use future_agent::session::sqlite_store::SqliteStore;
use serde_json::{json, Value};
use std::path::Path;

fn entry(id: &str) -> Value {
    json!({"id":id,"type":"user","role":"user","content":[{"type":"text","text":"synthetic question"}],"meta":{"run_id":"fixture-run"},
        "timestamp":"2026-01-01T00:00:00Z","future_field":{"preserve":true}})
}

fn event(session: &str, run: &str, index: i64) -> Value {
    json!({"event_type":"text_chunk","data":"{\"text\":\"synthetic\"}",
        "session_id":session,"run_id":run,"epoch":1,"idx":index,
        "session_idx":-1,"run_sequence":1,"timestamp":"2026-01-01T00:00:00Z"})
}

fn write_source(directory: &Path, session: &str, entries: &[Value]) -> Vec<u8> {
    std::fs::create_dir_all(directory).unwrap();
    let bytes = entries
        .iter()
        .map(|v| format!("{v}\n"))
        .collect::<String>()
        .into_bytes();
    std::fs::write(directory.join(format!("{session}.jsonl")), &bytes).unwrap();
    bytes
}

#[test]
fn identities_are_session_scoped_and_order_is_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store.replace("one", vec![entry("z"), entry("a")]).unwrap();
    store.replace("two", vec![entry("z")]).unwrap();
    assert_eq!(store.entries("one").unwrap(), vec![entry("z"), entry("a")]);
    assert_eq!(store.entries("two").unwrap(), vec![entry("z")]);
    let revision = store.revision("one").unwrap().unwrap();
    store.append("one", vec![entry("c")]).unwrap();
    assert!(store.revision("one").unwrap().unwrap() > revision);
}

#[test]
fn manager_uses_sqlite_after_import_and_never_resurrects_legacy_files() {
    use future_agent::session::{Manager, SessionEntry};
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("sessions");
    let original = write_source(&legacy, "s", &[entry("e")]);
    {
        let manager = Manager::new(legacy.clone());
        manager.initialize().unwrap();
        manager
            .append_entries(
                "s",
                &[SessionEntry::new_user(
                    "user",
                    json!("new synthetic message"),
                )],
            )
            .unwrap();
        assert_eq!(manager.load("s").unwrap().entries.len(), 2);
        assert_eq!(std::fs::read(legacy.join("s.jsonl")).unwrap(), original);
        manager.delete("s").unwrap();
        assert!(legacy.join("s.jsonl").exists());
    }
    let manager = Manager::new(legacy.clone());
    assert!(!manager.contains("s").unwrap());
    let mut session = future_agent::session::Session::new("/synthetic", "mock");
    session
        .entries
        .push(SessionEntry::new_user("user", json!("new session")));
    manager.save(&session).unwrap();
    assert!(!legacy.join(format!("{}.jsonl", session.id)).exists());
}

#[test]
fn entry_and_event_conflict_rolls_back_entire_commit() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store.replace("s", vec![entry("first")]).unwrap();
    store.append_event("s", event("s", "r", 0)).unwrap();
    let mut conflict = event("s", "r", 0);
    conflict["data"] = json!("different");
    let revision = store.revision("s").unwrap();
    assert!(store
        .commit("s", vec![entry("second")], vec![conflict])
        .is_err());
    assert_eq!(store.entries("s").unwrap(), vec![entry("first")]);
    assert_eq!(store.revision("s").unwrap(), revision);
    let mut conflict = entry("first");
    conflict["content"] = json!("different");
    assert!(store.append("s", vec![entry("second"), conflict]).is_err());
    assert_eq!(store.entries("s").unwrap().len(), 1);
}

#[test]
fn events_resume_after_reopen_and_delete_cascades() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    {
        let store = SqliteStore::open(&path).unwrap();
        store.replace("s", vec![entry("e")]).unwrap();
        store.append_event("s", event("s", "r", 1)).unwrap();
        store.append_event("s", event("s", "r", 0)).unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    let events = store.events("s", "r").unwrap();
    assert_eq!(events[0]["idx"], 0);
    assert_eq!(events[1]["event_id"], "s:r:1:1");
    store.prune_events("s", "r").unwrap();
    assert!(store.events("s", "r").unwrap().is_empty());
    store.append_event("s", event("s", "r", 0)).unwrap();
    store.delete("s").unwrap();
    assert!(store.events("s", "r").unwrap().is_empty());
    assert!(store.append_event("s", event("s", "r", 2)).is_err());
    assert!(store.ids(false).unwrap().is_empty());
}

#[test]
fn migration_preserves_sources_payloads_and_deletion_tombstones() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("sessions");
    let original = write_source(&legacy, "s", &[entry("e")]);
    let events = dir.path().join("run-events/s");
    let event_bytes = write_source(&events, "r", &[event("s", "r", 0)]);
    let path = dir.path().join("agent.db");
    {
        let store = SqliteStore::open(&path).unwrap();
        store.import_legacy(&legacy, None).unwrap();
        let wal = path.with_file_name("agent.db-wal");
        assert_eq!(
            std::fs::metadata(wal)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
            0
        );
        assert_eq!(store.entries("s").unwrap()[0], entry("e"));
        assert_eq!(store.events("s", "r").unwrap().len(), 1);
        store.delete("s").unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    store.import_legacy(&legacy, None).unwrap();
    assert!(store.ids(true).unwrap().is_empty());
    assert!(store.import_legacy(&legacy, Some("s")).is_err());
    assert_eq!(std::fs::read(legacy.join("s.jsonl")).unwrap(), original);
    assert_eq!(std::fs::read(events.join("r.jsonl")).unwrap(), event_bytes);
}

#[test]
fn broken_event_skips_whole_session_and_explicit_retry_recovers_it() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("sessions");
    write_source(&legacy, "good", &[entry("good-entry")]);
    write_source(&legacy, "bad", &[entry("bad-entry")]);
    let events = dir.path().join("run-events/bad");
    std::fs::create_dir_all(&events).unwrap();
    let source = events.join("r.jsonl");
    std::fs::write(&source, b"{synthetic-private-sentinel}\n").unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store.import_legacy(&legacy, None).unwrap();
    assert_eq!(store.ids(false).unwrap(), ["good"]);
    assert_eq!(store.ids(true).unwrap(), ["bad", "good"]);
    assert!(store.entries("bad").is_err());
    assert!(store.replace("bad", vec![entry("replacement")]).is_err());
    let report = serde_json::to_string(&store.import_records().unwrap()).unwrap();
    assert!(!report.contains("synthetic-private-sentinel"));
    assert!(report.contains("invalid_json"));
    write_source(&events, "r", &[event("bad", "r", 0)]);
    store.import_legacy(&legacy, None).unwrap();
    assert!(
        store.entries("bad").is_err(),
        "automatic startup does not retry"
    );
    store.import_legacy(&legacy, Some("bad")).unwrap();
    assert_eq!(store.entries("bad").unwrap().len(), 1);
    assert_eq!(store.events("bad", "r").unwrap().len(), 1);
}

#[test]
fn truncated_tail_is_recoverable_but_middle_corruption_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("sessions");
    let mut tail = write_source(&legacy, "tail", &[entry("e")]);
    tail.extend_from_slice(b"{\"id\":");
    std::fs::write(legacy.join("tail.jsonl"), &tail).unwrap();
    std::fs::write(
        legacy.join("middle.jsonl"),
        format!("{}\n{{broken}}\n{}\n", entry("a"), entry("b")),
    )
    .unwrap();
    std::fs::write(
        legacy.join("invalid-last.jsonl"),
        format!("{}\n{{\"type\":\"user\"}}", entry("a")),
    )
    .unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store.import_legacy(&legacy, None).unwrap();
    assert_eq!(store.ids(false).unwrap(), ["tail"]);
    assert_eq!(
        store
            .import_records()
            .unwrap()
            .iter()
            .find(|r| r.session_id == "tail")
            .unwrap()
            .warnings,
        1
    );
    assert_eq!(std::fs::read(legacy.join("tail.jsonl")).unwrap(), tail);
}

#[test]
fn conflicting_event_ids_are_session_failures_not_global_failures() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("sessions");
    write_source(&legacy, "bad", &[entry("e")]);
    write_source(&legacy, "good", &[entry("e")]);
    let mut first = event("bad", "r", 0);
    let mut second = event("bad", "r", 1);
    first["event_id"] = json!("same");
    second["event_id"] = json!("same");
    write_source(&dir.path().join("run-events/bad"), "r", &[first, second]);
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store.import_legacy(&legacy, None).unwrap();
    assert_eq!(store.ids(false).unwrap(), ["good"]);
    assert!(store.events("bad", "r").unwrap().is_empty());
}

#[test]
fn dangling_checkpoint_is_not_imported() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("sessions");
    let mut checkpoint = entry("cp");
    checkpoint["type"] = json!("compaction");
    checkpoint["content"] =
        json!({"schema_version":2,"covered_from_entry_id":"e","cutoff_entry_id":"missing"});
    write_source(&legacy, "s", &[entry("e"), checkpoint]);
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store.import_legacy(&legacy, None).unwrap();
    assert!(store.ids(false).unwrap().is_empty());
    assert_eq!(
        store.import_records().unwrap()[0].error_kind.as_deref(),
        Some("dangling_checkpoint")
    );
}

#[test]
fn malformed_source_paths_skip_their_session_without_blocking_others() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("sessions");
    write_source(&legacy, "good", &[entry("e")]);
    write_source(&legacy, "bad-events", &[entry("e")]);
    std::fs::create_dir_all(legacy.join("bad-transcript.jsonl")).unwrap();
    let events = dir.path().join("run-events/bad-events");
    std::fs::create_dir_all(events.parent().unwrap()).unwrap();
    std::fs::write(events, "not a directory").unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store.import_legacy(&legacy, None).unwrap();
    assert_eq!(store.ids(false).unwrap(), ["good"]);
    assert_eq!(store.ids(true).unwrap().len(), 3);
}

#[test]
fn concurrent_clones_share_one_ordered_writer() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store.replace("s", vec![entry("initial")]).unwrap();
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let store = store.clone();
            std::thread::spawn(move || {
                store
                    .append("s", vec![entry(&format!("entry-{i}"))])
                    .unwrap()
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(store.entries("s").unwrap().len(), 9);
}

#[test]
fn sqlite_failure_rolls_back_import_and_does_not_mark_it_complete() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("sessions");
    write_source(&legacy, "s", &[entry("e")]);
    write_source(&dir.path().join("run-events/s"), "r", &[event("s", "r", 0)]);
    let path = dir.path().join("agent.db");
    let store = SqliteStore::open(&path).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON run_events BEGIN SELECT RAISE(ABORT, 'synthetic write failure'); END;").unwrap();
    assert!(store.import_legacy(&legacy, None).is_err());
    assert!(store.ids(true).unwrap().is_empty());
    assert!(store.import_records().unwrap().is_empty());
    connection
        .execute_batch("DROP TRIGGER fail_import;")
        .unwrap();
    store.import_legacy(&legacy, None).unwrap();
    assert_eq!(store.entries("s").unwrap().len(), 1);
    assert_eq!(store.events("s", "r").unwrap().len(), 1);
}

#[test]
fn failed_terminal_commit_does_not_publish_a_successful_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = SqliteStore::open(&path).unwrap();
    let mut started = entry("started");
    started["type"] = json!("run_started");
    started["content"] = json!({"run_id":"r"});
    store.replace("s", vec![started]).unwrap();
    let mut terminal = entry("terminal");
    terminal["type"] = json!("run_terminal");
    terminal["content"] = json!({"run_id":"r","state":"completed"});
    let mut invalid = event("different-session", "r", 0);
    assert!(store
        .commit("s", vec![terminal.clone()], vec![invalid.clone()])
        .is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let state: String = connection
        .query_row(
            "SELECT status FROM runs WHERE session_id='s' AND run_id='r'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "running");
    invalid["session_id"] = json!("s");
    store.commit("s", vec![terminal], vec![invalid]).unwrap();
    let state: String = connection
        .query_row(
            "SELECT status FROM runs WHERE session_id='s' AND run_id='r'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "completed");
}

#[test]
fn current_metadata_and_per_run_usage_survive_rewrite_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = SqliteStore::open(&path).unwrap();
    let row = |id: &str, kind: &str, content: Value| json!({"id":id,"type":kind,"timestamp":"2026-01-01T00:00:00Z","content":content});
    let mut entries = vec![row(
        "info-0",
        "session_info",
        json!({"model":"old","tokens_in":0,"tokens_cache_r":0}),
    )];
    for (run, total, cache) in [("one", 100, 40), ("two", 180, 55)] {
        entries.push(row(
            &format!("start-{run}"),
            "run_started",
            json!({"run_id":run,"epoch":1,"run_sequence":if run=="one"{1}else{2}}),
        ));
        entries.push(row(
            &format!("info-{run}"),
            "session_info",
            json!({"model":"new","tokens_in":total,"tokens_cache_r":cache}),
        ));
        entries.push(row(
            &format!("end-{run}"),
            "run_terminal",
            json!({"run_id":run,"state":"completed","run_tokens":7,"run_duration_ms":10}),
        ));
    }
    store.replace("s", entries).unwrap();
    let snapshot = store.entries("s").unwrap();
    assert_eq!(
        snapshot
            .iter()
            .filter(|e| e["type"] == "session_info")
            .count(),
        1
    );
    assert_eq!(snapshot[0]["content"]["model"], "new");
    let usage: Vec<_> = snapshot
        .iter()
        .filter(|e| e["type"] == "run_terminal")
        .map(|e| {
            (
                e["content"]["input_tokens"].as_i64().unwrap(),
                e["content"]["cache_read_tokens"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(usage, vec![(100, 40), (80, 15)]);
    store.replace("s", snapshot.clone()).unwrap();
    drop(store);
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(store.entries("s").unwrap(), snapshot);
    let db = rusqlite::Connection::open(&path).unwrap();
    let usage:(i64,i64,i64,i64)=db.query_row("SELECT input_tokens,cache_read_tokens,output_tokens,run_sequence FROM runs WHERE session_id='s' AND run_id='two'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(usage, (80, 15, 7, 2));
}
