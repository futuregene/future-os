//! Tests for the session lifecycle command handlers.

use crate::test_support::TestHome;

use std::sync::Arc;

use crate::rpc::{ServerSession, SseBroadcaster};
use crate::{agent::Loop, rpc::ApprovalGate};

use crate::rpc::commands::session_lifecycle::MODEL_SYNC_FAIL_HOOK;
use crate::rpc::commands::test_support::*;
use crate::rpc::handle_command_internal;

#[test]
fn delete_session_fences_admission_and_reclaims_queued_snapshots() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session
        .read()
        .runtime
        .begin(Some("run-active"), Some("request-active"))
        .unwrap();

    let mut queued = make_cmd("prompt");
    queued.message = "must be reclaimed".to_string();
    queued.busy_policy = "enqueue_if_busy".to_string();
    queued.requested_run_id = "run-queued".to_string();
    queued.client_request_id = "request-queued".to_string();
    assert_eq!(
        parse_response(&handle_command_internal(&state, queued))["success"],
        true
    );
    assert!(session
        .read()
        .scheduled_setting_summary("run-queued")
        .is_some());

    let deleting = parse_response(&handle_command_internal(&state, make_cmd("delete_session")));
    assert_eq!(deleting["success"], false);
    assert_eq!(deleting["error_code"], "deleting");
    assert_eq!(deleting["error_data"]["queued_cancelled"], 1);
    assert!(session.read().deleting);
    assert!(session.read().scheduler.queued().is_empty());
    assert!(session
        .read()
        .scheduled_setting_summary("run-queued")
        .is_none());

    let mut rejected = make_cmd("prompt");
    rejected.message = "too late".to_string();
    rejected.client_request_id = "request-too-late".to_string();
    let rejected = parse_response(&handle_command_internal(&state, rejected));
    assert_eq!(rejected["success"], false);
    assert_eq!(rejected["error_code"], "deleting");
}

#[test]
fn delete_idle_session_removes_the_live_runtime() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    assert!(state.sessions.read().contains_key("default"));

    let response = parse_response(&handle_command_internal(&state, make_cmd("delete_session")));

    assert_eq!(response["success"], true);
    assert_eq!(response["data"]["deleted"], true);
    assert!(!state.sessions.read().contains_key("default"));
    assert!(session
        .read()
        .persistence
        .append(vec![crate::session::SessionEntry::new_assistant(
            serde_json::json!("late write"),
            vec![],
        )])
        .is_err());
}

#[test]
fn delete_reclaims_taskless_persistence_degraded_session() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    let lease = session
        .read()
        .runtime
        .begin(Some("run-degraded"), Some("request-degraded"))
        .unwrap();
    assert!(session
        .read()
        .runtime
        .mark_persistence_degraded(&lease, "disk full"));
    assert!(!session.read().runtime.has_owned_task());

    let response = parse_response(&handle_command_internal(&state, make_cmd("delete_session")));

    assert_eq!(response["success"], true);
    assert_eq!(response["data"]["deleted"], true);
    assert!(!state.sessions.read().contains_key("default"));
}

#[test]
fn shutdown_sets_flag() {
    let state = make_app_state();
    let cmd = make_cmd("shutdown");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert!(state
        .shutting_down
        .load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn create_session_rebinds_event_journal_to_live_broadcaster() {
    let state = make_app_state();
    let session_id = "journal-rebind".to_string();

    // Mimic fork/clone: construct with a broadcaster that create_session
    // will discard, so the live one must be (re)configured by it.
    let new_sess = ServerSession::new_with_queue_budget(
        session_id.clone(),
        Arc::new(tokio::sync::RwLock::new(Loop::new(
            Arc::new(EmptyProvider),
            "mock",
        ))),
        state.session_manager.clone(),
        &test_workspace(),
        Arc::new(SseBroadcaster::new()),
        ApprovalGate::default(),
        state.model_registry.clone(),
        state.queue_budget.clone(),
    );
    state.create_session(new_sess);

    let session_arc = {
        let sessions = state.sessions.read();
        sessions.get(&session_id).unwrap().clone()
    };
    let live_broadcaster = session_arc.read().broadcaster.clone();
    live_broadcaster.start_run("run-j".to_string(), 1);
    live_broadcaster.broadcast(crate::rpc::SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": "hello"}),
    ));

    let journal = state
        .session_manager
        .run_data_path(&session_id)
        .join("run-j.jsonl");
    assert!(
        journal.exists(),
        "live broadcaster must write the durable event journal"
    );
}

#[test]
fn list_sessions_returns_array() {
    let state = make_app_state();
    let cmd = make_cmd("list_sessions");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert!(resp["data"]["sessions"].is_array());
}

#[test]
fn list_session_ids_reports_all_files_including_corrupt() {
    let state = make_app_state();
    // Persist one real session.
    let mut session = crate::session::Session::new("/tmp", "mock");
    session
        .entries
        .push(crate::session::SessionEntry::session_info(
            serde_json::json!({"cwd": "/tmp", "model": "mock"}),
            "mock".to_string(),
            "low".to_string(),
        ));
    state.session_manager.save(&session).unwrap();
    // Drop a corrupt JSONL next to it — must STILL be reported as a live
    // session id (orphan cleanup depends on filename-only enumeration).
    let corrupt_id = "corrupt-session";
    std::fs::write(
        state
            .session_manager
            .dir
            .join(format!("{corrupt_id}.jsonl")),
        "{ not json",
    )
    .unwrap();

    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("list_session_ids"),
    ));
    assert_eq!(resp["success"], true);
    let mut ids: Vec<String> = resp["data"]["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    ids.sort();
    assert_eq!(ids, vec![session.id.clone(), corrupt_id.to_string()]);
}

