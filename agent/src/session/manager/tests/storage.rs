use super::*;

#[test]
fn manager_save_and_load() {
    let dir = std::env::temp_dir().join(format!(
        "future_test_session_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = Manager::new(dir.clone());
    let mut session = Session::new("/tmp/test", "gpt-4o");
    // Add session_info entry (model/thinking_level are in content JSON)
    session.entries.push(SessionEntry::session_info(
        serde_json::json!({"session_name": "test", "cwd": "/tmp/test", "model": "gpt-4o", "thinking_level": "high"}),
        "gpt-4o".to_string(),
        "high".to_string(),
    ));
    session
        .entries
        .push(SessionEntry::new_user("user", serde_json::json!("hello")));
    manager.save(&session).unwrap();

    let loaded = manager.load(&session.id).unwrap();
    assert_eq!(loaded.id, session.id);
    assert_eq!(loaded.model, "gpt-4o");
    assert_eq!(loaded.entries.len(), 2);

    // Cleanup
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn manager_writes_only_canonical_message_blocks_and_rehydrates_projections() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().to_path_buf());
    let mut provider_metadata = crate::types::ProviderMetadata::new();
    provider_metadata.insert("anthropic".into(), serde_json::json!({"signature": "sig"}));
    let message = crate::types::AgentMessage {
        role: "assistant".into(),
        content: vec![
            crate::types::ContentBlock::reasoning("thought", provider_metadata),
            crate::types::ContentBlock::text("answer"),
            crate::types::ContentBlock::tool_call(
                "call-1",
                "lookup",
                serde_json::json!({"q": "rust"}),
                Default::default(),
            ),
        ],
        ..Default::default()
    };
    let mut session = Session::new("/tmp/test", "claude");
    session.entries.push(agent_message_to_entry(&message));
    manager.save(&session).unwrap();

    let disk = raw_lines(&manager, &session.id);
    let assistant: serde_json::Value = disk
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .find(|value: &serde_json::Value| value["type"] == "assistant")
        .unwrap();
    assert!(assistant.get("thinking").is_none());
    assert!(assistant.get("tool_calls").is_none());
    assert_eq!(assistant["content"][0]["type"], "reasoning");
    assert_eq!(
        assistant["content"][0]["provider_metadata"]["anthropic"]["signature"],
        "sig"
    );

    let loaded = manager.load(&session.id).unwrap();
    let assistant = loaded
        .entries
        .iter()
        .find(|entry| entry.entry_type == ENTRY_TYPE_ASSISTANT)
        .unwrap();
    assert_eq!(assistant.thinking, "thought");
    assert_eq!(assistant.tool_calls.len(), 1);
}

/// The real outcome must survive the storage round-trip: the history page
/// the phone renders reads the block's `is_error`, and a reloaded session
/// hands the same flag back to the model.
#[test]
fn stored_tool_result_is_error_round_trips_to_history_and_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().to_path_buf());
    let mut session = Session::new("/tmp/test", "claude");
    let assistant = crate::types::AgentMessage {
        role: "assistant".into(),
        content: vec![
            crate::types::ContentBlock::tool_call(
                "call-1",
                "shell",
                serde_json::json!({"command": "cargo build"}),
                Default::default(),
            ),
            crate::types::ContentBlock::tool_call(
                "call-2",
                "shell",
                serde_json::json!({"command": "ls"}),
                Default::default(),
            ),
        ],
        ..Default::default()
    };
    let failed = crate::types::AgentMessage {
        role: "tool".into(),
        content: vec![crate::types::ContentBlock::tool_result(
            "call-1",
            "error[E0308]: mismatched types\n[exit: 101]",
            true,
        )],
        name: "shell".into(),
        ..Default::default()
    };
    let succeeded = crate::types::AgentMessage {
        role: "tool".into(),
        content: vec![crate::types::ContentBlock::tool_result(
            "call-2", "file.txt", false,
        )],
        name: "shell".into(),
        ..Default::default()
    };
    session.entries.push(agent_message_to_entry(&assistant));
    session.entries.push(agent_message_to_entry(&failed));
    session.entries.push(agent_message_to_entry(&succeeded));
    manager.save(&session).unwrap();

    let disk = raw_lines(&manager, &session.id);
    let entries: Vec<serde_json::Value> = disk
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let tool = |id: &str| {
        entries
            .iter()
            .find(|value| value["type"] == "tool" && value["content"][0]["tool_call_id"] == id)
            .unwrap_or_else(|| panic!("no stored tool entry for {id}"))
    };
    assert_eq!(
        tool("call-1")["content"][0]["is_error"],
        serde_json::json!(true)
    );
    // A success stays unflagged (same reading as `false`).
    assert!(tool("call-2")["content"][0].get("is_error").is_none());

    let loaded = manager.load(&session.id).unwrap();
    let messages = entries_to_agent_messages(&loaded.entries, false);
    let flags: Vec<(&str, bool)> = messages
        .iter()
        .filter(|message| message.role == "tool")
        .map(|message| match &message.content[0] {
            crate::types::ContentBlock::ToolResult {
                tool_call_id,
                is_error,
                ..
            } => (tool_call_id.as_str(), *is_error),
            other => panic!("tool message without a tool_result block: {other:?}"),
        })
        .collect();
    assert_eq!(flags, vec![("call-1", true), ("call-2", false)]);
}

