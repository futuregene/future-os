//! Serde carriers for the Tier-1 event payloads — the JSON shapes the agent
//! broadcasts inside `StreamEvent.data`, lifted into typed structs so the
//! encode/decode layer shares one definition with the wire fixtures.
//!
//! Wire notes:
//! - Keys are snake_case (the journal/event vocabulary).
//! - The broadcast serializer injects a redundant `"type"` key into most
//!   event payloads; the typed form never carries it. Encoders strip it,
//!   decoders reconstruct WITHOUT it — every consumer keys off the envelope
//!   type, and the TUI explicitly strips the injected key.
//! - The broadcast serializer omits empty strings and absent optionals; the
//!   `skip_serializing_if` attributes mirror that so reconstructions match
//!   the wire shape exactly.

use serde::{Deserialize, Serialize};

// ── shared usage shape ───────────────────────────────────────────────────────

/// Token accounting (provider-reported). Optional fields appear only when
/// the provider reports them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageData {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_cost: Option<f64>,
}

// ── text streams ─────────────────────────────────────────────────────────────

/// text_chunk: one assistant text token.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TextChunkData {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
}

/// user_message: the user's prompt echoed to observers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UserMessageData {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
}

/// thinking_delta: one reasoning-stream fragment.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThinkingDeltaData {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
}

/// thinking_start / thinking_end: lifecycle markers (no payload today, but
/// tolerate a text fragment if a provider ever sends one).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThinkingMarkerData {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
}

// ── run lifecycle ────────────────────────────────────────────────────────────

/// agent_start: anchors the run's wall-clock start.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentStartData {
    #[serde(default)]
    pub started_at_ms: u64,
}

/// agent_end: authoritative run totals. All fields optional — the early
/// task-spawn failure path emits only `error`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentEndData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// The run's output-token total (wire shape nests it: `usage:
    /// {output_tokens}`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<AgentEndUsage>,
    /// "incomplete" when the stream was truncated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The `usage` sub-object of agent_end.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentEndUsage {
    #[serde(default)]
    pub output_tokens: u64,
}

/// usage: token accounting emitted at provider stop.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageEventData {
    #[serde(default)]
    pub usage: UsageData,
}

/// error: a run-level error.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ErrorEventData {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

// ── tool lifecycle ───────────────────────────────────────────────────────────

/// tool_start: tool execution began.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolStartData {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_name: String,
    /// Tool-call arguments, JSON-serialised (object or JSON-encoded string).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_args: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tc_index: Option<i32>,
}

/// tool_delta: streaming tool-argument fragment.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolDeltaData {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tc_index: Option<i32>,
}

/// tool_end: tool execution finished, with structured semantics consumers
/// must not re-parse out of `text` (shell exit codes/soft-fail, write/edit
/// target paths).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolEndData {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_soft_fail: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
}

// ── approvals ────────────────────────────────────────────────────────────────

/// approval_decision: the user's verdict on a parked approval card.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApprovalDecisionData {
    #[serde(default)]
    pub approval_request_id: String,
    #[serde(default)]
    pub tool_id: String,
    /// "approved" | "rejected" | "cancelled".
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub note: String,
}

// The approval_request card travels as the typed ApprovalRequestInfo message
// (see encode/decode): its shape overlaps get_state pendingApprovals, so one
// definition serves both paths.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_chunk_roundtrip() {
        let data: TextChunkData = serde_json::from_value(json!({"text": "hi"})).unwrap();
        assert_eq!(data.text, "hi");
        assert_eq!(serde_json::to_value(&data).unwrap(), json!({"text": "hi"}));
    }

    #[test]
    fn agent_end_sparse_failure_shape() {
        // Task-spawn failure emits only error.
        let data: AgentEndData = serde_json::from_value(json!({"error": "boom"})).unwrap();
        assert_eq!(data.error.as_deref(), Some("boom"));
        assert_eq!(data.state, None);
        assert_eq!(
            serde_json::to_value(&data).unwrap(),
            json!({"error": "boom"})
        );
    }

    #[test]
    fn agent_end_full_shape() {
        let data: AgentEndData = serde_json::from_value(json!({
            "state": "completed",
            "usage": {"output_tokens": 42},
            "duration_ms": 1234
        }))
        .unwrap();
        assert_eq!(
            data.usage.as_ref().map(|usage| usage.output_tokens),
            Some(42)
        );
        assert_eq!(data.duration_ms, Some(1234));
        assert_eq!(
            serde_json::to_value(&data).unwrap(),
            json!({
                "state": "completed",
                "usage": {"output_tokens": 42},
                "duration_ms": 1234
            })
        );
    }

    #[test]
    fn tool_end_semantics_survive() {
        let data: ToolEndData = serde_json::from_value(json!({
            "tool_id": "c1",
            "tool_name": "shell",
            "text": "",
            "exit_code": 1,
            "is_soft_fail": true
        }))
        .unwrap();
        assert_eq!(data.exit_code, Some(1));
        assert_eq!(data.is_soft_fail, Some(true));
        let value = serde_json::to_value(&data).unwrap();
        assert!(value.get("text").is_none(), "empty text is omitted");
        assert_eq!(value["exit_code"], json!(1));
    }

    #[test]
    fn usage_data_keeps_optional_provider_fields() {
        let data: UsageEventData = serde_json::from_value(json!({
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150,
                "cache_read_tokens": 20,
                "credit_cost": 0.0002
            }
        }))
        .unwrap();
        assert_eq!(data.usage.cache_read_tokens, Some(20));
        assert_eq!(data.usage.credit_cost, Some(0.0002));
        assert_eq!(data.usage.cache_write_tokens, None);
    }
}