/// Audit item 1 contract: list_sessions rows carry canonical camelCase
/// keys AND the legacy snake_case spellings, with identical values, so
/// pre-migration clients keep working.
#[test]
fn list_sessions_emits_canonical_keys() {
    let state = make_app_state();
    // list_sessions reads persisted session summaries, so persist one —
    // the summary fields come from the session_info entry.
    let mut session = crate::session::Session::new("/tmp", "mock");
    session.name = "My session".to_string();
    session
        .entries
        .push(crate::session::SessionEntry::session_info(
            serde_json::json!({"cwd": "/tmp", "model": "mock", "session_name": "My session"}),
            "mock".to_string(),
            "low".to_string(),
        ));
    session.entries.push(crate::session::SessionEntry::new_user(
        "user",
        serde_json::json!("hello"),
    ));
    state.session_manager.save(&session).unwrap();

    let resp = parse_response(&handle_command_internal(&state, make_cmd("list_sessions")));
    assert_eq!(resp["success"], true);
    let sessions = resp["data"]["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1);
    let entry = &sessions[0];
    assert!(!entry["id"].as_str().unwrap().is_empty());
    assert_eq!(entry["sessionName"], "My session");
    assert_eq!(entry["firstMessage"], "hello");
    for canonical in [
        "sessionName",
        "updatedAt",
        "parentSessionId",
        "firstMessage",
        "queryCount",
        "isStreaming",
    ] {
        assert!(entry.get(canonical).is_some(), "missing `{canonical}`");
    }
    // Canonical only — no legacy snake_case aliases.
    for legacy in [
        "session_name",
        "updated_at",
        "parent_session_id",
        "first_message",
        "query_count",
        "is_streaming",
    ] {
        assert!(
            entry.get(legacy).is_none(),
            "legacy `{legacy}` must be absent"
        );
    }
}

/// Audit item 1 contract: get_state carries canonical camelCase keys; the
/// one key whose spelling changed (`sessionName`) is additionally emitted
/// under its legacy `session_name` name for pre-migration clients.
#[test]
fn list_streaming_sessions_reports_only_streaming() {
    let state = make_app_state();
    let cmd = make_cmd("list_streaming_sessions");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(
        resp["data"]["sessionIds"].as_array().unwrap().len(),
        0,
        "nothing streams at startup"
    );

    state.sessions.read()["default"]
        .read()
        .is_streaming
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("list_streaming_sessions"),
    ));
    let ids = resp["data"]["sessionIds"].as_array().unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0], "default");
}

#[test]
fn switch_session_validates_and_succeeds() {
    let state = make_app_state();

    let cmd = make_cmd_for("switch_session", "");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("No session selected"));

    let cmd = make_cmd_for("switch_session", "ghost");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("not found"));

    let cmd = make_cmd_for("switch_session", "default");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["cancelled"], false);
}

#[test]
fn delete_session_requires_session_id() {
    let state = make_app_state();
    let cmd = make_cmd_for("delete_session", "");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("No session selected to delete"));
}

#[test]
fn delete_session_reports_unremovable_disk_file() {
    let state = make_app_state();
    save_via(
        &state,
        "ghost",
        "mock",
        vec![crate::session::SessionEntry::new_user(
            "user",
            serde_json::json!("x"),
        )],
    );
    // Replace the JSONL file with a directory so remove_file fails.
    let path = state.session_manager.find("ghost").expect("saved session");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir_all(&path).unwrap();

    let cmd = make_cmd_for("delete_session", "ghost");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "delete_failed");
    let _ = std::fs::remove_dir_all(&path);
}

