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
    #[serde(default)]
    pub requested_run_id: String,
    #[serde(default)]
    pub client_request_id: String,
    #[serde(default)]
    pub busy_policy: String,

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_data: Option<serde_json::Value>,
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
            error_code: None,
            error_data: None,
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
            error_code: None,
            error_data: None,
        };
        serde_json::to_string(&resp).unwrap_or_default()
    }

    pub fn build_fail_code(
        id: &str,
        command: &str,
        code: &str,
        err: &str,
        details: impl Into<serde_json::Value>,
    ) -> String {
        let resp = Self {
            resp_type: "response".to_string(),
            id: id.to_string(),
            command: command.to_string(),
            success: false,
            data: None,
            error: Some(err.to_string()),
            error_code: Some(code.to_string()),
            error_data: Some(details.into()),
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
    epoch: i64,
    idx: i64,
    events: Vec<SseEvent>,
    projection_events: Vec<SseEvent>,
}

pub struct RunAttachment {
    pub receiver: broadcast::Receiver<SseEvent>,
    pub events: Vec<SseEvent>,
    pub truncated: bool,
    pub projection: Option<RunProjectionSnapshot>,
}

#[derive(Debug, Clone)]
pub struct RunProjectionSnapshot {
    pub run_id: String,
    pub epoch: i64,
    pub cursor: i64,
    pub events: Vec<SseEvent>,
}

/// Per-session SSE broadcaster. Also the **single stamping point** (P1): it
/// assigns each event's `run_id` + monotonic `idx` and buffers the current run
/// for `events_since` — all under one lock, so broadcast order matches idx order.
#[derive(Clone)]
pub struct SseBroadcaster {
    tx: broadcast::Sender<SseEvent>,
    run: std::sync::Arc<parking_lot::Mutex<RunState>>,
    /// Number of times a consumer's cursor predates the replay ring (ring
    /// truncation / idx gap), forcing a resync via the projection snapshot.
    /// Observability metric for the "ring truncation must be explicitly
    /// visible" acceptance criterion; expected to stay 0 in healthy runs.
    truncation_count: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Number of times a live subscriber fell behind the broadcast channel
    /// (tokio `RecvError::Lagged`) and the gRPC stream was terminated for cursor
    /// resume. Observability metric for the "broadcast lag" criterion; a spike
    /// means a client couldn't keep up with the event rate.
    lag_count: std::sync::Arc<std::sync::atomic::AtomicU64>,
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
                epoch: 0,
                idx: 0,
                events: Vec::new(),
                projection_events: Vec::new(),
            })),
            truncation_count: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            lag_count: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Subscribe to SSE events
    pub fn subscribe(&self) -> broadcast::Receiver<SseEvent> {
        self.tx.subscribe()
    }

    pub fn last_idx(&self) -> i64 {
        self.run.lock().idx.saturating_sub(1)
    }

    pub fn current_run_id(&self) -> String {
        self.run.lock().run_id.clone()
    }

    /// Count of ring-truncation resyncs: times a consumer's cursor fell behind
    /// the replay ring and had to recover via the projection snapshot.
    /// Observability metric; expected to stay 0 in healthy runs (a non-zero
    /// value means a client lagged far enough to lose the incremental tail).
    pub fn truncation_count(&self) -> u64 {
        self.truncation_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Record that a live subscriber lagged behind the broadcast channel (the
    /// gRPC layer calls this when it observes `RecvError::Lagged`).
    pub fn record_lag(&self) -> u64 {
        self.lag_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
    }

    /// Count of live-subscriber lag events (see `record_lag`). Observability
    /// metric; expected to stay 0 unless a client can't keep up with the rate.
    pub fn lag_count(&self) -> u64 {
        self.lag_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Atomically register a receiver and snapshot the requested run tail.
    /// `broadcast` uses the same run lock, so no event can land in the window
    /// between the snapshot and subscription.
    pub fn attach(&self, run_id: &str, after_idx: i64) -> anyhow::Result<RunAttachment> {
        let run = self.run.lock();
        if run.run_id != run_id {
            anyhow::bail!("run `{run_id}` is not the active run");
        }
        let receiver = self.tx.subscribe();
        let min_idx = run.events.first().map(|event| event.idx).unwrap_or(run.idx);
        let truncated = after_idx.saturating_add(1) < min_idx;
        if truncated {
            let count = self
                .truncation_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            tracing::warn!(
                run_id,
                requested_after_idx = after_idx,
                min_available_idx = min_idx,
                truncation_count = count,
                "run replay ring truncated; returning projection snapshot"
            );
        }
        let events = run
            .events
            .iter()
            .filter(|event| !truncated && event.idx > after_idx)
            .cloned()
            .collect();
        let projection = truncated.then(|| RunProjectionSnapshot {
            run_id: run.run_id.clone(),
            epoch: run.epoch,
            cursor: run.idx.saturating_sub(1),
            events: run.projection_events.clone(),
        });
        Ok(RunAttachment {
            receiver,
            events,
            truncated,
            projection,
        })
    }

    /// Stamp `run_id` + `epoch` + monotonic `idx`, buffer the event, and
    /// broadcast — all under one lock so stream order matches idx order (no
    /// reordering race).
    pub fn broadcast(&self, mut event: SseEvent) {
        let mut run = self.run.lock();
        event.run_id = run.run_id.clone();
        event.epoch = run.epoch;
        event.idx = run.idx;
        run.idx += 1;
        apply_to_projection(&mut run.projection_events, &event);
        run.events.push(event.clone());
        if run.events.len() > MAX_RUN_EVENTS {
            let overflow = run.events.len() - MAX_RUN_EVENTS;
            run.events.drain(0..overflow);
        }
        // tokio broadcast semantics: send() only fails when there are NO
        // active receivers — normal for ephemeral sessions before a client
        // subscribes, so the error is ignored.  When the ring buffer is
        // full, send() does NOT fail; it drops the oldest events and slow
        // receivers observe RecvError::Lagged, then resync via
        // `events_since` (which reports the gap via min_idx).
        let _ = self.tx.send(event);
    }

    /// Begin a new user run: set `run_id` + `epoch`, reset `idx`, clear the
    /// buffer. `epoch` is the run's monotonic generation within the session
    /// (from the runtime lease), stamped on every event of this run.
    pub fn start_run(&self, run_id: String, epoch: i64) {
        let mut run = self.run.lock();
        run.run_id = run_id;
        run.epoch = epoch;
        run.idx = 0;
        run.events.clear();
        run.projection_events.clear();
    }

    /// Current-run events with `idx > since_idx`, plus the earliest idx still in
    /// the buffer (`min_idx`, 0 if empty). A stale run id is an explicit error;
    /// it must never silently return another run's events. A
    /// full backfill (`since_idx < 0`) whose result starts above `min_idx == 0`
    /// — i.e. `min_idx > 0` — means the run's prefix was dropped on overflow, so
    /// the caller can surface the gap instead of silently reconstructing a
    /// truncated message.
    pub fn events_since(
        &self,
        run_id: &str,
        since_idx: i64,
    ) -> anyhow::Result<(String, Vec<SseEvent>, i64, Option<RunProjectionSnapshot>)> {
        let run = self.run.lock();
        if run.run_id != run_id {
            anyhow::bail!("run `{run_id}` is not the active run");
        }
        let min_idx = run.events.first().map(|e| e.idx).unwrap_or(0);
        let truncated = since_idx.saturating_add(1) < min_idx;
        if truncated {
            let count = self
                .truncation_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            tracing::warn!(
                run_id,
                requested_after_idx = since_idx,
                min_available_idx = min_idx,
                truncation_count = count,
                "run event query crossed replay-ring boundary; returning projection snapshot"
            );
        }
        let events = run
            .events
            .iter()
            .filter(|event| !truncated && event.idx > since_idx)
            .cloned()
            .collect();
        let projection = truncated.then(|| RunProjectionSnapshot {
            run_id: run.run_id.clone(),
            epoch: run.epoch,
            cursor: run.idx.saturating_sub(1),
            events: run.projection_events.clone(),
        });
        Ok((run.run_id.clone(), events, min_idx, projection))
    }
}

/// Fold a run event into the durable-in-memory semantic projection.
///
/// The replay ring is intentionally bounded, while the projection must retain
/// enough information to rebuild the visible run after that ring truncates.
/// High-frequency deltas are coalesced into their preceding semantic segment;
/// lifecycle, tool terminal, approval, usage, error, and terminal events keep
/// their original ordering and cursor.
fn apply_to_projection(projection: &mut Vec<SseEvent>, event: &SseEvent) {
    // `text_delta` (raw provider-stream token) duplicates `text_chunk` (the
    // on_text-derived token); consumers project the latter, so retaining both
    // would duplicate assistant output.
    if event.event_type == "text_delta" {
        return;
    }

    let coalescible = matches!(
        event.event_type.as_str(),
        "text_chunk" | "thinking_delta" | "toolcall_delta" | "tool_delta"
    );
    if coalescible {
        if let Some(previous) = projection
            .last_mut()
            .filter(|previous| previous.event_type == event.event_type)
        {
            if let (Ok(mut previous_data), Ok(next_data)) = (
                serde_json::from_str::<serde_json::Value>(&previous.data),
                serde_json::from_str::<serde_json::Value>(&event.data),
            ) {
                let same_tool_stream =
                    !matches!(event.event_type.as_str(), "toolcall_delta" | "tool_delta")
                        || ["tool_id", "tc_index"]
                            .iter()
                            .all(|key| previous_data.get(key) == next_data.get(key));
                if let (Some(previous_text), Some(next_text)) = (
                    previous_data.get("text").and_then(|value| value.as_str()),
                    next_data.get("text").and_then(|value| value.as_str()),
                ) {
                    if !same_tool_stream {
                        projection.push(event.clone());
                        return;
                    }
                    let combined = format!("{previous_text}{next_text}");
                    previous_data["text"] = serde_json::Value::String(combined);
                    previous.data = serde_json::to_string(&previous_data).unwrap_or_default();
                    // The folded segment represents every source event through
                    // this cursor, so live resume starts strictly after it.
                    previous.idx = event.idx;
                    return;
                }
            }
        }
    }

    projection.push(event.clone());
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
    /// `run_id` + `epoch` + `idx` are the run-scoped identity; `session_id` is
    /// added at the gRPC wire boundary (the stream is session-scoped).
    pub run_id: String,
    pub epoch: i64,
    pub idx: i64,
}

impl SseEvent {
    pub fn new(event_type: &str, data: serde_json::Value) -> Self {
        Self {
            event_type: event_type.to_string(),
            data: serde_json::to_string(&data).unwrap_or_default(),
            run_id: String::new(),
            epoch: 0,
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
        assert!(cmd.busy_policy.is_empty());
    }

    #[test]
    fn rpc_command_prompt_busy_policy_uses_camel_case_wire_name() {
        let json = r#"{
            "id": "cmd2b",
            "type": "prompt",
            "sessionId": "s1",
            "message": "hello",
            "busyPolicy": "enqueue_if_busy"
        }"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.busy_policy, "enqueue_if_busy");
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
        assert!(parsed.get("error_code").is_none());
        assert!(parsed.get("error_data").is_none());
    }

    #[test]
    fn rpc_response_structured_failure_keeps_human_message() {
        let json_str = RpcResponse::build_fail_code(
            "id2b",
            "prompt",
            "busy",
            "session already has an active run",
            serde_json::json!({"active_run_id": "run-a"}),
        );
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["success"], false);
        assert_eq!(parsed["error"], "session already has an active run");
        assert_eq!(parsed["error_code"], "busy");
        assert_eq!(parsed["error_data"]["active_run_id"], "run-a");
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
        b.start_run("run1".to_string(), 1);
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
        let (rid, evs, min_idx, projection) = b.events_since("run1", 0).unwrap();
        assert_eq!(rid, "run1");
        assert_eq!(evs.len(), 2);
        assert_eq!((evs[0].idx, evs[1].idx), (1, 2));
        assert_eq!(evs[0].run_id, "run1");
        // Nothing dropped yet → earliest buffered idx is still 0 (no gap).
        assert_eq!(min_idx, 0);
        assert!(projection.is_none());

        // From -1 → all three (idx 0,1,2).
        let (_, all, _, _) = b.events_since("run1", -1).unwrap();
        assert_eq!(all.iter().map(|e| e.idx).collect::<Vec<_>>(), vec![0, 1, 2]);

        // New run resets idx + clears buffer.
        b.start_run("run2".to_string(), 1);
        b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        let (rid2, evs2, _, _) = b.events_since("run2", -1).unwrap();
        assert_eq!(rid2, "run2");
        assert_eq!(evs2.len(), 1);
        assert_eq!((evs2[0].idx, evs2[0].run_id.as_str()), (0, "run2"));

        assert!(b.events_since("run1", 100).is_err());
    }

    #[test]
    fn attach_has_no_snapshot_subscribe_window() {
        let b = SseBroadcaster::new();
        b.start_run("run1".to_string(), 1);
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "a"}),
        ));
        let mut attachment = b.attach("run1", -1).unwrap();
        assert_eq!(attachment.events.len(), 1);

        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "b"}),
        ));
        let live = attachment.receiver.try_recv().unwrap();
        assert_eq!(live.idx, 1);
        assert!(!attachment.truncated);
    }

    #[test]
    fn attach_reports_truncated_ring_and_rejects_other_run() {
        let b = SseBroadcaster::new();
        b.start_run("run1".to_string(), 1);
        for idx in 0..=MAX_RUN_EVENTS {
            b.broadcast(SseEvent::new(
                "text_chunk",
                serde_json::json!({"text": idx.to_string()}),
            ));
        }
        let mut attachment = b.attach("run1", -1).unwrap();
        assert!(attachment.truncated);
        assert!(attachment.events.is_empty());
        let snapshot = attachment.projection.take().unwrap();
        assert_eq!(snapshot.run_id, "run1");
        assert_eq!(snapshot.cursor, MAX_RUN_EVENTS as i64);
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].idx, MAX_RUN_EVENTS as i64);
        let projected_data: serde_json::Value =
            serde_json::from_str(&snapshot.events[0].data).unwrap();
        assert!(projected_data["text"]
            .as_str()
            .is_some_and(|text| text.starts_with('0') && text.ends_with("2000")));

        // Receiver registration and snapshot capture share the run lock: the
        // first live event starts exactly after the snapshot cursor.
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "live"}),
        ));
        let live = attachment.receiver.try_recv().unwrap();
        assert_eq!(live.idx, snapshot.cursor + 1);

        let (_, replay, _, replay_projection) = b.events_since("run1", -1).unwrap();
        assert!(replay.is_empty());
        assert_eq!(
            replay_projection.as_ref().map(|value| value.cursor),
            Some(live.idx)
        );
        assert!(b.attach("run2", -1).is_err());
    }

    #[test]
    fn truncation_counter_tracks_ring_overflow_resyncs() {
        let b = SseBroadcaster::new();
        b.start_run("run1".to_string(), 1);
        assert_eq!(b.truncation_count(), 0);

        // Within the ring: a full backfill is NOT a truncation.
        for idx in 0..10 {
            b.broadcast(SseEvent::new(
                "text_chunk",
                serde_json::json!({"text": idx.to_string()}),
            ));
        }
        let _ = b.events_since("run1", -1).unwrap();
        assert_eq!(
            b.truncation_count(),
            0,
            "in-ring backfill is not a truncation"
        );

        // Overflow the ring; now a backfill whose cursor predates the ring is a
        // truncation, and each such resync is counted (attach + events_since).
        for idx in 0..=MAX_RUN_EVENTS {
            b.broadcast(SseEvent::new(
                "text_chunk",
                serde_json::json!({"text": idx.to_string()}),
            ));
        }
        let attachment = b.attach("run1", -1).unwrap();
        assert!(attachment.truncated);
        assert_eq!(b.truncation_count(), 1);
        let _ = b.events_since("run1", -1).unwrap();
        assert_eq!(
            b.truncation_count(),
            2,
            "events_since truncation is counted too"
        );
    }

    #[test]
    fn lag_counter_is_observable_and_starts_at_zero() {
        let b = SseBroadcaster::new();
        assert_eq!(b.lag_count(), 0);
        b.record_lag();
        b.record_lag();
        assert_eq!(b.lag_count(), 2);
        // The counter is shared across clones (the gRPC layer holds a clone).
        let clone = b.clone();
        clone.record_lag();
        assert_eq!(b.lag_count(), 3);
    }

    #[test]
    fn concurrent_broadcasts_to_one_broadcaster_yield_contiguous_idx() {
        use std::sync::Arc;
        use std::thread;
        const THREADS: usize = 8;
        const PER_THREAD: usize = 250; // 8 * 250 = 2000 == MAX_RUN_EVENTS (no overflow)
        let b = Arc::new(SseBroadcaster::new());
        b.start_run("run1".to_string(), 1);
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let b = b.clone();
                thread::spawn(move || {
                    for n in 0..PER_THREAD {
                        b.broadcast(SseEvent::new("text_chunk", serde_json::json!({"text": n})));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        // The single stamping lock serializes concurrent broadcasts: every event
        // got a unique, contiguous idx (no gaps, no duplicates) under contention.
        let total = (THREADS * PER_THREAD) as i64;
        assert_eq!(b.last_idx(), total - 1);
        assert_eq!(b.truncation_count(), 0);
        let (run_id, events, min_idx, projection) =
            b.events_since("run1", total - 1 - 100).unwrap();
        assert_eq!(run_id, "run1");
        assert_eq!(min_idx, 0);
        assert!(projection.is_none());
        let expected_start = total - 100; // first idx > (total - 1 - 100)
        assert_eq!(events.len(), 100);
        for (i, event) in events.iter().enumerate() {
            assert_eq!(event.idx, expected_start + i as i64, "contiguous, no gaps");
            assert_eq!(event.run_id, "run1");
            assert_eq!(event.epoch, 1);
        }
    }

    #[test]
    fn projection_preserves_semantic_order_while_coalescing_deltas() {
        let b = SseBroadcaster::new();
        b.start_run("run1".to_string(), 1);
        b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        b.broadcast(SseEvent::new(
            "thinking_delta",
            serde_json::json!({"text": "a"}),
        ));
        b.broadcast(SseEvent::new(
            "thinking_delta",
            serde_json::json!({"text": "b"}),
        ));
        b.broadcast(SseEvent::new(
            "tool_start",
            serde_json::json!({"tool_id": "t1", "tool_name": "read"}),
        ));
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "hello"}),
        ));
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": " world"}),
        ));
        for idx in 0..MAX_RUN_EVENTS {
            b.broadcast(SseEvent::new(
                "usage",
                serde_json::json!({"usage": {"output_tokens": idx}}),
            ));
        }

        let snapshot = b.attach("run1", -1).unwrap().projection.unwrap();
        assert_eq!(
            snapshot
                .events
                .iter()
                .take(4)
                .map(|event| event.event_type.as_str())
                .collect::<Vec<_>>(),
            vec!["agent_start", "thinking_delta", "tool_start", "text_chunk"]
        );
        let thinking: serde_json::Value = serde_json::from_str(&snapshot.events[1].data).unwrap();
        let text: serde_json::Value = serde_json::from_str(&snapshot.events[3].data).unwrap();
        assert_eq!(thinking["text"], "ab");
        assert_eq!(text["text"], "hello world");
    }
}
