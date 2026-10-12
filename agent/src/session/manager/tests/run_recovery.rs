use super::*;

#[test]
fn append_only_run_journal_roundtrips() {
    // Simulate a full append-only run lifecycle on disk and verify the
    // journal loads cleanly: the model context excludes markers, the
    // authoritative session_info is the last one, and the run boundary is
    // recorded by a started + terminal marker pair.
    let (dir, manager) = temp_manager("journal");
    let info = SessionEntry::session_info(
        serde_json::json!({"cwd": "/a", "model": "m", "session_name": "n", "tokens_out": 0}),
        "m".to_string(),
        "low".to_string(),
    );
    let session = Session::snapshot(
        "s-journal".to_string(),
        "/a".to_string(),
        "m".to_string(),
        "n".to_string(),
        String::new(),
        vec![info],
    );
    manager.save(&session).unwrap();

    // Run 1: user + run_started appended at accept; assistant + run_terminal
    // + refreshed session_info appended at commit.
    manager
        .append_entries(
            "s-journal",
            &[
                SessionEntry::new_user("user", serde_json::json!("q1")),
                SessionEntry::run_started("run-1", 1),
            ],
        )
        .unwrap();
    let commit_info = SessionEntry::session_info(
        serde_json::json!({"cwd": "/a", "model": "m", "session_name": "n", "tokens_out": 100}),
        "m".to_string(),
        "low".to_string(),
    );
    manager
        .append_entries(
            "s-journal",
            &[
                SessionEntry::new_assistant(serde_json::json!("a1"), vec![]),
                SessionEntry::run_terminal("run-1", RUN_STATE_COMPLETED, 100, 500, None),
                commit_info,
            ],
        )
        .unwrap();

    let loaded = manager.load("s-journal").unwrap();
    // Authoritative metadata is the last (commit) snapshot.
    assert_eq!(loaded.get_session_info().unwrap()["tokens_out"], 100);
    // Model context is exactly the conversation, no markers.
    let msgs = entries_to_agent_messages(&loaded.entries, false);
    assert_eq!(msgs.len(), 2);
    // The run boundary is recoverable from the markers.
    let started: Vec<_> = loaded
        .entries
        .iter()
        .filter(|e| e.entry_type == ENTRY_TYPE_RUN_STARTED)
        .collect();
    let terminal: Vec<_> = loaded
        .entries
        .iter()
        .filter(|e| e.entry_type == ENTRY_TYPE_RUN_TERMINAL)
        .collect();
    assert_eq!(started.len(), 1);
    assert_eq!(terminal.len(), 1);
    assert_eq!(started[0].content.as_ref().unwrap()["run_id"], "run-1");
    assert_eq!(terminal[0].content.as_ref().unwrap()["state"], "completed");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unterminated_run_id_scans_only_markers_from_disk() {
    let (dir, manager) = temp_manager("unterminated-scan");
    let info = SessionEntry::session_info(
        serde_json::json!({"cwd": "/a", "model": "m", "session_name": "n"}),
        "m".to_string(),
        "low".to_string(),
    );
    let session = Session::snapshot(
        "s-scan".to_string(),
        "/a".to_string(),
        "m".to_string(),
        "n".to_string(),
        String::new(),
        vec![info],
    );
    manager.save(&session).unwrap();

    // No markers yet.
    assert_eq!(manager.unterminated_run_id("s-scan").unwrap(), None);
    // Absent file → None, not an error.
    assert_eq!(manager.unterminated_run_id("does-not-exist").unwrap(), None);

    // A completed run leaves nothing open.
    manager
        .append_entries(
            "s-scan",
            &[
                SessionEntry::new_user("user", serde_json::json!("q1")),
                SessionEntry::run_started("run-1", 1),
                SessionEntry::new_assistant(serde_json::json!("a1"), vec![]),
                SessionEntry::run_terminal("run-1", RUN_STATE_COMPLETED, 5, 50, None),
            ],
        )
        .unwrap();
    assert_eq!(manager.unterminated_run_id("s-scan").unwrap(), None);

    // An interrupted run (started, no terminal) is detected — even with a
    // large assistant line in between, which the cheap scan must skip.
    let big = "x".repeat(10_000);
    manager
        .append_entries(
            "s-scan",
            &[
                SessionEntry::new_user("user", serde_json::json!("q2")),
                SessionEntry::run_started("run-2", 2),
                SessionEntry::new_assistant(serde_json::json!(big), vec![]),
            ],
        )
        .unwrap();
    assert_eq!(
        manager.unterminated_run_id("s-scan").unwrap(),
        Some("run-2".to_string())
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn append_run_start_closes_previous_run_in_same_boundary() {
    let (dir, manager) = temp_manager("atomic-run-start");
    let session = Session::snapshot(
        "s-atomic".to_string(),
        "/a".to_string(),
        "m".to_string(),
        "n".to_string(),
        String::new(),
        vec![SessionEntry::session_info(
            serde_json::json!({"cwd": "/a", "model": "m"}),
            "m".to_string(),
            "low".to_string(),
        )],
    );
    manager.save(&session).unwrap();
    manager
        .append_entries(
            "s-atomic",
            &[
                SessionEntry::new_user("user", serde_json::json!("old")),
                SessionEntry::run_started("old-run", 1),
            ],
        )
        .unwrap();

    manager
        .append_run_start(
            "s-atomic",
            SessionEntry::new_user("user", serde_json::json!("new")),
            SessionEntry::run_started("new-run", 2),
        )
        .unwrap();

    let loaded = manager.load("s-atomic").unwrap();
    assert_eq!(
        crate::session::find_run_terminal(&loaded.entries, "old-run")
            .and_then(|value| value.get("state").cloned()),
        Some(serde_json::json!(RUN_STATE_INTERRUPTED_BY_RESTART))
    );
    assert_eq!(
        crate::session::find_unterminated_run(&loaded.entries),
        Some("new-run".to_string())
    );
    let old_terminal_index = loaded
        .entries
        .iter()
        .position(|entry| {
            entry.entry_type == ENTRY_TYPE_RUN_TERMINAL
                && entry.content.as_ref().and_then(|value| value.get("run_id"))
                    == Some(&serde_json::json!("old-run"))
        })
        .unwrap();
    let new_user_index = loaded
        .entries
        .iter()
        .position(|entry| entry_text_starts_with(entry, "new"))
        .unwrap();
    assert!(old_terminal_index < new_user_index);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn append_run_start_closes_an_unterminated_run() {
    let (_dir, manager) = temp_manager("runstart-open");
    let snapshot = Session::snapshot(
        "s1".to_string(),
        "/tmp".to_string(),
        "mock".to_string(),
        String::new(),
        String::new(),
        vec![
            SessionEntry::new_user("user", serde_json::json!("first")),
            SessionEntry::run_started("run-open", 1),
        ],
    );
    manager.save(&snapshot).unwrap();

    manager
        .append_run_start(
            "s1",
            SessionEntry::new_user("user", serde_json::json!("second")),
            SessionEntry::run_started("run-new", 2),
        )
        .unwrap();
    let loaded = manager.load("s1").unwrap();
    // The open run was closed with an interrupted terminal marker.
    let healed = loaded
        .entries
        .iter()
        .find(|e| {
            e.entry_type == ENTRY_TYPE_RUN_TERMINAL
                && e.content
                    .as_ref()
                    .and_then(|c| c.get("run_id"))
                    .and_then(|v| v.as_str())
                    == Some("run-open")
        })
        .expect("interrupted terminal appended");
    assert_eq!(
        healed.content.as_ref().unwrap()["state"],
        RUN_STATE_INTERRUPTED_BY_RESTART
    );
    // …followed by the new user message and the new run's start marker.
    let last = loaded.entries.last().unwrap();
    assert_eq!(last.entry_type, ENTRY_TYPE_RUN_STARTED);
}

#[test]
fn append_run_start_on_clean_history_appends_two_entries() {
    let (_dir, manager) = temp_manager("runstart-clean");
    let snapshot = Session::snapshot(
        "s1".to_string(),
        "/tmp".to_string(),
        "mock".to_string(),
        String::new(),
        String::new(),
        vec![
            SessionEntry::new_user("user", serde_json::json!("first")),
            SessionEntry::run_started("run-a", 1),
            SessionEntry::run_terminal("run-a", RUN_STATE_COMPLETED, 1, 1, None),
        ],
    );
    manager.save(&snapshot).unwrap();
    let before = manager.load("s1").unwrap().entries.len();
    manager
        .append_run_start(
            "s1",
            SessionEntry::new_user("user", serde_json::json!("second")),
            SessionEntry::run_started("run-b", 2),
        )
        .unwrap();
    let after = manager.load("s1").unwrap().entries.len();
    assert_eq!(after - before, 2, "no healing entry needed");
}
