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
        "delete_sessions",
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
        // The stable code is what a client holding a stale row for this session
        // keys off: it means "this session does not exist", not "retry me".
        assert_eq!(
            response["error_code"],
            future_rpc::session_errors::SESSION_NOT_FOUND_CODE,
            "{session_id:?} must be reported as definitively missing: {response}"
        );
    }
    // The store is still usable afterwards — the rejection is not corruption.
    let response = parse_response(&handle_command_internal(&state, make_cmd("get_state")));
    assert_eq!(response["success"], true, "{response}");
}

// ── coverage batch 24: per-line residuals ─────────────────────────────

/// `get_tool_call_args` is the third read in the tool-inspection family: the
/// same scope requirement, and its own arm in the dispatcher's inner match (the
/// `_` arm is `get_tool_output`, so a mis-typed name must not silently read
/// output instead of arguments).
#[test]
fn tool_call_args_reads_its_own_arm_and_still_requires_the_call_id() {
    let state = make_app_state();

    // The whole scope is required, exactly like the sibling reads.
    let mut cmd = make_cmd("get_tool_call_args");
    cmd.session_id = "default".into();
    cmd.run_id = "run-1".into();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], false, "{response}");
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains("toolCallId are required"));

    // A well-formed query on a run that recorded nothing reads its own arm and
    // answers with no arguments rather than an error.
    let mut cmd = make_cmd("get_tool_call_args");
    cmd.session_id = "default".into();
    cmd.run_id = "run-with-no-tools".into();
    cmd.tool_call_id = Some("call-none".into());
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], true, "{response}");
    assert_eq!(
        response["data"]["toolCallId"],
        serde_json::json!("call-none")
    );
    assert!(
        response["data"]["arguments"].is_null(),
        "the arguments arm answers with `arguments`, not the output arm: {response}"
    );
    assert!(
        response["data"].get("output").is_none(),
        "a mis-typed name must not fall through to the tool-output arm: {response}"
    );
}

/// `get_reasoning` shares the tool-inspection family's session+run scope but
/// needs no call identity: the index of the reasoning block rides `offset`.
#[test]
fn get_reasoning_reads_its_own_arm_with_the_block_index_in_offset() {
    let state = make_app_state();

    let mut cmd = make_cmd("get_reasoning");
    cmd.session_id = "default".into();
    cmd.run_id = "".into();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], false, "{response}");
    assert!(response["error"].as_str().unwrap().contains("runId"));

    // A well-formed read on a run that recorded nothing answers its own arm with
    // an explicit miss, not an error and not a tool read.
    let mut cmd = make_cmd("get_reasoning");
    cmd.session_id = "default".into();
    cmd.run_id = "run-with-no-thinking".into();
    cmd.offset = Some(0);
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], true, "{response}");
    assert_eq!(response["data"]["index"], serde_json::json!(0));
    assert!(response["data"]["text"].is_null(), "{response}");
    assert!(
        response["data"].get("arguments").is_none() && response["data"].get("output").is_none(),
        "the reasoning arm must not fall through to a tool read: {response}"
    );
}

/// Dispatch completeness: every command the shared policy registry knows must
/// reach a real arm. The dispatcher's final `_ => "unknown command"` arm is the
/// tripwire for a gap between the registry and the match below it, so any
/// registry entry that produces exactly that error is a missing arm.
#[test]
fn every_registered_command_reaches_a_dispatch_arm() {
    let registry = future_rpc::command_policy::KNOWN_COMMANDS;
    assert!(
        registry.len() >= 80,
        "the registry is the dispatcher's contract: {} entries",
        registry.len()
    );
    for cmd_type in registry {
        let state = make_app_state();
        let cmd: RpcCommand = serde_json::from_str(&format!(
            r#"{{"id":"registry-sweep","type":"{cmd_type}","sessionId":"default"}}"#
        ))
        .unwrap();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        let error = resp["error"].as_str().unwrap_or("");
        assert!(
            !error.contains(&format!("unknown command: {cmd_type}")),
            "{cmd_type} is registered but has no dispatch arm: {error}"
        );
    }
}

/// Dispatch completeness, the other direction: an arm the dispatcher has, but
/// the shared policy registry does not know, can never run.
///
/// The registry is checked *above* the match (`handle_command_internal` answers
/// `unknown command` before dispatch, because a client needs a deadline for
/// every command it may send), so an arm added without a policy is dead code
/// that fails at the far end of a round trip. `release_queued_run` lived there
/// for two days: implemented, wired through the host and both clients, and
/// answering "unknown command: release_queued_run" the moment a reader pressed
/// "send this one now" on a parked queued message — the one control that could
/// take it out of the queue. The reverse sweep above could not see it, because
/// it starts from the registry.
///
/// The names are read off the dispatcher's own source rather than a list kept
/// here: a list would be one more thing to forget to update. What it reads is
/// the two shapes the dispatcher is written in — `"name" => …` arms and
/// `cmd_type == "name"` checks — so a command added some other way (a
/// `matches!(cmd_type.as_str(), …)` gate) is not covered by this sweep.
#[test]
fn every_dispatch_arm_is_a_registered_command() {
    const SOURCE: &str = include_str!("mod.rs");

    fn quoted_names(text: &str) -> Vec<&str> {
        let mut names = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find('"') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('"') else { break };
            let name = &after[..end];
            if !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            {
                names.push(name);
            }
            rest = &after[end + 1..];
        }
        names
    }

    let mut arms: Vec<&str> = Vec::new();
    for line in SOURCE.lines() {
        // `"name" => handler`, or several names before one arrow.
        if let Some((head, _)) = line.split_once("=>") {
            if head.trim_start().starts_with('"') {
                arms.extend(quoted_names(head));
            }
        }
        // `if cmd_type == "name"`.
        if let Some(at) = line.find("cmd_type == ") {
            arms.extend(quoted_names(&line[at + "cmd_type == ".len()..]));
        }
    }
    // The sweep must be reading the dispatcher, or it proves nothing — and the
    // command that was missing is named here, so a refactor that stops matching
    // the file's shape fails loudly instead of passing empty.
    assert!(
        arms.len() >= 60 && arms.contains(&"release_queued_run") && arms.contains(&"prompt"),
        "the sweep did not read the dispatcher ({} names)",
        arms.len()
    );

    for cmd_type in arms {
        assert!(
            future_rpc::command_policy::command_policy(cmd_type).is_some(),
            "{cmd_type} has a dispatch arm but no entry in the shared command \
             policy, so every client asking for it is refused before dispatch"
        );
    }
}
