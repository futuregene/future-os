//! Semantic stream-event parsing shared by all agent clients (channel
//! bridges today; GUI/TUI/CLI as they adopt the crate).
//!
//! [`parse_agent_event`] reads the typed `StreamEvent.payload` first (new
//! agents) and falls back to the JSON `data` string (old agents, and
//! pass-through event types that have no typed member). Both paths produce
//! the same [`AgentEvent`] semantics — the wire-parity fixtures pin it.

use crate::proto;
use serde_json::Value;

/// Events from the agent event stream, shaped for client consumption.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    TextChunk(String),
    ThinkingStart,
    ThinkingDelta(String),
    ThinkingEnd,
    AgentStart,
    AgentEnd {
        error: Option<String>,
        /// Canonical terminal state (`completed` / `cancelled` / `error` /
        /// `incomplete`); lets a bridge tell a cancellation apart from a clean
        /// completion without parsing free-text error strings.
        state: Option<String>,
    },
    ToolStart {
        tool_id: String,
        tool_name: String,
        tool_args: Option<String>,
    },
    ToolDelta {
        tool_id: String,
        text: String,
    },
    ToolEnd {
        tool_id: String,
        text: Option<String>,
    },
    ApprovalRequest {
        approval_request_id: String,
        tool_id: String,
        tool_name: String,
        kind: String,
        risk_level: String,
        title: String,
        summary: String,
        requested_action: Value,
    },
    Error(String),
    Ping,
}

/// Parse a StreamEvent into an AgentEvent, paired with the event's canonical
/// `run_id` so callers can drop events that belong to a different run on the
/// same session (another client, or a stale tail after a supersede) instead
/// of letting a foreign `agent_end` finalize their reply.
pub fn parse_agent_event(event: &proto::StreamEvent) -> Option<(String, AgentEvent)> {
    let parsed =
        typed_agent_event(event).or_else(|| data_agent_event(&event.r#type, &event.data))?;
    Some((event.run_id.clone(), parsed))
}

/// Typed path: read the event payload oneof (present on new agents).
/// Returns None for kinds without an AgentEvent mapping (usage, user_message,
/// approval_decision) and for absent payloads — the caller falls back.
fn typed_agent_event(event: &proto::StreamEvent) -> Option<AgentEvent> {
    use proto::event_payload::Kind;
    let kind = event.payload.as_ref()?.kind.as_ref()?;
    let parsed = match kind {
        Kind::TextChunk(data) => AgentEvent::TextChunk(data.text.clone()),
        Kind::ThinkingStart(_) => AgentEvent::ThinkingStart,
        Kind::ThinkingDelta(data) => AgentEvent::ThinkingDelta(data.text.clone()),
        Kind::ThinkingEnd(_) => AgentEvent::ThinkingEnd,
        Kind::AgentStart(_) => AgentEvent::AgentStart,
        Kind::AgentEnd(data) => AgentEvent::AgentEnd {
            error: data.error.clone(),
            state: data.state.clone(),
        },
        Kind::ToolStart(data) => AgentEvent::ToolStart {
            tool_id: data.tool_id.clone(),
            tool_name: data.tool_name.clone(),
            tool_args: string_tool_args(&data.tool_args),
        },
        Kind::ToolDelta(data) => AgentEvent::ToolDelta {
            tool_id: data.tool_id.clone(),
            text: data.text.clone(),
        },
        Kind::ToolEnd(data) => AgentEvent::ToolEnd {
            tool_id: data.tool_id.clone(),
            text: if data.text.is_empty() {
                None
            } else {
                Some(data.text.clone())
            },
        },
        Kind::ApprovalRequest(info) => AgentEvent::ApprovalRequest {
            approval_request_id: info.approval_request_id.clone(),
            tool_id: info.tool_id.clone(),
            tool_name: info.tool_name.clone(),
            kind: info.kind.clone(),
            risk_level: info.risk_level.clone(),
            title: info.title.clone(),
            summary: info.summary.clone(),
            requested_action: crate::decode::inflate_json_value(&info.requested_action),
        },
        Kind::Error(data) => AgentEvent::Error(if data.error.is_empty() {
            "unknown error".to_string()
        } else {
            data.error.clone()
        }),
        // No AgentEvent variant: fall through to the data path (which also
        // yields None for these types).
        _ => return None,
    };
    Some(parsed)
}

/// Wire semantics: bridges consume tool arguments only in string form
/// (matching `data["tool_args"].as_str()` on the JSON path). The typed
/// carrier serializes the original JSON value, so a value that parses back
/// to a JSON string was a string on the wire; objects stay None.
fn string_tool_args(raw: &str) -> Option<String> {
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::String(inner)) => Some(inner),
        _ => None,
    }
}

