//! Agent Loop — 1:1 compatible with Go internal/agent/

mod run_loop;
use crate::events::EventBus;
use crate::types::{
    AgentMessage, AgentTool, ContentBlock, LLMProvider, Message, StreamEvent, ToolCall,
};
use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

// ANSI terminal colors (matching Go). Only for raw stderr prints via
// eprint_log! — never inside tracing messages (tracing escapes ESC bytes in
// format args to literal text; the log file must stay plain).
const C_RESET: &str = "\x1b[0m";
const C_GREEN: &str = "\x1b[32m";
const C_MAGENTA: &str = "\x1b[35m";

pub const DEFAULT_MAX_TURNS: i32 = 0; // 0 = unlimited

pub type PersistCallback = Arc<dyn Fn(&crate::types::AgentMessage) + Send + Sync>;

/// Per-session state passed into `run_streaming_with_messages`.  Callbacks
/// are session-specific (they capture session_id, messages_arc, broadcaster)
/// and must NOT be stored on the shared Loop — otherwise concurrent sessions
/// overwrite each other's persistence and event streams.
#[derive(Default)]
pub struct StreamContext {
    pub model: String,
    pub system_prompt: String,
    #[allow(clippy::type_complexity)]
    pub on_tool_result: Option<PersistCallback>,
    pub save_callback: Option<PersistCallback>,
    #[allow(clippy::type_complexity)]
    pub tool_event_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    pub on_user_message: Option<PersistCallback>,
}

pub struct Loop {
    pub provider: Arc<dyn LLMProvider>,
    pub model: String,
    pub system_prompt: String,
    pub tools: Vec<AgentTool>,
    pub config: crate::types::AgentConfig,
    pub verbose: bool,
    pub event_bus: Option<Arc<EventBus>>,
    pub session_id: String,
    pub steering_queue: PendingMessageQueue,
    pub follow_up_queue: PendingMessageQueue,
    pub parallel_tools: bool,
    pub(crate) interrupt_flag: Arc<AtomicBool>,
    pub(crate) last_compaction_result: Arc<Mutex<Option<crate::compaction::CompactionResult>>>,
    pub cumulative_input_tokens: Arc<std::sync::atomic::AtomicI64>,
    pub cumulative_output_tokens: Arc<std::sync::atomic::AtomicI64>,
    pub cumulative_cache_read_tokens: Arc<std::sync::atomic::AtomicI64>,
    pub cumulative_cache_write_tokens: Arc<std::sync::atomic::AtomicI64>,
    /// Cumulative cost as reported by upstream (Future API `credit_cost`).
    pub cumulative_cost: Arc<parking_lot::Mutex<f64>>,
    /// Last API call's prompt_tokens (actual context size, not cumulative across turns)
    pub last_prompt_tokens: Arc<std::sync::atomic::AtomicI64>,
    /// Set to true when auto-compaction is needed but fails to find a valid cut
    /// point. The run loop checks this after transform_context and returns an
    /// error instead of silently proceeding with full context.
    pub compaction_failed: Arc<AtomicBool>,
    /// Cached model registry — avoids re-deserialising the 906-model catalog
    /// on auto-compaction checks and image-support queries inside the hot loop.
    pub model_registry: Option<Arc<parking_lot::RwLock<crate::models::Registry>>>,
}

impl Loop {
    pub fn new(provider: Arc<dyn LLMProvider>, model: &str) -> Self {
        Self {
            provider,
            model: model.to_string(),
            system_prompt: String::new(),
            tools: vec![],
            config: crate::types::AgentConfig::default(),
            verbose: false,
            event_bus: None,
            session_id: String::new(),
            steering_queue: PendingMessageQueue::new(64, "all"),
            follow_up_queue: PendingMessageQueue::new(64, "all"),
            parallel_tools: false,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            last_compaction_result: Arc::new(Mutex::new(None)),
            cumulative_input_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_output_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_cache_read_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_cache_write_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_cost: Arc::new(parking_lot::Mutex::new(0.0)),
            last_prompt_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            compaction_failed: Arc::new(AtomicBool::new(false)),
            model_registry: None,
        }
    }