// ── coverage batch 1: get_fork_messages ─────────────────────────────────

#[test]
fn get_fork_messages_unknown_session_returns_empty() {
    let state = make_app_state();
    let cmd = make_cmd_for("get_fork_messages", "ghost");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["messages"], serde_json::json!([]));
}

#[test]
fn get_fork_messages_extracts_first_text_block_only() {
    let state = make_app_state();
    let user_plain = crate::session::SessionEntry::new_user("user", serde_json::json!("plain"));
    let user_blocks = crate::session::SessionEntry::new_user(
        "user",
        serde_json::json!([
            {"type": "text", "text": "visible question"},
            {"type": "text", "text": "agent-injected attachment list"},
        ]),
    );
    let assistant =
        crate::session::SessionEntry::new_assistant(serde_json::json!("answer"), vec![]);
    save_via(
        &state,
        "fork-src",
        "mock",
        vec![user_plain, user_blocks, assistant],
    );

    let cmd = make_cmd_for("get_fork_messages", "fork-src");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let messages = resp["data"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2, "only user entries are fork points");
    assert_eq!(messages[0]["content"], "plain");
    assert_eq!(messages[1]["content"], "visible question");
    assert!(messages[0]["timestamp"].is_string());
}

#[test]
fn get_fork_messages_handles_legacy_bare_string_content() {
    let state = make_app_state();
    // A pre-block-array journal stored user content as a bare string. The
    // save path canonicalizes string content to a block array, so this legacy
    // shape is written directly to disk to exercise the fallback branch.
    std::fs::create_dir_all(&state.session_manager.dir).unwrap();
    std::fs::write(
        state.session_manager.dir.join("legacy-src.jsonl"),
        r#"{"id":"legacy-user-1","type":"user","role":"user","content":"plain legacy text","timestamp":"2024-01-01T00:00:00Z"}"#,
    )
    .unwrap();

    let cmd = make_cmd_for("get_fork_messages", "legacy-src");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let messages = resp["data"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "plain legacy text");
}

// ── coverage batch 1: new_session variants ──────────────────────────────

#[test]
fn new_session_generates_id_and_registers() {
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("new_session")));
    assert_eq!(resp["success"], true);
    let new_id = resp["data"]["sessionId"].as_str().unwrap();
    assert!(!new_id.is_empty());
    assert!(state.get_session(new_id).is_some());
}

#[test]
fn new_session_honors_explicit_id_cwd_model_level_and_provenance() {
    let state = make_app_state();
    let mut cmd = make_cmd_for("new_session", "ns-explicit");
    cmd.cwd = "/tmp/some-workspace/ ".to_string();
    cmd.model_id = "explicit/model".to_string();
    cmd.level = "low".to_string();
    cmd.created_by = "gui".to_string();
    cmd.source_meta = "{\"thread\":\"t1\"}".to_string();
    cmd.parent_session = "parent-1".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["sessionId"], "ns-explicit");
    let session = state.get_session("ns-explicit").unwrap();
    let sess = session.read();
    assert_eq!(sess.created_by, "gui");
    assert_eq!(sess.source_meta, serde_json::json!({"thread": "t1"}));
    assert_eq!(sess.parent_session_id, "parent-1");
    assert_eq!(sess.model, "explicit/model");
    assert_eq!(sess.thinking_level, "low");
    assert_eq!(sess.cwd, "/tmp/some-workspace");
}

#[test]
fn new_session_accepts_initial_human_readable_title() {
    let state = make_app_state();
    let mut cmd = make_cmd_for("new_session", "ns-titled");
    cmd.name = "把 readme 更新成中文".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let session = state.get_session("ns-titled").unwrap();
    assert_eq!(session.read().session_name(), "把 readme 更新成中文");

    // No name → default empty (clients derive a display name later).
    let cmd2 = make_cmd_for("new_session", "ns-untitled");
    let resp2 = parse_response(&handle_command_internal(&state, cmd2));
    assert_eq!(resp2["success"], true);
    let sess2 = state.get_session("ns-untitled").unwrap();
    assert_eq!(sess2.read().session_name(), "");
}

#[test]
fn new_session_legacy_provenance_via_custom_instructions() {
    let state = make_app_state();
    let mut cmd = make_cmd_for("new_session", "ns-legacy");
    cmd.custom_instructions = r#"{"createdBy":"mobile","sourceMeta":{"chat":"c1"}}"#.to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let session = state.get_session("ns-legacy").unwrap();
    let sess = session.read();
    assert_eq!(sess.created_by, "mobile");
    assert_eq!(sess.source_meta, serde_json::json!({"chat": "c1"}));
}

