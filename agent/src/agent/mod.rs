//! Agent Loop — 1:1 compatible with Go internal/agent/

use crate::events::{
    self, agent_end, agent_end_with_stop_reason, agent_start, error_event, message_end,
    message_start, text_delta, text_end, text_start, thinking_delta, thinking_end, thinking_start,
    tool_end, tool_start, toolcall_delta, toolcall_end, toolcall_start, turn_start, usage_event,
    EventBus,
};
use crate::types::{
    AgentMessage, AgentTool, AgentToolCall, ContentBlock, ConvertFromLLM, ConvertToLLM,
    LLMProvider, Message, StreamEvent, ToolCall,
};
use anyhow::{anyhow, Result};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_stream::StreamExt;

// ANSI terminal colors (matching Go)
const C_RESET: &str = "\x1b[0m";
const C_RED: &str = "\x1b[31m";
const C_GREEN: &str = "\x1b[32m";
const C_MAGENTA: &str = "\x1b[35m";

pub const DEFAULT_MAX_TURNS: i32 = 50;

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
    pub(crate) interrupt_tx: Mutex<Option<mpsc::Sender<()>>>,
    pub(crate) last_compaction_result: Arc<Mutex<Option<crate::compaction::CompactionResult>>>,
    pub tool_event_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    pub cumulative_input_tokens: Arc<std::sync::atomic::AtomicI64>,
    pub cumulative_output_tokens: Arc<std::sync::atomic::AtomicI64>,
    pub cumulative_cache_read_tokens: Arc<std::sync::atomic::AtomicI64>,
    pub cumulative_cache_write_tokens: Arc<std::sync::atomic::AtomicI64>,
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
            interrupt_tx: Mutex::new(None),
            last_compaction_result: Arc::new(Mutex::new(None)),
            tool_event_callback: None,
            cumulative_input_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_output_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_cache_read_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_cache_write_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
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

    /// RunStreamingWithMessages runs the agent loop with pre-existing messages.
    /// Returns (final_text, all_messages).
    /// `interrupt_rx` is an optional channel that, when fired, interrupts the current stream.
    pub async fn run_streaming_with_messages(
        &self,
        mut messages: Vec<AgentMessage>,
        on_text: impl Fn(String) + Send + 'static,
        on_event: impl Fn(StreamEvent) + Send + 'static,
        mut interrupt_rx: Option<tokio::sync::mpsc::Receiver<()>>,
    ) -> Result<(String, Vec<AgentMessage>)> {
        // Validate: last message must not be from assistant
        if let Some(last) = messages.last() {
            if last.role == "assistant" {
                return Err(anyhow!("agent: last message must not be from assistant"));
            }
        }

        let max_turns = if self.config.max_turns > 0 {
            self.config.max_turns as usize
        } else {
            DEFAULT_MAX_TURNS as usize
        };

        // Emit agent_start
        if let Some(ref bus) = self.event_bus {
            bus.emit(agent_start(&self.session_id, &self.model, ""));
        }
        on_event(StreamEvent {
            event_type: "agent_start".to_string(),
            text: String::new(),
            tool_call: None,
            tool_name: String::new(),
            tool_id: String::new(),
            usage: None,
            stop_reason: String::new(),
            error_text: String::new(),
        });

        let tool_defs: Vec<_> = self.tools.iter().map(|t| t.def.clone()).collect();
        let mut last_error = None;
        let mut last_stop_reason = String::new();
        let mut retry_attempt = 0;

        for turn in 0..max_turns {
            // Drain steering queue FIRST
            let steering_before = self.steering_queue.len();
            messages = self.drain_steering(messages);

            // Check cancellation — only exit if no steering was just drained
            if self.is_interrupted() {
                if steering_before == 0 {
                    // Pure interrupt → exit cleanly
                    if let Some(ref bus) = self.event_bus {
                        bus.emit(agent_end("interrupted", None));
                    }
                    return Ok((String::new(), messages));
                }
                // Steering message was queued → reset interrupt flag and continue
                self.clear_interrupt();
            }

            // Emit turn_start
            if let Some(ref bus) = self.event_bus {
                bus.emit(turn_start(turn));
            }

            // Apply TransformContext if configured (e.g., compaction)
            let work_messages = if let Some(ref transform_fn) = self.config.transform_context {
                let before_len = messages.len();
                let llm_msgs: Vec<Message> = ConvertToLLM(&messages);
                let transformed = transform_fn(llm_msgs, String::new());
                let result = ConvertFromLLM(transformed);
                if result.len() < before_len {
                    // Compaction happened
                    if let Some(ref bus) = self.event_bus {
                        bus.emit(events::compaction_start("auto"));
                    }
                    let mut comp_result = self.last_compaction_result.lock().unwrap();
                    let (tokens_before, summary) = if let Some(ref r) = *comp_result {
                        (r.tokens_before, r.summary.clone())
                    } else {
                        (0, String::new())
                    };
                    *comp_result = None;
                    if let Some(ref bus) = self.event_bus {
                        bus.emit(events::compaction_end(
                            tokens_before,
                            &summary,
                            false,
                            "auto",
                        ));
                    }
                }
                result
            } else {
                messages.clone()
            };

            // Emit message_start
            if let Some(ref bus) = self.event_bus {
                bus.emit(message_start("assistant"));
            }

            // Convert to LLM format
            let llm_messages: Vec<Message> = ConvertToLLM(&work_messages);

            // Stream chat
            let stream_result = self
                .provider
                .stream_chat(
                    self.model.clone(),
                    llm_messages,
                    tool_defs.clone(),
                    self.system_prompt.clone(),
                )
                .await;

            let mut rx = match stream_result {
                Ok(rx) => rx,
                Err(e) => {
                    if let Some(ref bus) = self.event_bus {
                        bus.emit(error_event(&e.to_string()));
                    }
                    last_error = Some(e);
                    if self.config.max_retries > 0
                        && retry_attempt < self.config.max_retries as usize
                    {
                        retry_attempt += 1;
                        let delay_ms = 2000 * (1 << (retry_attempt - 1));
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(events::auto_retry_start(
                                retry_attempt,
                                self.config.max_retries as usize,
                                delay_ms,
                            ));
                        }
                        sleep(Duration::from_millis(delay_ms as u64)).await;
                        continue;
                    }
                    if let Some(ref bus) = self.event_bus {
                        bus.emit(agent_end("error", None));
                    }
                    return Err(last_error.unwrap().context("turn 0"));
                }
            };

            // Reset retry on successful stream
            if retry_attempt > 0 {
                if let Some(ref bus) = self.event_bus {
                    bus.emit(events::auto_retry_end());
                }
                retry_attempt = 0;
            }

            // Process stream events
            let mut assistant_text = String::new();
            let mut reasoning_text = String::new();
            let mut tool_calls: Vec<ToolCall> = vec![];
            let mut total_usage: Option<crate::types::Usage> = None;
            let mut current_tc: Option<AgentToolCall> = None;
            let mut output_started = false;
            let mut stream_error = None;

            loop {
                let event = if let Some(ref mut irx) = interrupt_rx {
                    tokio::select! {
                        event_opt = rx.next() => event_opt,
                        _ = irx.recv() => {
                            stream_error = Some(anyhow!("interrupted"));
                            break;
                        }
                    }
                } else {
                    rx.next().await
                };

                let event = match event {
                    Some(e) => e,
                    None => break,
                };
                on_event(event.clone());

                match event.event_type.as_str() {
                    "thinking_start" => {
                        if self.verbose {
                            eprint!("\n{}[thinking]{} ", C_MAGENTA, C_RESET);
                        }
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(thinking_start());
                        }
                    }
                    "thinking_delta" => {
                        reasoning_text.push_str(&event.text);
                        if self.verbose {
                            eprint!("{}", event.text);
                        }
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(thinking_delta(&event.text));
                        }
                    }
                    "thinking_end" => {
                        if self.verbose {
                            eprintln!();
                        }
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(thinking_end());
                        }
                    }
                    "text" | "text_delta" => {
                        assistant_text.push_str(&event.text);
                        if self.verbose && !output_started {
                            output_started = true;
                            eprintln!();
                        }
                        on_text(event.text.clone());
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(text_delta(&event.text));
                        }
                    }
                    "text_start" => {
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(text_start());
                        }
                    }
                    "text_end" => {
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(text_end());
                        }
                    }
                    "toolcall_start" => {
                        current_tc = Some(AgentToolCall {
                            id: event.tool_id.clone(),
                            name: event.tool_name.clone(),
                            args: serde_json::Value::Null,
                        });
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(toolcall_start(&event.tool_name, &event.tool_id));
                        }
                    }
                    "toolcall_delta" => {
                        if let Some(ref mut tc) = current_tc {
                            if let serde_json::Value::String(ref mut s) = tc.args {
                                s.push_str(&event.text);
                            } else {
                                tc.args = serde_json::Value::String(event.text.clone());
                            }
                        }
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(toolcall_delta(&event.text));
                        }
                    }
                    "tool_call" | "toolcall_end" => {
                        if let Some(tc) = current_tc.take() {
                            tool_calls.push(ToolCall {
                                id: tc.id.clone(),
                                call_type: "function".to_string(),
                                function: crate::types::ToolCallFn {
                                    name: tc.name.clone(),
                                    arguments: tc.args,
                                },
                            });
                            if let Some(ref bus) = self.event_bus {
                                bus.emit(toolcall_end());
                            }
                        }
                    }
                    "tool_start" => {
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(tool_start(&event.tool_name, &event.tool_id));
                        }
                    }
                    "tool_end" => {
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(tool_end(&event.tool_name));
                        }
                    }
                    "usage" => {
                        if let Some(ref u) = event.usage {
                            self.cumulative_input_tokens
                                .fetch_add(u.prompt_tokens, std::sync::atomic::Ordering::Relaxed);
                            self.last_prompt_tokens
                                .store(u.prompt_tokens, std::sync::atomic::Ordering::Relaxed);
                            self.cumulative_output_tokens.fetch_add(
                                u.completion_tokens,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            if let Some(cache_r) = u.cache_read_tokens {
                                self.cumulative_cache_read_tokens
                                    .fetch_add(cache_r, std::sync::atomic::Ordering::Relaxed);
                            }
                            if let Some(cache_w) = u.cache_write_tokens {
                                self.cumulative_cache_write_tokens
                                    .fetch_add(cache_w, std::sync::atomic::Ordering::Relaxed);
                            }
                            total_usage = Some(u.clone());
                            if let Some(ref bus) = self.event_bus {
                                bus.emit(usage_event(u));
                            }
                        }
                        if !event.stop_reason.is_empty() {
                            last_stop_reason = event.stop_reason.clone();
                        }
                    }
                    "stop" => {
                        // done
                    }
                    "error" => {
                        stream_error = Some(anyhow!("{}", event.error_text));
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(error_event(&event.error_text));
                        }
                    }
                    _ => {}
                }
            }

            // Check for stream errors before processing results
            if let Some(err) = stream_error {
                last_error = Some(err);
                // If steering messages are pending, drain and restart
                if !self.steering_queue.is_empty() {
                    messages = self.drain_steering(messages);
                    last_error = None;
                    continue;
                }
                if let Some(ref bus) = self.event_bus {
                    bus.emit(agent_end("error", None));
                }
                return Err(last_error.unwrap());
            }

            // Emit message_end
            if let Some(ref bus) = self.event_bus {
                bus.emit(message_end("assistant"));
            }

            // Build assistant message
            let mut assistant_msg = AgentMessage {
                role: "assistant".to_string(),
                content: if !assistant_text.is_empty() {
                    vec![ContentBlock::text(&assistant_text)]
                } else {
                    vec![]
                },
                thinking: reasoning_text.clone(),
                tool_calls: vec![],
                ..Default::default()
            };

            // Convert LLM tool calls to agent tool calls
            for tc in &tool_calls {
                assistant_msg.tool_calls.push(AgentToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    args: tc.function.arguments.clone(),
                });
            }
            messages.push(assistant_msg);

            // Check stop condition
            if let Some(ref stop_fn) = self.config.stop_condition {
                let llm_msgs: Vec<Message> = ConvertToLLM(&messages);
                if stop_fn(llm_msgs, &assistant_text) {
                    if let Some(ref bus) = self.event_bus {
                        bus.emit(agent_end_with_stop_reason(
                            "stop_condition",
                            total_usage.as_ref(),
                            &last_stop_reason,
                        ));
                    }
                    return Ok((assistant_text, messages));
                }
            }

            // If no tool calls, check follow-up queue
            if tool_calls.is_empty() {
                if !self.follow_up_queue.is_empty() {
                    messages = self.drain_follow_up(messages);
                    if let Some(ref bus) = self.event_bus {
                        bus.emit(events::turn_end(turn));
                    }
                    // Emit agent_start so the TUI creates a new assistant block
                    // for the follow-up response (under the follow-up user message).
                    on_event(StreamEvent {
                        event_type: "agent_start".to_string(),
                        text: String::new(),
                        tool_call: None,
                        tool_name: String::new(),
                        tool_id: String::new(),
                        usage: None,
                        stop_reason: String::new(),
                        error_text: String::new(),
                    });
                    continue;
                }
                if let Some(ref bus) = self.event_bus {
                    bus.emit(agent_end_with_stop_reason(
                        "complete",
                        total_usage.as_ref(),
                        &last_stop_reason,
                    ));
                }
                return Ok((assistant_text, messages));
            }

            // Execute tools
            self.execute_tools(turn, &tool_calls, &mut messages).await;

            if let Some(ref bus) = self.event_bus {
                bus.emit(events::turn_end(turn));
            }

            last_error = None;
        }

        if let Some(ref bus) = self.event_bus {
            bus.emit(agent_end_with_stop_reason(
                "max_turns",
                None,
                &last_stop_reason,
            ));
        }

        if let Some(last_error) = last_error {
            return Err(last_error.context("exceeded max turns"));
        }
        Err(anyhow!("exceeded max turns ({})", max_turns))
    }

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
        // For now, use sequential execution since AgentConfig contains non-Clone fields
        // TODO: implement true parallel execution with Arc<AgentConfig>
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
        for tc in tool_calls {
            let start = Instant::now();

            // Broadcast tool_start (include tool_call for args)
            if let Some(ref cb) = self.tool_event_callback {
                cb(StreamEvent {
                    event_type: "tool_start".to_string(),
                    text: String::new(),
                    tool_call: Some(tc.clone()),
                    tool_name: tc.function.name.clone(),
                    tool_id: tc.id.clone(),
                    usage: None,
                    stop_reason: String::new(),
                    error_text: String::new(),
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
                    eprintln!(
                        "\n{}{}{} ✗ {:-12} {:6}ms  {}",
                        color, tag, C_RESET, tool_name, duration, err
                    );
                } else {
                    eprintln!(
                        "\n{}{}{} ✓ {:-12} {:6}ms",
                        color, tag, C_RESET, tool_name, duration
                    );
                }
            }

            // Broadcast tool_end
            if let Some(ref cb) = self.tool_event_callback {
                cb(StreamEvent {
                    event_type: "tool_end".to_string(),
                    text: result.clone(),
                    tool_call: None,
                    tool_name: tool_name.clone(),
                    tool_id: tc.id.clone(),
                    usage: None,
                    stop_reason: String::new(),
                    error_text: err_str.clone().unwrap_or_default(),
                });
            }

            let tool_msg = self.new_tool_result(&tc.id, &result, err_str.as_deref());
            messages.push(tool_msg);
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
        let mut result: Result<String> = Err(anyhow!("tool {} not found", tool_name));
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
        let mut tx = self.interrupt_tx.lock().unwrap();
        if let Some(ref sender) = *tx {
            let _ = sender.try_send(());
        }
        *tx = None;
    }

    fn is_interrupted(&self) -> bool {
        let tx = self.interrupt_tx.lock().unwrap();
        tx.is_some()
    }

    fn clear_interrupt(&self) {
        let mut tx = self.interrupt_tx.lock().unwrap();
        *tx = None;
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

    fn new_tool_result(&self, call_id: &str, result: &str, err: Option<&str>) -> AgentMessage {
        let text = if let Some(e) = err {
            format!("Error: {}", e)
        } else {
            result.to_string()
        };
        AgentMessage {
            role: "tool".to_string(),
            content: vec![ContentBlock::text(&text)],
            tool_call_id: call_id.to_string(),
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
        let mut rx = self.rx.lock().unwrap();
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
        self.rx.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn set_mode(&mut self, mode: &str) {
        self.mode = mode.to_string();
    }

    pub fn clear(&self) {
        let mut rx = self.rx.lock().unwrap();
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
