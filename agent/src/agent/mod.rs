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

// ANSI terminal colors (matching Go)
const C_RESET: &str = "\x1b[0m";
const C_RED: &str = "\x1b[31m";
const C_GREEN: &str = "\x1b[32m";
const C_MAGENTA: &str = "\x1b[35m";

pub const DEFAULT_MAX_TURNS: i32 = 0; // 0 = unlimited

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
    pub tool_event_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    /// Called after each tool result is pushed to messages, so the session
    /// can be persisted incrementally during long streaming runs.
    pub on_tool_result: Option<Arc<dyn Fn() + Send + Sync>>,
    /// General save callback — also called after assistant messages are
    /// pushed, not just tool results.
    pub save_callback: Option<Arc<dyn Fn() + Send + Sync>>,
    pub cumulative_input_tokens: Arc<std::sync::atomic::AtomicI64>,
    pub cumulative_output_tokens: Arc<std::sync::atomic::AtomicI64>,
    pub cumulative_cache_read_tokens: Arc<std::sync::atomic::AtomicI64>,
    pub cumulative_cache_write_tokens: Arc<std::sync::atomic::AtomicI64>,
    /// Cumulative cost as reported by upstream (Future API `credit_cost`).
    pub cumulative_cost: Arc<parking_lot::Mutex<f64>>,
    /// Last API call's prompt_tokens (actual context size, not cumulative across turns)
    pub last_prompt_tokens: Arc<std::sync::atomic::AtomicI64>,
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
            tool_event_callback: None,
            on_tool_result: None,
            save_callback: None,
            cumulative_input_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_output_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_cache_read_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_cache_write_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_cost: Arc::new(parking_lot::Mutex::new(0.0)),
            last_prompt_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
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
            .run_streaming_with_messages(messages, on_text, |_| {}, None)
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
    ) {
        let use_parallel = if !self.config.tools_execution_mode.is_empty() {
            self.config.tools_execution_mode == "parallel"
        } else {
            self.parallel_tools
        };

        if use_parallel && tool_calls.len() > 1 {
            self.execute_tools_parallel(turn, tool_calls, messages)
                .await;
        } else {
            self.execute_tools_sequential(turn, tool_calls, messages)
                .await;
        }
    }

    async fn execute_tools_parallel(
        &self,
        turn: usize,
        tool_calls: &[ToolCall],
        messages: &mut Vec<AgentMessage>,
    ) {
        // AgentConfig contains non-Clone hooks, so parallel mode currently
        // preserves deterministic sequential execution.
        self.execute_tools_sequential(turn, tool_calls, messages)
            .await;
    }

    async fn execute_tools_sequential(
        &self,
        _turn: usize,
        tool_calls: &[ToolCall],
        messages: &mut Vec<AgentMessage>,
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
            if let Some(ref cb) = self.tool_event_callback {
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
                let color = if err_str.is_some() { C_RED } else { C_GREEN };
                if let Some(ref err) = err_str {
                    tracing::error!(
                        "{} {}✗ {:-12} {:6}ms  {}",
                        tag,
                        color,
                        tool_name,
                        duration,
                        err
                    );
                } else {
                    tracing::info!("{} {}✓ {:-12} {:6}ms", tag, color, tool_name, duration);
                }
            }

            // Broadcast tool_end
            if let Some(ref cb) = self.tool_event_callback {
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
            if let Some(ref cb) = self.on_tool_result {
                cb();
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

    fn drain_steering(&self, mut messages: Vec<AgentMessage>) -> Vec<AgentMessage> {
        let msgs = self.steering_queue.drain();
        for msg in msgs {
            messages.insert(0, self.new_user_message(msg));
        }
        messages
    }

    fn drain_follow_up(&self, mut messages: Vec<AgentMessage>) -> Vec<AgentMessage> {
        let msgs = self.follow_up_queue.drain();
        for msg in msgs {
            messages.push(self.new_user_message(msg));
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
