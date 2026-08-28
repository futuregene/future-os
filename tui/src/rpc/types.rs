//! RPC types for FutureAgent communication — 1:1 port of `tui/src/rpc/types.ts`.
//!
//! The wire format is JSON (the agent's `RpcResponse.data`), so the typed
//! surface below mirrors the TS interfaces exactly — including the mixed
//! casing of `get_state`'s payload (camelCase for most sub-objects, snake_case
//! for `recentTerminalAcks` / `requestedRun`), which the TS types hard-code.

use serde::{Deserialize, Deserializer, Serialize};

/// Tolerate `null` for fields that the agent emits as `null` on fresh
/// sessions (e.g. `"extensions": null`) — TS reads `?? []` / `?.` everywhere;
/// this mirrors that (null → default).
pub(crate) fn de_null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    let opt = Option::<T>::deserialize(d)?;
    Ok(opt.unwrap_or_default())
}

// ============================================================================
// RPC Command
// ============================================================================

/// `busyPolicy` values: "enqueue_if_busy" (default) | "supersede_session".
pub const BUSY_ENQUEUE: &str = "enqueue_if_busy";

/// `ThinkingLevel` — "off" | "minimal" | "low" | "medium" | "high" | "xhigh".
pub type ThinkingLevel = String;

// ============================================================================
// RPC Responses
// ============================================================================

/// `RunAck` from `rpc/types.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunAck {
    pub run_id: String,
    pub run_epoch: i64,
    pub accepted_state: String, // "existing" | "running" | "queued"
    #[serde(default)]
    pub run_sequence: Option<i64>,
    #[serde(default)]
    pub queue_position: Option<i64>,
}

// ============================================================================
// RPC State
// ============================================================================

/// `RpcSessionState` from `rpc/types.ts`. CamelCase keys on the wire
/// (`get_state` data), except `recentTerminalAcks` / `requestedRun` which the
/// TS types declare with snake_case keys.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcSessionState {
    #[serde(default)]
    pub agent_instance_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub thinking_level: String,
    #[serde(default)]
    pub is_streaming: bool,
    #[serde(default)]
    pub is_compacting: bool,
    #[serde(default)]
    pub session_file: Option<String>,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub session_name: Option<String>,
    #[serde(default)]
    pub explicit_session: bool,
    #[serde(default)]
    pub auto_compaction_enabled: bool,
    #[serde(default)]
    pub query_count: i64,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub permission_level: Option<String>,
    #[serde(default, deserialize_with = "de_null_default")]
    pub skills: Vec<String>,
    #[serde(default, deserialize_with = "de_null_default")]
    pub context_files: Vec<String>,
    #[serde(default, deserialize_with = "de_null_default")]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub context_tokens: Option<i64>,
    #[serde(default)]
    pub context_window: Option<i64>,
    #[serde(default)]
    pub context_percent: Option<f64>,
    #[serde(default)]
    pub tokens_in: Option<i64>,
    #[serde(default)]
    pub tokens_out: Option<i64>,
    #[serde(default)]
    pub tokens_cache_r: Option<i64>,
    #[serde(default)]
    pub tokens_cache_w: Option<i64>,
    #[serde(default)]
    pub total_cost: Option<f64>,
    #[serde(default)]
    pub active_run: Option<ActiveRunState>,
    #[serde(default, deserialize_with = "de_null_default")]
    pub queued_runs: Vec<QueuedRunState>,
    #[serde(default)]
    pub queued_count: Option<i64>,
    #[serde(default, deserialize_with = "de_null_default")]
    pub recent_terminal_acks: Vec<RecentTerminalAck>,
    #[serde(default)]
    pub requested_run: Option<RunTerminalState>,
}

/// `RecentTerminalAck` — camelCase wire keys on the wire today: the #384
/// typed-RPC decoder (`decode.rs`) emits `runId` / `runSequence` /
/// `clientRequestId` (`TerminalAck` in `packages/rpc`). The snake_case
/// aliases keep pre-#384 agents working — their JSON `data` string carried
/// `run_id` / `run_sequence` / `client_request_id` (as the TS type
/// hard-coded). Without the camelCase rename every `get_state` after the
/// session's first terminal ack failed to parse, freezing the footer at its
/// last good value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentTerminalAck {
    #[serde(alias = "run_id")]
    pub run_id: String,
    #[serde(alias = "run_sequence")]
    pub run_sequence: i64,
    #[serde(alias = "client_request_id")]
    pub client_request_id: String,
    /// "terminal" | "cancelled" | "failed".
    pub state: String,
    /// e.g. "superseded".
    pub reason: String,
}

