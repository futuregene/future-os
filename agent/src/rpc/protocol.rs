use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

// ─── RPC Command (stdin) ────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcCommand {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type")]
    pub cmd_type: String,

    // Prompting
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub images: Vec<crate::types::ImageContent>,
    #[serde(default)]
    pub attachments: Vec<crate::types::Attachment>,
    #[serde(default)]
    pub streaming_behavior: String,
    #[serde(default)]
    pub parent_session: String,

    // set_model
    #[serde(default)]
    pub model_id: String,

    // set_thinking_level
    #[serde(default)]
    pub level: String,

    // set_steering_mode / set_follow_up_mode
    #[serde(default)]
    pub mode: String,

    // compact
    #[serde(default)]
    pub custom_instructions: String,

    // set_auto_compaction / set_auto_retry
    #[serde(default)]
    pub enabled: bool,

    // shell
    #[serde(default)]
    pub command: String,

    // Session
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub entry_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cwd: String,

    // set_system_prompt
    #[serde(default)]
    pub system_prompt: String,

    // set_tools / disable_tools
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    // set_ephemeral
    pub ephemeral: bool,

    // set_enabled_models
    #[serde(default)]
    pub enabled_models: Option<Vec<String>>,

    // get_events_since (P1)
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub since_idx: i64,

    // set_sandbox_policy — populated from the typed proto sub-message by the
    // gRPC layer (not part of the JSON command surface).
    #[serde(skip)]
    pub sandbox_policy: Option<crate::sandbox::SandboxPolicy>,
}

// ─── RPC Response (stdout) ───────────────────────────────────────────────

// ─── RPC Response (stdout) ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    #[serde(rename = "type")]
    pub resp_type: String,
    #[serde(default)]
    pub id: String,
    pub command: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl RpcResponse {
    pub(super) fn ok(id: &str, command: &str, data: impl Into<serde_json::Value>) -> String {
        let resp = Self {
            resp_type: "response".to_string(),
            id: id.to_string(),
            command: command.to_string(),
            success: true,
            data: Some(data.into()),
            error: None,
        };
        serde_json::to_string(&resp).unwrap_or_default()
    }

    pub fn build_fail(id: &str, command: &str, err: &str) -> String {
        let resp = Self {
            resp_type: "response".to_string(),
            id: id.to_string(),
            command: command.to_string(),
            success: false,
            data: None,
            error: Some(err.to_string()),
        };
        serde_json::to_string(&resp).unwrap_or_default()
    }
}

// ─── SSE Event Broadcaster ──────────────────────────────────────────────

/// Max buffered events per run (for `events_since` backfill). Oldest dropped.
/// Only the *current* run is buffered (cleared on `start_run`), so this is a
/// per-session ceiling, not cumulative. Sized to comfortably hold a long
/// generation's per-token `text_chunk` stream; on overflow the oldest are
/// dropped and `events_since` reports the resulting gap via `min_idx`.
/// Max events buffered per run for `events_since` resync.
/// 2000 is sufficient — a client that falls behind 2000 events
/// is effectively disconnected and should reconnect.
const MAX_RUN_EVENTS: usize = 2_000;

struct RunState {
    run_id: String,
    idx: i64,
    events: Vec<SseEvent>,
}

/// Per-session SSE broadcaster. Also the **single stamping point** (P1): it
/// assigns each event's `run_id` + monotonic `idx` and buffers the current run
/// for `events_since` — all under one lock, so broadcast order matches idx order.
#[derive(Clone)]
pub struct SseBroadcaster {
    tx: broadcast::Sender<SseEvent>,
    run: std::sync::Arc<parking_lot::Mutex<RunState>>,
}

impl SseBroadcaster {
    pub fn new() -> Self {
        // 256 slots is enough — measured rate is ~15-30 events/sec during
        // streaming, so 256 slots tolerates ~10s of client lag.  A client
        // behind by more than 256 events is effectively disconnected and
        // should resync via `events_since` anyway.
        let (tx, _) = broadcast::channel(256);
        Self {
            tx,
            run: std::sync::Arc::new(parking_lot::Mutex::new(RunState {
                run_id: String::new(),
                idx: 0,
                events: Vec::new(),
            })),
        }
    }

