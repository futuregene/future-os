//! Serde-serialisable carriers for the big read-command payloads — the wire
//! payload half of the typed-RPC contract. Each struct mirrors a message in
//! `packages/rpc/proto/future.proto`'s "Response payload contracts" section and is the
//! single source for constructing the corresponding `RpcResponse.data` JSON
//! (agent side, Serialize) AND for decoding it back (client fallback path,
//! Deserialize). Because both directions land in the same types, the JSON
//! fallback and the typed proto path stay in parity by construction.
//!
//! Wire casing: camelCase for get_state / list_sessions / get_events_since
//! (via `rename_all = "camelCase"`); get_session_entries mirrors the on-disk
//! entry schema and stays snake_case. The legacy casing migration window is
//! retired — canonical camelCase keys only.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// get_state payload (proto `SessionState`).
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetStatePayload {
    pub agent_instance_id: String,
    pub model: String,
    pub image_support: bool,
    pub thinking_level: String,
    pub is_streaming: bool,
    pub is_compacting: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Canonical key `sessionName`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    pub explicit_session: bool,
    pub auto_compaction_enabled: bool,
    pub query_count: usize,
    pub version: String,
    pub cwd: String,
    pub skills: Vec<String>,
    pub context_files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
    pub context_window: i64,
    pub context_tokens: i64,
    pub context_percent: f64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub tokens_cache_r: i64,
    pub tokens_cache_w: i64,
    pub total_cost: f64,
    pub permission_level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub created_by: String,
    /// Free-form source metadata as recorded at creation — carried as the raw
    /// JSON value the session stores (string, object, or null).
    pub source_meta: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_run: Option<RunStateSnapshot>,
    pub queued_runs: Vec<QueuedRunState>,
    /// Canonical camelCase keys.
    pub recent_terminal_acks: Vec<TerminalAck>,
    pub queued_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupted_run: Option<RunStateSnapshot>,
    /// Terminal journal content of the requested run (run_terminal shape:
    /// run_id, state, run_tokens, run_duration_ms, error?).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_run: Option<Value>,
    /// Approval-request card payloads the session is parked on.
    pub pending_approvals: Vec<Value>,
}

/// A run's live state (get_state `activeRun` / `interruptedRun`; proto
/// `RunStateSnapshot`).
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStateSnapshot {
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    // Emitted as null (not omitted) when unknown, matching the pre-migration
    // shape of activeRun.
    pub run_sequence: Option<u64>,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_idx: Option<i64>,
}

/// A queued run (get_state `queuedRuns`; proto `QueuedRunState`).
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedRunState {
    pub run_id: String,
    pub run_sequence: u64,
    pub client_request_id: String,
    pub state: String,
    pub queue_position: usize,
    pub accepted_at: String,
    pub display_text: String,
}

/// A terminal acknowledgement (get_state `recentTerminalAcks`; proto
/// `TerminalAck`).
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalAck {
    pub run_id: String,
    pub run_sequence: u64,
    pub client_request_id: String,
    pub state: String,
    pub reason: String,
}

/// One list_sessions row (proto `SessionSummary`). `session_name` /
/// `first_message` serialize as null when absent.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummaryPayload {
    pub id: String,
    pub session_name: Option<String>,
    pub model: String,
    pub cwd: String,
    pub updated_at: String,
    pub parent_session_id: String,
    pub first_message: Option<String>,
    pub query_count: usize,
    pub is_streaming: bool,
}

/// One displayable session entry (get_session_entries; proto `SessionEntry`).
/// Field names match the on-disk JSONL schema (snake_case) — `content` is the
/// display text for message entries and the raw session_info JSON object for
/// the session_info entry.
#[derive(Serialize, Deserialize)]
pub struct SessionEntryPayload {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_type: Option<String>,
    pub role: String,
    pub content: Value,
    pub name: String,
    pub tool_args: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result_is_error: Option<bool>,
}

/// One page returned by `get_session_entries`.
#[derive(Serialize, Deserialize)]
pub struct SessionEntriesPage {
    pub entries: Vec<SessionEntryPayload>,
    #[serde(default, rename = "hasMore", alias = "has_more")]
    pub has_more: bool,
    #[serde(default, rename = "nextOffset", alias = "next_offset")]
    pub next_offset: i64,
}

/// One replayed event (get_events_since; proto `ReplayEvent`). Keys mirror
/// the StreamEvent envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayEventPayload {
    #[serde(rename = "type")]
    #[serde(default)]
    pub event_type: String,
    #[serde(default)]
    pub data: String,
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub idx: i64,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub epoch: i64,
    #[serde(default)]
    pub event_id: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub session_idx: i64,
    #[serde(default)]
    pub run_sequence: i64,
}

