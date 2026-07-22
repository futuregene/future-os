//! Event bus — 1:1 compatible with Go internal/events/

use chrono::{DateTime, Local};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

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
        if *self.closed.read() {
            return None;
        }
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        self.subscribers.write().insert(id.to_string(), tx);
        Some(rx)
    }

    pub fn unsubscribe(&self, id: &str) {
        self.subscribers.write().remove(id);
    }

    /// OnEvent registers a callback-based listener for a specific event type.
    /// Use "*" as event_type for wildcard (all events).
    pub fn on_event(
        &self,
        event_type: &str,
        callback: Arc<dyn Fn(AgentEvent) + Send + Sync>,
    ) -> String {
        let mut next = self.next_id.write();
        *next += 1;
        let id = format!("listener_{}", *next);

        if event_type == "*" {
            self.star_callbacks.write().push(callback);
        } else {
            self.callbacks
                .write()
                .insert(id.clone(), (event_type.to_string(), callback));
        }
        id
    }

    pub fn off_event(&self, id: &str) {
        self.callbacks.write().remove(id);
    }

    /// Emit sends an event to all subscribers (non-blocking, may drop for slow consumers).
    /// Callback listeners are invoked synchronously (never dropped).
    pub fn emit(&self, event: AgentEvent) {
        if *self.closed.read() {
            return;
        }

        // Collect channel senders
        let senders: Vec<_> = self.subscribers.read().values().cloned().collect();
        let callbacks: Vec<_> = self.callbacks.read().values().cloned().collect();
        let star_callbacks: Vec<_> = self.star_callbacks.read().clone();

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
        *self.closed.write() = true;
        // Drop senders to close channels
        self.subscribers.write().clear();
        self.callbacks.write().clear();
        self.star_callbacks.write().clear();
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

pub fn tool_result(
    tool_name: &str,
    tool_id: &str,
    result: &str,
    err: &str,
    duration_ms: i64,
) -> AgentEvent {
    AgentEvent::new("tool_result")
        .with_str("tool_name", tool_name)
        .with_str("tool_id", tool_id)
        .with_str("result", result)
        .with_str("error", err)
        .with_i64("duration_ms", duration_ms)
}

pub fn tool_start(tool_name: &str, tool_id: &str) -> AgentEvent {
    AgentEvent::new("tool_start")
        .with_str("tool_name", tool_name)
        .with_str("tool_id", tool_id)
}

pub fn tool_end(tool_name: &str, tool_id: &str) -> AgentEvent {
    AgentEvent::new("tool_end")
        .with_str("tool_name", tool_name)
        .with_str("tool_id", tool_id)
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
    if let Some(cache_r) = u.cache_read_tokens {
        e.data
            .insert("cache_read_tokens".to_string(), serde_json::json!(cache_r));
    }
    if let Some(cache_w) = u.cache_write_tokens {
        e.data
            .insert("cache_write_tokens".to_string(), serde_json::json!(cache_w));
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ─── AgentEvent constructors ────────────────────────────────────────────

    #[test]
    fn agent_event_new() {
        let e = AgentEvent::new("test_event");
        assert_eq!(e.event_type, "test_event");
        assert!(e.data.is_empty());
    }

    #[test]
    fn agent_event_with_str() {
        let e = AgentEvent::new("test").with_str("key", "value");
        assert_eq!(e.data["key"], serde_json::json!("value"));
    }

    #[test]
    fn agent_event_with_i64() {
        let e = AgentEvent::new("test").with_i64("count", 42);
        assert_eq!(e.data["count"], serde_json::json!(42));
    }

    #[test]
    fn agent_event_with_bool() {
        let e = AgentEvent::new("test").with_bool("flag", true);
        assert_eq!(e.data["flag"], serde_json::json!(true));
    }

    #[test]
    fn agent_event_with_data() {
        let e = AgentEvent::new("test").with_data("custom", serde_json::json!({"nested": [1,2]}));
        assert_eq!(e.data["custom"]["nested"], serde_json::json!([1, 2]));
    }

    #[test]
    fn agent_event_chaining() {
        let e = AgentEvent::new("chain")
            .with_str("a", "1")
            .with_i64("b", 2)
            .with_bool("c", false);
        assert_eq!(e.data.len(), 3);
    }

    #[test]
    fn agent_event_serialization() {
        let e = AgentEvent::new("test").with_str("msg", "hello");
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["type"], "test");
        assert_eq!(json["data"]["msg"], "hello");
        assert!(json.get("timestamp").is_some());
    }

    // ─── EventBus subscribe / emit ──────────────────────────────────────────

    #[tokio::test]
    async fn subscribe_and_receive() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe("sub1").unwrap();
        bus.emit(AgentEvent::new("test").with_str("data", "hello"));
        let received = rx.recv().await.unwrap();
        assert_eq!(received.event_type, "test");
        assert_eq!(received.data["data"], serde_json::json!("hello"));
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe("sub1").unwrap();
        let mut rx2 = bus.subscribe("sub2").unwrap();
        bus.emit(AgentEvent::new("broadcast"));
        assert_eq!(rx1.recv().await.unwrap().event_type, "broadcast");
        assert_eq!(rx2.recv().await.unwrap().event_type, "broadcast");
    }

    #[test]
    fn unsubscribe_stops_receiving() {
        let bus = EventBus::new();
        let _rx = bus.subscribe("sub1").unwrap();
        bus.unsubscribe("sub1");
        // After unsubscribe, emit should not panic
        bus.emit(AgentEvent::new("test"));
    }

    #[test]
    fn subscribe_on_closed_bus_returns_none() {
        let bus = EventBus::new();
        bus.close();
        assert!(bus.subscribe("sub1").is_none());
    }

    #[test]
    fn emit_on_closed_bus_is_noop() {
        let bus = EventBus::new();
        bus.close();
        // Should not panic
        bus.emit(AgentEvent::new("test"));
    }

    // ─── EventBus on_event callbacks ────────────────────────────────────────

    #[test]
    fn on_event_specific_type() {
        let bus = EventBus::new();
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();
        bus.on_event(
            "text_delta",
            Arc::new(move |_| {
                count_clone.fetch_add(1, Ordering::SeqCst);
            }),
        );
        bus.emit(AgentEvent::new("text_delta"));
        bus.emit(AgentEvent::new("other_event"));
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn on_event_wildcard() {
        let bus = EventBus::new();
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();
        bus.on_event(
            "*",
            Arc::new(move |_| {
                count_clone.fetch_add(1, Ordering::SeqCst);
            }),
        );
        bus.emit(AgentEvent::new("event_a"));
        bus.emit(AgentEvent::new("event_b"));
        bus.emit(AgentEvent::new("event_c"));
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn off_event_removes_listener() {
        let bus = EventBus::new();
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();
        let id = bus.on_event(
            "test",
            Arc::new(move |_| {
                count_clone.fetch_add(1, Ordering::SeqCst);
            }),
        );
        bus.emit(AgentEvent::new("test"));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        bus.off_event(&id);
        bus.emit(AgentEvent::new("test"));
        assert_eq!(count.load(Ordering::SeqCst), 1); // no increment
    }

    #[test]
    fn close_clears_all_listeners() {
        let bus = EventBus::new();
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();
        bus.on_event(
            "*",
            Arc::new(move |_| {
                count_clone.fetch_add(1, Ordering::SeqCst);
            }),
        );
        bus.close();
        bus.emit(AgentEvent::new("test"));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    // ─── Event constructor helpers ──────────────────────────────────────────

    #[test]
    fn agent_start_event() {
        let e = agent_start("sess_1", "gpt-4o", "user");
        assert_eq!(e.event_type, "agent_start");
        assert_eq!(e.data["session_id"], serde_json::json!("sess_1"));
        assert_eq!(e.data["model"], serde_json::json!("gpt-4o"));
    }

    #[test]
    fn agent_end_event_without_usage() {
        let e = agent_end("completed", None);
        assert_eq!(e.event_type, "agent_end");
        assert_eq!(e.data["reason"], serde_json::json!("completed"));
        assert!(!e.data.contains_key("usage"));
    }

    #[test]
    fn agent_end_event_with_usage() {
        let u = crate::types::Usage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cache_read_tokens: Some(80),
            cache_write_tokens: Some(20),
            credit_cost: None,
        };
        let e = agent_end("completed", Some(&u));
        let usage = &e.data["usage"];
        assert_eq!(usage["input_tokens"], serde_json::json!(100));
        assert_eq!(usage["output_tokens"], serde_json::json!(50));
        assert_eq!(usage["total_tokens"], serde_json::json!(150));
    }

    #[test]
    fn agent_end_with_stop_reason_event() {
        let e = super::agent_end_with_stop_reason("completed", None, "max_tokens");
        assert_eq!(e.data["stop_reason"], serde_json::json!("max_tokens"));
    }

    #[test]
    fn agent_end_with_empty_stop_reason_event() {
        let e = super::agent_end_with_stop_reason("completed", None, "");
        assert!(!e.data.contains_key("stop_reason"));
    }

    #[test]
    fn turn_events() {
        assert_eq!(turn_start(1).event_type, "turn_start");
        assert_eq!(turn_start(1).data["turn"], serde_json::json!(1));
        assert_eq!(turn_end(5).event_type, "turn_end");
        assert_eq!(turn_end(5).data["turn"], serde_json::json!(5));
    }

    #[test]
    fn message_events() {
        assert_eq!(message_start("user").event_type, "message_start");
        assert_eq!(
            message_start("user").data["role"],
            serde_json::json!("user")
        );
        assert_eq!(message_end("assistant").event_type, "message_end");
    }

    #[test]
    fn text_events() {
        assert_eq!(text_start().event_type, "text_start");
        assert_eq!(text_delta("hello").event_type, "text_delta");
        assert_eq!(text_delta("hello").data["text"], serde_json::json!("hello"));
        assert_eq!(text_end().event_type, "text_end");
    }

    #[test]
    fn thinking_events() {
        assert_eq!(thinking_start().event_type, "thinking_start");
        assert_eq!(thinking_delta("hmm").event_type, "thinking_delta");
        assert_eq!(thinking_end().event_type, "thinking_end");
    }

    #[test]
    fn toolcall_events() {
        let e = toolcall_start("shell", "call_1");
        assert_eq!(e.event_type, "toolcall_start");
        assert_eq!(e.data["tool_name"], serde_json::json!("shell"));
        assert_eq!(e.data["tool_id"], serde_json::json!("call_1"));

        assert_eq!(toolcall_delta("partial").event_type, "toolcall_delta");
        assert_eq!(toolcall_end().event_type, "toolcall_end");
    }

    #[test]
    fn tool_result_event() {
        let e = tool_result("shell", "call_1", "file.txt", "", 150);
        assert_eq!(e.event_type, "tool_result");
        assert_eq!(e.data["result"], serde_json::json!("file.txt"));
        assert_eq!(e.data["duration_ms"], serde_json::json!(150));
    }

    #[test]
    fn tool_start_end_events() {
        assert_eq!(tool_start("read", "c1").event_type, "tool_start");
        assert_eq!(tool_end("read", "c1").event_type, "tool_end");
    }

    #[test]
    fn error_event_creates_error_type() {
        let e = super::error_event("something broke");
        assert_eq!(e.event_type, "error");
        assert_eq!(e.data["error"], serde_json::json!("something broke"));
    }

    #[test]
    fn compaction_events() {
        assert_eq!(
            compaction_start("context_full").event_type,
            "compaction_start"
        );
        let e = compaction_end(5000, "summary text", false, "context_full");
        assert_eq!(e.event_type, "compaction_end");
        assert_eq!(e.data["tokens_before"], serde_json::json!(5000));
        assert_eq!(e.data["aborted"], serde_json::json!(false));
    }

    #[test]
    fn auto_retry_events() {
        let e = auto_retry_start(2, 3, 4000);
        assert_eq!(e.event_type, "auto_retry_start");
        assert_eq!(e.data["attempt"], serde_json::json!(2));
        assert_eq!(e.data["max_attempts"], serde_json::json!(3));
        assert_eq!(e.data["delay_ms"], serde_json::json!(4000));
        assert_eq!(auto_retry_end().event_type, "auto_retry_end");
    }

    #[test]
    fn usage_event_with_cache() {
        let u = crate::types::Usage {
            prompt_tokens: 200,
            completion_tokens: 100,
            total_tokens: 300,
            cache_read_tokens: Some(150),
            cache_write_tokens: Some(50),
            credit_cost: None,
        };
        let e = usage_event(&u);
        assert_eq!(e.event_type, "usage");
        assert_eq!(e.data["input_tokens"], serde_json::json!(200));
        assert_eq!(e.data["cache_read_tokens"], serde_json::json!(150));
        assert_eq!(e.data["cache_write_tokens"], serde_json::json!(50));
    }

    #[test]
    fn usage_event_without_cache() {
        let u = crate::types::Usage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cache_read_tokens: None,
            cache_write_tokens: None,
            credit_cost: None,
        };
        let e = usage_event(&u);
        assert!(!e.data.contains_key("cache_read_tokens"));
        assert!(!e.data.contains_key("cache_write_tokens"));
    }
}
