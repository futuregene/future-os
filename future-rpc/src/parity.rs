//! Wire-parity round-trips: for every Tier-1 command, a realistic fixture of
//! the agent's dual-written JSON must survive encode → typed → decode
//! unchanged (canonical casing, null-vs-absent semantics, legacy aliases).
//! The same fixtures pin the fallback matrix: data-only (old agent),
//! typed-only (future state), and both.

use crate::{decode, encode, proto};
use serde_json::{json, Value};

/// Encode `fixture` (the agent's wire JSON), then decode all three wire
/// states — dual-write, data-only (old agent), typed-only (future) — and
/// require the exact original value every time.
fn assert_command_parity(command: &str, fixture: Value) {
    let payload = encode::response_payload(command, &fixture)
        .unwrap_or_else(|| panic!("{command}: encode returned None"));

    let resp = proto::RpcResponse {
        id: "req-1".to_string(),
        r#type: "response".to_string(),
        command: command.to_string(),
        success: true,
        data: fixture.to_string(),
        error: String::new(),
        error_code: String::new(),
        error_data: String::new(),
        payload: Some(payload.clone()),
    };
    assert_eq!(
        decode::response_data(&resp),
        fixture,
        "{command}: dual-write decode must equal the wire JSON"
    );

    let resp_old = proto::RpcResponse {
        payload: None,
        ..resp.clone()
    };
    assert_eq!(
        decode::response_data(&resp_old),
        fixture,
        "{command}: data-only fallback must equal the wire JSON"
    );

    let resp_new = proto::RpcResponse {
        data: String::new(),
        payload: Some(payload),
        ..resp.clone()
    };
    assert_eq!(
        decode::response_data(&resp_new),
        fixture,
        "{command}: typed-only decode must equal the wire JSON"
    );
}

/// Full get_state wire JSON: canonical camelCase keys plus the migration
/// aliases (sessionName→session_name, snake_case TerminalAck keys), null
/// run-sequence semantics on the interrupted run, an object source_meta, a
/// full approval card and a completed run_terminal.
///
/// Parsed from a raw string: the fixture is too nested for the `json!`
/// macro's default recursion limit.
fn get_state_fixture() -> Value {
    serde_json::from_str(
        r#"{
            "agentInstanceId": "agent-1",
            "model": "future/future-model",
            "imageSupport": true,
            "thinkingLevel": "medium",
            "isStreaming": true,
            "isCompacting": false,
            "sessionId": "s1",
            "sessionName": "Demo",
            "session_name": "Demo",
            "explicitSession": true,
            "autoCompactionEnabled": true,
            "queryCount": 2,
            "version": "1.0.5",
            "cwd": "/w",
            "skills": ["skill-a"],
            "contextFiles": ["CLAUDE.md"],
            "contextWindow": 200000,
            "contextTokens": 1234,
            "contextPercent": 0.62,
            "tokensIn": 100,
            "tokensOut": 50,
            "tokensCacheR": 10,
            "tokensCacheW": 5,
            "totalCost": 0.01,
            "permissionLevel": "workspace",
            "createdBy": "gui",
            "sourceMeta": {"threadId": "t1"},
            "activeRun": {
                "runId": "r1",
                "epoch": 2,
                "runSequence": 5,
                "state": "running",
                "lastEventIdx": 9
            },
            "queuedRuns": [{
                "runId": "r2",
                "runSequence": 6,
                "clientRequestId": "c1",
                "state": "queued",
                "queuePosition": 0,
                "acceptedAt": "2026-08-06T10:00:00Z",
                "displayText": "next"
            }],
            "recentTerminalAcks": [{
                "runId": "r0",
                "runSequence": 4,
                "clientRequestId": "c0",
                "state": "cancelled",
                "reason": "superseded",
                "run_id": "r0",
                "run_sequence": 4,
                "client_request_id": "c0"
            }],
            "queuedCount": 1,
            "interruptedRun": {
                "runId": "r-old",
                "runSequence": null,
                "state": "interrupted_by_restart"
            },
            "requestedRun": {
                "run_id": "r0",
                "state": "completed",
                "run_tokens": 123,
                "run_duration_ms": 4567
            },
            "pendingApprovals": [{
                "type": "approval_request",
                "approval_request_id": "approval_1",
                "session_id": "s1",
                "tool_id": "call_1",
                "tool_name": "shell",
                "kind": "tool",
                "risk_level": "medium",
                "title": "Run command",
                "summary": "ls -la",
                "requested_action": {"command": "ls -la"},
                "action": {"command": "ls -la", "cwd": "/w"},
                "sandbox_boundary": null,
                "save_suggestion": {
                    "match_kind": "command_prefix",
                    "match_value": "ls",
                    "decision": "approve"
                },
                "reviewer": "user"
            }]
        }"#,
    )
    .expect("fixture parses")
}