    /// Subscribe to SSE events
    pub fn subscribe(&self) -> broadcast::Receiver<SseEvent> {
        self.tx.subscribe()
    }

    /// Stamp `run_id` + monotonic `idx`, buffer the event, and broadcast — all
    /// under one lock so stream order matches idx order (no reordering race).
    pub fn broadcast(&self, mut event: SseEvent) {
        let mut run = self.run.lock();
        event.run_id = run.run_id.clone();
        event.idx = run.idx;
        run.idx += 1;
        run.events.push(event.clone());
        if run.events.len() > MAX_RUN_EVENTS {
            let overflow = run.events.len() - MAX_RUN_EVENTS;
            run.events.drain(0..overflow);
        }
        // broadcast::Sender::send returns Err(SendError) when the ring buffer
        // is full (all receivers are >256 events behind).  This is expected
        // for slow/disconnected clients — warn so it's diagnosable.
        if let Err(tokio::sync::broadcast::error::SendError(_)) = self.tx.send(event) {
            tracing::warn!(
                "SSE broadcast channel full (256 events) — slow client(s) will receive Lagged"
            );
        }
    }

    /// Begin a new user run: set `run_id`, reset `idx`, clear the buffer.
    pub fn start_run(&self, run_id: String) {
        let mut run = self.run.lock();
        run.run_id = run_id;
        run.idx = 0;
        run.events.clear();
    }

    /// Current-run events with `idx > since_idx`, plus the earliest idx still in
    /// the buffer (`min_idx`, 0 if empty). If `run_id` no longer matches (a new
    /// run started), return the current run_id + all its buffered events. A
    /// full backfill (`since_idx < 0`) whose result starts above `min_idx == 0`
    /// — i.e. `min_idx > 0` — means the run's prefix was dropped on overflow, so
    /// the caller can surface the gap instead of silently reconstructing a
    /// truncated message.
    pub fn events_since(&self, run_id: &str, since_idx: i64) -> (String, Vec<SseEvent>, i64) {
        let run = self.run.lock();
        let min_idx = run.events.first().map(|e| e.idx).unwrap_or(0);
        if run.run_id == run_id {
            let events = run
                .events
                .iter()
                .filter(|e| e.idx > since_idx)
                .cloned()
                .collect();
            (run.run_id.clone(), events, min_idx)
        } else {
            (run.run_id.clone(), run.events.clone(), min_idx)
        }
    }
}

impl Default for SseBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

/// SSE Event structure
#[derive(Debug, Clone, Default)]
pub struct SseEvent {
    pub event_type: String,
    pub data: String,
    /// P1: stamped by `SseBroadcaster::broadcast` (callers leave default).
    pub run_id: String,
    pub idx: i64,
}

impl SseEvent {
    pub fn new(event_type: &str, data: serde_json::Value) -> Self {
        Self {
            event_type: event_type.to_string(),
            data: serde_json::to_string(&data).unwrap_or_default(),
            run_id: String::new(),
            idx: 0,
        }
    }
}