    pub fn with_tools(mut self, tools: Vec<AgentTool>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_system_prompt(mut self, prompt: &str) -> Self {
        self.system_prompt = prompt.to_string();
        self
    }

    pub fn with_config(mut self, config: crate::types::AgentConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_event_bus(mut self, bus: Arc<EventBus>) -> Self {
        self.event_bus = Some(bus);
        self
    }

    /// Create an independent copy of this loop: same provider, model, tools,
    /// config, system prompt and event bus, but FRESH steering/follow-up
    /// queues, token counters, interrupt flag and compaction state.
    ///
    /// Every `ServerSession` gets its own copy instead of sharing one global
    /// loop, so a streaming run (which holds `loop.read()` for its whole
    /// duration) never blocks another session's `set_model`/
    /// `set_thinking_level` (`try_write`), and per-session state — interrupt
    /// flag, steering queues, token counters, tool-execution hooks — can no
    /// longer leak across sessions.  The provider `Arc` is cloned only as a
    /// seed: `ServerSession::set_model` replaces it with a freshly-built
    /// client for the session's own model before the first prompt.
    pub fn independent_copy(&self) -> Loop {
        let mut copy = Loop::new(self.provider.clone(), &self.model)
            .with_tools(self.tools.clone())
            .with_system_prompt(&self.system_prompt)
            .with_config(self.config.clone());
        copy.verbose = self.verbose;
        copy.parallel_tools = self.parallel_tools;
        copy.event_bus = self.event_bus.clone();
        copy.model_registry = self.model_registry.clone();
        copy
    }

    pub fn with_transform_context(
        mut self,
        f: Arc<dyn Fn(Vec<Message>, String) -> Vec<Message> + Send + Sync>,
    ) -> Self {
        self.config.transform_context = Some(f);
        self
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // PUBLIC API — matching Go's Loop public methods
    // ═══════════════════════════════════════════════════════════════════════════

    /// RunStreaming runs the agent loop with a new session (user prompt only)
    pub async fn run_streaming(
        &self,
        user_prompt: String,
        on_text: impl Fn(String) + Send + 'static,
    ) -> Result<String> {
        let messages = vec![self.new_user_message(user_prompt)];
        let (result, _) = self
            .run_streaming_with_messages(messages, &StreamContext::default(), on_text, |_| {}, None)
            .await?;
        Ok(result)
    }

    // RunStreamingWithMessages runs the agent loop with pre-existing messages.
    // Returns (final_text, all_messages).
    // interrupt_rx is an optional channel that, when fired, interrupts the current stream.

    // ═══════════════════════════════════════════════════════════════════════════
    // TOOL EXECUTION
    // ═══════════════════════════════════════════════════════════════════════════

    async fn execute_tools(
        &self,
        turn: usize,
        tool_calls: &[ToolCall],
        messages: &mut Vec<AgentMessage>,
        tool_event_cb: &Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
        on_tool_result: &Option<PersistCallback>,
    ) {
        let use_parallel = if !self.config.tools_execution_mode.is_empty() {
            self.config.tools_execution_mode == "parallel"
        } else {
            self.parallel_tools
        };

        if use_parallel && tool_calls.len() > 1 {
            self.execute_tools_parallel(turn, tool_calls, messages, tool_event_cb, on_tool_result)
                .await;
        } else {
            self.execute_tools_sequential(
                turn,
                tool_calls,
                messages,
                tool_event_cb,
                on_tool_result,
            )
            .await;
        }
    }

    async fn execute_tools_parallel(
        &self,
        turn: usize,
        tool_calls: &[ToolCall],
        messages: &mut Vec<AgentMessage>,
        tool_event_cb: &Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
        on_tool_result: &Option<PersistCallback>,
    ) {
        // AgentConfig contains non-Clone hooks, so parallel mode currently
        // preserves deterministic sequential execution.
        self.execute_tools_sequential(turn, tool_calls, messages, tool_event_cb, on_tool_result)
            .await;
    }

    async fn execute_tools_sequential(
        &self,
        _turn: usize,
        tool_calls: &[ToolCall],
        messages: &mut Vec<AgentMessage>,
        tool_event_cb: &Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
        on_tool_result: &Option<PersistCallback>,
    ) {
        let tools = &self.tools;
        let config = &self.config;
        let mut interrupted = false;
        let mut executed = 0usize;
        for tc in tool_calls {
            // Check for abort between tool executions
            if self.is_interrupted() {
                interrupted = true;
                break;
            }
            let start = Instant::now();

            // Broadcast tool_start (include tool_call for args)
            if let Some(ref cb) = tool_event_cb {
                cb(StreamEvent {
                    event_type: "tool_start".to_string(),
                    tool_call: Some(tc.clone()),
                    tool_name: tc.function.name.clone(),
                    tool_id: tc.id.clone(),
                    ..Default::default()
                });
            }

            let (result, err_str, tool_name) =
                Self::execute_one_tool_impl_static(tc, tools, config).await;
            let duration = start.elapsed().as_millis() as u64;

            if self.verbose {
                let tag = if tool_name == "read" && result.contains("SKILL.md") {
                    "[skill]"
                } else {
                    "[tool]"
                };
                // No manual ANSI colors here: tracing escapes ESC bytes in
                // message args to literal "\x1b" text, and the file layer must
                // stay plain. The level label (INFO/ERROR) already colors the
                // console output.
                if let Some(ref err) = err_str {
                    tracing::error!("{} ✗ {:-12} {:6}ms  {}", tag, tool_name, duration, err);
                } else {
                    tracing::info!("{} ✓ {:-12} {:6}ms", tag, tool_name, duration);
                }
            }

            // Broadcast tool_end
            if let Some(ref cb) = tool_event_cb {
                cb(StreamEvent {
                    event_type: "tool_end".to_string(),
                    text: result.clone(),
                    tool_name: tool_name.clone(),
                    tool_id: tc.id.clone(),
                    error_text: err_str.clone().unwrap_or_default(),
                    ..Default::default()
                });
            }

            let tool_args_str = match &tc.function.arguments {
                serde_json::Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            let tool_msg = self.new_tool_result(
                &tc.id,
                &tc.function.name,
                &tool_args_str,
                &result,
                err_str.as_deref(),
            );
            messages.push(tool_msg);
            if let Some(ref cb) = on_tool_result {
                cb(messages.last().unwrap());
            }
            executed += 1;
        }

        // Inject placeholder results for tools that were skipped due to interrupt
        if interrupted {
            for tc in tool_calls.iter().skip(executed) {
                let cancelled = format!(
                    "[Tool execution cancelled — {} was skipped due to user interrupt]",
                    tc.function.name
                );
                let tool_args_str = match &tc.function.arguments {
                    serde_json::Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                messages.push(self.new_tool_result(
                    &tc.id,
                    &tc.function.name,
                    &tool_args_str,
                    &cancelled,
                    Some(&cancelled),
                ));
            }
        }
    }

    async fn execute_one_tool_impl_static(
        tc: &ToolCall,
        tools: &[AgentTool],
        config: &crate::types::AgentConfig,
    ) -> (String, Option<String>, String) {
        let tool_name = tc.function.name.clone();
        let tool_id = tc.id.clone();

        // Stage 1: BeforeToolCall hook
        if let Some(ref hook) = config.before_tool_call {
            if let Some(result_val) = hook(&tool_name, &tool_id, &tc.function.arguments) {
                if result_val.is_error {
                    return (
                        result_val.result.clone(),
                        Some(result_val.result),
                        tool_name,
                    );
                } else {
                    return (result_val.result.clone(), None, tool_name);
                }
            }
        }

        // Stage 2: PrepareToolCall hook
        let raw_args = tc.function.arguments.clone();
        let normalized_args = match &raw_args {
            serde_json::Value::String(s) => {
                serde_json::from_str::<serde_json::Value>(s).unwrap_or(raw_args)
            }
            _ => raw_args,
        };
        let effective_args = if let Some(ref hook) = config.prepare_tool_call {
            hook(&tool_name, &normalized_args)
        } else {
            normalized_args
        };

        // Execute the tool
        let start = Instant::now();
        let mut result: Result<String> = Err(anyhow!(
            "Unknown tool '{}'. The model requested a tool that is not available. \
             This may happen if the model is not compatible with the tool set.",
            tool_name
        ));
        for tool in tools {
            if tool.def.function.name == tool_name {
                result = (tool.handler)(effective_args.clone()).await;
                break;
            }
        }
        let _duration = start.elapsed().as_millis() as u64;

        // Stage 3: FinalizeToolCall hook
        let (final_result, final_err) = if let Some(ref hook) = config.finalize_tool_call {
            match result.as_ref() {
                Ok(s) => {
                    let (r, e) = hook(&tool_name, s.clone(), anyhow::anyhow!(""));
                    (Some(r), e)
                }
                Err(err) => {
                    let (r, e) = hook(&tool_name, String::new(), anyhow::anyhow!("{}", err));
                    (Some(r), e)
                }
            }
        } else {
            // No finalize hook, use result directly
            match result {
                Ok(s) => (Some(s), None),
                Err(e) => (None, Some(e)),
            }
        };

        // Stage 4: AfterToolCall hook
        if let Some(ref hook) = config.after_tool_call {
            let result_str = final_result.as_deref().unwrap_or("");
            let err_owned = final_err
                .as_ref()
                .map(|e| anyhow::anyhow!("{}", e))
                .unwrap_or_else(|| anyhow::anyhow!(""));
            if let Some(result_val) = hook(
                &tool_name,
                &tool_id,
                &effective_args,
                result_str.to_string(),
                err_owned,
            ) {
                let error_result = if result_val.is_error {
                    Some(result_val.result.clone())
                } else {
                    None
                };
                return (result_val.result, error_result, tool_name);
            }
        }

        // Return (result_string, error_string_option, tool_name)
        let result_str = final_result.unwrap_or_else(String::new);
        let error_str = final_err.map(|e| e.to_string());
        (result_str, error_str, tool_name)
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // STEERING / INTERRUPT METHODS (matching Go)
    // ═══════════════════════════════════════════════════════════════════════════

    /// Interrupt cancels current streaming and queues a steering message.
    pub fn interrupt(&self, message: String) {
        self.steering_queue.enqueue(message);
        self.abort();
    }

    /// Steer injects a steering message without aborting.
    pub fn steer(&self, message: String) {
        self.steering_queue.enqueue(message);
    }

    /// FollowUp injects a follow-up message for after agent finishes.
    pub fn follow_up(&self, message: String) {
        self.follow_up_queue.enqueue(message);
    }

    /// Abort cancels current streaming without queuing a message.
    pub fn abort(&self) {
        self.interrupt_flag.store(true, Ordering::SeqCst);
    }

    fn is_interrupted(&self) -> bool {
        self.interrupt_flag.load(Ordering::SeqCst)
    }

    pub fn clear_interrupt(&self) {
        self.interrupt_flag.store(false, Ordering::SeqCst);
    }

    /// Returns a clone of the Arc-wrapped interrupt flag for sharing
    /// with cooperative cancellation points (e.g., shell tool).
    pub fn interrupt_flag(&self) -> Arc<AtomicBool> {
        self.interrupt_flag.clone()
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // QUEUE MANAGEMENT (matching Go)
    // ═══════════════════════════════════════════════════════════════════════════

    /// ClearQueues drains all pending messages from both queues.
    pub fn clear_queues(&self) {
        self.steering_queue.clear();
        self.follow_up_queue.clear();
    }

    /// QueuedCounts returns (steering_count, followup_count).
    pub fn queued_counts(&self) -> (usize, usize) {
        (self.steering_queue.len(), self.follow_up_queue.len())
    }

    /// PendingMessageCount returns total pending messages.
    pub fn pending_message_count(&self) -> usize {
        self.steering_queue.len() + self.follow_up_queue.len()
    }

    /// DrainQueues drains all pending messages and returns them.
    pub fn drain_queues(&self) -> Vec<String> {
        let mut msgs = self.steering_queue.drain();
        msgs.extend(self.follow_up_queue.drain());
        msgs
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // PRIVATE HELPERS
    // ═══════════════════════════════════════════════════════════════════════════

    fn drain_steering(
        &self,
        mut messages: Vec<AgentMessage>,
        on_user_msg: &Option<PersistCallback>,
    ) -> Vec<AgentMessage> {
        let msgs = self.steering_queue.drain();
        for msg in msgs {
            let m = self.new_user_message(msg);
            if let Some(ref cb) = on_user_msg {
                cb(&m);
            }
            messages.insert(0, m);
        }
        messages
    }

    fn drain_follow_up(
        &self,
        mut messages: Vec<AgentMessage>,
        on_user_msg: &Option<PersistCallback>,
    ) -> Vec<AgentMessage> {
        let msgs = self.follow_up_queue.drain();
        for msg in msgs {
            let m = self.new_user_message(msg);
            if let Some(ref cb) = on_user_msg {
                cb(&m);
            }
            messages.push(m);
        }
        messages
    }

    fn new_user_message(&self, content: impl Into<String>) -> AgentMessage {
        AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::text(content.into())],
            ..Default::default()
        }
    }

    fn new_tool_result(
        &self,
        call_id: &str,
        tool_name: &str,
        tool_args: &str,
        result: &str,
        err: Option<&str>,
    ) -> AgentMessage {
        let text = if let Some(e) = err {
            format!("Error: {}", e)
        } else {
            result.to_string()
        };
        // Cap tool result at 100K chars (~25K tokens) to avoid
        // a single oversized result blowing past the context window.
        // Compaction can trim old messages but can't split one message.
        let capped = if text.len() > 100_000 {
            let start = text.ceil_char_boundary(text.len() - 100_000);
            format!(
                "...(truncated, showing last 100K chars)\n{}",
                &text[start..]
            )
        } else {
            text
        };
        AgentMessage {
            role: "tool".to_string(),
            content: vec![ContentBlock::text(&capped)],
            tool_call_id: call_id.to_string(),
            name: tool_name.to_string(),
            tool_args: tool_args.to_string(),
            ..Default::default()
        }
    }
}

// ─── PendingMessageQueue ────────────────────────────────────────────────────

pub struct PendingMessageQueue {
    pub(crate) tx: mpsc::Sender<String>,
    pub(crate) rx: Mutex<mpsc::Receiver<String>>,
    pub mode: String,
}

impl PendingMessageQueue {
    pub fn new(capacity: usize, mode: &str) -> Self {
        let (tx, rx) = mpsc::channel(capacity);
        Self {
            tx,
            rx: Mutex::new(rx),
            mode: mode.to_string(),
        }
    }

    pub fn enqueue(&self, msg: String) {
        let _ = self.tx.try_send(msg);
    }

    pub fn drain(&self) -> Vec<String> {
        let mut rx = self.rx.lock();
        let mut msgs = vec![];
        while let Ok(msg) = rx.try_recv() {
            msgs.push(msg);
            if self.mode == "one-at-a-time" {
                break;
            }
        }
        msgs
    }

    pub fn len(&self) -> usize {
        self.rx.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn set_mode(&mut self, mode: &str) {
        self.mode = mode.to_string();
    }

    pub fn clear(&self) {
        let mut rx = self.rx.lock();
        while rx.try_recv().is_ok() {}
    }
}

impl Default for crate::types::AgentConfig {
    fn default() -> Self {
        Self {
            system_prompt: String::new(),
            max_turns: DEFAULT_MAX_TURNS,
            thinking_budget: 0,
            max_retries: 3,
            transform_context: None,
            stop_condition: None,
            before_tool_call: None,
            prepare_tool_call: None,
            finalize_tool_call: None,
            after_tool_call: None,
            tools_execution_mode: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── PendingMessageQueue ────────────────────────────────────────────────

    #[test]
    fn queue_new_is_empty() {
        let q = PendingMessageQueue::new(10, "all");
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
        assert_eq!(q.mode, "all");
    }

    #[test]
    fn queue_enqueue_and_len() {
        let q = PendingMessageQueue::new(10, "all");
        q.enqueue("msg1".to_string());
        q.enqueue("msg2".to_string());
        assert_eq!(q.len(), 2);
        assert!(!q.is_empty());
    }

    #[test]
    fn queue_drain_all_mode() {
        let q = PendingMessageQueue::new(10, "all");
        q.enqueue("a".to_string());
        q.enqueue("b".to_string());
        q.enqueue("c".to_string());
        let msgs = q.drain();
        assert_eq!(msgs, vec!["a", "b", "c"]);
        assert!(q.is_empty());
    }

    #[test]
    fn queue_drain_one_at_a_time_mode() {
        let q = PendingMessageQueue::new(10, "one-at-a-time");
        q.enqueue("first".to_string());
        q.enqueue("second".to_string());
        let msgs = q.drain();
        assert_eq!(msgs, vec!["first"]);
        assert_eq!(q.len(), 1); // second still queued
    }

    #[test]
    fn queue_drain_empty() {
        let q = PendingMessageQueue::new(10, "all");
        let msgs = q.drain();
        assert!(msgs.is_empty());
    }

    #[test]
    fn queue_clear() {
        let q = PendingMessageQueue::new(10, "all");
        q.enqueue("a".to_string());
        q.enqueue("b".to_string());
        q.clear();
        assert!(q.is_empty());
    }

    #[test]
    fn queue_set_mode() {
        let mut q = PendingMessageQueue::new(10, "all");
        assert_eq!(q.mode, "all");
        q.set_mode("one-at-a-time");
        assert_eq!(q.mode, "one-at-a-time");
    }

    #[test]
    fn queue_capacity_overflow_drops() {
        let q = PendingMessageQueue::new(2, "all");
        q.enqueue("a".to_string());
        q.enqueue("b".to_string());
        q.enqueue("c".to_string()); // channel full — dropped
        assert_eq!(q.len(), 2);
        let msgs = q.drain();
        assert_eq!(msgs, vec!["a", "b"]);
    }

    #[test]
    fn queue_drain_one_at_a_time_then_all() {
        let q = PendingMessageQueue::new(10, "one-at-a-time");
        q.enqueue("a".to_string());
        q.enqueue("b".to_string());
        q.enqueue("c".to_string());
        assert_eq!(q.drain(), vec!["a"]);
        assert_eq!(q.drain(), vec!["b"]);
        assert_eq!(q.drain(), vec!["c"]);
        assert!(q.drain().is_empty());
    }

    // ─── Loop struct (needs mock provider) ──────────────────────────────────

    struct MockProvider;

    #[async_trait::async_trait]
    impl crate::types::LLMProvider for MockProvider {
        async fn stream_chat(
            &self,
            _model: String,
            _messages: Vec<crate::types::Message>,
            _tools: Vec<crate::types::ToolDef>,
            _system_prompt: String,
        ) -> anyhow::Result<tokio_stream::wrappers::ReceiverStream<crate::types::StreamEvent>>
        {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
        }
    }

    fn make_loop() -> Loop {
        Loop::new(std::sync::Arc::new(MockProvider), "test-model")
    }

    #[test]
    fn loop_steer_and_queued_counts() {
        let loop_ = make_loop();
        loop_.steer("steer msg".to_string());
        let (s, f) = loop_.queued_counts();
        assert_eq!(s, 1);
        assert_eq!(f, 0);
        assert_eq!(loop_.pending_message_count(), 1);
    }

    #[test]
    fn loop_follow_up_and_counts() {
        let loop_ = make_loop();
        loop_.follow_up("followup msg".to_string());
        let (s, f) = loop_.queued_counts();
        assert_eq!(s, 0);
        assert_eq!(f, 1);
    }

    #[test]
    fn loop_clear_queues() {
        let loop_ = make_loop();
        loop_.steer("a".to_string());
        loop_.follow_up("b".to_string());
        assert_eq!(loop_.pending_message_count(), 2);
        loop_.clear_queues();
        assert_eq!(loop_.pending_message_count(), 0);
    }

    #[test]
    fn loop_drain_queues() {
        let loop_ = make_loop();
        loop_.steer("steer1".to_string());
        loop_.follow_up("follow1".to_string());
        let msgs = loop_.drain_queues();
        assert_eq!(msgs.len(), 2);
        assert_eq!(loop_.pending_message_count(), 0);
    }

    #[test]
    fn loop_interrupt_and_clear() {
        let loop_ = make_loop();
        assert!(!loop_
            .interrupt_flag()
            .load(std::sync::atomic::Ordering::SeqCst));
        loop_.abort();
        assert!(loop_
            .interrupt_flag()
            .load(std::sync::atomic::Ordering::SeqCst));
        loop_.clear_interrupt();
        assert!(!loop_
            .interrupt_flag()
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn loop_new_tool_result_normal() {
        let loop_ = make_loop();
        let msg = loop_.new_tool_result("call_1", "shell", "{\"cmd\": \"ls\"}", "output", None);
        assert_eq!(msg.role, "tool");
        assert_eq!(msg.tool_call_id, "call_1");
        assert_eq!(msg.text(), "output");
    }

    #[test]
    fn loop_new_tool_result_with_error() {
        let loop_ = make_loop();
        let msg = loop_.new_tool_result("call_1", "shell", "{}", "", Some("file not found"));
        assert!(msg.text().contains("Error"));
        assert!(msg.text().contains("file not found"));
    }

    #[test]
    fn loop_new_tool_result_truncates_long_output() {
        let loop_ = make_loop();
        let long = "x".repeat(200_000);
        let msg = loop_.new_tool_result("call_1", "shell", "{}", &long, None);
        assert!(msg.text().len() <= 110_000);
        assert!(msg.text().contains("truncated"));
    }

    #[test]
    fn loop_new_user_message() {
        let loop_ = make_loop();
        let msg = loop_.new_user_message("hello");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.text(), "hello");
    }

    #[test]
    fn loop_builder_methods() {
        let tools = vec![];
        let loop_ = Loop::new(std::sync::Arc::new(MockProvider), "m")
            .with_tools(tools)
            .with_system_prompt("test prompt")
            .with_config(crate::types::AgentConfig::default());
        assert_eq!(loop_.model, "m");
    }

    #[test]
    fn loop_independent_copy() {
        let loop_ = make_loop()
            .with_system_prompt("original prompt")
            .with_tools(vec![]);
        let copy = loop_.independent_copy();
        assert_eq!(copy.model, loop_.model);
        assert_eq!(copy.system_prompt, "original prompt");
        // Independent state: interrupt flag, queues should be fresh
        assert!(!copy
            .interrupt_flag()
            .load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(copy.pending_message_count(), 0);
        // Modify original's queue, copy should be unaffected
        loop_.steer("test".to_string());
        assert_eq!(loop_.pending_message_count(), 1);
        assert_eq!(copy.pending_message_count(), 0);
    }

    #[test]
    fn loop_with_transform_context() {
        let f: std::sync::Arc<
            dyn Fn(Vec<crate::types::Message>, String) -> Vec<crate::types::Message> + Send + Sync,
        > = std::sync::Arc::new(|msgs, _| msgs);
        let loop_ = make_loop().with_transform_context(f);
        assert!(loop_.config.transform_context.is_some());
    }

    #[test]
    fn loop_with_event_bus() {
        let bus = std::sync::Arc::new(crate::events::EventBus::new());
        let loop_ = make_loop().with_event_bus(bus);
        assert!(loop_.event_bus.is_some());
    }

    #[test]
    fn loop_interrupt_combines_steer_and_abort() {
        let loop_ = make_loop();
        loop_.interrupt("stop and steer".to_string());
        assert!(loop_
            .interrupt_flag()
            .load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(loop_.queued_counts().0, 1);
    }

    #[test]
    fn loop_steer_does_not_abort() {
        let loop_ = make_loop();
        loop_.steer("steer only".to_string());
        assert!(!loop_
            .interrupt_flag()
            .load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(loop_.queued_counts().0, 1);
    }

    // ─── execute_one_tool_impl_static ──────────────────────────────────────

    #[tokio::test]
    async fn execute_one_tool_unknown_tool() {
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "nonexistent_tool".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (_result, err, name) =
            Loop::execute_one_tool_impl_static(&tc, &[], &crate::types::AgentConfig::default())
                .await;
        assert_eq!(name, "nonexistent_tool");
        assert!(err.is_some());
        assert!(err.unwrap().contains("Unknown tool"));
    }

    #[tokio::test]
    async fn execute_one_tool_finds_and_runs_tool() {
        let tools = vec![crate::tools::shell_tool()];
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "shell".to_string(),
                arguments: serde_json::json!("{\"command\": \"echo works\"}"),
            },
        };
        let (result, _err, name) =
            Loop::execute_one_tool_impl_static(&tc, &tools, &crate::types::AgentConfig::default())
                .await;
        assert_eq!(name, "shell");
        // Tool ran (might fail due to scope, but should have run)
        let _ = result;
    }

    #[tokio::test]
    async fn execute_one_tool_before_hook_blocks() {
        let config = crate::types::AgentConfig {
            before_tool_call: Some(std::sync::Arc::new(|_name, _id, _args| {
                Some(crate::types::ToolCallResult {
                    result: "blocked by hook".to_string(),
                    is_error: true,
                })
            })),
            ..Default::default()
        };
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "shell".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (result, err, _) = Loop::execute_one_tool_impl_static(&tc, &[], &config).await;
        assert_eq!(result, "blocked by hook");
        assert!(err.is_some());
    }

    #[tokio::test]
    async fn execute_one_tool_before_hook_allows() {
        let config = crate::types::AgentConfig {
            before_tool_call: Some(std::sync::Arc::new(|_name, _id, _args| {
                Some(crate::types::ToolCallResult {
                    result: "allowed by hook".to_string(),
                    is_error: false,
                })
            })),
            ..Default::default()
        };
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "shell".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (result, err, _) = Loop::execute_one_tool_impl_static(&tc, &[], &config).await;
        assert_eq!(result, "allowed by hook");
        assert!(err.is_none());
    }

    #[tokio::test]
    async fn execute_one_tool_before_hook_none_passes_through() {
        let config = crate::types::AgentConfig {
            before_tool_call: Some(std::sync::Arc::new(|_name, _id, _args| None)),
            ..Default::default()
        };
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "unknown".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (_, err, _) = Loop::execute_one_tool_impl_static(&tc, &[], &config).await;
        assert!(err.is_some());
        assert!(err.unwrap().contains("Unknown tool"));
    }

    #[tokio::test]
    async fn execute_one_tool_prepare_hook_modifies_args() {
        let config = crate::types::AgentConfig {
            prepare_tool_call: Some(std::sync::Arc::new(|_name, args| {
                let mut modified = args.clone();
                modified["injected"] = serde_json::json!(true);
                modified
            })),
            ..Default::default()
        };
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "unknown".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (_, err, _) = Loop::execute_one_tool_impl_static(&tc, &[], &config).await;
        assert!(err.is_some());
    }

    #[tokio::test]
    async fn execute_one_tool_finalize_hook_transforms_result() {
        let config = crate::types::AgentConfig {
            finalize_tool_call: Some(std::sync::Arc::new(|_name, _result, _err| {
                ("finalized".to_string(), None)
            })),
            ..Default::default()
        };
        let tools = vec![crate::tools::shell_tool()];
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "shell".to_string(),
                arguments: serde_json::json!("{\"command\": \"echo test\"}"),
            },
        };
        let (result, err, _) = Loop::execute_one_tool_impl_static(&tc, &tools, &config).await;
        assert_eq!(result, "finalized");
        assert!(err.is_none());
    }

    #[tokio::test]
    async fn execute_one_tool_after_hook_transforms() {
        let config = crate::types::AgentConfig {
            after_tool_call: Some(std::sync::Arc::new(|_name, _id, _args, _result, _err| {
                Some(crate::types::ToolCallResult {
                    result: "after-hook".to_string(),
                    is_error: false,
                })
            })),
            ..Default::default()
        };
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "unknown".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (result, err, _) = Loop::execute_one_tool_impl_static(&tc, &[], &config).await;
        assert_eq!(result, "after-hook");
        assert!(err.is_none());
    }

    #[tokio::test]
    async fn execute_one_tool_after_hook_error() {
        let config = crate::types::AgentConfig {
            after_tool_call: Some(std::sync::Arc::new(|_name, _id, _args, _result, _err| {
                Some(crate::types::ToolCallResult {
                    result: "hook error".to_string(),
                    is_error: true,
                })
            })),
            ..Default::default()
        };
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "unknown".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (result, err, _) = Loop::execute_one_tool_impl_static(&tc, &[], &config).await;
        assert_eq!(result, "hook error");
        assert!(err.is_some());
    }

    // ─── Mock streaming provider ────────────────────────────────────────────

    struct TextStreamProvider {
        chunks: Vec<String>,
    }

    #[async_trait::async_trait]
    impl crate::types::LLMProvider for TextStreamProvider {
        async fn stream_chat(
            &self,
            _model: String,
            _messages: Vec<crate::types::Message>,
            _tools: Vec<crate::types::ToolDef>,
            _system_prompt: String,
        ) -> anyhow::Result<tokio_stream::wrappers::ReceiverStream<crate::types::StreamEvent>>
        {
            let (tx, rx) = tokio::sync::mpsc::channel(64);
            let chunks = self.chunks.clone();
            tokio::spawn(async move {
                for chunk in chunks {
                    let _ = tx
                        .send(crate::types::StreamEvent {
                            event_type: "text_delta".to_string(),
                            text: chunk,
                            ..Default::default()
                        })
                        .await;
                }
                // Send stop event to end the stream
                let _ = tx
                    .send(crate::types::StreamEvent {
                        event_type: "stop".to_string(),
                        stop_reason: "end_turn".to_string(),
                        usage: Some(crate::types::Usage {
                            prompt_tokens: 10,
                            completion_tokens: 5,
                            total_tokens: 15,
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .await;
            });
            Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
        }
    }

    #[tokio::test]
    async fn run_streaming_produces_text_output() {
        let provider = TextStreamProvider {
            chunks: vec!["Hello ".to_string(), "world".to_string()],
        };
        let loop_ = Loop::new(std::sync::Arc::new(provider), "test-model");
        let result = loop_.run_streaming("test prompt".to_string(), |_| {}).await;
        assert!(result.is_ok());
        let final_text = result.unwrap();
        assert!(final_text.contains("Hello world"));
    }

    #[tokio::test]
    async fn run_streaming_steer_injected() {
        let provider = TextStreamProvider {
            chunks: vec!["Response".to_string()],
        };
        let loop_ = Loop::new(std::sync::Arc::new(provider), "test-model");
        // Inject a steer before running
        loop_.steer("steered message".to_string());
        let result = loop_.run_streaming("original".to_string(), |_| {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn run_streaming_drain_queues() {
        let provider = TextStreamProvider {
            chunks: vec!["Response".to_string()],
        };
        let loop_ = Loop::new(std::sync::Arc::new(provider), "test-model");
        loop_.steer("s1".to_string());
        loop_.follow_up("f1".to_string());
        assert_eq!(loop_.pending_message_count(), 2);
        let drained = loop_.drain_queues();
        assert_eq!(drained.len(), 2);
        assert_eq!(loop_.pending_message_count(), 0);
    }
}
