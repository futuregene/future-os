use super::*;

/// A dangling tool_call (assistant saved, tool never executed) is still
/// patched with a placeholder in memory — but the file stays untouched.
#[test]
fn load_repairs_dangling_tool_calls_in_memory_only() {
    let dir = std::env::temp_dir().join(format!(
        "future_test_dangling_{}",
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
        "dangling tool_call must get a placeholder"
    );
    assert_eq!(tool_entries[0].tool_call_id, "tc1");

    let on_disk = raw_lines(&manager, &session.id);
    assert_eq!(
        on_disk.lines().count(),
        2,
        "dangling repair must not be persisted to the session file"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Regression: after a crash between assistant(tool_calls) persist and tool
/// execution, a subsequent restart appends run markers + a new user message
/// ahead of the orphaned tool_calls.  `repair_dangling_tool_calls` must find
/// the orphaned assistant even when it is NOT the last entry, and insert
/// placeholder tool responses so the conversation stays API-valid.
#[test]
fn load_reattaches_late_real_tool_result_without_duplicate_placeholder() {
    let (dir, manager) = temp_manager("late-tool-result");
    let assistant = SessionEntry::new_assistant(
        serde_json::json!("reading"),
        vec![ToolCall {
            id: "late-call".into(),
            call_type: "function".into(),
            function: crate::types::ToolCallFn {
                name: "read".into(),
                arguments: serde_json::json!({}),
            },
        }],
    );
    let real = SessionEntry::new_tool("late-call", "real result");
    let real_id = real.id.clone();
    let session = Session::snapshot(
        "late-session".into(),
        "/tmp".into(),
        "mock".into(),
        "late".into(),
        String::new(),
        vec![
            SessionEntry::new_user("user", serde_json::json!("question")),
            SessionEntry::run_started("old-run", 1),
            assistant,
            SessionEntry::run_terminal("old-run", "cancelled", 0, 0, None),
            real,
        ],
    );
    manager.save(&session).unwrap();
    let loaded = manager.load("late-session").unwrap();
    let tools: Vec<_> = loaded
        .entries
        .iter()
        .filter(|e| e.entry_type == ENTRY_TYPE_TOOL)
        .collect();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].id, real_id);
    let a = loaded
        .entries
        .iter()
        .position(|e| e.entry_type == ENTRY_TYPE_ASSISTANT)
        .unwrap();
    assert_eq!(loaded.entries[a + 1].id, real_id);
    // The manager's SQLite store holds the database open; Windows refuses
    // to delete a directory that still has an open file inside it.
    drop(manager);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn repair_dangling_tool_calls_finds_orphan_after_restart() {
    let (dir, manager) = temp_manager("repair-orphan-restart");
    let session = Session::snapshot(
        "s-repair".to_string(),
        "/tmp".to_string(),
        "m".to_string(),
        "n".to_string(),
        String::new(),
        vec![SessionEntry::session_info(
            serde_json::json!({"cwd": "/tmp", "model": "m"}),
            "m".to_string(),
            "low".to_string(),
        )],
    );
    manager.save(&session).unwrap();

    // --- Simulate a crash mid-run ---
    // The assistant message with tool_calls was persisted, but the tool
    // results were NOT — the process died between save_callback and
    // on_tool_result.
    let tc = ToolCall {
        id: "tc1".to_string(),
        call_type: "function".to_string(),
        function: crate::types::ToolCallFn {
            name: "read".to_string(),
            arguments: serde_json::json!({"path": "/etc/hosts"}),
        },
    };
    manager
        .append_entries(
            "s-repair",
            &[
                SessionEntry::new_user("user", serde_json::json!("old question")),
                SessionEntry::run_started("old-run", 1),
                SessionEntry::new_assistant(serde_json::json!("let me read that"), vec![tc]),
            ],
        )
        .unwrap();

    // --- Simulate restart: new prompt arrives ---
    // append_run_start closes old-run (appends run_terminal: interrupted)
    // and appends the new user + run_started entries.  The orphaned
    // assistant(tool_calls) is now buried in the middle, NOT at the end.
    manager
        .append_run_start(
            "s-repair",
            SessionEntry::new_user("user", serde_json::json!("new question")),
            SessionEntry::run_started("new-run", 2),
        )
        .unwrap();

    // --- Load: repair_dangling_tool_calls should fire ---
    let loaded = manager.load("s-repair").unwrap();

    // The repair must have inserted a placeholder tool result for tc1.
    let tool_entries: Vec<_> = loaded
        .entries
        .iter()
        .filter(|e| e.entry_type == ENTRY_TYPE_TOOL)
        .collect();
    assert!(
        !tool_entries.is_empty(),
        "repair_dangling_tool_calls should insert placeholder tool entries"
    );
    let placeholder = &tool_entries[0];
    assert_eq!(placeholder.tool_call_id, "tc1");
    assert!(
        placeholder
            .content
            .as_ref()
            .and_then(|c| c.as_str())
            .is_some_and(|s| s.starts_with(TOOL_LOST_PLACEHOLDER_PREFIX)),
        "placeholder content should start with '{}', got: {:?}",
        TOOL_LOST_PLACEHOLDER_PREFIX,
        placeholder.content
    );

    // The placeholder must appear IMMEDIATELY after the orphaned
    // assistant, before the run_terminal marker — otherwise the API
    // would see a user/tool ordering violation.
    let assistant_idx = loaded
        .entries
        .iter()
        .position(|e| e.entry_type == ENTRY_TYPE_ASSISTANT && !e.tool_calls.is_empty())
        .unwrap();
    let tool_idx = loaded
        .entries
        .iter()
        .position(|e| e.entry_type == ENTRY_TYPE_TOOL)
        .unwrap();
    assert_eq!(
        tool_idx,
        assistant_idx + 1,
        "placeholder tool entry must immediately follow the orphaned assistant"
    );

    // --- Verify entries_to_agent_messages is API-valid ---
    // After repair, every assistant with tool_calls must have matching
    // tool entries.  We verify by converting to AgentMessage and checking
    // that each assistant's tool_call_ids all have tool responses.
    let msgs = entries_to_agent_messages(&loaded.entries, false);
    let mut pending: std::collections::HashSet<String> = std::collections::HashSet::new();
    for msg in &msgs {
        match msg.role.as_str() {
            "assistant" => {
                // An assistant with tool_calls must not appear while
                // there are still pending tool_call_ids.
                assert!(
                    pending.is_empty(),
                    "pending tool_call_ids ({:?}) before new assistant",
                    pending
                );
                for tc in msg.tool_calls() {
                    pending.insert(tc.id.clone());
                }
            }
            "tool" => {
                let tool_call_id = msg.tool_call_id();
                let removed = pending.remove(&tool_call_id);
                assert!(
                    removed,
                    "tool entry with tool_call_id={} has no matching \
                     assistant tool_call",
                    tool_call_id
                );
            }
            _ => {
                // A user/system message between tool_calls and their
                // responses is an API violation.
                assert!(
                    pending.is_empty(),
                    "pending tool_call_ids ({:?}) before non-tool message role={}",
                    pending,
                    msg.role
                );
            }
        }
    }
    assert!(
        pending.is_empty(),
        "unresolved tool_call_ids after message walk: {:?}",
        pending
    );

    let _ = std::fs::remove_dir_all(dir);
}
