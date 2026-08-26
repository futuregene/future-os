//! The `SessionEntry` journal-line model and the entry-type constants.

use crate::types::ToolCall;
use crate::utils::generate_entry_id;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

// Entry type constants (matching Go)
pub const ENTRY_TYPE_USER: &str = "user";
pub const ENTRY_TYPE_ASSISTANT: &str = "assistant";
pub const ENTRY_TYPE_TOOL: &str = "tool";
pub const ENTRY_TYPE_SYSTEM: &str = "system";
pub const ENTRY_TYPE_COMPACTION: &str = "compaction";
pub const ENTRY_TYPE_MODEL_CHANGE: &str = "model_change";
pub const ENTRY_TYPE_LABEL: &str = "label";
pub const ENTRY_TYPE_SESSION_INFO: &str = "session_info";
pub const ENTRY_TYPE_THINKING_LEVEL_CHANGE: &str = "thinking_level_change";
pub const ENTRY_TYPE_CUSTOM: &str = "custom";
pub const ENTRY_TYPE_CUSTOM_MESSAGE: &str = "custom_message";
/// Run lifecycle markers. These bound a run in the append-only journal:
/// `run_started` is written durably with the accepted user message, and
/// `run_terminal` is written at the run's commit boundary. A `run_started`
/// with no matching `run_terminal` identifies a run interrupted by a crash or
/// agent restart (see the restart-recovery protocol). They carry no model
/// content and are filtered out of every conversation/context projection.
pub const ENTRY_TYPE_RUN_STARTED: &str = "run_started";
pub const ENTRY_TYPE_RUN_TERMINAL: &str = "run_terminal";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(rename = "role", default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    #[serde(rename = "tool_calls", default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(
        deserialize_with = "deserialize_timestamp_lenient",
        default = "default_timestamp"
    )]
    pub timestamp: DateTime<Local>,
    #[serde(
        rename = "tool_call_id",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_args: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thinking: String,
    /// Structured per-entry metadata (not model-visible). For user entries this
    /// carries `{ "attachments": [{ path, kind, name }] }` — the files the user
    /// attached, referenced by original absolute path (never copied). Populated
    /// from `AgentMessage.metadata`; absent on entries without metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

/// Lenient timestamp deserializer: tries standard ISO 8601 first, then
/// falls back to appending the local timezone offset when the string is
/// missing one (common in hand-edited or migrated JSONL files). If both
/// fail, returns the current local time so the session entry is at least
/// loadable rather than dropped silently.
fn deserialize_timestamp_lenient<'de, D>(deserializer: D) -> Result<DateTime<Local>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    // Standard ISO 8601 (with timezone). chrono's `parse_from_rfc3339` is
    // lenient about the date/time separator, so the common space-separated
    // variant ("2024-01-02 03:04:05+08:00", with or without a fraction)
    // already parses here — a dedicated space-separator branch would be
    // unreachable (verified empirically against the pinned chrono).
    if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
        return Ok(dt.with_timezone(&chrono::Local));
    }
    // Try appending local timezone offset.
    let local_offset = chrono::Local::now().offset().to_string();
    let with_tz = format!("{s}{local_offset}");
    if let Ok(dt) = DateTime::parse_from_rfc3339(&with_tz) {
        tracing::warn!(
            "Session entry had timestamp without timezone (\"{s}\"); \
             repaired to \"{with_tz}\". Consider fixing the source file."
        );
        return Ok(dt.with_timezone(&chrono::Local));
    }
    // Last resort: current time so the entry isn't lost.
    tracing::warn!(
        "Session entry has unparseable timestamp (\"{s}\"); \
         falling back to current time."
    );
    Ok(chrono::Local::now())
}

fn default_timestamp() -> DateTime<Local> {
    chrono::Local::now()
}

