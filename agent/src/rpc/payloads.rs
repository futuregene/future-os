//! Typed, serde-serialisable carriers for the big read-command payloads
//! (audit item 1). Each struct mirrors a message in `proto/future.proto`'s
//! "Response payload contracts" section — the single source of truth for the
//! shape of the corresponding `RpcResponse.data` JSON.
//!
//! Wire casing: camelCase for get_state / list_sessions / get_events_since
//! (via `rename_all = "camelCase"`); get_session_entries mirrors the on-disk
//! entry schema and stays snake_case. During the migration window the builders
//! call [`inject_legacy_aliases`] to also emit the old spellings of the keys
//! whose casing changed, so pre-migration clients keep working.

use serde::Serialize;
use serde_json::Value;

/// Duplicate canonical keys under their legacy spellings (only when the
/// canonical key is present). Keeps pre-migration clients working while new
/// consumers read the canonical casing; drop the aliases once released clients
/// that still read the legacy keys have retired.
pub fn inject_legacy_aliases(payload: &mut Value, aliases: &[(&str, &str)]) {
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    for &(canonical, legacy) in aliases {
        if let Some(value) = object.get(canonical).cloned() {
            object.insert(legacy.to_string(), value);
        }
    }
}

/// get_state payload (proto `SessionState`).
#[derive(Serialize)]
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
    /// Canonical key `sessionName`; builders also emit the legacy
    /// `session_name` alias via [`inject_legacy_aliases`].
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
    /// Canonical camelCase keys; builders also emit snake_case aliases via
    /// [`inject_legacy_aliases`].
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
#[derive(Serialize)]
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
#[derive(Serialize)]
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
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalAck {
    pub run_id: String,
    pub run_sequence: u64,
    pub client_request_id: String,
    pub state: String,
    pub reason: String,
}

/// One list_sessions row (proto `SessionSummary`). Builders emit snake_case
/// legacy aliases alongside the canonical camelCase keys. `session_name` /
/// `first_message` serialize as null when absent, matching the pre-migration
/// shape.
#[derive(Serialize)]
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
#[derive(Serialize)]
pub struct SessionEntryPayload {
    pub id: String,
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
}

/// One replayed event (get_events_since; proto `ReplayEvent`). Keys mirror
/// the StreamEvent envelope.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayEventPayload {
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: String,
    pub run_id: String,
    pub idx: i64,
    pub session_id: String,
    pub epoch: i64,
    pub event_id: String,
    pub timestamp: String,
    pub session_idx: i64,
    pub run_sequence: i64,
}

/// get_events_since payload (proto `EventsSince`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsSincePayload {
    pub run_id: String,
    pub events: Vec<ReplayEventPayload>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<ProjectionPayload>,
}

/// A compressed projection snapshot (proto `ProjectionSnapshot`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionPayload {
    pub run_id: String,
    pub cursor: i64,
    pub events: Vec<ReplayEventPayload>,
}

/// Map one broadcaster/journal event into its replay payload.
pub fn replay_event_payload(event: &crate::rpc::SseEvent) -> ReplayEventPayload {
    ReplayEventPayload {
        event_type: event.event_type.clone(),
        data: event.data.clone(),
        run_id: event.run_id.clone(),
        idx: event.idx,
        session_id: event.session_id.clone(),
        epoch: event.epoch,
        event_id: event.event_id.clone(),
        timestamp: event.timestamp.clone(),
        session_idx: event.session_idx,
        run_sequence: event.run_sequence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inject_legacy_aliases_duplicates_present_keys_only() {
        let mut value = json!({"sessionName": "demo", "queuedCount": 0});
        inject_legacy_aliases(
            &mut value,
            &[("sessionName", "session_name"), ("missing", "legacy")],
        );
        assert_eq!(value["session_name"], json!("demo"));
        assert!(value.get("legacy").is_none());
    }

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
            created_by: "gui".to_string(),
            source_meta: Value::Null,
            active_run: None,
            queued_runs: vec![],
            recent_terminal_acks: vec![],
            queued_count: 0,
            interrupted_run: None,
            requested_run: None,
            pending_approvals: vec![],
        };
        let mut value = serde_json::to_value(&payload).unwrap();
        inject_legacy_aliases(&mut value, &[("sessionName", "session_name")]);

        assert_eq!(value["agentInstanceId"], json!("agent-1"));
        assert_eq!(value["autoCompactionEnabled"], json!(true));
        assert_eq!(value["tokensCacheR"], json!(3));
        assert!(value.get("sessionName").is_none(), "empty name is omitted");
        assert!(value.get("session_name").is_none());
        assert!(value.get("extensions").is_none());
    }

    #[test]
    fn session_summary_payload_takes_legacy_aliases() {
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
        let mut value = serde_json::to_value(&payload).unwrap();
        inject_legacy_aliases(
            &mut value,
            &[
                ("sessionName", "session_name"),
                ("updatedAt", "updated_at"),
                ("parentSessionId", "parent_session_id"),
                ("firstMessage", "first_message"),
                ("queryCount", "query_count"),
                ("isStreaming", "is_streaming"),
            ],
        );
        assert_eq!(value["sessionName"], json!("My session"));
        assert_eq!(value["session_name"], json!("My session"));
        assert_eq!(value["isStreaming"], json!(true));
        assert_eq!(value["is_streaming"], json!(true));
        assert_eq!(value["updatedAt"], json!("2026-08-05 12:00:00"));
        assert_eq!(value["updated_at"], json!("2026-08-05 12:00:00"));
    }

    #[test]
    fn session_entry_payload_skips_absent_optionals() {
        let payload = SessionEntryPayload {
            id: "e1".to_string(),
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
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["content"], json!("hi"));
        for key in [
            "thinking",
            "meta",
            "tool_calls",
            "output_tokens",
            "duration_ms",
        ] {
            assert!(value.get(key).is_none(), "{key} must be omitted");
        }
    }

    #[test]
    fn terminal_ack_camel_keys_with_snake_aliases() {
        let ack = TerminalAck {
            run_id: "r1".to_string(),
            run_sequence: 7,
            client_request_id: "c1".to_string(),
            state: "cancelled".to_string(),
            reason: "superseded".to_string(),
        };
        let mut value = serde_json::to_value(&ack).unwrap();
        inject_legacy_aliases(
            &mut value,
            &[
                ("runId", "run_id"),
                ("runSequence", "run_sequence"),
                ("clientRequestId", "client_request_id"),
            ],
        );
        assert_eq!(value["runId"], json!("r1"));
        assert_eq!(value["run_id"], json!("r1"));
        assert_eq!(value["clientRequestId"], json!("c1"));
        assert_eq!(value["client_request_id"], json!("c1"));
    }
}
