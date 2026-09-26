//! Tests for the command dispatcher itself and cross-cutting helpers.

use crate::llm::schema::ModelRequest;
use crate::rpc::RpcCommand;
use crate::types::LLMProvider;

use crate::rpc::commands::test_support::*;
use crate::rpc::handle_command_internal;

#[test]
fn unknown_command_returns_error() {
    let state = make_app_state();
    let cmd = make_cmd("nonexistent_command");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("unknown command"));
}

#[test]
fn lifecycle_marker_helpers_recognize_markers() {
    assert!(is_lifecycle_marker(crate::session::ENTRY_TYPE_RUN_STARTED));
    assert!(is_lifecycle_marker(crate::session::ENTRY_TYPE_RUN_TERMINAL));
    assert!(!is_lifecycle_marker("user"));
    assert!(!is_lifecycle_marker("assistant"));
}

#[test]
fn sessionless_commands_do_not_require_session_id() {
    // Regression: every sessionless command must be dispatched WITHOUT
    // resolving a session. If one is accidentally moved into the
    // session-scoped branch, an empty session_id trips the resolution gate
    // and the caller gets "session not found — pass a valid session_id..."
    // (that exact phrase is unique to the gate). make_cmd() always injects
    // a session id so it can't catch this — build each command by hand with
    // an empty session and assert we never hit the gate.
    //
    // `reload_auth` and `shutdown` are deliberately excluded: they carry
    // process-global side effects (credential reload / shutdown flag) that
    // don't belong in a swept table.
    let sessionless = [
        "get_agent_info",
        "get_agent_readiness",
        "list_models",
        "list_providers",
        "list_sessions",
        "list_streaming_sessions",
        "new_session",
        "switch_session",
        "delete_session",
        "get_fork_messages",
        "get_commands",
        "refresh_skills",
        "probe_sandbox",
        "probe_windows_sandbox",
        "reset_windows_sandbox",
        "set_enabled_models",
    ];
    for cmd_type in sessionless {
        let state = make_app_state();
        let cmd: RpcCommand = serde_json::from_str(&format!(
            r#"{{"id":"test_cmd","type":"{cmd_type}","sessionId":""}}"#
        ))
        .unwrap();
        assert!(cmd.session_id.is_empty());
        let resp = parse_response(&handle_command_internal(&state, cmd));
        let error = resp["error"].as_str().unwrap_or("");
        // The command must actually exist (the fallback echoes cmd_type, so
        // a successful dispatch and a typo both return command == cmd_type —
        // "unknown command" in the error is the real tell).
        assert!(
            !error.contains("unknown command"),
            "sessionless cmd {cmd_type} is not a known command: {error}"
        );
        // And it must not have failed at the session-resolution gate. A
        // command may still fail for its own reasons (e.g. switch_session
        // with an empty target) — that's fine; only the gate phrase is a
        // regression signal.
        assert!(
            !error.contains("pass a valid session_id"),
            "sessionless cmd {cmd_type} required a session: {error}"
        );
    }
}

#[test]
fn typed_payload_encodes_real_read_command_envelopes() {
    let state = make_app_state();
    // Session-scoped read commands.
    for cmd_type in ["get_state", "list_sessions", "get_session_entries"] {
        let envelope = parse_response(&handle_command_internal(&state, make_cmd(cmd_type)));
        assert_eq!(envelope["success"], true, "{cmd_type} must succeed");
        let data = &envelope["data"];
        let payload = future_rpc::encode::response_payload(cmd_type, data);
        assert!(payload.is_some(), "{cmd_type}: typed payload must encode");
    }
    // Sessionless commands.
    for cmd_type in [
        "get_agent_info",
        "get_agent_readiness",
        "list_models",
        "get_commands",
        "refresh_skills",
    ] {
        let envelope = parse_response(&handle_command_internal(&state, make_cmd(cmd_type)));
        assert_eq!(envelope["success"], true, "{cmd_type} must succeed");
        let data = &envelope["data"];
        let payload = future_rpc::encode::response_payload(cmd_type, data);
        assert!(payload.is_some(), "{cmd_type}: typed payload must encode");
    }
}

// ── coverage batch 1: sessionless dispatch + config-write paths ─────────

#[test]
fn empty_provider_yields_an_empty_stream() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        use tokio_stream::StreamExt;
        let provider = EmptyProvider;
        let mut stream = provider
            .stream_model(ModelRequest {
                model: "mock".into(),
                system_prompt: String::new(),
                messages: vec![],
                tools: vec![],
            })
            .await
            .unwrap();
        assert!(stream.next().await.is_none());
    });
}