#[test]
fn new_session_restores_entries_from_disk() {
    let state = make_app_state();
    save_via(
        &state,
        "ns-restore",
        "mock",
        vec![
            crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": state.welcome_cwd, "model": "disk/model-x"}),
                "disk/model-x".to_string(),
                "low".to_string(),
            ),
            crate::session::SessionEntry::new_user("user", serde_json::json!("restored hi")),
            crate::session::SessionEntry::new_assistant(
                serde_json::json!("restored reply"),
                vec![],
            ),
        ],
    );
    let cmd = make_cmd_for("new_session", "ns-restore");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let session = state.get_session("ns-restore").unwrap();
    let sess = session.read();
    assert_eq!(sess.model, "disk/model-x");
    assert_eq!(sess.messages.read().len(), 2);
}

// ── coverage batch 1: get_session_entries ───────────────────────────────

#[test]
fn get_session_entries_empty_for_unknown_live_session() {
    let state = make_app_state();
    // "default" is live but has nothing on disk.
    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("get_session_entries"),
    ));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["entries"], serde_json::json!([]));
}

#[test]
fn get_session_entries_renders_roles_and_run_stats() {
    let state = make_app_state();
    let info_old = crate::session::SessionEntry::session_info(
        serde_json::json!({"cwd": "old", "model": "mock", "session_name": "old"}),
        "mock".to_string(),
        "low".to_string(),
    );
    let user = crate::session::SessionEntry::new_user(
        "user",
        serde_json::json!([
            {"type": "text", "text": "question"},
            {"type": "text", "text": "attachment paths"},
        ]),
    );
    let mut assistant = crate::session::SessionEntry::new_assistant(
        serde_json::json!([{"type": "text", "text": "answer"}]),
        vec![],
    );
    assistant.thinking = "deep thought".to_string();
    let mut tool = crate::session::SessionEntry::new_tool("call-1", "tool output");
    tool.content = Some(serde_json::json!([{
        "type": "tool_result",
        "tool_call_id": "call-1",
        "content": "tool output",
        "is_error": false
    }]));
    let terminal = crate::session::SessionEntry::run_terminal(
        "run-1",
        crate::session::RUN_STATE_COMPLETED,
        42,
        1500,
        None,
    );
    let info_new = crate::session::SessionEntry::session_info(
        serde_json::json!({"cwd": "new", "model": "mock", "session_name": "fresh"}),
        "mock".to_string(),
        "xhigh".to_string(),
    );
    save_via(
        &state,
        "default",
        "mock",
        vec![info_old, user, assistant, tool, terminal, info_new],
    );

    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("get_session_entries"),
    ));
    assert_eq!(resp["success"], true);
    let entries = resp["data"]["entries"].as_array().unwrap();
    // session_info (deduped to one), user, assistant, tool.
    assert_eq!(entries.len(), 4);
    let info = &entries[0];
    assert_eq!(info["content"]["session_name"], "fresh");
    let user_entry = &entries[1];
    assert_eq!(user_entry["content"], "question");
    let assistant_entry = &entries[2];
    assert_eq!(assistant_entry["content"], "answer");
    assert_eq!(assistant_entry["thinking"], "deep thought");
    assert_eq!(assistant_entry["output_tokens"], 42);
    assert_eq!(assistant_entry["duration_ms"], 1500);
    let tool_entry = &entries[3];
    assert_eq!(tool_entry["content"], "tool output");
    assert_eq!(tool_entry["tool_call_id"], "call-1");
    assert_eq!(tool_entry["tool_result_is_error"], false);
}

#[test]
fn get_session_entries_reports_a_corrupt_persisted_history() {
    let state = make_app_state();
    save_via(
        &state,
        "default",
        "mock",
        vec![crate::session::SessionEntry::new_user(
            "user",
            serde_json::json!("question"),
        )],
    );
    let path = state.session_manager.session_path("default");
    let raw = std::fs::read_to_string(&path).expect("read session");
    let first = raw.lines().next().expect("session row");
    std::fs::write(&path, format!("{first}\n{{not-json}}\n{first}\n")).expect("corrupt middle row");

    let response = parse_response(&handle_command_internal(
        &state,
        make_cmd("get_session_entries"),
    ));
    assert_eq!(response["success"], false);
    assert_eq!(response["error_code"], "session_history_unreadable");
    assert!(response["error"]
        .as_str()
        .is_some_and(|error| error.contains("Unable to load session history")));
}