#[test]
fn manager_delete() {
    let dir = std::env::temp_dir().join(format!(
        "future_test_delete_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = Manager::new(dir.clone());
    let session = Session::new("/tmp/test", "model");
    manager.save(&session).unwrap();
    assert!(manager.contains(&session.id).unwrap());
    manager.delete(&session.id).unwrap();
    assert!(!manager.contains(&session.id).unwrap());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn manager_find_nonexistent() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().to_path_buf());
    assert!(!manager.contains("nonexistent_id").unwrap());
}

#[test]
fn append_entries_persists_and_loads() {
    let dir = std::env::temp_dir().join(format!(
        "future_test_append_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = Manager::new(dir.clone());
    let mut session = Session::new("/tmp/test", "gpt-4o");
    session
        .entries
        .push(SessionEntry::new_user("user", serde_json::json!("hello")));
    manager.save(&session).unwrap();

    // Append a second user entry
    let appended = vec![SessionEntry::new_assistant(
        serde_json::json!("hi there"),
        vec![],
    )];
    manager.append_entries(&session.id, &appended).unwrap();

    // Append to non-existent session should error
    let result = manager.append_entries("nonexistent", &appended);
    assert!(result.is_err());

    // Load and verify both entries are present
    let loaded = manager.load(&session.id).unwrap();
    assert_eq!(loaded.entries.len(), 2);
    assert_eq!(loaded.entries[0].role, "user");
    assert_eq!(loaded.entries[1].role, "assistant");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_legacy_jsonl_rehydrates_string_content_thinking_and_tool_calls() {
    let dir = std::env::temp_dir().join(format!(
        "future_test_legacy_jsonl_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let manager = Manager::new(dir.clone());
    std::fs::write(
        manager.dir.join("legacy.jsonl"),
        concat!(
            r#"{"id":"u","type":"user","role":"user","content":"plain user"}"#, "\n",
            r#"{"id":"a","type":"assistant","role":"assistant","content":"plain answer","thinking":"legacy reasoning","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":{"path":"/tmp/a"}}}]}"#, "\n",
            r#"{"id":"t","type":"tool","role":"tool","content":"legacy result","tool_call_id":"call_1"}"#, "\n",
        ),
    )
    .unwrap();

    let loaded = manager.load("legacy").unwrap();
    assert!(entry_text_starts_with(&loaded.entries[0], "plain"));
    let messages = entries_to_agent_messages(&loaded.entries, false);
    assert_eq!(messages[0].text(), "plain user");
    assert!(matches!(
        messages[1].content.first(),
        Some(crate::types::ContentBlock::Reasoning { text, .. }) if text == "legacy reasoning"
    ));
    assert!(matches!(
        messages[1].content.iter().find(|block| matches!(block, crate::types::ContentBlock::ToolCall { .. })),
        Some(crate::types::ContentBlock::ToolCall { id, name, args, .. })
            if id == "call_1" && name == "read" && args == &serde_json::json!({"path": "/tmp/a"})
    ));
    assert!(matches!(
        messages[2].content.as_slice(),
        [crate::types::ContentBlock::ToolResult { tool_call_id, content, .. }]
            if tool_call_id == "call_1" && content == "legacy result"
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn load_rejects_session_with_only_blank_lines() {
    let (_dir, manager) = temp_manager("blank-only");
    std::fs::create_dir_all(&manager.dir).unwrap();
    std::fs::write(manager.dir.join("blank.jsonl"), "\n\n  \n").unwrap();
    let error = manager.load("blank").unwrap_err();
    assert!(error.to_string().contains("has no entries"), "{error}");
}

#[test]
fn delete_without_run_data_is_ok() {
    let (_dir, manager) = temp_manager("delete-no-run-data");
    let session = Session::snapshot(
        "del".to_string(),
        "/x".to_string(),
        "m".to_string(),
        "n".to_string(),
        String::new(),
        vec![SessionEntry::new_user("user", serde_json::json!("hi"))],
    );
    manager.save(&session).unwrap();
    manager.delete("del").unwrap();
    assert!(!manager.contains("del").unwrap());
}

/// Opening storage for the first time is the slow path on shared network
/// storage, so it announces the database it is opening. The progress line is
/// only evaluated when a subscriber has the level enabled, which is what the
/// CLI and the desktop app do.
#[test]
fn opening_storage_reports_the_database_path() {
    let _sink = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_writer(std::io::sink)
            .with_ansi(false)
            .finish(),
    );
    let dir = std::env::temp_dir().join(format!(
        "future_test_storage_open_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = Manager::new(dir.clone());
    assert!(!manager.database_path().exists());
    let _store = manager.storage().unwrap();
    assert!(
        manager.database_path().exists(),
        "the first read must have opened the database"
    );
}