impl SessionEntry {
    pub fn new_user(role: &str, content: serde_json::Value) -> Self {
        Self {
            id: generate_entry_id(),
            entry_type: ENTRY_TYPE_USER.to_string(),
            role: role.to_string(),
            content: Some(content),
            tool_calls: vec![],
            timestamp: Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }

    pub fn new_assistant(content: serde_json::Value, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            id: generate_entry_id(),
            entry_type: ENTRY_TYPE_ASSISTANT.to_string(),
            role: "assistant".to_string(),
            content: Some(content),
            tool_calls,
            timestamp: Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }

    pub fn new_tool(call_id: &str, content: &str) -> Self {
        Self {
            id: generate_entry_id(),
            entry_type: ENTRY_TYPE_TOOL.to_string(),
            role: "tool".to_string(),
            content: Some(serde_json::json!(content)),
            tool_calls: vec![],
            timestamp: Local::now(),
            tool_call_id: call_id.to_string(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }

    /// Build the `session_info` metadata entry prepended to every saved session.
    /// `content` holds the token/cost/name JSON snapshot; `model`/`thinking_level`
    /// pin the session's active settings. All other fields take entry defaults.
    pub fn session_info(
        content: serde_json::Value,
        _model: String,
        _thinking_level: String,
    ) -> Self {
        Self {
            id: generate_entry_id(),
            entry_type: ENTRY_TYPE_SESSION_INFO.to_string(),
            role: ENTRY_TYPE_SYSTEM.to_string(),
            content: Some(content),
            tool_calls: vec![],
            timestamp: Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }

    /// Marker written durably with the accepted user message to record that a
    /// run with this canonical id began. `content` carries `{ run_id, epoch }`.
    pub fn run_started(run_id: &str, epoch: u64) -> Self {
        Self::run_started_with_sequence(run_id, epoch, None)
    }

    pub fn run_started_with_sequence(run_id: &str, epoch: u64, run_sequence: Option<u64>) -> Self {
        let mut content = serde_json::json!({ "run_id": run_id, "epoch": epoch });
        if let Some(sequence) = run_sequence {
            content["run_sequence"] = serde_json::json!(sequence);
        }
        Self {
            id: generate_entry_id(),
            entry_type: ENTRY_TYPE_RUN_STARTED.to_string(),
            role: ENTRY_TYPE_SYSTEM.to_string(),
            content: Some(content),
            tool_calls: vec![],
            timestamp: Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }

    /// Marker written at a run's commit boundary. `content` carries
    /// `{ run_id, state, run_tokens, run_duration_ms }` plus `error` when
    /// `state` is `error`, plus `truncation` when `state` is `incomplete`.
    /// A run is only recoverable as completed once this marker is durable.
    pub fn run_terminal(
        run_id: &str,
        state: &str,
        run_tokens: i64,
        run_duration_ms: i64,
        error: Option<&str>,
    ) -> Self {
        Self::run_terminal_with_truncation(run_id, state, run_tokens, run_duration_ms, error, None)
    }

    /// `run_terminal` with an optional truncation context (set only when the
    /// run ended `incomplete` because the model stream cut off). The context
    /// records how far the run had progressed so consumers can tell "cut off
    /// mid-work" from "model went silent immediately".
    pub fn run_terminal_with_truncation(
        run_id: &str,
        state: &str,
        run_tokens: i64,
        run_duration_ms: i64,
        error: Option<&str>,
        truncation: Option<&crate::agent::StreamTruncation>,
    ) -> Self {
        let mut content = serde_json::json!({
            "run_id": run_id,
            "state": state,
            "run_tokens": run_tokens,
            "run_duration_ms": run_duration_ms,
        });
        if let Some(error) = error {
            content["error"] = serde_json::Value::String(error.to_string());
        }
        if let Some(t) = truncation {
            content["truncation"] = serde_json::json!({
                "turns_so_far": t.turns_so_far,
                "output_len": t.output_len,
                "tool_calls_so_far": t.tool_calls_so_far,
                "detected_by": t.detected_by,
            });
        }
        Self {
            id: generate_entry_id(),
            entry_type: ENTRY_TYPE_RUN_TERMINAL.to_string(),
            role: ENTRY_TYPE_SYSTEM.to_string(),
            content: Some(content),
            tool_calls: vec![],
            timestamp: Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── SessionEntry constructors ──────────────────────────────────────────

    #[test]
    fn new_user_entry() {
        let e = SessionEntry::new_user("user", serde_json::json!("hello"));
        assert_eq!(e.entry_type, ENTRY_TYPE_USER);
        assert_eq!(e.role, "user");
        assert!(!e.id.is_empty());
    }

    #[test]
    fn new_assistant_entry() {
        let tool_calls = vec![crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "shell".to_string(),
                arguments: serde_json::json!({"cmd": "ls"}),
            },
        }];
        let e = SessionEntry::new_assistant(serde_json::json!("answer"), tool_calls);
        assert_eq!(e.entry_type, ENTRY_TYPE_ASSISTANT);
        assert_eq!(e.role, "assistant");
        assert_eq!(e.tool_calls.len(), 1);
        assert_eq!(e.tool_calls[0].function.name, "shell");
    }

    #[test]
    fn new_tool_entry() {
        let e = SessionEntry::new_tool("call_123", "file contents here");
        assert_eq!(e.entry_type, ENTRY_TYPE_TOOL);
        assert_eq!(e.role, "tool");
        assert_eq!(e.tool_call_id, "call_123");
    }

    #[test]
    fn session_info_entry() {
        let content = serde_json::json!({"session_name": "test", "model": "gpt-4o", "thinking_level": "high"});
        let e = SessionEntry::session_info(content, "gpt-4o".to_string(), "high".to_string());
        assert_eq!(e.entry_type, ENTRY_TYPE_SESSION_INFO);
        assert_eq!(e.role, ENTRY_TYPE_SYSTEM);
        let c = e.content.as_ref().unwrap();
        assert_eq!(c["model"], "gpt-4o");
        assert_eq!(c["thinking_level"], "high");
    }

    #[test]
    fn entry_ids_are_unique() {
        let e1 = SessionEntry::new_user("user", serde_json::json!("a"));
        let e2 = SessionEntry::new_user("user", serde_json::json!("b"));
        assert_ne!(e1.id, e2.id);
    }

    // ─── lenient timestamp ──────────────────────────────────────────────────

    #[test]
    fn deserialize_timestamp_space_separator() {
        let json = r#"{"id":"t","type":"u","timestamp":"2026-07-23 10:30:00+08:00"}"#;
        let entry: SessionEntry = serde_json::from_str(json).unwrap();
        // Timezone conversion depends on CI location, just verify it parses
        assert!(entry.timestamp.timestamp() > 0);
    }

    #[test]
    fn deserialize_timestamp_with_fractional_space() {
        let json = r#"{"id":"t","type":"u","timestamp":"2026-07-23 10:30:00.500+08:00"}"#;
        let entry: SessionEntry = serde_json::from_str(json).unwrap();
        assert!(entry.timestamp.timestamp() > 0);
    }

    #[test]
    fn deserialize_timestamp_unparseable_falls_back() {
        let json = r#"{"id":"t","type":"u","timestamp":"not-a-timestamp"}"#;
        let entry: SessionEntry = serde_json::from_str(json).unwrap();
        // Should fall back to current time (not an error)
        let now = chrono::Local::now();
        let diff = (now - entry.timestamp).num_seconds().abs();
        assert!(diff < 5, "fallback time should be close to now");
    }

    #[test]
    fn deserialize_timestamp_space_variants_and_default() {
        // Space separator with timezone (no fraction).
        let entry: SessionEntry = serde_json::from_str(
            r#"{"id":"t","type":"user","role":"user","timestamp":"2026-07-17 12:44:27+08:00"}"#,
        )
        .unwrap();
        assert_eq!(entry.timestamp.format("%Y").to_string(), "2026");
        // Space separator with fraction and timezone.
        let entry: SessionEntry = serde_json::from_str(
            r#"{"id":"t","type":"user","role":"user","timestamp":"2026-07-17 12:44:27.161+08:00"}"#,
        )
        .unwrap();
        assert_eq!(entry.timestamp.format("%Y").to_string(), "2026");
        // Missing timestamp → default (now).
        let entry: SessionEntry =
            serde_json::from_str(r#"{"id":"t","type":"user","role":"user"}"#).unwrap();
        let age = chrono::Local::now() - entry.timestamp;
        assert!(age.num_seconds() < 60);
    }

    fn parse_lenient_ts(ts: &str) -> DateTime<Local> {
        use serde::de::IntoDeserializer;
        let de: serde::de::value::StrDeserializer<serde::de::value::Error> = ts.into_deserializer();
        deserialize_timestamp_lenient(de).unwrap()
    }

    #[test]
    fn lenient_timestamp_space_separated_variants() {
        // Space separator + fractional seconds + colon offset. Compare in UTC
        // so the assertion is independent of the runner's local timezone
        // (CI runs UTC; a local-time assertion would read +08:00's date).
        let dt = parse_lenient_ts("2024-01-02 03:04:05.123+08:00");
        assert_eq!(
            dt.with_timezone(&chrono::Utc)
                .format("%Y-%m-%d")
                .to_string(),
            "2024-01-01"
        );
        // chrono's `%.f` consumes the fraction only when present, so the
        // fraction-less spelling parses through the SAME variant — this is
        // why no separate fraction-less branch exists below.
        let dt = parse_lenient_ts("2024-01-02 03:04:05+08:00");
        assert_eq!(
            dt.with_timezone(&chrono::Utc)
                .format("%H:%M:%S")
                .to_string(),
            "19:04:05"
        );
    }
}