#[test]
fn get_session_entries_covers_compaction_billed_deltas_and_empty_info() {
    let state = make_app_state();
    // A session_info with no content exercises the `if let Some(content)`
    // false branch in the run-stats scan (a corrupted/legacy metadata entry).
    let mut info_no_content = crate::session::SessionEntry::session_info(
        serde_json::json!({}),
        "mock".to_string(),
        "low".to_string(),
    );
    info_no_content.content = None;
    // A billed-usage snapshot: tokens_in / tokens_cache_r feed the per-run
    // deltas surfaced on the final assistant entry.
    let info_billed = crate::session::SessionEntry::session_info(
        serde_json::json!({"tokens_in": 100, "tokens_cache_r": 50}),
        "mock".to_string(),
        "low".to_string(),
    );
    let assistant = crate::session::SessionEntry::new_assistant(
        serde_json::json!([{"type": "text", "text": "answer"}]),
        vec![],
    );
    let terminal = crate::session::SessionEntry::run_terminal(
        "run-1",
        crate::session::RUN_STATE_COMPLETED,
        42,
        1500,
        None,
    );
    let mut compaction = crate::session::SessionEntry::new_user("user", serde_json::json!(null));
    compaction.entry_type = crate::session::ENTRY_TYPE_COMPACTION.to_string();
    compaction.role = "system".to_string();
    compaction.content = Some(serde_json::json!({"checkpoint_id": "cp-1"}));

    save_via(
        &state,
        "default",
        "mock",
        vec![
            info_no_content,
            info_billed,
            assistant,
            terminal,
            compaction,
        ],
    );

    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("get_session_entries"),
    ));
    assert_eq!(resp["success"], true);
    let entries = resp["data"]["entries"].as_array().unwrap();

    // The assistant entry carries the billed-usage deltas (100 in / 50 cache)
    // derived from the session_info counters.
    let assistant_entry = entries
        .iter()
        .find(|e| e["entry_type"] == "assistant")
        .unwrap();
    assert_eq!(assistant_entry["output_tokens"], 42);
    assert_eq!(assistant_entry["duration_ms"], 1500);
    assert_eq!(assistant_entry["input_tokens"], 100);
    assert_eq!(assistant_entry["cache_read_tokens"], 50);

    // The compaction entry keeps its raw checkpoint content and clears the
    // display text.
    let compaction_entry = entries
        .iter()
        .find(|e| e["entry_type"] == crate::session::ENTRY_TYPE_COMPACTION)
        .unwrap();
    assert_eq!(compaction_entry["content"], "");
    assert_eq!(compaction_entry["checkpoint"]["checkpoint_id"], "cp-1");
}

#[test]
fn get_session_entries_paginates_only_when_offset_is_explicit() {
    let state = make_app_state();
    let entries = (0..5)
        .map(|i| crate::session::SessionEntry::new_user("user", serde_json::json!(format!("q{i}"))))
        .collect();
    save_via(&state, "default", "mock", entries);

    let mut first_cmd = make_cmd("get_session_entries");
    first_cmd.offset = Some(0);
    first_cmd.limit = Some(2);
    let first = parse_response(&handle_command_internal(&state, first_cmd));
    assert_eq!(first["data"]["entries"].as_array().unwrap().len(), 2);
    assert_eq!(first["data"]["hasMore"], true);
    assert_eq!(first["data"]["nextOffset"], 2);
    let version = state
        .session_manager
        .session_file_version("default")
        .expect("session version");
    let first_projection = state
        .session_manager
        .cached_display_entries("default", &version)
        .expect("first page caches the stable projection");

    let mut second_cmd = make_cmd("get_session_entries");
    second_cmd.offset = Some(2);
    second_cmd.limit = Some(2);
    let second = parse_response(&handle_command_internal(&state, second_cmd));
    assert_eq!(second["data"]["entries"].as_array().unwrap().len(), 2);
    assert_eq!(second["data"]["nextOffset"], 4);
    let second_projection = state
        .session_manager
        .cached_display_entries("default", &version)
        .expect("second page reuses the stable projection");
    assert!(Arc::ptr_eq(&first_projection, &second_projection));

    let legacy = parse_response(&handle_command_internal(
        &state,
        make_cmd("get_session_entries"),
    ));
    assert_eq!(legacy["data"]["entries"].as_array().unwrap().len(), 5);
    assert!(legacy["data"].get("hasMore").is_none());

    save_via(
        &state,
        "default",
        "mock",
        vec![crate::session::SessionEntry::new_user(
            "user",
            serde_json::json!("replacement"),
        )],
    );
    assert!(state
        .session_manager
        .cached_display_entries("default", &version)
        .is_none());
}

// ── coverage batch 1: fork / clone ──────────────────────────────────────

#[test]
fn fork_requires_entry_id() {
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("fork")));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("No message selected"));
}