/// `QueuedRunState` — camelCase wire keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedRunState {
    pub run_id: String,
    pub run_sequence: i64,
    pub client_request_id: String,
    #[serde(default)]
    pub state: String,
    pub queue_position: i64,
    pub accepted_at: String,
    pub display_text: String,
}

/// `ActiveRunState` — camelCase wire keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveRunState {
    pub run_id: String,
    pub epoch: i64,
    #[serde(default)]
    pub run_sequence: Option<i64>,
    /// "starting" | "running" | "cancelling" | "cancellation_stuck" |
    /// "persistence_degraded" | "finalizing".
    pub state: String,
    pub last_event_idx: i64,
}

/// `RunTerminalState` — wire keys are snake_case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTerminalState {
    pub run_id: String,
    /// "completed" | "error" | "cancelled" | "incomplete" |
    /// "interrupted_by_restart".
    pub state: String,
    pub run_tokens: i64,
    pub run_duration_ms: i64,
    #[serde(default)]
    pub error: Option<String>,
}

// ============================================================================
// Session Summary (from list_sessions)
// ============================================================================

/// `SessionSummary` from `rpc/types.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    #[serde(default, deserialize_with = "de_null_default")]
    pub cwd: String,
    #[serde(default, deserialize_with = "de_null_default")]
    pub updated_at: String,
    #[serde(default, deserialize_with = "de_null_default")]
    pub model: String,
    #[serde(default)]
    pub session_name: Option<String>,
    #[serde(default)]
    pub parent_session_id: Option<String>,
    #[serde(default)]
    pub is_streaming: Option<bool>,
    // extra field carried by list_sessions (first message preview)
    #[serde(default)]
    pub first_message: Option<String>,
}

// ============================================================================
// Model Info (from get_available_models / list_models)
// ============================================================================

/// Port of `ModelInfo` from `rpc/types.ts` (from get_available_models).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    /// Display name (was "name").
    pub label: String,
    pub provider: String,
    /// Was "image".
    #[serde(default)]
    pub supports_images: bool,
    /// Default thinking level for this model.
    #[serde(default)]
    pub thinking_level: String,
    #[serde(default)]
    pub context_window: u64,
    #[serde(default)]
    pub is_default: bool,
}

impl ModelInfo {
    /// `provider/id` when the provider is non-empty, else bare `id`
    /// (mirrors `item.provider ? \`${item.provider}/${item.id}\` : item.id` —
    /// an empty provider string is falsy in JS).
    pub fn full_id(&self) -> String {
        if self.provider.is_empty() {
            self.id.clone()
        } else {
            format!("{}/{}", self.provider, self.id)
        }
    }
}

// ============================================================================
// Agent Events
// ============================================================================

/// A single projected stream event (`AgentEvent` in grpc-client.ts). `data`
/// is the parsed `StreamEvent.data` payload (empty object when absent) —
/// the TS client spreads it over the envelope keys.
#[derive(Debug, Clone, Default)]
pub struct AgentEvent {
    pub r#type: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub epoch: i64,
    pub idx: i64,
    pub event_id: Option<String>,
    pub timestamp: Option<String>,
    pub projection_snapshot: bool,
    pub snapshot_cursor: i64,
    pub snapshot_events: Vec<ProjectedRunEvent>,
    pub data: serde_json::Value,
}

impl AgentEvent {
    /// `data.text` helper (text_chunk / tool_delta / agent_end payloads).
    pub fn text(&self) -> &str {
        self.data
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
    }
}

/// `ProjectedRunEvent` — one compressed event inside a projection snapshot.
#[derive(Debug, Clone)]
pub struct ProjectedRunEvent {
    pub r#type: String,
    pub data: String,
    pub idx: i64,
}

/// `eventListeners` receive these (mirrors grpc-client.ts `EventListener`).
pub type EventListener = Box<dyn Fn(&AgentEvent) + Send + 'static>;

#[cfg(test)]
mod tests {
    use super::*;