fn list_sessions_fixture() -> Value {
    json!({
        "sessions": [
            {
                "id": "s1",
                "sessionName": "My session",
                "session_name": "My session",
                "model": "future/future-model",
                "cwd": "/w",
                "updatedAt": "2026-08-05 12:00:00",
                "updated_at": "2026-08-05 12:00:00",
                "parentSessionId": "p1",
                "parent_session_id": "p1",
                "firstMessage": "hello",
                "first_message": "hello",
                "queryCount": 3,
                "query_count": 3,
                "isStreaming": false,
                "is_streaming": false
            },
            {
                // Unnamed session: sessionName / firstMessage are JSON null.
                "id": "s2",
                "sessionName": null,
                "session_name": null,
                "model": "m",
                "cwd": "/w2",
                "updatedAt": "2026-08-06 09:00:00",
                "updated_at": "2026-08-06 09:00:00",
                "parentSessionId": "",
                "parent_session_id": "",
                "firstMessage": null,
                "first_message": null,
                "queryCount": 0,
                "query_count": 0,
                "isStreaming": true,
                "is_streaming": true
            }
        ]
    })
}

fn get_session_entries_fixture() -> Value {
    json!({
        "entries": [
            {
                "id": "e1",
                "role": "user",
                "content": "hello world",
                "name": "",
                "tool_args": "",
                "timestamp": "2026-08-06T10:00:00+08:00"
            },
            {
                // session_info entry: content is a JSON OBJECT, not text.
                "id": "e2",
                "role": "system",
                "content": {
                    "model": "future/future-model",
                    "thinking_level": "medium",
                    "session_name": "Demo",
                    "cwd": "/w",
                    "tokens_in": 100,
                    "tokens_out": 50,
                    "total_cost": 0.01
                },
                "name": "session_info",
                "tool_args": "",
                "timestamp": "2026-08-06T10:00:00+08:00"
            },
            {
                "id": "e3",
                "role": "assistant",
                "content": "I'll take a look.",
                "name": "",
                "tool_args": "",
                "timestamp": "2026-08-06T10:00:05+08:00",
                "thinking": "Let me check the file.",
                "meta": {"attachments": [{"path": "/tmp/a.txt", "kind": "file"}]},
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "read", "arguments": "{\"path\":\"a.txt\"}"}
                }],
                "output_tokens": 42,
                "duration_ms": 1234
            },
            {
                "id": "e4",
                "role": "tool",
                "content": "file contents",
                "name": "read",
                "tool_args": "{\"path\":\"a.txt\"}",
                "timestamp": "2026-08-06T10:00:06+08:00"
            }
        ]
    })
}

fn get_events_since_fixture() -> Value {
    json!({
        "runId": "r1",
        "events": [
            {
                "type": "agent_start",
                "data": "{\"type\":\"agent_start\",\"started_at_ms\":1750000000000}",
                "runId": "r1",
                "idx": 0,
                "sessionId": "s1",
                "epoch": 1,
                "eventId": "ev-0",
                "timestamp": "2026-08-06T10:00:00Z",
                "sessionIdx": -1,
                "runSequence": 1
            },
            {
                "type": "text_chunk",
                "data": "{\"type\":\"text_chunk\",\"text\":\"hi\"}",
                "runId": "r1",
                "idx": 1,
                "sessionId": "s1",
                "epoch": 1,
                "eventId": "ev-1",
                "timestamp": "2026-08-06T10:00:01Z",
                "sessionIdx": -1,
                "runSequence": 1
            }
        ],
        "truncated": false
    })
}