#[test]
fn fork_fails_when_parent_not_on_disk() {
    let state = make_app_state();
    let mut cmd = make_cmd("fork");
    cmd.entry_id = "entry-1".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("not found on disk"));
}

#[test]
fn fork_creates_new_session_from_entry_point() {
    let state = make_app_state();
    let user = crate::session::SessionEntry::new_user("user", serde_json::json!("fork here"));
    let entry_id = user.id.clone();
    save_via(
        &state,
        "default",
        "mock",
        vec![
            user,
            crate::session::SessionEntry::new_assistant(serde_json::json!("reply"), vec![]),
        ],
    );

    let mut cmd = make_cmd("fork");
    cmd.entry_id = entry_id;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let fork_id = resp["data"]["sessionId"].as_str().unwrap().to_string();
    assert!(!fork_id.is_empty());
    assert!(state.get_session(&fork_id).is_some());
    // Forked history was loaded into memory so a later save cannot
    // truncate it.
    let session = state.get_session(&fork_id).unwrap();
    assert!(!session.read().messages.read().is_empty());
}

#[test]
fn fork_from_explicit_parent_session() {
    let state = make_app_state();
    let user = crate::session::SessionEntry::new_user("user", serde_json::json!("parent msg"));
    let entry_id = user.id.clone();
    save_via(&state, "parent-disk", "mock", vec![user]);

    let mut cmd = make_cmd("fork");
    cmd.entry_id = entry_id;
    cmd.parent_session = "parent-disk".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let fork_id = resp["data"]["sessionId"].as_str().unwrap();
    assert!(state.get_session(fork_id).is_some());
}

#[test]
fn clone_rejects_empty_session() {
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("clone")));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("Nothing to clone"));
}

#[test]
fn clone_fails_when_disk_session_missing() {
    let state = make_app_state();
    {
        let session = state.get_session("default").unwrap();
        session
            .read()
            .messages
            .write()
            .push(crate::types::AgentMessage::new_user(
                "user",
                serde_json::json!("in-memory only"),
            ));
    }
    let resp = parse_response(&handle_command_internal(&state, make_cmd("clone")));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("not found on disk"));
}

#[test]
fn clone_rejects_disk_session_with_idless_last_entry() {
    let state = make_app_state();
    {
        let session = state.get_session("default").unwrap();
        session
            .read()
            .messages
            .write()
            .push(crate::types::AgentMessage::new_user(
                "user",
                serde_json::json!("in-memory only"),
            ));
    }
    // A disk session whose last entry carries no id -> the clone leaf id
    // resolves empty and hits the "no messages found" arm.
    let mut entry = crate::session::SessionEntry::new_user("user", serde_json::json!("legacy"));
    entry.id = String::new();
    save_via(&state, "default", "mock", vec![entry]);
    let resp = parse_response(&handle_command_internal(&state, make_cmd("clone")));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("no messages found"));
}

#[test]
fn clone_succeeds_from_leaf_entry() {
    let state = make_app_state();
    {
        let session = state.get_session("default").unwrap();
        session
            .read()
            .messages
            .write()
            .push(crate::types::AgentMessage::new_user(
                "user",
                serde_json::json!("clone me"),
            ));
    }
    save_via(
        &state,
        "default",
        "mock",
        vec![
            // A session_info entry with a model makes the forked model
            // non-empty, so clone also syncs it into the new session's
            // agent loop.
            crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": "/tmp", "model": "mock"}),
                "mock".to_string(),
                "high".to_string(),
            ),
            crate::session::SessionEntry::new_user("user", serde_json::json!("clone me")),
            crate::session::SessionEntry::new_assistant(serde_json::json!("reply"), vec![]),
        ],
    );
    let resp = parse_response(&handle_command_internal(&state, make_cmd("clone")));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["cancelled"], false);
}

// ── coverage batch 1: reload_config ─────────────────────────────────────

#[test]
fn session_scoped_command_requires_known_session() {
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd_for("get_messages", "ghost"),
    ));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("session not found"));
}

#[test]
fn list_sessions_reports_enumeration_error() {
    // A regular file where the session dir should be breaks read_dir.
    let dir = test_session_dir();
    std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
    std::fs::write(&dir, "not a dir").unwrap();
    let state = make_app_state_with(dir, Arc::new(crate::runtime::GlobalQueueBudget::defaults()));

    let resp = parse_response(&handle_command_internal(&state, make_cmd("list_sessions")));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("enumerate sessions"));

    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("list_session_ids"),
    ));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("enumerate session files"));
}