// ── tool inspection: scope validation, empty pages, storage failures ────

#[test]
fn tool_inspection_requires_its_whole_scope_and_reports_an_empty_page() {
    let state = make_app_state();

    // Every field the query needs is required, and the message names the
    // contract — a client that forgets the toolCallId is told which one.
    for (cmd_type, run_id, tool_call_id) in [
        ("list_tool_calls", "", Some("call-1".to_string())),
        ("get_tool_output", "", Some("call-1".to_string())),
        ("get_tool_output", "run-1", None),
        ("get_tool_output", "run-1", Some(String::new())),
    ] {
        let mut cmd = make_cmd(cmd_type);
        cmd.session_id = "default".into();
        cmd.run_id = run_id.into();
        cmd.tool_call_id = tool_call_id;
        let response = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(
            response["success"], false,
            "{cmd_type} accepted an incomplete scope: {response}"
        );
        assert!(response["error"]
            .as_str()
            .unwrap()
            .contains("sessionId, runId and (for call-scoped reads) toolCallId are required"));
    }

    // A well-formed query for a run that recorded nothing is an empty page,
    // not an error — and the cursor must not advance.
    let mut cmd = make_cmd("list_tool_calls");
    cmd.session_id = "default".into();
    cmd.run_id = "run-with-no-tools".into();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], true, "{response}");
    assert_eq!(response["data"]["tools"], serde_json::json!([]));
    assert_eq!(response["data"]["hasMore"], false);
    assert_eq!(response["data"]["nextOffset"], 0);

    let mut cmd = make_cmd("get_tool_output");
    cmd.session_id = "default".into();
    cmd.run_id = "run-with-no-tools".into();
    cmd.tool_call_id = Some("call-none".into());
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], true, "{response}");
    assert!(response["data"]["output"].is_null());
}

/// A session store that cannot be opened (the configured `agent.db` path is a
/// directory) must surface a retryable error from every entry point instead of
/// panicking or pretending the session is empty.
fn app_state_with_unopenable_store() -> crate::rpc::AppState {
    let dir = test_session_dir();
    std::fs::create_dir_all(dir.join("agent.db")).unwrap();
    make_app_state_with(
        dir,
        std::sync::Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
    )
}

#[test]
fn storage_failures_are_reported_with_their_own_error_code() {
    let state = app_state_with_unopenable_store();

    // Tool inspection reads the store directly (no session resolution).
    let mut cmd = make_cmd("list_tool_calls");
    cmd.session_id = "default".into();
    cmd.run_id = "run-1".into();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], false, "{response}");

    // Browsing an unknown session has to inspect storage before it can say
    // "not found", so an unopenable store is a distinct, retryable failure.
    let mut cmd = make_cmd("get_session_entries");
    cmd.session_id = "cold-session".into();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], false, "{response}");
    assert_eq!(response["error_code"], "session_storage_unavailable");
    assert_eq!(response["error_data"]["retryable"], true);

    // Resolving a session that is not resident also goes through storage.
    let mut cmd = make_cmd("get_messages");
    cmd.session_id = "cold-session".into();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], false, "{response}");
    assert_eq!(response["error_code"], "session_storage_unavailable");

    // A resident session whose history has not been hydrated yet reaches the
    // hydration gate, which must fail loudly rather than run on an empty
    // context (the test-support session ships pre-hydrated, so unset it).
    let session = state.sessions.read().get("default").cloned().unwrap();
    session.write().history_loaded = false;
    let mut cmd = make_cmd("get_messages");
    cmd.session_id = "default".into();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], false, "{response}");
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains("Unable to restore session context"));
}

// ── get_session_entries: unknown ids never fall back to another session ──

#[test]
fn get_session_entries_rejects_an_empty_or_unknown_session_id() {
    let state = make_app_state();
    for session_id in ["", "no-such-session"] {
        let mut cmd = make_cmd("get_session_entries");
        cmd.session_id = session_id.into();
        let response = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(
            response["success"], false,
            "{session_id:?} must not resolve to a session: {response}"
        );
        assert!(response["error"]
            .as_str()
            .unwrap()
            .contains("session not found"));
    }
    // The store is still usable afterwards — the rejection is not corruption.
    let response = parse_response(&handle_command_internal(&state, make_cmd("get_state")));
    assert_eq!(response["success"], true, "{response}");
}

// ── coverage batch 24: per-line residuals ─────────────────────────────
