//! Wire-parity round-trips: for every Tier-1 command, a realistic fixture of
//! the agent's canonical JSON must survive encode → typed → decode unchanged
//! (canonical casing, null-vs-absent semantics). Typed commands ride the
//! typed `payload` only (empty `data`); untyped commands keep the JSON string.

use crate::{decode, encode, proto};
use serde_json::{json, Value};

/// Encode `fixture` (the agent's canonical wire JSON), then decode the typed
/// payload back and require the exact original value.
fn assert_command_parity(command: &str, fixture: Value) {
    let payload = encode::response_payload(command, &fixture)
        .unwrap_or_else(|| panic!("{command}: encode returned None"));

    let resp = proto::RpcResponse {
        id: "req-1".to_string(),
        r#type: "response".to_string(),
        command: command.to_string(),
        success: true,
        data: String::new(),
        error: String::new(),
        error_code: String::new(),
        error_data: String::new(),
        payload: Some(payload),
    };
    assert_eq!(
        decode::response_data(&resp),
        fixture,
        "{command}: typed-only decode must equal the wire JSON"
    );
}

/// Full get_state wire JSON: canonical camelCase keys, null run-sequence
/// semantics on the interrupted run, an object source_meta, a full approval
/// card and a completed run_terminal.
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
            "createdBy": "desktop",
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
                "reason": "superseded"
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
                "model": "future/future-model",
                "cwd": "/w",
                "updatedAt": "2026-08-05 12:00:00",
                "parentSessionId": "p1",
                "firstMessage": "hello",
                "queryCount": 3,
                "isStreaming": false
            },
            {
                // Unnamed session: sessionName / firstMessage are JSON null.
                "id": "s2",
                "sessionName": null,
                "model": "m",
                "cwd": "/w2",
                "updatedAt": "2026-08-06 09:00:00",
                "parentSessionId": "",
                "firstMessage": null,
                "queryCount": 0,
                "isStreaming": true
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
fn get_events_since_has_more_wire_parity() {
    // A paged response (hasMore: true) must survive the typed round-trip;
    // the flag is absent on the wire when false (legacy shape).
    let mut fixture = get_events_since_fixture();
    fixture["hasMore"] = json!(true);
    assert_command_parity("get_events_since", fixture);
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

// ── prompt ack / models / info / skills / ops commands ──────────────────────

#[test]
fn prompt_ack_wire_parity() {
    // Running ack: queue identity absent.
    assert_command_parity(
        "prompt",
        json!({
            "run_id": "run-a",
            "run_epoch": 7,
            "accepted_state": "running"
        }),
    );
    // Queued ack: queue identity present, run_epoch zero.
    assert_command_parity(
        "prompt",
        json!({
            "run_id": "run-b",
            "run_epoch": 0,
            "accepted_state": "queued",
            "run_sequence": 3,
            "queue_position": 0
        }),
    );
}

#[test]
fn list_models_wire_parity() {
    assert_command_parity(
        "list_models",
        json!({
            "models": [
                {
                    "id": "m1",
                    "label": "Model One",
                    "provider": "future",
                    "supportsImages": true,
                    "thinkingLevel": "high",
                    "contextWindow": 200000,
                    "isDefault": true,
                    "description": "旗舰模型",
                    "descriptionEn": "Flagship model",
                    "recommended": true
                },
                {
                    // Catalog entry without descriptions: JSON null.
                    "id": "m2",
                    "label": "Model Two",
                    "provider": "future",
                    "supportsImages": false,
                    "thinkingLevel": "off",
                    "contextWindow": 8000,
                    "isDefault": false,
                    "description": null,
                    "descriptionEn": null,
                    "recommended": false
                }
            ],
            "defaultModel": "m1",
            "isScoped": false,
            "builtinProviders": {
                "openai": {
                    "name": "OpenAI",
                    "modelCount": 12,
                    "baseUrl": "https://api.openai.com/v1"
                }
            }
        }),
    );
    // Without builtinProviders (the default list_models call).
    assert_command_parity(
        "list_models",
        json!({
            "models": [],
            "defaultModel": "",
            "isScoped": false
        }),
    );
}

#[test]
fn get_agent_info_wire_parity() {
    assert_command_parity(
        "get_agent_info",
        json!({
            "version": "1.0.5",
            "agentInstanceId": "agent-1",
            "skillsCount": 4
        }),
    );
}

#[test]
fn get_commands_wire_parity() {
    assert_command_parity(
        "get_commands",
        json!({
            "commands": [
                {
                    "name": "review",
                    "description": "Review a pull request",
                    "nameZh": "评审",
                    "descriptionZh": "评审拉取请求",
                    "source": "skill"
                },
                {
                    // Skill without localized fields: JSON null.
                    "name": "init",
                    "description": "Initialize a CLAUDE.md",
                    "nameZh": null,
                    "descriptionZh": null,
                    "source": "skill"
                }
            ]
        }),
    );
}

#[test]
fn compact_wire_parity() {
    assert_command_parity(
        "compact",
        json!({
            "tokensBefore": 100000,
            "tokensAfter": 20000,
            "summary": "The user asked about X.",
            "messagesRemoved": 80000
        }),
    );
}

#[test]
fn shell_wire_parity() {
    assert_command_parity("shell", json!({"output": "hello\n", "exitCode": 0}));
    assert_command_parity("shell", json!({"output": "boom", "exitCode": 127}));
}

#[test]
fn cycle_model_wire_parity() {
    assert_command_parity(
        "cycle_model",
        json!({"model": "provider/next", "thinkingLevel": "high", "isScoped": false}),
    );
    // Empty-catalog edge case: isScoped absent.
    assert_command_parity("cycle_model", json!({"model": "", "thinkingLevel": ""}));
}

#[test]
fn sync_future_models_wire_parity() {
    assert_command_parity(
        "sync_future_models",
        json!({"synced": true, "modelCount": 42}),
    );
}

#[test]
fn refresh_skills_wire_parity() {
    // snake_case wire shape — the exception among these payloads.
    assert_command_parity(
        "refresh_skills",
        json!({"skills_count": 2, "skills": ["a", "b"], "refreshed": true}),
    );
}

#[test]
fn get_session_stats_wire_parity() {
    // cost travels as a float in the typed form (proto double); the agent's
    // current hardcoded `0` integer literal normalizes to 0.0 on that path,
    // which every consumer reads as the same number.
    assert_command_parity(
        "get_session_stats",
        json!({
            "sessionFile": "",
            "sessionId": "s1",
            "userMessages": 3,
            "assistantMessages": 3,
            "toolCalls": 5,
            "toolResults": 5,
            "totalMessages": 16,
            "tokens": {"input": 0, "output": 0, "cacheRead": 0, "total": 0},
            "cost": 0.0
        }),
    );
}

#[test]
fn get_runtime_metrics_wire_parity() {
    // Healthy journal: eventJournalError is JSON null (the agent emits null,
    // not an absent key, for Option fields built with json!).
    assert_command_parity(
        "get_runtime_metrics",
        json!({
            "sessionId": "s1",
            "activeRunGauge": 1,
            "staleEpochDrops": 0,
            "persistenceDegraded": 0,
            "broadcastLag": 2,
            "ringTruncations": 0,
            "activeRunId": "run-1",
            "queuedRuns": 1,
            "queuedBytes": 256,
            "eventJournalHealthy": true,
            "eventJournalError": null
        }),
    );
    // Idle + degraded journal: activeRunId null, error present.
    assert_command_parity(
        "get_runtime_metrics",
        json!({
            "sessionId": "s1",
            "activeRunGauge": 0,
            "staleEpochDrops": 1,
            "persistenceDegraded": 3,
            "broadcastLag": 0,
            "ringTruncations": 1,
            "activeRunId": null,
            "queuedRuns": 0,
            "queuedBytes": 0,
            "eventJournalHealthy": false,
            "eventJournalError": "disk full"
        }),
    );
}

#[test]
fn get_session_events_since_wire_parity() {
    assert_command_parity(
        "get_session_events_since",
        json!({
            "events": [{
                "type": "model_changed",
                "data": "{\"model\":\"provider/next\"}",
                "sessionId": "s1",
                "sessionIdx": 4,
                "eventId": "sev-1",
                "timestamp": "2026-08-06T10:00:00Z"
            }]
        }),
    );
}

// ── event payloads ───────────────────────────────────────────────────────────

/// Event wire-parity: encode the real wire `data` JSON into the typed form,
/// then verify (a) data-first decode returns the original JSON verbatim
/// (byte-stable for journal/NATS consumers during the migration window) and
/// (b) typed-only decode reconstructs the canonical shape — the wire JSON
/// minus the redundant injected `type` key.
fn assert_event_parity(event_type: &str, wire_data: Value) {
    let encoded = encode::event_payload(event_type, &wire_data.to_string())
        .unwrap_or_else(|| panic!("{event_type}: event encode returned None"));

    // Dual-write: the original data string wins, byte-stable.
    let event = proto::StreamEvent {
        r#type: event_type.to_string(),
        data: wire_data.to_string(),
        payload: Some(encoded.clone()),
        ..Default::default()
    };
    assert_eq!(
        decode::event_data(&event),
        wire_data,
        "{event_type}: data-first decode must return the wire JSON"
    );
    assert_eq!(
        decode::event_data_json(&event),
        wire_data.to_string(),
        "{event_type}: data-first decode must return the wire string verbatim"
    );

    // Typed-only (future state): canonical reconstruction = wire minus the
    // redundant injected `type` key.
    let mut canonical = wire_data.clone();
    if let Some(object) = canonical.as_object_mut() {
        object.remove("type");
    }
    let typed_only = proto::StreamEvent {
        r#type: event_type.to_string(),
        data: String::new(),
        payload: Some(encoded),
        ..Default::default()
    };
    assert_eq!(
        decode::event_data(&typed_only),
        canonical,
        "{event_type}: typed-only decode must equal the canonical shape"
    );
}

#[test]
fn text_chunk_event_parity() {
    assert_event_parity("text_chunk", json!({"text": "Hello"}));
}

#[test]
fn user_message_event_parity() {
    assert_event_parity("user_message", json!({"text": "do the thing"}));
}

#[test]
fn error_event_parity() {
    // Inline run-error build (no injected type key)...
    assert_event_parity("error", json!({"error": "model exploded"}));
    // ...and the provider-derived variant with the injected type key: both
    // reconstruct to the same canonical shape.
    assert_event_parity("error", json!({"type": "error", "error": "provider error"}));
}

#[test]
fn agent_start_event_parity() {
    assert_event_parity(
        "agent_start",
        json!({"type": "agent_start", "started_at_ms": 1750000000000_u64}),
    );
}

#[test]
fn agent_end_event_parity() {
    // Clean completion with run totals.
    assert_event_parity(
        "agent_end",
        json!({
            "type": "agent_end",
            "state": "completed",
            "usage": {"output_tokens": 42},
            "duration_ms": 1234
        }),
    );
    // Truncated stream marker.
    assert_event_parity(
        "agent_end",
        json!({
            "type": "agent_end",
            "state": "completed",
            "usage": {"output_tokens": 7},
            "duration_ms": 99,
            "reason": "incomplete"
        }),
    );
    // Early task-spawn failure: only the error.
    assert_event_parity(
        "agent_end",
        json!({"type": "agent_end", "error": "Failed to start accepted run task"}),
    );
}

#[test]
fn thinking_event_parity() {
    assert_event_parity(
        "thinking_delta",
        json!({"type": "thinking_delta", "text": "Let me consider..."}),
    );
    // Lifecycle markers carry no payload.
    assert_event_parity("thinking_start", json!({"type": "thinking_start"}));
    assert_event_parity("thinking_end", json!({"type": "thinking_end"}));
}

#[test]
fn tool_start_event_parity() {
    assert_event_parity(
        "tool_start",
        json!({
            "type": "tool_start",
            "tool_name": "shell",
            "tool_id": "call_1",
            "tool_args": {"command": "ls -la"}
        }),
    );
    // JSON-encoded-string arguments variant (some providers).
    assert_event_parity(
        "tool_start",
        json!({
            "type": "tool_start",
            "tool_name": "read",
            "tool_id": "call_2",
            "tool_args": "{\"path\":\"a.txt\"}"
        }),
    );
}

#[test]
fn tool_delta_event_parity() {
    assert_event_parity(
        "tool_delta",
        json!({
            "type": "tool_delta",
            "tool_id": "call_1",
            "text": "{\"comma",
            "tc_index": 2
        }),
    );
}

#[test]
fn tool_end_event_parity() {
    // Shell soft-fail semantics. The serializer omits empty `text` (it only
    // inserts non-empty strings), so no `text` key here.
    assert_event_parity(
        "tool_end",
        json!({
            "type": "tool_end",
            "tool_name": "shell",
            "tool_id": "call_1",
            "exit_code": 1,
            "is_soft_fail": true
        }),
    );
    // write/edit target path + output.
    assert_event_parity(
        "tool_end",
        json!({
            "type": "tool_end",
            "tool_name": "write",
            "tool_id": "call_2",
            "text": "File written",
            "target_path": "/w/a.txt"
        }),
    );
    // Error result.
    assert_event_parity(
        "tool_end",
        json!({
            "type": "tool_end",
            "tool_name": "shell",
            "tool_id": "call_3",
            "text": "partial output",
            "error": "command failed"
        }),
    );
}

#[test]
fn usage_event_parity() {
    assert_event_parity(
        "usage",
        json!({
            "type": "usage",
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150,
                "cache_read_tokens": 20,
                "cache_write_tokens": 5,
                "credit_cost": 0.0002
            }
        }),
    );
}

#[test]
fn approval_request_event_parity() {
    assert_event_parity(
        "approval_request",
        json!({
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
        }),
    );
}

#[test]
fn approval_decision_event_parity() {
    assert_event_parity(
        "approval_decision",
        json!({
            "type": "approval_decision",
            "approval_request_id": "approval_1",
            "tool_id": "call_1",
            "status": "approved",
            "note": ""
        }),
    );
}

#[test]
fn pass_through_events_stay_untyped() {
    // Settings/ping/sideband events have no typed member; encode returns
    // None and they keep riding the JSON `data` string.
    for (event_type, data) in [
        ("model_changed", json!({"model": "provider/next"})),
        ("ping", json!({"type": "ping"})),
        (
            "tool_sandboxed",
            json!({"type": "tool_sandboxed", "command": "ls"}),
        ),
        ("persistence_error", json!({"error": "disk full"})),
        (
            "compaction_end",
            json!({"type": "compaction_end", "tokens_before": 1}),
        ),
    ] {
        assert!(
            encode::event_payload(event_type, &data.to_string()).is_none(),
            "{event_type}: must stay on the JSON data string"
        );
    }
}

/// Hot-path budget: text_chunk is emitted per token, so its typed encode
/// must stay cheap. Generous ceiling (5µs/event) — a regression beyond this
/// fails the build instead of silently taxing the stream. Measured cost on
/// dev hardware is well under 1µs.
#[test]
fn text_chunk_encode_stays_within_budget() {
    let data = r#"{"text":"a streaming token"}"#.to_string();
    // Warm-up (allocator/JIT-free Rust, but keep the first-touch cost out).
    for _ in 0..1000 {
        std::hint::black_box(encode::event_payload("text_chunk", &data));
    }
    let iterations = 20_000;
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(encode::event_payload("text_chunk", &data));
    }
    let per_event = start.elapsed() / iterations;
    assert!(
        per_event < std::time::Duration::from_micros(5),
        "text_chunk encode too slow: {per_event:?}/event"
    );
}