/// JSON fallback path: parse the `data` string (old agents, pass-through
/// events like ping, and typed kinds the caller does not map).
fn data_agent_event(event_type: &str, data: &str) -> Option<AgentEvent> {
    match event_type {
        "ping" => Some(AgentEvent::Ping),
        "agent_start" => Some(AgentEvent::AgentStart),
        "agent_end" => {
            let parsed: Option<Value> = serde_json::from_str(data).ok();
            let error = parsed
                .as_ref()
                .and_then(|d| d["error"].as_str().map(|s| s.to_string()));
            let state = parsed
                .as_ref()
                .and_then(|d| d["state"].as_str().map(|s| s.to_string()));
            Some(AgentEvent::AgentEnd { error, state })
        }
        "text_chunk" => {
            let text = serde_json::from_str::<Value>(data)
                .ok()
                .and_then(|d| d["text"].as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            Some(AgentEvent::TextChunk(text))
        }
        "thinking_start" => Some(AgentEvent::ThinkingStart),
        "thinking_delta" => {
            let text = serde_json::from_str::<Value>(data)
                .ok()
                .and_then(|d| d["text"].as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            Some(AgentEvent::ThinkingDelta(text))
        }
        "thinking_end" => Some(AgentEvent::ThinkingEnd),
        "tool_start" => {
            let data = serde_json::from_str::<Value>(data).ok()?;
            Some(AgentEvent::ToolStart {
                tool_id: data["tool_id"].as_str().unwrap_or("").to_string(),
                tool_name: data["tool_name"].as_str().unwrap_or("").to_string(),
                tool_args: data["tool_args"].as_str().map(|s| s.to_string()),
            })
        }
        "tool_delta" => {
            let data = serde_json::from_str::<Value>(data).ok()?;
            Some(AgentEvent::ToolDelta {
                tool_id: data["tool_id"].as_str().unwrap_or("").to_string(),
                text: data["text"].as_str().unwrap_or("").to_string(),
            })
        }
        "tool_end" => {
            let data = serde_json::from_str::<Value>(data).ok()?;
            Some(AgentEvent::ToolEnd {
                tool_id: data["tool_id"].as_str().unwrap_or("").to_string(),
                text: data["text"].as_str().map(|s| s.to_string()),
            })
        }
        "approval_request" => {
            let data = serde_json::from_str::<Value>(data).ok()?;
            Some(AgentEvent::ApprovalRequest {
                approval_request_id: data["approval_request_id"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                tool_id: data["tool_id"].as_str().unwrap_or("").to_string(),
                tool_name: data["tool_name"].as_str().unwrap_or("").to_string(),
                kind: data["kind"].as_str().unwrap_or("").to_string(),
                risk_level: data["risk_level"].as_str().unwrap_or("").to_string(),
                title: data["title"].as_str().unwrap_or("").to_string(),
                summary: data["summary"].as_str().unwrap_or("").to_string(),
                requested_action: data["requested_action"].clone(),
            })
        }
        "error" => {
            let msg = serde_json::from_str::<Value>(data)
                .ok()
                .and_then(|d| d["error"].as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown error".to_string());
            Some(AgentEvent::Error(msg))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(event_type: &str, data: &str) -> proto::StreamEvent {
        proto::StreamEvent {
            r#type: event_type.to_string(),
            data: data.to_string(),
            run_id: "run_1".to_string(),
            idx: 0,
            ..Default::default()
        }
    }

    /// Same event with the typed payload populated and `data` emptied —
    /// the new-agent wire state. Every parse test runs against both twins.
    fn make_typed_event(event_type: &str, data: &str) -> Option<proto::StreamEvent> {
        let payload = crate::encode::event_payload(event_type, data)?;
        Some(proto::StreamEvent {
            r#type: event_type.to_string(),
            data: String::new(),
            run_id: "run_1".to_string(),
            idx: 0,
            payload: Some(payload),
            ..Default::default()
        })
    }

    fn parsed(event: proto::StreamEvent) -> Option<AgentEvent> {
        parse_agent_event(&event).map(|(_, ev)| ev)
    }

    /// Run one assertion against both the JSON-data twin (old agent) and the
    /// typed twin (new agent) so the two paths cannot drift.
    fn parse_both_twins<F>(event_type: &str, data: &str, check: F)
    where
        F: Fn(Option<AgentEvent>),
    {
        check(parsed(make_event(event_type, data)));
        if let Some(typed) = make_typed_event(event_type, data) {
            check(parsed(typed));
        }
    }

    #[test]
    fn parse_event_carries_run_id() {
        let event = make_event("text_chunk", r#"{"text":"hi"}"#);
        let (run_id, ev) = parse_agent_event(&event).expect("parsed");
        assert_eq!(run_id, "run_1");
        assert!(matches!(ev, AgentEvent::TextChunk(t) if t == "hi"));
    }

    #[test]
    fn parse_agent_end_state() {
        parse_both_twins(
            "agent_end",
            r#"{"state":"cancelled"}"#,
            |event| match event {
                Some(AgentEvent::AgentEnd { state, error }) => {
                    assert_eq!(state.as_deref(), Some("cancelled"));
                    assert!(error.is_none());
                }
                other => panic!("expected AgentEnd, got {:?}", other),
            },
        );
    }

    #[test]
    fn parse_ping() {
        // Ping is a pass-through event (no typed member): the data path only.
        assert!(matches!(
            parsed(make_event("ping", "{}")),
            Some(AgentEvent::Ping)
        ));
    }

    #[test]
    fn parse_agent_start() {
        parse_both_twins("agent_start", r#"{"started_at_ms":175}"#, |event| {
            assert!(matches!(event, Some(AgentEvent::AgentStart)))
        });
    }

    #[test]
    fn parse_agent_end_no_error() {
        parse_both_twins("agent_end", "{}", |event| match event {
            Some(AgentEvent::AgentEnd { error, .. }) => assert!(error.is_none()),
            other => panic!("expected AgentEnd, got {:?}", other),
        });
    }

    #[test]
    fn parse_agent_end_with_error() {
        parse_both_twins(
            "agent_end",
            r#"{"error":"rate limited"}"#,
            |event| match event {
                Some(AgentEvent::AgentEnd { error, .. }) => {
                    assert_eq!(error.as_deref(), Some("rate limited"))
                }
                other => panic!("expected AgentEnd, got {:?}", other),
            },
        );
    }

    #[test]
    fn parse_text_chunk() {
        parse_both_twins(
            "text_chunk",
            r#"{"text":"Hello world"}"#,
            |event| match event {
                Some(AgentEvent::TextChunk(text)) => assert_eq!(text, "Hello world"),
                other => panic!("expected TextChunk, got {:?}", other),
            },
        );
    }

    #[test]
    fn parse_text_chunk_empty_data() {
        parse_both_twins("text_chunk", "{}", |event| match event {
            Some(AgentEvent::TextChunk(text)) => assert_eq!(text, ""),
            other => panic!("expected TextChunk, got {:?}", other),
        });
    }

    #[test]
    fn parse_thinking_start() {
        parse_both_twins("thinking_start", "{}", |event| {
            assert!(matches!(event, Some(AgentEvent::ThinkingStart)))
        });
    }

    #[test]
    fn parse_thinking_delta() {
        parse_both_twins(
            "thinking_delta",
            r#"{"text":"Let me think"}"#,
            |event| match event {
                Some(AgentEvent::ThinkingDelta(text)) => assert_eq!(text, "Let me think"),
                other => panic!("expected ThinkingDelta, got {:?}", other),
            },
        );
    }

    #[test]
    fn parse_thinking_end() {
        parse_both_twins("thinking_end", "{}", |event| {
            assert!(matches!(event, Some(AgentEvent::ThinkingEnd)))
        });
    }

    #[test]
    fn parse_tool_start() {
        parse_both_twins(
            "tool_start",
            r#"{"tool_id":"call_1","tool_name":"shell","tool_args":"{\"command\":\"ls\"}"}"#,
            |event| match event {
                Some(AgentEvent::ToolStart {
                    tool_id,
                    tool_name,
                    tool_args,
                    ..
                }) => {
                    assert_eq!(tool_id, "call_1");
                    assert_eq!(tool_name, "shell");
                    assert_eq!(tool_args.as_deref(), Some("{\"command\":\"ls\"}"));
                }
                other => panic!("expected ToolStart, got {:?}", other),
            },
        );
    }

    #[test]
    fn parse_tool_start_object_args_stay_none() {
        // Wire semantics: bridges consume string-form arguments only; an
        // object-valued tool_args is None on both paths.
        parse_both_twins(
            "tool_start",
            r#"{"tool_id":"call_1","tool_name":"shell","tool_args":{"command":"ls"}}"#,
            |event| match event {
                Some(AgentEvent::ToolStart { tool_args, .. }) => assert!(tool_args.is_none()),
                other => panic!("expected ToolStart, got {:?}", other),
            },
        );
    }

    #[test]
    fn parse_tool_start_missing_args() {
        parse_both_twins(
            "tool_start",
            r#"{"tool_id":"call_1","tool_name":"read"}"#,
            |event| match event {
                Some(AgentEvent::ToolStart { tool_args, .. }) => assert!(tool_args.is_none()),
                other => panic!("expected ToolStart, got {:?}", other),
            },
        );
    }

    #[test]
    fn parse_tool_start_invalid_json() {
        assert!(parsed(make_event("tool_start", "not json")).is_none());
    }

    #[test]
    fn parse_tool_delta() {
        parse_both_twins(
            "tool_delta",
            r#"{"tool_id":"call_1","text":"partial output"}"#,
            |event| match event {
                Some(AgentEvent::ToolDelta { tool_id, text }) => {
                    assert_eq!(tool_id, "call_1");
                    assert_eq!(text, "partial output");
                }
                other => panic!("expected ToolDelta, got {:?}", other),
            },
        );
    }

    #[test]
    fn parse_tool_end() {
        parse_both_twins(
            "tool_end",
            r#"{"tool_id":"call_1","text":"file1.txt"}"#,
            |event| match event {
                Some(AgentEvent::ToolEnd { tool_id, text }) => {
                    assert_eq!(tool_id, "call_1");
                    assert_eq!(text.as_deref(), Some("file1.txt"));
                }
                other => panic!("expected ToolEnd, got {:?}", other),
            },
        );
    }

    #[test]
    fn parse_tool_end_no_text() {
        parse_both_twins("tool_end", r#"{"tool_id":"call_1"}"#, |event| match event {
            Some(AgentEvent::ToolEnd { text, .. }) => assert!(text.is_none()),
            other => panic!("expected ToolEnd, got {:?}", other),
        });
    }

    #[test]
    fn parse_approval_request() {
        parse_both_twins(
            "approval_request",
            r#"{
                "approval_request_id": "req_1",
                "tool_id": "call_1",
                "tool_name": "shell",
                "kind": "sandbox",
                "risk_level": "high",
                "title": "Dangerous command",
                "summary": "rm -rf /",
                "requested_action": {"command": "rm -rf /"}
            }"#,
            |event| match event {
                Some(AgentEvent::ApprovalRequest {
                    approval_request_id,
                    tool_name,
                    risk_level,
                    title,
                    summary,
                    requested_action,
                    ..
                }) => {
                    assert_eq!(approval_request_id, "req_1");
                    assert_eq!(tool_name, "shell");
                    assert_eq!(risk_level, "high");
                    assert_eq!(title, "Dangerous command");
                    assert_eq!(summary, "rm -rf /");
                    assert_eq!(requested_action["command"], "rm -rf /");
                }
                other => panic!("expected ApprovalRequest, got {:?}", other),
            },
        );
    }

    #[test]
    fn parse_error_event() {
        parse_both_twins(
            "error",
            r#"{"error":"something went wrong"}"#,
            |event| match event {
                Some(AgentEvent::Error(msg)) => assert_eq!(msg, "something went wrong"),
                other => panic!("expected Error, got {:?}", other),
            },
        );
    }

    #[test]
    fn parse_error_event_invalid_json() {
        match parsed(make_event("error", "not json")) {
            Some(AgentEvent::Error(msg)) => assert_eq!(msg, "unknown error"),
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[test]
    fn parse_unknown_event_returns_none() {
        assert!(parsed(make_event("custom_event", "{}")).is_none());
    }

    #[test]
    fn parse_empty_type_returns_none() {
        assert!(parsed(make_event("", "{}")).is_none());
    }
}