// ─── Approval Gate ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── RpcCommand deserialization ──────────────────────────────────────────

    #[test]
    fn rpc_command_minimal() {
        let json = r#"{"id":"cmd1","type":"get_state","sessionId":"s1"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.id, "cmd1");
        assert_eq!(cmd.cmd_type, "get_state");
        assert_eq!(cmd.session_id, "s1");
        assert!(cmd.message.is_empty());
    }

    #[test]
    fn rpc_command_prompt() {
        let json = r#"{
            "id": "cmd2",
            "type": "prompt",
            "sessionId": "s1",
            "message": "hello",
            "streamingBehavior": "realtime"
        }"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.cmd_type, "prompt");
        assert_eq!(cmd.message, "hello");
        assert_eq!(cmd.streaming_behavior, "realtime");
    }

    #[test]
    fn rpc_command_set_model() {
        let json = r#"{"id":"cmd3","type":"set_model","sessionId":"s1","modelId":"openai/gpt-4o"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.model_id, "openai/gpt-4o");
    }

    #[test]
    fn rpc_command_thinking_level() {
        let json = r#"{"id":"cmd4","type":"set_thinking_level","sessionId":"s1","level":"high"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.level, "high");
    }

    #[test]
    fn rpc_command_mode_field() {
        let json = r#"{"id":"cmd5","type":"set_steering_mode","sessionId":"s1","mode":"auto"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.mode, "auto");
    }

    #[test]
    fn rpc_command_shell() {
        let json = r#"{"id":"cmd6","type":"shell","sessionId":"s1","command":"ls -la"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.command, "ls -la");
    }

    #[test]
    fn rpc_command_cwd() {
        let json = r#"{"id":"cmd7","type":"set_cwd","sessionId":"s1","cwd":"/tmp/project"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.cwd, "/tmp/project");
    }

    #[test]
    fn rpc_command_enabled_flag() {
        let json = r#"{"id":"cmd8","type":"set_auto_compaction","sessionId":"s1","enabled":true}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert!(cmd.enabled);
    }

    #[test]
    fn rpc_command_disabled_flag() {
        let json =
            r#"{"id":"cmd8b","type":"set_auto_compaction","sessionId":"s1","enabled":false}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert!(!cmd.enabled);
    }

    #[test]
    fn rpc_command_new_session_defaults() {
        let json = r#"{"id":"cmd9","type":"new_session"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert!(cmd.session_id.is_empty());
        assert!(cmd.cwd.is_empty());
        assert!(cmd.model_id.is_empty());
        assert!(cmd.custom_instructions.is_empty());
    }

    #[test]
    fn rpc_command_system_prompt() {
        let json = r#"{"id":"cmd10","type":"set_system_prompt","sessionId":"s1","systemPrompt":"You are helpful"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.system_prompt, "You are helpful");
    }

    #[test]
    fn rpc_command_tools_list() {
        let json = r#"{"id":"cmd11","type":"set_tools","sessionId":"s1","tools":["shell","read","write"]}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.tools, vec!["shell", "read", "write"]);
    }

    #[test]
    fn rpc_command_entry_id() {
        let json = r#"{"id":"cmd12","type":"fork","sessionId":"s1","entryId":"entry_abc"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.entry_id, "entry_abc");
    }

    #[test]
    fn rpc_command_name() {
        let json =
            r#"{"id":"cmd13","type":"set_session_name","sessionId":"s1","name":"My Session"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.name, "My Session");
    }

    #[test]
    fn rpc_command_ephemeral() {
        let json = r#"{"id":"cmd14","type":"set_ephemeral","sessionId":"s1","ephemeral":true}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert!(cmd.ephemeral);
    }

    #[test]
    fn rpc_command_events_since() {
        let json = r#"{"id":"cmd15","type":"get_events_since","sessionId":"s1","runId":"run_1","sinceIdx":5}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.run_id, "run_1");
        assert_eq!(cmd.since_idx, 5);
    }

    #[test]
    fn rpc_command_parent_session() {
        let json = r#"{"id":"cmd16","type":"fork","sessionId":"s1","parentSession":"parent_1","entryId":"e1"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.parent_session, "parent_1");
    }

    #[test]
    fn rpc_command_approval_mode() {
        let json = r#"{"id":"cmd17","type":"approval_decision","sessionId":"s1","entryId":"req_1","mode":"approved","message":"looks safe"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.mode, "approved");
        assert_eq!(cmd.entry_id, "req_1");
        assert_eq!(cmd.message, "looks safe");
    }

    #[test]
    fn rpc_command_sandbox_policy_skipped() {
        // sandbox_policy is #[serde(skip)] — should not appear in JSON
        let json = r#"{"id":"cmd18","type":"set_sandbox_policy","sessionId":"s1"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert!(cmd.sandbox_policy.is_none());
    }

    #[test]
    fn rpc_command_compact_with_instructions() {
        let json = r#"{"id":"cmd19","type":"compact","sessionId":"s1","customInstructions":"summarize in detail"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.custom_instructions, "summarize in detail");
    }

    // ─── RpcResponse serialization ───────────────────────────────────────────

    #[test]
    fn rpc_response_ok_format() {
        let json_str = RpcResponse::ok("id1", "get_state", serde_json::json!({"model": "gpt-4o"}));
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["type"], "response");
        assert_eq!(parsed["id"], "id1");
        assert_eq!(parsed["command"], "get_state");
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["data"]["model"], "gpt-4o");
        assert!(parsed.get("error").is_none());
    }

    #[test]
    fn rpc_response_fail_format() {
        let json_str = RpcResponse::build_fail("id2", "prompt", "session not found");
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["type"], "response");
        assert_eq!(parsed["id"], "id2");
        assert_eq!(parsed["command"], "prompt");
        assert_eq!(parsed["success"], false);
        assert_eq!(parsed["error"], "session not found");
        assert!(parsed.get("data").is_none());
    }

    #[test]
    fn rpc_response_ok_null_data() {
        let json_str = RpcResponse::ok("id3", "abort", serde_json::json!({}));
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["success"], true);
        assert!(parsed["data"].is_object());
    }

    #[test]
    fn rpc_response_ok_with_complex_data() {
        let data = serde_json::json!({
            "sessions": [{"id": "s1", "name": "test"}],
            "count": 1,
            "nested": {"deep": [1, 2, 3]}
        });
        let json_str = RpcResponse::ok("id4", "list_sessions", data.clone());
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["data"]["count"], 1);
        assert_eq!(
            parsed["data"]["nested"]["deep"],
            serde_json::json!([1, 2, 3])
        );
    }

    // ─── SseEvent ────────────────────────────────────────────────────────────

    #[test]
    fn sse_event_new_sets_type_and_data() {
        let event = SseEvent::new("text_chunk", serde_json::json!({"text": "hello"}));
        assert_eq!(event.event_type, "text_chunk");
        let parsed: serde_json::Value = serde_json::from_str(&event.data).unwrap();
        assert_eq!(parsed["text"], "hello");
        assert!(event.run_id.is_empty());
        assert_eq!(event.idx, 0);
    }

    #[test]
    fn sse_event_default() {
        let event = SseEvent::default();
        assert!(event.event_type.is_empty());
        assert!(event.data.is_empty());
    }

    // ─── SseBroadcaster (P1) ────────────────────────────────────────────────

    #[test]
    fn stamps_run_id_idx_and_backfills() {
        let b = SseBroadcaster::new();
        b.start_run("run1".to_string());
        b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "a"}),
        ));
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "b"}),
        ));

        // Backfill from idx 0 → the two events after idx 0 (idx 1, 2), in order.
        let (rid, evs, min_idx) = b.events_since("run1", 0);
        assert_eq!(rid, "run1");
        assert_eq!(evs.len(), 2);
        assert_eq!((evs[0].idx, evs[1].idx), (1, 2));
        assert_eq!(evs[0].run_id, "run1");
        // Nothing dropped yet → earliest buffered idx is still 0 (no gap).
        assert_eq!(min_idx, 0);

        // From -1 → all three (idx 0,1,2).
        let (_, all, _) = b.events_since("run1", -1);
        assert_eq!(all.iter().map(|e| e.idx).collect::<Vec<_>>(), vec![0, 1, 2]);

        // New run resets idx + clears buffer.
        b.start_run("run2".to_string());
        b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        let (rid2, evs2, _) = b.events_since("run2", -1);
        assert_eq!(rid2, "run2");
        assert_eq!(evs2.len(), 1);
        assert_eq!((evs2[0].idx, evs2[0].run_id.as_str()), (0, "run2"));

        // Stale run_id → returns current run + all its events (caller realigns).
        let (rid3, evs3, _) = b.events_since("run1", 100);
        assert_eq!(rid3, "run2");
        assert_eq!(evs3.len(), 1);
    }
}
