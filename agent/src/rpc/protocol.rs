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

    // bash
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
const MAX_RUN_EVENTS: usize = 5000;

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
    run: std::sync::Arc<std::sync::Mutex<RunState>>,
}

impl SseBroadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(4096);
        Self {
            tx,
            run: std::sync::Arc::new(std::sync::Mutex::new(RunState {
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
        let mut run = self.run.lock().unwrap();
        event.run_id = run.run_id.clone();
        event.idx = run.idx;
        run.idx += 1;
        run.events.push(event.clone());
        if run.events.len() > MAX_RUN_EVENTS {
            let overflow = run.events.len() - MAX_RUN_EVENTS;
            run.events.drain(0..overflow);
        }
        let _ = self.tx.send(event);
    }

    /// Begin a new user run: set `run_id`, reset `idx`, clear the buffer.
    pub fn start_run(&self, run_id: String) {
        let mut run = self.run.lock().unwrap();
        run.run_id = run_id;
        run.idx = 0;
        run.events.clear();
    }

    /// Current-run events with `idx > since_idx`. If `run_id` no longer matches
    /// (a new run started), return the current run_id + all its buffered events.
    pub fn events_since(&self, run_id: &str, since_idx: i64) -> (String, Vec<SseEvent>) {
        let run = self.run.lock().unwrap();
        if run.run_id == run_id {
            let events = run
                .events
                .iter()
                .filter(|e| e.idx > since_idx)
                .cloned()
                .collect();
            (run.run_id.clone(), events)
        } else {
            (run.run_id.clone(), run.events.clone())
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
mod p1_broadcaster_tests {
    use super::*;

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
        let (rid, evs) = b.events_since("run1", 0);
        assert_eq!(rid, "run1");
        assert_eq!(evs.len(), 2);
        assert_eq!((evs[0].idx, evs[1].idx), (1, 2));
        assert_eq!(evs[0].run_id, "run1");

        // From -1 → all three (idx 0,1,2).
        let (_, all) = b.events_since("run1", -1);
        assert_eq!(all.iter().map(|e| e.idx).collect::<Vec<_>>(), vec![0, 1, 2]);

        // New run resets idx + clears buffer.
        b.start_run("run2".to_string());
        b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        let (rid2, evs2) = b.events_since("run2", -1);
        assert_eq!(rid2, "run2");
        assert_eq!(evs2.len(), 1);
        assert_eq!((evs2[0].idx, evs2[0].run_id.as_str()), (0, "run2"));

        // Stale run_id → returns current run + all its events (caller realigns).
        let (rid3, evs3) = b.events_since("run1", 100);
        assert_eq!(rid3, "run2");
        assert_eq!(evs3.len(), 1);
    }
}
