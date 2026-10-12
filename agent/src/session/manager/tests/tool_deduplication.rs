use super::*;

/// Regression test for the HTTP 400 "Messages with role 'tool' must be a
/// response to a preceding message with 'tool_calls'" failure seen when
/// resuming a session: a load-time repair previously PERSISTED a "tool
/// execution lost" placeholder while the owning agent was still mid-tool;
/// the real tool result was appended afterwards, leaving two tool entries
/// with the same tool_call_id.  Load must now heal this in memory (keep
/// the real result, drop the placeholder) without touching the file.
#[test]
fn load_dedupes_tool_results_preferring_real_over_placeholder() {
    let dir = std::env::temp_dir().join(format!(
        "future_test_dedupe_{}",
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
    // Placeholder written by a stale repair, then the real result.
    session.entries.push(SessionEntry::new_tool(
        "tc1",
        "[Tool execution lost — shell was not executed before the session was interrupted]",
    ));
    session
        .entries
        .push(SessionEntry::new_tool("tc1", "real output"));
    manager.save(&session).unwrap();

    let loaded = manager.load(&session.id).unwrap();
    let tool_entries: Vec<_> = loaded
        .entries
        .iter()
        .filter(|e| e.entry_type == ENTRY_TYPE_TOOL)
        .collect();
    assert_eq!(
        tool_entries.len(),
        1,
        "duplicate tool entries must be deduped"
    );
    assert_eq!(
        tool_entries[0].content.as_ref().unwrap(),
        &serde_json::json!([{
            "type": "tool_result",
            "tool_call_id": "tc1",
            "content": "real output"
        }]),
        "the real result must win over the placeholder"
    );

    // The file on disk must NOT be rewritten by load: read-only callers
    // (session list, get_session_entries) can run while the owning agent
    // is mid-run, and persisting repairs is what created the duplicates.
    let on_disk = raw_lines(&manager, &session.id);
    assert_eq!(
        on_disk.lines().count(),
        4,
        "load must not persist healed entries back to the session file"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Two REAL tool results with the same tool_call_id: keep the first.
#[test]
fn load_dedupes_tool_results_keeping_first_real() {
    let dir = std::env::temp_dir().join(format!(
        "future_test_dedupe2_{}",
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
    session
        .entries
        .push(SessionEntry::new_tool("tc1", "first result"));
    session
        .entries
        .push(SessionEntry::new_tool("tc1", "second result"));
    manager.save(&session).unwrap();

    let loaded = manager.load(&session.id).unwrap();
    let tool_entries: Vec<_> = loaded
        .entries
        .iter()
        .filter(|e| e.entry_type == ENTRY_TYPE_TOOL)
        .collect();
    assert_eq!(tool_entries.len(), 1);
    assert_eq!(
        tool_entries[0].content.as_ref().unwrap(),
        &serde_json::json!([{
            "type": "tool_result",
            "tool_call_id": "tc1",
            "content": "first result"
        }])
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dedupe_tool_entries_drops_placeholder_when_real_result_arrives() {
    let (_dir, manager) = temp_manager("dedupe");
    let placeholder = SessionEntry::new_tool(
        "call-1",
        "[Tool execution lost — worker crashed before the result was written]",
    );
    let real = SessionEntry::new_tool("call-1", "real output");
    let snapshot = Session::snapshot(
        "s1".to_string(),
        "/tmp".to_string(),
        "mock".to_string(),
        String::new(),
        String::new(),
        vec![
            SessionEntry::new_user("user", serde_json::json!("hi")),
            placeholder,
            real,
        ],
    );
    manager.save(&snapshot).unwrap();
    let loaded = manager.load("s1").unwrap();
    let tool_entries: Vec<_> = loaded
        .entries
        .iter()
        .filter(|e| e.entry_type == ENTRY_TYPE_TOOL)
        .collect();
    assert_eq!(tool_entries.len(), 1);
    assert_eq!(
        tool_entries[0].content.as_ref().unwrap(),
        &serde_json::json!([{
            "type": "tool_result",
            "tool_call_id": "call-1",
            "content": "real output"
        }])
    );
}

#[test]
fn dedupe_placeholder_detects_array_content_form() {
    let (_dir, manager) = temp_manager("dedupe-array");
    let mut placeholder = SessionEntry::new_tool("call-1", "ignored");
    placeholder.content = Some(serde_json::json!([
        {"type": "text", "text": "[Tool execution lost — worker crashed]"}
    ]));
    let real = SessionEntry::new_tool("call-1", "real output");
    let snapshot = Session::snapshot(
        "s1".to_string(),
        "/tmp".to_string(),
        "mock".to_string(),
        String::new(),
        String::new(),
        vec![
            SessionEntry::new_user("user", serde_json::json!("hi")),
            placeholder,
            real,
        ],
    );
    manager.save(&snapshot).unwrap();
    let loaded = manager.load("s1").unwrap();
    let tools: Vec<_> = loaded
        .entries
        .iter()
        .filter(|e| e.entry_type == ENTRY_TYPE_TOOL)
        .collect();
    assert_eq!(tools.len(), 1);
    assert_eq!(
        tools[0].content.as_ref().unwrap(),
        &serde_json::json!([{
            "type": "tool_result",
            "tool_call_id": "call-1",
            "content": "real output"
        }])
    );
}
