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
        "list_models",
        "list_sessions",
        "list_streaming_sessions",
        "new_session",
        "switch_session",
        "delete_session",
        "get_fork_messages",
        "get_commands",
        "refresh_skills",
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

// ── coverage batch 24: per-line residuals ─────────────────────────────
