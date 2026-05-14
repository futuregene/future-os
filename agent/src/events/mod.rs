//! Event bus — 1:1 compatible with Go internal/events/

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: HashMap<String, serde_json::Value>,
    pub timestamp: DateTime<Local>,
}

impl AgentEvent {
    pub fn new(event_type: impl Into<String>) -> Self {
        Self {
            event_type: event_type.into(),
            data: HashMap::new(),
            timestamp: Local::now(),
        }
    }

    pub fn with_data(mut self, key: &str, value: serde_json::Value) -> Self {
        self.data.insert(key.to_string(), value);
        self
    }

    pub fn with_str(mut self, key: &str, value: &str) -> Self {
        self.data.insert(key.to_string(), serde_json::json!(value));
        self
    }

    pub fn with_i64(mut self, key: &str, value: i64) -> Self {
        self.data.insert(key.to_string(), serde_json::json!(value));
        self
    }

    pub fn with_bool(mut self, key: &str, value: bool) -> Self {
        self.data.insert(key.to_string(), serde_json::json!(value));
        self
    }
}

pub type EventListener = Arc<dyn Fn(AgentEvent) + Send + Sync>;
type CallbackEntry = (String, Arc<dyn Fn(AgentEvent) + Send + Sync>);

pub struct EventBus {
    subscribers: RwLock<HashMap<String, tokio::sync::mpsc::Sender<AgentEvent>>>,
    callbacks: RwLock<HashMap<String, CallbackEntry>>,
    #[allow(clippy::type_complexity)]
    star_callbacks: RwLock<Vec<Arc<dyn Fn(AgentEvent) + Send + Sync>>>,
    next_id: RwLock<usize>,
    closed: RwLock<bool>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(HashMap::new()),
            callbacks: RwLock::new(HashMap::new()),
            star_callbacks: RwLock::new(Vec::new()),
            next_id: RwLock::new(0),
            closed: RwLock::new(false),
        }
    }

    /// Subscribe adds a subscriber and returns a receive channel.
    /// Buffer size: 64 events. Returns None if bus is closed.
    pub fn subscribe(&self, id: &str) -> Option<tokio::sync::mpsc::Receiver<AgentEvent>> {
        if *self.closed.read().unwrap() {
            return None;
        }
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        self.subscribers.write().unwrap().insert(id.to_string(), tx);
        Some(rx)
    }

    pub fn unsubscribe(&self, id: &str) {
        self.subscribers.write().unwrap().remove(id);
    }

    /// OnEvent registers a callback-based listener for a specific event type.
    /// Use "*" as event_type for wildcard (all events).
    pub fn on_event(
        &self,
        event_type: &str,
        callback: Arc<dyn Fn(AgentEvent) + Send + Sync>,
    ) -> String {
        let mut next = self.next_id.write().unwrap();
        *next += 1;
        let id = format!("listener_{}", *next);

        if event_type == "*" {
            self.star_callbacks.write().unwrap().push(callback);
        } else {
            self.callbacks
                .write()
                .unwrap()
                .insert(id.clone(), (event_type.to_string(), callback));
        }
        id
    }

    pub fn off_event(&self, id: &str) {
        self.callbacks.write().unwrap().remove(id);
    }

    /// Emit sends an event to all subscribers (non-blocking, may drop for slow consumers).
    /// Callback listeners are invoked synchronously (never dropped).
    pub fn emit(&self, event: AgentEvent) {
        if *self.closed.read().unwrap() {
            return;
        }

        // Collect channel senders
        let senders: Vec<_> = self.subscribers.read().unwrap().values().cloned().collect();
        let callbacks: Vec<_> = self.callbacks.read().unwrap().values().cloned().collect();
        let star_callbacks: Vec<_> = self.star_callbacks.read().unwrap().clone();

        // Channel subscribers (non-blocking, may drop)
        let event_clone = event.clone();
        for tx in senders {
            let _ = tx.try_send(event_clone.clone());
        }

        // Callback listeners (synchronous, never dropped)
        for (typ, callback) in callbacks {
            if typ == event.event_type {
                callback(event.clone());
            }
        }
        for callback in star_callbacks {
            callback(event.clone());
        }
    }

    pub fn close(&self) {
        *self.closed.write().unwrap() = true;
        // Drop senders to close channels
        self.subscribers.write().unwrap().clear();
        self.callbacks.write().unwrap().clear();
        self.star_callbacks.write().unwrap().clear();
    }
}

// ─── Event constructors (matching Go helpers) ────────────────────────────────

pub fn agent_start(session_id: &str, model: &str, _reason: &str) -> AgentEvent {
    AgentEvent::new("agent_start")
        .with_str("session_id", session_id)
        .with_str("model", model)
}

