use super::*;

#[test]
fn metadata_update_does_not_persist_dangling_tool_repair() {
    let dir = std::env::temp_dir().join(format!(
        "future_test_metadata_dangling_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = Manager::new(dir.clone());
    let mut session = Session::new("/tmp/test", "gpt-4o");
    session.entries.push(SessionEntry::session_info(
        serde_json::json!({"model": "gpt-4o"}),
        "gpt-4o".to_string(),
        String::new(),
    ));
    session.entries.push(SessionEntry::new_assistant(
        serde_json::json!("running tool"),
        vec![crate::types::ToolCall {
            id: "tc1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "shell".to_string(),
                arguments: serde_json::json!({"cmd": "ls"}),
            },
        }],
    ));
    manager.save(&session).unwrap();

    manager
        .update_session_info(
            &session.id,
            "session_name",
            serde_json::json!("renamed while running"),
        )
        .unwrap();

    let on_disk = raw_lines(&manager, &session.id);
    let entries: Vec<SessionEntry> = on_disk
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    // The metadata update appends a fresh authoritative session_info rather
    // than rewriting through the repair pipeline, so no synthetic tool result
    // is persisted for the dangling tool_call.
    assert!(
        entries
            .iter()
            .all(|entry| entry.entry_type != ENTRY_TYPE_TOOL),
        "metadata update must not persist a synthetic tool result"
    );
    // The dangling assistant entry is left untouched (still exactly one).
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.entry_type == ENTRY_TYPE_ASSISTANT)
            .count(),
        1
    );
    // The authoritative (last) session_info carries the new name; the update
    // is append-only, so it lands after the original session_info.
    let last_info = entries
        .iter()
        .rev()
        .find(|entry| entry.entry_type == ENTRY_TYPE_SESSION_INFO)
        .expect("a session_info snapshot must exist");
    assert_eq!(
        last_info
            .content
            .as_ref()
            .and_then(|content| content.get("session_name")),
        Some(&serde_json::json!("renamed while running"))
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_reads_last_session_info_as_authoritative() {
    let (dir, manager) = temp_manager("last-info");
    let info_v0 = SessionEntry::session_info(
        serde_json::json!({"cwd": "/a", "model": "old", "session_name": "first", "parent_session_id": ""}),
        "old".to_string(),
        "low".to_string(),
    );
    let session = Session::snapshot(
        "s-last".to_string(),
        "/a".to_string(),
        "old".to_string(),
        "first".to_string(),
        String::new(),
        vec![
            info_v0,
            SessionEntry::new_user("user", serde_json::json!("hi")),
        ],
    );
    manager.save(&session).unwrap();
    // Append a newer authoritative session_info (as a run commit would).
    let info_v1 = SessionEntry::session_info(
        serde_json::json!({"cwd": "/a", "model": "new", "session_name": "renamed", "parent_session_id": ""}),
        "new".to_string(),
        "high".to_string(),
    );
    manager.append_entries("s-last", &[info_v1]).unwrap();

    let loaded = manager.load("s-last").unwrap();
    assert_eq!(loaded.model, "new");
    assert_eq!(loaded.name, "renamed");
    assert_eq!(loaded.get_session_info().unwrap()["model"], "new");
    let _ = std::fs::remove_dir_all(dir);
}

/// A session whose entries carry no `session_info` (every legacy-imported
/// transcript, since the importer never synthesises one) must still accept a
/// metadata update. The reader used to require an existing `session_info`
/// row and failed with a bare "Query returned no rows", so `future session
/// set --title/--thinking/--model` on an imported session reported success
/// while persisting nothing.
#[test]
fn metadata_update_works_without_an_existing_session_info_entry() {
    let (dir, manager) = temp_manager("update-info-no-snapshot");
    // Exactly the shape the legacy importer produces: conversation entries
    // only, no session_info.
    manager
        .storage()
        .unwrap()
        .replace(
            "s-imported",
            vec![serde_json::json!({
                "id": "e1", "type": "user", "role": "user",
                "timestamp": "2026-01-01T00:00:00Z", "content": "hi"
            })],
        )
        .unwrap();
    assert!(
        manager
            .load("s-imported")
            .unwrap()
            .get_session_info()
            .is_none(),
        "precondition: no session_info exists yet"
    );

    manager
        .update_session_info("s-imported", "session_name", serde_json::json!("Named"))
        .unwrap();

    let loaded = manager.load("s-imported").unwrap();
    assert_eq!(loaded.name, "Named");
    assert_eq!(loaded.get_session_info().unwrap()["session_name"], "Named");
    // The conversation itself must survive the metadata write.
    assert_eq!(loaded.entries.len(), 2, "user entry + appended snapshot");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn update_session_info_appends_complete_snapshot() {
    let (dir, manager) = temp_manager("update-info");
    let info = SessionEntry::session_info(
        serde_json::json!({"cwd": "/a", "model": "m1", "session_name": "n1", "tokens_in": 5}),
        "m1".to_string(),
        "low".to_string(),
    );
    let session = Session::snapshot(
        "s-upd".to_string(),
        "/a".to_string(),
        "m1".to_string(),
        "n1".to_string(),
        String::new(),
        vec![
            info,
            SessionEntry::new_user("user", serde_json::json!("hi")),
        ],
    );
    manager.save(&session).unwrap();

    // Update one field; the appended snapshot must remain complete (other
    // fields are merged over the latest session_info, not lost).
    manager
        .update_session_info("s-upd", "model", serde_json::json!("m2"))
        .unwrap();

    let loaded = manager.load("s-upd").unwrap();
    let info = loaded.get_session_info().unwrap();
    assert_eq!(info["model"], "m2");
    assert_eq!(info["session_name"], "n1");
    assert_eq!(info["tokens_in"], 5);
    // A new session_info was appended (append-only), not rewritten in place.
    let info_count = loaded
        .entries
        .iter()
        .filter(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO)
        .count();
    assert_eq!(info_count, 1);
    let _ = std::fs::remove_dir_all(dir);
}

/// `update_session_info_fields` reads the current metadata and then writes,
/// so it must take the write lock at `BEGIN`. A deferred transaction keeps
/// the read snapshot it opened while a *different* connection to the same
/// agent.db (every live session owns one) commits; SQLite then fails the
/// upgrade immediately with SQLITE_BUSY_SNAPSHOT — "database is locked" —
/// bypassing both `busy_timeout` and the retry ladder. That is the error the
/// startup metadata sync records while it is only logged by its caller, and
/// the session then rejects every prompt with "session persistence is
/// unavailable: database is locked" until the agent restarts.
#[test]
fn metadata_update_survives_a_concurrent_commit_on_another_connection() {
    let (dir, manager) = temp_manager("update-info-race");
    let info = SessionEntry::session_info(
        serde_json::json!({"cwd": "/a", "model": "m1", "session_name": "n1"}),
        "m1".to_string(),
        "low".to_string(),
    );
    let session = Session::snapshot(
        "s-race".to_string(),
        "/a".to_string(),
        "m1".to_string(),
        "n1".to_string(),
        String::new(),
        vec![
            info,
            SessionEntry::new_user("user", serde_json::json!("hi")),
        ],
    );
    manager.save(&session).unwrap();

    // Hold the write lock from a second connection and release it only after
    // the update has read its snapshot, so a deferred read-then-write loses
    // the upgrade while the update below waits for the lock at `BEGIN`.
    let blocker = rusqlite::Connection::open(manager.database_path()).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        blocker.execute_batch("COMMIT").unwrap();
    });
    let updated = manager.update_session_info("s-race", "model", serde_json::json!("m2"));
    release.join().unwrap();
    updated.expect("a concurrent commit must not lose the metadata update");

    let loaded = manager.load("s-race").unwrap();
    assert_eq!(loaded.get_session_info().unwrap()["model"], "m2");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn load_reads_model_from_model_change_entries() {
    let (_dir, manager) = temp_manager("load-model-change");
    let snapshot = Session::snapshot(
        "s1".to_string(),
        "/tmp".to_string(),
        String::new(),
        String::new(),
        String::new(),
        vec![
            SessionEntry::new_user("user", serde_json::json!("hi")),
            SessionEntry {
                id: generate_id(),
                entry_type: ENTRY_TYPE_MODEL_CHANGE.to_string(),
                role: ENTRY_TYPE_SYSTEM.to_string(),
                content: Some(serde_json::json!({"model": "deepseek/deepseek-chat"})),
                tool_calls: vec![],
                timestamp: Local::now(),
                tool_call_id: String::new(),
                name: String::new(),
                tool_args: String::new(),
                thinking: String::new(),
                meta: None,
            },
        ],
    );
    manager.save(&snapshot).unwrap();
    let loaded = manager.load("s1").unwrap();
    assert_eq!(loaded.model, "deepseek/deepseek-chat");
}

/// A metadata update for a session whose row is gone: the append would fail on a
/// foreign key, so the store names the real cause instead. This is the state a
/// caller reaches after deleting a session and then using its id from a stale
/// handle.
#[test]
fn a_metadata_update_for_a_missing_session_row_names_that_cause() {
    let (_dir, manager) = temp_manager("update-info-missing-row");
    // Entries exist (so the transcript is not the problem) but the `sessions`
    // row does not: delete the session and keep the entries.
    manager
        .storage()
        .unwrap()
        .replace(
            "s-gone",
            vec![serde_json::json!({
                "id": "e1", "type": "user", "role": "user",
                "timestamp": "2026-01-01T00:00:00Z", "content": "hi"
            })],
        )
        .unwrap();
    manager.test_execute("DELETE FROM sessions WHERE id='s-gone'");

    let mut fields = serde_json::Map::new();
    fields.insert("session_name".to_string(), serde_json::json!("renamed"));
    let error = manager
        .update_session_info_fields("s-gone", fields)
        .expect_err("a missing row cannot receive an entry");
    let message = format!("{error:#}");
    assert!(
        message.contains("session not found: no row for s-gone"),
        "the real cause is named instead of a foreign-key violation: {message}"
    );
}