#[test]
fn delete_session_with_active_run_returns_deleting() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session
        .read()
        .runtime
        .begin(Some("run-active"), Some("request-active"))
        .unwrap();

    let resp = parse_response(&handle_command_internal(&state, make_cmd("delete_session")));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "deleting");
    assert_eq!(resp["error_data"]["active_run_id"], "run-active");
    assert_eq!(resp["error_data"]["retryable"], true);
    // The session stays live behind the deletion fence.
    assert!(state.get_session("default").is_some());
}

#[test]
fn new_session_applies_user_settings() {
    let home = TestHome::new();
    let settings_path = home.settings_path();
    std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    std::fs::write(&settings_path, r#"{"defaultPermissionLevel": "workspace"}"#).unwrap();

    let state = make_app_state();
    let cmd = make_cmd_for("new_session", "ns-settings");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let session = state.get_session("ns-settings").unwrap();
    assert_eq!(session.read().get_permission_level(), "workspace");
}

#[test]
fn new_session_restores_entries_without_disk_model() {
    let state = make_app_state();
    // No session_info entry → disk model resolves empty and the effective
    // model falls back to the session's default.
    save_via(
        &state,
        "ns-nomodel",
        "mock",
        vec![crate::session::SessionEntry::new_user(
            "user",
            serde_json::json!("hi"),
        )],
    );
    let cmd = make_cmd_for("new_session", "ns-nomodel");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let session = state.get_session("ns-nomodel").unwrap();
    assert_eq!(session.read().messages.read().len(), 1);
}

#[test]
fn get_session_entries_handles_empty_tool_and_rich_meta() {
    let state = make_app_state();
    let mut assistant = crate::session::SessionEntry::new_assistant(
        serde_json::json!("with tools"),
        vec![crate::types::ToolCall {
            id: "call-1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "read".to_string(),
                arguments: serde_json::json!({"path": "x"}),
            },
        }],
    );
    assistant.meta = Some(serde_json::json!({"attachments": []}));
    let empty_tool = crate::session::SessionEntry::new_tool("call-1", "");
    save_via(&state, "default", "mock", vec![assistant, empty_tool]);

    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("get_session_entries"),
    ));
    assert_eq!(resp["success"], true);
    let entries = resp["data"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries[0]["tool_calls"].is_array());
    assert!(entries[0]["meta"].is_object());
    assert_eq!(entries[1]["content"], "");
}

#[test]
fn fork_inherits_parent_disk_model() {
    let state = make_app_state();
    let user = crate::session::SessionEntry::new_user("user", serde_json::json!("fork me"));
    let entry_id = user.id.clone();
    save_via(
        &state,
        "default",
        "mock",
        vec![
            crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": state.welcome_cwd, "model": "disk/model-y"}),
                "disk/model-y".to_string(),
                "low".to_string(),
            ),
            user,
        ],
    );

    let mut cmd = make_cmd("fork");
    cmd.entry_id = entry_id;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let fork_id = resp["data"]["sessionId"].as_str().unwrap();
    let fork = state.get_session(fork_id).unwrap();
    assert_eq!(fork.read().model, "disk/model-y");
}

#[cfg(unix)]
#[test]
fn fork_and_clone_report_save_errors() {
    let state = make_app_state();
    let user = crate::session::SessionEntry::new_user("user", serde_json::json!("fork me"));
    let entry_id = user.id.clone();
    save_via(&state, "default", "mock", vec![user]);
    {
        let session = state.get_session("default").unwrap();
        session
            .read()
            .messages
            .write()
            .push(crate::types::AgentMessage::new_user(
                "user",
                serde_json::json!("clone me"),
            ));
    }
    // Read-only session dir → the forked/clone save fails. (Windows ignores
    // the readonly bit on directories, hence cfg(unix).)
    let dir = state.session_manager.run_data_path("default");
    let sess_dir = dir.parent().unwrap().parent().unwrap().to_path_buf();
    let mut perms = std::fs::metadata(&sess_dir).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&sess_dir, perms.clone()).unwrap();

    let mut cmd = make_cmd("fork");
    cmd.entry_id = entry_id;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("failed to save forked"));

    let resp = parse_response(&handle_command_internal(&state, make_cmd("clone")));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("failed to save cloned"));

    let mut perms = std::fs::metadata(&sess_dir).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&sess_dir, perms).unwrap();
}

#[test]
fn delete_session_reports_close_failure() {
    let state = make_app_state();
    state
        .get_session("default")
        .unwrap()
        .read()
        .persistence
        .fail_next_close();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("delete_session")));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "delete_failed");
}

#[test]
fn new_session_legacy_provenance_invalid_json() {
    let state = make_app_state();
    let mut cmd = make_cmd_for("new_session", "ns-bad-json");
    cmd.custom_instructions = "not valid json".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let session = state.get_session("ns-bad-json").unwrap();
    assert_eq!(session.read().created_by, "tui");
}