/// get_events_since payload (proto `EventsSince`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsSincePayload {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub events: Vec<ReplayEventPayload>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<ProjectionPayload>,
    /// True when the page was cut short (request `max_events` / server size
    /// budget) and more events follow. Absent on the wire when false so
    /// legacy fixtures and old agents round-trip unchanged.
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_more: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// A compressed projection snapshot (proto `ProjectionSnapshot`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionPayload {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub cursor: i64,
    #[serde(default)]
    pub events: Vec<ReplayEventPayload>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn get_state_payload_uses_camel_case_and_skips_absent_optionals() {
        let payload = GetStatePayload {
            agent_instance_id: "agent-1".to_string(),
            model: "m".to_string(),
            image_support: true,
            thinking_level: "medium".to_string(),
            is_streaming: false,
            is_compacting: false,
            session_file: Some(String::new()),
            session_id: Some("s1".to_string()),
            session_name: None,
            explicit_session: true,
            auto_compaction_enabled: true,
            query_count: 2,
            version: "v".to_string(),
            cwd: "/w".to_string(),
            skills: vec![],
            context_files: vec![],
            extensions: None,
            context_window: 200_000,
            context_tokens: 10,
            context_percent: 0.5,
            tokens_in: 1,
            tokens_out: 2,
            tokens_cache_r: 3,
            tokens_cache_w: 4,
            total_cost: 0.1,
            permission_level: "workspace".to_string(),
            parent_session_id: None,
            created_by: "desktop".to_string(),
            source_meta: Value::Null,
            active_run: None,
            queued_runs: vec![],
            recent_terminal_acks: vec![],
            queued_count: 0,
            interrupted_run: None,
            requested_run: None,
            pending_approvals: vec![],
        };
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["agentInstanceId"], json!("agent-1"));
        assert_eq!(value["autoCompactionEnabled"], json!(true));
        assert_eq!(value["tokensCacheR"], json!(3));
        assert!(value.get("sessionName").is_none(), "empty name is omitted");
        assert!(value.get("session_name").is_none(), "no legacy alias");
        assert!(value.get("extensions").is_none());
    }

    #[test]
    fn session_summary_payload_camel_case() {
        let payload = SessionSummaryPayload {
            id: "s1".to_string(),
            session_name: Some("My session".to_string()),
            model: "m".to_string(),
            cwd: "/w".to_string(),
            updated_at: "2026-08-05 12:00:00".to_string(),
            parent_session_id: String::new(),
            first_message: Some("hello".to_string()),
            query_count: 1,
            is_streaming: true,
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["sessionName"], json!("My session"));
        assert!(value.get("session_name").is_none(), "no legacy alias");
        assert_eq!(value["isStreaming"], json!(true));
        assert!(value.get("is_streaming").is_none());
        assert_eq!(value["updatedAt"], json!("2026-08-05 12:00:00"));
        assert!(value.get("updated_at").is_none());
    }

    #[test]
    fn session_entry_payload_skips_absent_optionals() {
        let payload = SessionEntryPayload {
            id: "e1".to_string(),
            entry_type: None,
            role: "user".to_string(),
            content: Value::String("hi".to_string()),
            name: String::new(),
            tool_args: String::new(),
            timestamp: "2026-08-05T12:00:00+08:00".to_string(),
            thinking: None,
            meta: None,
            tool_calls: None,
            output_tokens: None,
            duration_ms: None,
            input_tokens: None,
            cache_read_tokens: None,
            checkpoint: None,
            tool_call_id: None,
            tool_result_is_error: None,
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["content"], json!("hi"));
        for key in [
            "thinking",
            "meta",
            "tool_calls",
            "output_tokens",
            "duration_ms",
            "input_tokens",
            "cache_read_tokens",
            "tool_call_id",
            "tool_result_is_error",
        ] {
            assert!(value.get(key).is_none(), "{key} must be omitted");
        }
    }

    #[test]
    fn terminal_ack_camel_case_keys() {
        let ack = TerminalAck {
            run_id: "r1".to_string(),
            run_sequence: 7,
            client_request_id: "c1".to_string(),
            state: "cancelled".to_string(),
            reason: "superseded".to_string(),
        };
        let value = serde_json::to_value(&ack).unwrap();
        assert_eq!(value["runId"], json!("r1"));
        assert!(value.get("run_id").is_none(), "no legacy alias");
        assert_eq!(value["clientRequestId"], json!("c1"));
        assert!(value.get("client_request_id").is_none());
    }

    /// Canonical camelCase JSON must round-trip through the identical struct.
    #[test]
    fn get_state_roundtrip_canonical_json() {
        let payload = GetStatePayload {
            agent_instance_id: "agent-1".to_string(),
            model: "m".to_string(),
            image_support: true,
            thinking_level: "medium".to_string(),
            is_streaming: true,
            is_compacting: false,
            session_file: None,
            session_id: Some("s1".to_string()),
            session_name: Some("Demo".to_string()),
            explicit_session: true,
            auto_compaction_enabled: true,
            query_count: 2,
            version: "v".to_string(),
            cwd: "/w".to_string(),
            skills: vec!["skill".to_string()],
            context_files: vec![],
            extensions: None,
            context_window: 200_000,
            context_tokens: 10,
            context_percent: 0.5,
            tokens_in: 1,
            tokens_out: 2,
            tokens_cache_r: 3,
            tokens_cache_w: 4,
            total_cost: 0.1,
            permission_level: "workspace".to_string(),
            parent_session_id: None,
            created_by: "desktop".to_string(),
            source_meta: json!({"thread": "t1"}),
            active_run: Some(RunStateSnapshot {
                run_id: "r1".to_string(),
                epoch: Some(1),
                run_sequence: None,
                state: "running".to_string(),
                last_event_idx: Some(4),
            }),
            queued_runs: vec![],
            recent_terminal_acks: vec![TerminalAck {
                run_id: "r0".to_string(),
                run_sequence: 3,
                client_request_id: "c0".to_string(),
                state: "cancelled".to_string(),
                reason: "superseded".to_string(),
            }],
            queued_count: 0,
            interrupted_run: None,
            requested_run: None,
            pending_approvals: vec![],
        };
        let canonical = serde_json::to_value(&payload).unwrap();
        let decoded: GetStatePayload = serde_json::from_value(canonical.clone()).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), canonical);
    }
}