pub fn agent_end(reason: &str, usage: Option<&crate::types::Usage>) -> AgentEvent {
    let mut e = AgentEvent::new("agent_end").with_str("reason", reason);
    if let Some(u) = usage {
        let mut usage_map = serde_json::Map::new();
        usage_map.insert(
            "input_tokens".to_string(),
            serde_json::json!(u.prompt_tokens),
        );
        usage_map.insert(
            "output_tokens".to_string(),
            serde_json::json!(u.completion_tokens),
        );
        usage_map.insert(
            "cache_read_tokens".to_string(),
            serde_json::json!(u.cache_read_tokens),
        );
        usage_map.insert(
            "cache_write_tokens".to_string(),
            serde_json::json!(u.cache_write_tokens),
        );
        usage_map.insert(
            "total_tokens".to_string(),
            serde_json::json!(u.total_tokens),
        );
        e.data
            .insert("usage".to_string(), serde_json::Value::Object(usage_map));
    }
    e
}

pub fn agent_end_with_stop_reason(
    reason: &str,
    usage: Option<&crate::types::Usage>,
    stop_reason: &str,
) -> AgentEvent {
    let mut e = agent_end(reason, usage);
    if !stop_reason.is_empty() {
        e.data
            .insert("stop_reason".to_string(), serde_json::json!(stop_reason));
    }
    e
}

pub fn turn_end(turn: usize) -> AgentEvent {
    AgentEvent::new("turn_end").with_i64("turn", turn as i64)
}

pub fn message_end(role: &str) -> AgentEvent {
    AgentEvent::new("message_end").with_str("role", role)
}

pub fn compaction_start(reason: &str) -> AgentEvent {
    AgentEvent::new("compaction_start").with_str("reason", reason)
}

pub fn compaction_end(
    tokens_before: i32,
    summary: &str,
    aborted: bool,
    reason: &str,
) -> AgentEvent {
    AgentEvent::new("compaction_end")
        .with_i64("tokens_before", tokens_before as i64)
        .with_str("summary", summary)
        .with_bool("aborted", aborted)
        .with_str("reason", reason)
}

pub fn auto_retry_start(attempt: usize, max_attempts: usize, delay_ms: usize) -> AgentEvent {
    AgentEvent::new("auto_retry_start")
        .with_i64("attempt", attempt as i64)
        .with_i64("max_attempts", max_attempts as i64)
        .with_i64("delay_ms", delay_ms as i64)
}

pub fn auto_retry_end() -> AgentEvent {
    AgentEvent::new("auto_retry_end")
}

pub fn turn_start(turn: usize) -> AgentEvent {
    AgentEvent::new("turn_start").with_i64("turn", turn as i64)
}

pub fn message_start(role: &str) -> AgentEvent {
    AgentEvent::new("message_start").with_str("role", role)
}

pub fn text_start() -> AgentEvent {
    AgentEvent::new("text_start")
}
pub fn text_delta(text: &str) -> AgentEvent {
    AgentEvent::new("text_delta").with_str("text", text)
}
pub fn text_end() -> AgentEvent {
    AgentEvent::new("text_end")
}

pub fn thinking_start() -> AgentEvent {
    AgentEvent::new("thinking_start")
}
pub fn thinking_delta(text: &str) -> AgentEvent {
    AgentEvent::new("thinking_delta").with_str("text", text)
}
pub fn thinking_end() -> AgentEvent {
    AgentEvent::new("thinking_end")
}

pub fn toolcall_start(tool_name: &str, tool_id: &str) -> AgentEvent {
    AgentEvent::new("toolcall_start")
        .with_str("tool_name", tool_name)
        .with_str("tool_id", tool_id)
}

pub fn toolcall_delta(text: &str) -> AgentEvent {
    AgentEvent::new("toolcall_delta").with_str("text", text)
}

pub fn toolcall_end() -> AgentEvent {
    AgentEvent::new("toolcall_end")
}

pub fn tool_result(tool_name: &str, result: &str, err: &str, duration_ms: i64) -> AgentEvent {
    AgentEvent::new("tool_result")
        .with_str("tool_name", tool_name)
        .with_str("result", result)
        .with_str("error", err)
        .with_i64("duration_ms", duration_ms)
}

pub fn tool_start(tool_name: &str, tool_id: &str) -> AgentEvent {
    AgentEvent::new("tool_start")
        .with_str("tool_name", tool_name)
        .with_str("tool_id", tool_id)
}

pub fn tool_end(tool_name: &str) -> AgentEvent {
    AgentEvent::new("tool_end").with_str("tool_name", tool_name)
}

pub fn error_event(msg: &str) -> AgentEvent {
    AgentEvent::new("error").with_str("error", msg)
}

pub fn usage_event(u: &crate::types::Usage) -> AgentEvent {
    let mut e = AgentEvent::new("usage");
    e.data.insert(
        "input_tokens".to_string(),
        serde_json::json!(u.prompt_tokens),
    );
    e.data.insert(
        "output_tokens".to_string(),
        serde_json::json!(u.completion_tokens),
    );
    e.data.insert(
        "total_tokens".to_string(),
        serde_json::json!(u.total_tokens),
    );
    e
}