#[test]
fn new_session_legacy_provenance_with_typed_source_meta() {
    let state = make_app_state();
    let mut cmd = make_cmd_for("new_session", "ns-typed-meta");
    cmd.source_meta = "{\"chat\":\"c1\"}".to_string();
    cmd.custom_instructions = "{\"createdBy\":\"legacy\"}".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let session = state.get_session("ns-typed-meta").unwrap();
    assert_eq!(session.read().created_by, "legacy");
}

#[test]
fn get_session_entries_skips_orphan_terminal_marker() {
    let state = make_app_state();
    // A run_terminal with no preceding assistant marker (orphan terminal)
    // must not fabricate run stats.
    save_via(
        &state,
        "default",
        "mock",
        vec![
            crate::session::SessionEntry::run_terminal(
                "run-1",
                crate::session::RUN_STATE_COMPLETED,
                0,
                0,
                None,
            ),
            crate::session::SessionEntry::new_user("user", serde_json::json!("hi")),
        ],
    );
    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("get_session_entries"),
    ));
    assert_eq!(resp["success"], true);
}

#[test]
fn fork_warns_when_model_sync_fails() {
    let state = make_app_state();
    let user = crate::session::SessionEntry::new_user("user", serde_json::json!("fork here"));
    let entry_id = user.id.clone();
    // A unique parent id gates the hook against parallel tests that fork
    // the "default" session. A session_info entry makes the forked model
    // non-empty, so the fork reaches the model-sync block (and consumes
    // the hook).
    save_via(
        &state,
        "fork-warn-parent",
        "mock",
        vec![
            crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": state.welcome_cwd, "model": "mock"}),
                "mock".to_string(),
                "high".to_string(),
            ),
            user,
        ],
    );
    *MODEL_SYNC_FAIL_HOOK.lock() = Some((
        "fork-warn-parent".to_string(),
        Box::new(|sess: &mut ServerSession| {
            // Closing persistence makes the subsequent model-sync
            // `update_info` fail, exercising the warn arm.
            let _ = sess.persistence.close();
        }),
    ));

    let mut cmd = make_cmd("fork");
    cmd.entry_id = entry_id;
    cmd.parent_session = "fork-warn-parent".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

#[test]
fn clone_warns_when_model_sync_fails() {
    let state = make_app_state();
    // A dedicated session id gates the hook against parallel tests that
    // clone the "default" session.
    let _ = parse_response(&handle_command_internal(
        &state,
        make_cmd_for("new_session", "clone-warn"),
    ));
    {
        let session = state.get_session("clone-warn").unwrap();
        session
            .read()
            .messages
            .write()
            .push(crate::types::AgentMessage::new_user(
                "user",
                serde_json::json!("clone me"),
            ));
    }
    save_via(
        &state,
        "clone-warn",
        "mock",
        vec![
            crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": "/tmp", "model": "mock"}),
                "mock".to_string(),
                "high".to_string(),
            ),
            crate::session::SessionEntry::new_user("user", serde_json::json!("clone me")),
            crate::session::SessionEntry::new_assistant(serde_json::json!("reply"), vec![]),
        ],
    );
    *MODEL_SYNC_FAIL_HOOK.lock() = Some((
        "clone-warn".to_string(),
        Box::new(|sess: &mut ServerSession| {
            let _ = sess.persistence.close();
        }),
    ));

    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd_for("clone", "clone-warn"),
    ));
    assert_eq!(resp["success"], true);
}

#[test]
fn clone_with_empty_disk_model_completes() {
    let state = make_app_state();
    {
        let session = state.get_session("default").unwrap();
        session
            .read()
            .messages
            .write()
            .push(crate::types::AgentMessage::new_user(
                "user",
                serde_json::json!("clone me"),
            ));
    }
    // A disk session with messages but no session_info/model_change → the
    // forked model resolves empty, skipping the model-sync block.
    save_via(
        &state,
        "default",
        "mock",
        vec![
            crate::session::SessionEntry::new_user("user", serde_json::json!("clone me")),
            crate::session::SessionEntry::new_assistant(serde_json::json!("reply"), vec![]),
        ],
    );
    let resp = parse_response(&handle_command_internal(&state, make_cmd("clone")));
    assert_eq!(resp["success"], true);
}

#[test]
fn new_session_with_invalid_settings_file_uses_defaults() {
    let home = TestHome::new();
    let settings_path = home.settings_path();
    std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    std::fs::write(&settings_path, "not valid json").unwrap();

    let state = make_app_state();
    let cmd = make_cmd_for("new_session", "ns-bad-settings");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}