#[test]
fn get_state_wire_parity() {
    assert_command_parity("get_state", get_state_fixture());
}

#[test]
fn list_sessions_wire_parity() {
    assert_command_parity("list_sessions", list_sessions_fixture());
}

#[test]
fn get_session_entries_wire_parity() {
    assert_command_parity("get_session_entries", get_session_entries_fixture());
}

#[test]
fn get_events_since_wire_parity() {
    assert_command_parity("get_events_since", get_events_since_fixture());
}

#[test]
fn get_events_since_with_projection_parity() {
    let fixture = json!({
        "runId": "r1",
        "events": [],
        "truncated": true,
        "projection": {
            "runId": "r1",
            "cursor": 512,
            "events": [{
                "type": "tool_end",
                "data": "{\"type\":\"tool_end\",\"tool_id\":\"c1\",\"text\":\"ok\"}",
                "runId": "r1",
                "idx": 512,
                "sessionId": "s1",
                "epoch": 1,
                "eventId": "ev-512",
                "timestamp": "2026-08-06T10:05:00Z",
                "sessionIdx": -1,
                "runSequence": 1
            }]
        }
    });
    assert_command_parity("get_events_since", fixture);
}

#[test]
fn typed_getters_match_struct_semantics() {
    let fixture = get_state_fixture();
    let payload = encode::response_payload("get_state", &fixture).unwrap();
    let resp = proto::RpcResponse {
        id: "req-1".to_string(),
        r#type: "response".to_string(),
        command: "get_state".to_string(),
        success: true,
        data: String::new(),
        error: String::new(),
        error_code: String::new(),
        error_data: String::new(),
        payload: Some(payload),
    };
    let state = decode::decode_get_state(&resp).expect("typed get_state decode");
    assert_eq!(state.session_name.as_deref(), Some("Demo"));
    assert_eq!(state.source_meta, json!({"threadId": "t1"}));
    let interrupted = state.interrupted_run.expect("interrupted run");
    assert_eq!(interrupted.epoch, None, "omitted epoch stays absent");
    assert_eq!(interrupted.run_sequence, None, "null runSequence → None");
    let ack = &state.recent_terminal_acks[0];
    assert_eq!(ack.run_id, "r0");
    assert_eq!(state.pending_approvals.len(), 1);
    assert_eq!(
        state.pending_approvals[0]["save_suggestion"]["match_value"],
        "ls"
    );
}

#[test]
fn legacy_only_json_decodes_via_typed_getters() {
    // A pre-migration agent wrote snake_case-only rows; the fallback path
    // must still produce the struct via serde aliases.
    let legacy = json!({
        "sessions": [{
            "id": "s1",
            "session_name": "Legacy",
            "model": "m",
            "cwd": "/w",
            "updated_at": "2026-01-01 00:00:00",
            "parent_session_id": "",
            "first_message": "hi",
            "query_count": 1,
            "is_streaming": false
        }]
    });
    let resp = proto::RpcResponse {
        id: "req-1".to_string(),
        r#type: "response".to_string(),
        command: "list_sessions".to_string(),
        success: true,
        data: legacy.to_string(),
        error: String::new(),
        error_code: String::new(),
        error_data: String::new(),
        payload: None,
    };
    let rows = decode::decode_list_sessions(&resp).expect("legacy fallback decode");
    assert_eq!(rows[0].session_name.as_deref(), Some("Legacy"));
}

#[test]
fn encode_unknown_command_yields_none() {
    assert!(encode::response_payload("set_model", &json!({"model": "m"})).is_none());
    assert!(encode::response_payload("get_state", &json!("not an object")).is_none());
}

#[test]
fn empty_response_decodes_to_null() {
    let resp = proto::RpcResponse {
        id: "req-1".to_string(),
        r#type: "response".to_string(),
        command: "new_session".to_string(),
        success: true,
        data: String::new(),
        error: String::new(),
        error_code: String::new(),
        error_data: String::new(),
        payload: None,
    };
    assert_eq!(decode::response_data(&resp), Value::Null);
    assert_eq!(decode::response_data_str(&resp), "");
}