    fn model(provider: &str, id: &str) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            label: id.into(),
            provider: provider.into(),
            supports_images: false,
            thinking_level: "off".into(),
            context_window: 128_000,
            is_default: false,
        }
    }

    #[test]
    fn full_id_prepends_provider() {
        assert_eq!(model("openai", "gpt-4o").full_id(), "openai/gpt-4o");
    }

    #[test]
    fn full_id_with_empty_provider_is_bare_id() {
        assert_eq!(model("", "local-model").full_id(), "local-model");
    }

    #[test]
    fn get_state_parses_real_payload_with_camel_case_terminal_acks() {
        // Mirrors the real agent payload after #384 typed-RPC: camelCase
        // everywhere, including TerminalAck keys (decode.rs emits runId /
        // runSequence / clientRequestId).
        let json = r#"{
            "model": "deepseek-v4-pro",
            "thinkingLevel": "high",
            "isStreaming": true,
            "sessionId": "s1",
            "explicitSession": true,
            "autoCompactionEnabled": false,
            "queryCount": 3,
            "activeRun": {"runId": "r1", "epoch": 2, "state": "running", "lastEventIdx": 10},
            "queuedRuns": [{"runId": "r2", "runSequence": 3, "clientRequestId": "c", "state": "queued", "queuePosition": 1, "acceptedAt": "2026-08-07T00:00:00Z", "displayText": "hi"}],
            "recentTerminalAcks": [{"runId": "r0", "runSequence": 1, "clientRequestId": "x", "state": "terminal", "reason": "superseded"}]
        }"#;
        let state: RpcSessionState = serde_json::from_str(json).expect("parse");
        assert_eq!(state.model.as_deref(), Some("deepseek-v4-pro"));
        assert_eq!(state.thinking_level, "high");
        assert!(state.is_streaming);
        assert_eq!(state.session_id, "s1");
        assert!(state.explicit_session);
        assert!(!state.auto_compaction_enabled);
        assert_eq!(state.query_count, 3);
        let active = state.active_run.expect("active run");
        assert_eq!(active.run_id, "r1");
        assert_eq!(active.last_event_idx, 10);
        assert_eq!(state.queued_runs.len(), 1);
        assert_eq!(state.queued_runs[0].queue_position, 1);
        assert_eq!(state.recent_terminal_acks.len(), 1);
        assert_eq!(state.recent_terminal_acks[0].run_id, "r0");
        assert_eq!(state.recent_terminal_acks[0].reason, "superseded");
    }

    #[test]
    fn get_state_accepts_legacy_snake_case_terminal_acks() {
        // Pre-#384 agents sent the JSON `data` string with snake_case ack
        // keys; the aliases must keep those decodable too.
        let json = r#"{
            "thinkingLevel": "high",
            "recentTerminalAcks": [{"run_id": "r0", "run_sequence": 1, "client_request_id": "x", "state": "terminal", "reason": "superseded"}]
        }"#;
        let state: RpcSessionState = serde_json::from_str(json).expect("parse");
        assert_eq!(state.recent_terminal_acks.len(), 1);
        assert_eq!(state.recent_terminal_acks[0].run_id, "r0");
        assert_eq!(state.recent_terminal_acks[0].run_sequence, 1);
        assert_eq!(state.recent_terminal_acks[0].client_request_id, "x");
    }

    #[test]
    fn missing_get_state_fields_default() {
        let state: RpcSessionState =
            serde_json::from_str(r#"{"thinkingLevel": "off"}"#).expect("parse");
        assert_eq!(state.session_id, "");
        assert_eq!(state.query_count, 0);
        assert!(state.queued_runs.is_empty());
        assert!(state.active_run.is_none());
        assert_eq!(state.total_cost, None);
    }

    fn agent_event_with_data(data: serde_json::Value) -> AgentEvent {
        AgentEvent {
            r#type: "text_chunk".into(),
            session_id: None,
            run_id: None,
            epoch: 0,
            idx: 0,
            event_id: None,
            timestamp: None,
            projection_snapshot: false,
            snapshot_cursor: 0,
            snapshot_events: Vec::new(),
            data,
        }
    }

    #[test]
    fn agent_event_text_reads_data_text() {
        let ev = agent_event_with_data(serde_json::json!({"text": "hello"}));
        assert_eq!(ev.text(), "hello");
    }

    #[test]
    fn agent_event_text_defaults_to_empty() {
        let ev = agent_event_with_data(serde_json::json!({}));
        assert_eq!(ev.text(), "");
        // Non-string text is not returned either.
        let ev = agent_event_with_data(serde_json::json!({"text": 5}));
        assert_eq!(ev.text(), "");
    }
}
