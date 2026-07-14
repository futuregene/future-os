use crate::events::{
    self, agent_end, agent_end_with_stop_reason, agent_start, error_event, message_end,
    message_start, text_delta, text_end, text_start, thinking_delta, thinking_end, thinking_start,
    tool_end, tool_start, toolcall_delta, toolcall_end, toolcall_start, turn_start, usage_event,
};
use crate::types::{
    AgentMessage, AgentToolCall, ContentBlock, ConvertFromLLM, ConvertToLLM, Message, StreamEvent,
    ToolCall,
};
use anyhow::{anyhow, Result};
use std::time::Duration;
use tokio::time::sleep;
use tokio_stream::StreamExt;

use super::{Loop, C_GREEN, C_MAGENTA, C_RESET, DEFAULT_MAX_TURNS};

const STREAM_EVENT_IDLE_TIMEOUT: Duration = Duration::from_secs(45);
const COMPLETE_TOOL_CALL_IDLE_TIMEOUT: Duration = Duration::from_secs(15);

impl Loop {
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
                return Err(anyhow!(
                    "Internal error: conversation ended with an assistant message. \
                     This is a bug — please report it."
                ));
            }
        }

        let max_turns = if self.config.max_turns > 0 {
            self.config.max_turns as usize
        } else {
            DEFAULT_MAX_TURNS.max(0) as usize // 0 = unlimited
        };

        // Emit agent_start
        if let Some(ref bus) = self.event_bus {
            bus.emit(agent_start(&self.session_id, &self.model, ""));
        }
        on_event(StreamEvent {
            event_type: "agent_start".to_string(),
            ..Default::default()
        });

        let tool_defs: Vec<_> = self.tools.iter().map(|t| t.def.clone()).collect();
        let mut last_error: Option<anyhow::Error> = None;
        let mut last_stop_reason = String::new();
        let mut retry_attempt = 0;

        if self.verbose {
            tracing::info!(
                "[agent] starting run model={} msgs={} tools={}",
                self.model,
                messages.len(),
                tool_defs.len()
            );
        }

        let mut turn: usize = 0;
        loop {
            // Check max turn limit (0 = unlimited)
            if max_turns > 0 && turn >= max_turns {
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
                return Err(anyhow!(
                    "Reached the turn limit ({}). The agent tried too many tool-call \
                     rounds without completing. You can increase the limit in settings \
                     (max_turns) or try a simpler prompt.",
                    max_turns
                ));
            }
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

            if self.verbose {
                tracing::info!("[agent] turn={} calling LLM model={} msgs={} tools={} sys_prompt_len={} msg_chars={}",
                    turn,
                    self.model,
                    llm_messages.len(),
                    tool_defs.len(),
                    self.system_prompt.len(),
                    llm_messages.iter().map(|m| {
                        m.content.as_ref().map(|c| c.to_string().len()).unwrap_or(0)
                    }).sum::<usize>()
                );
            }

            // Stream chat — interruptible so a stop during connect / TLS /
            // time-to-first-byte takes effect immediately instead of blocking
            // on the request to return (especially noticeable on Windows where
            // flaky connections make this phase slow).
            let stream_result = match self
                .await_or_interrupt(
                    self.provider.stream_chat(
                        self.model.clone(),
                        llm_messages,
                        tool_defs.clone(),
                        self.system_prompt.clone(),
                    ),
                    interrupt_rx.as_mut(),
                )
                .await
            {
                Some(r) => r,
                None => {
                    if let Some(ref bus) = self.event_bus {
                        bus.emit(agent_end("interrupted", None));
                    }
                    return Ok((String::new(), messages));
                }
            };

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
                        // If this looks like a context-length or body-size
                        // error, compact before retrying. Auto-compaction
                        // only runs BEFORE a turn (based on last turn's token
                        // count), so it can't help on the first call.
                        let err_msg = format!("{}", last_error.as_ref().unwrap());
                        if is_retryable_size_error(&err_msg) {
                            if let Some(ref bus) = self.event_bus {
                                bus.emit(events::compaction_start("auto"));
                            }
                            let context_window = 200000i32;
                            let reserve = ((context_window as f64 * 0.1) as i32).max(16384);
                            let (compacted, compact_result) = crate::compaction::compact(
                                ConvertToLLM(&messages),
                                &crate::compaction::CompactOptions {
                                    reserve_tokens: reserve,
                                    keep_recent_tokens: reserve,
                                    context_window,
                                    tokens_before: 999999, // force compaction
                                },
                            );
                            messages = ConvertFromLLM(compacted);
                            if let Some(r) = compact_result {
                                *self.last_compaction_result.lock().unwrap() = Some(r);
                            }
                            if let Some(ref bus) = self.event_bus {
                                bus.emit(events::compaction_end(0, "", false, "auto"));
                            }
                        }
                        // Don't burn a retry (and its backoff) if the user
                        // already asked to stop.
                        if self.is_interrupted() {
                            if let Some(ref bus) = self.event_bus {
                                bus.emit(agent_end("interrupted", None));
                            }
                            return Ok((String::new(), messages));
                        }
                        retry_attempt += 1;
                        let delay_ms = 2000 * (1 << (retry_attempt - 1));
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(events::auto_retry_start(
                                retry_attempt,
                                self.config.max_retries as usize,
                                delay_ms,
                            ));
                        }
                        // Interruptible backoff: wake up immediately when the
                        // user hits stop instead of sleeping out the full delay
                        // (2s + 4s + 8s = up to 14s of unresponsiveness).
                        if self
                            .sleep_or_interrupt(
                                Duration::from_millis(delay_ms as u64),
                                interrupt_rx.as_mut(),
                            )
                            .await
                        {
                            if let Some(ref bus) = self.event_bus {
                                bus.emit(agent_end("interrupted", None));
                            }
                            return Ok((String::new(), messages));
                        }
                        continue;
                    }
                    if let Some(ref bus) = self.event_bus {
                        bus.emit(agent_end("error", None));
                    }
                    let err = last_error.unwrap();
                    tracing::error!("LLM call failed: {:#}", err);
                    return Err(err);
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
            let mut current_tool_calls: Vec<Option<AgentToolCall>> = vec![];
            let mut output_started = false;
            let mut was_outputting = false;
            let mut stream_error = None;
            // Set when the LLM layer signals the stream was cut off (idle
            // timeout or premature EOF without a finish_reason / `[DONE]`).
            // The accumulated text is a prefix, not a finished answer.
            let mut stream_truncated = false;

            loop {
                let event_idle_timeout = if current_tool_calls
                    .iter()
                    .any(|tc| tc.as_ref().map(tool_call_args_complete).unwrap_or(false))
                {
                    COMPLETE_TOOL_CALL_IDLE_TIMEOUT
                } else {
                    STREAM_EVENT_IDLE_TIMEOUT
                };

                let mut event_timed_out = false;
                let event = if let Some(ref mut irx) = interrupt_rx {
                    match tokio::time::timeout(event_idle_timeout, async {
                        tokio::select! {
                            event_opt = rx.next() => event_opt,
                            _ = irx.recv() => {
                                stream_error = Some(anyhow!("interrupted"));
                                None
                            }
                        }
                    })
                    .await
                    {
                        Ok(inner) => inner,
                        Err(_) => {
                            event_timed_out = true;
                            None
                        }
                    }
                } else {
                    match tokio::time::timeout(event_idle_timeout, rx.next()).await {
                        Ok(inner) => inner,
                        Err(_) => {
                            event_timed_out = true;
                            None
                        }
                    }
                };

                let event = match event {
                    Some(e) => e,
                    None => {
                        // No event for the whole idle window means the LLM layer
                        // went silent without delivering a terminal event — the
                        // stream stalled. Mark it truncated so the turn ends as
                        // `incomplete`, not a silent `complete`. (A normal end
                        // arrives as the channel closing right after a `stop`,
                        // which is not a timeout.)
                        if event_timed_out {
                            stream_truncated = true;
                        }
                        break;
                    }
                };
                on_event(event.clone());

                // Close the text-output block before switching to a different
                // event type — text_end may never arrive from the LLM.
                let is_text = matches!(event.event_type.as_str(), "text" | "text_delta");
                if self.verbose && was_outputting && !is_text {
                    eprintln!();
                    was_outputting = false;
                }
                if is_text && self.verbose {
                    was_outputting = true;
                }

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
                            eprintln!(); // blank line after thinking
                        }
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(thinking_end());
                        }
                    }
                    "text" | "text_delta" => {
                        assistant_text.push_str(&event.text);
                        if self.verbose && !output_started {
                            output_started = true;
                            eprint!("\n{}[output]{} ", C_GREEN, C_RESET);
                        }
                        if self.verbose {
                            eprint!("{}", event.text);
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
                        if self.verbose {
                            eprintln!(); // blank line after output
                        }
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(text_end());
                        }
                    }
                    "toolcall_start" => {
                        // Some providers (e.g. GLM/Z.AI without tool_stream) send
                        // id+name in every argument chunk instead of just the first.
                        // When the tool ID matches an existing tool call at this
                        // index, treat it as a delta (append args) rather than
                        // starting a new call.
                        //
                        // Always prefer the longer string — it's more complete.
                        // Some gateways (e.g. Aliyun MaaS) may send chunks out of
                        // prefix order, or send a trailing fragment that is shorter
                        // than the accumulated args. Overwriting longer data with
                        // shorter data is the primary cause of argument loss.
                        let idx = event.tc_index;
                        if idx < current_tool_calls.len() {
                            if let Some(ref mut existing) = current_tool_calls[idx] {
                                if existing.id == event.tool_id {
                                    // Same tool call at same index — append args
                                    if let Some(ref tc) = event.tool_call {
                                        if let serde_json::Value::String(ref new_args) =
                                            tc.function.arguments
                                        {
                                            let mut updated = false;
                                            if let serde_json::Value::String(ref mut s) =
                                                existing.args
                                            {
                                                if new_args.len() > s.len() {
                                                    if new_args.starts_with(s.as_str()) {
                                                        s.push_str(&new_args[s.len()..]);
                                                    } else {
                                                        *s = new_args.clone();
                                                    }
                                                }
                                                updated = true;
                                            }
                                            if !updated {
                                                existing.args =
                                                    serde_json::Value::String(new_args.clone());
                                            }
                                        }
                                    }
                                    // Emit toolcall_delta so the TUI can stream arg display
                                    if let Some(ref bus) = self.event_bus {
                                        bus.emit(toolcall_delta(
                                            event
                                                .tool_call
                                                .as_ref()
                                                .and_then(|tc| {
                                                    if let serde_json::Value::String(ref s) =
                                                        tc.function.arguments
                                                    {
                                                        Some(s.as_str())
                                                    } else {
                                                        None
                                                    }
                                                })
                                                .unwrap_or(""),
                                        ));
                                    }
                                    continue;
                                }
                            }
                        }

                        // Expand vec to accommodate this index if needed
                        if idx >= current_tool_calls.len() {
                            current_tool_calls.resize(idx + 1, None);
                        }

                        // Finalize any existing tool call at this index (different id)
                        if let Some(tc) = current_tool_calls[idx].take() {
                            tool_calls.push(finalize_agent_tool_call(tc));
                            if let Some(ref bus) = self.event_bus {
                                bus.emit(toolcall_end());
                            }
                        }

                        let args = event
                            .tool_call
                            .as_ref()
                            .map(|tc| tc.function.arguments.clone())
                            .unwrap_or(serde_json::Value::Null);
                        current_tool_calls[idx] = Some(AgentToolCall {
                            id: event.tool_id.clone(),
                            name: event.tool_name.clone(),
                            args,
                        });
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(toolcall_start(&event.tool_name, &event.tool_id));
                        }
                    }
                    "toolcall_delta" => {
                        let idx = event.tc_index;
                        if idx >= current_tool_calls.len() {
                            current_tool_calls.resize(idx + 1, None);
                        }
                        if let Some(tc_ref) = &mut current_tool_calls[idx] {
                            if let serde_json::Value::String(ref mut s) = tc_ref.args {
                                // Some proxies (e.g. Anthropic → OpenAI) send the
                                // full current state of the tool input JSON in every
                                // delta, not incremental fragments. Detect this by
                                // checking if the delta starts with '{' — incremental
                                // OpenAI fragments always start with ',' or another
                                // continuation character. When the accumulated args
                                // are empty, just "{}", or a prefix of the delta,
                                // replace instead of concatenating to avoid corrupt
                                // JSON like {}{"path":...}.
                                if event.text.starts_with('{')
                                    && (s.is_empty()
                                        || s == "{}"
                                        || event.text.starts_with(s.as_str()))
                                {
                                    *s = event.text.clone();
                                } else {
                                    s.push_str(&event.text);
                                }
                            } else {
                                tc_ref.args = serde_json::Value::String(event.text.clone());
                            }
                        } else {
                            // Delta arrived before start — create placeholder
                            current_tool_calls[idx] = Some(AgentToolCall {
                                id: String::new(),
                                name: String::new(),
                                args: serde_json::Value::String(event.text.clone()),
                            });
                        }
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(toolcall_delta(&event.text));
                        }
                    }
                    "tool_call" | "toolcall_end" => {
                        if let Some(ref u) = event.usage {
                            self.process_usage_event(u, &mut total_usage);
                        }
                        for tc_opt in current_tool_calls.iter_mut() {
                            if let Some(tc) = tc_opt.take() {
                                tool_calls.push(finalize_agent_tool_call(tc));
                                if let Some(ref bus) = self.event_bus {
                                    bus.emit(toolcall_end());
                                }
                            }
                        }
                    }
                    "tool_start" => {
                        if self.verbose {
                            tracing::info!("[tool] {} → starting", event.tool_name);
                        }
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(tool_start(&event.tool_name, &event.tool_id));
                        }
                    }
                    "tool_end" => {
                        if self.verbose {
                            tracing::info!("[tool] {} ← done", event.tool_name);
                        }
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(tool_end(&event.tool_name));
                        }
                    }
                    "usage" => {
                        if let Some(ref u) = event.usage {
                            self.process_usage_event(u, &mut total_usage);
                        }
                        if !event.stop_reason.is_empty() {
                            last_stop_reason = event.stop_reason.clone();
                        }
                    }
                    "stop" => {
                        // A `truncated` stop_reason means the stream was cut off
                        // mid-flight (idle timeout / premature EOF) rather than
                        // reaching a real finish. Remember it so the turn ends as
                        // `incomplete` instead of `complete`.
                        if event.stop_reason == "truncated" {
                            stream_truncated = true;
                        }
                        // Process usage if attached to this event (e.g. when
                        // the same chunk carries both usage and finish_reason).
                        if let Some(ref u) = event.usage {
                            self.process_usage_event(u, &mut total_usage);
                        }
                        for tc_opt in current_tool_calls.iter_mut() {
                            if let Some(tc) = tc_opt.take() {
                                tool_calls.push(finalize_agent_tool_call(tc));
                                if let Some(ref bus) = self.event_bus {
                                    bus.emit(toolcall_end());
                                }
                            }
                        }
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

            for tc_opt in current_tool_calls.iter_mut() {
                if let Some(tc) = tc_opt.take() {
                    tool_calls.push(finalize_agent_tool_call(tc));
                    if let Some(ref bus) = self.event_bus {
                        bus.emit(toolcall_end());
                    }
                }
            }

            // Check for stream errors before processing results
            if let Some(_err) = stream_error {
                // If steering messages are pending, drain and restart
                if !self.steering_queue.is_empty() {
                    messages = self.drain_steering(messages);
                    continue;
                }
                if let Some(ref bus) = self.event_bus {
                    bus.emit(agent_end("interrupted", None));
                }
                return Ok((String::new(), messages));
            }

            // Check for pending interrupt (may have arrived during API call
            // or last stream event — tokio::select! can pick stream end over
            // the interrupt channel)
            if let Some(ref mut irx) = interrupt_rx {
                if irx.try_recv().is_ok() {
                    // Same interrupt path as above
                    if self.steering_queue.is_empty() {
                        if let Some(ref bus) = self.event_bus {
                            bus.emit(agent_end("interrupted", None));
                        }
                        return Ok((String::new(), messages));
                    }
                    messages = self.drain_steering(messages);
                }
            }

            // Close any open output block (text_end may not have been emitted).
            if self.verbose && output_started {
                eprintln!();
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

            // Stream was truncated mid-reply: the assistant text is a prefix,
            // not a finished answer. End the turn as `incomplete` (keeping the
            // partial text so it isn't lost) rather than draining the follow-up
            // queue or presenting a cut-off reply as `complete`. Tool calls, if
            // any, are left unexecuted — their arguments may be partial.
            if stream_truncated {
                if let Some(ref bus) = self.event_bus {
                    bus.emit(agent_end_with_stop_reason(
                        "incomplete",
                        total_usage.as_ref(),
                        &last_stop_reason,
                    ));
                }
                if self.verbose {
                    tracing::warn!(
                        "[agent] stream truncated turns={} output_len={}",
                        turn + 1,
                        assistant_text.len()
                    );
                }
                return Ok((assistant_text, messages));
            }

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
                        ..Default::default()
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
                if self.verbose {
                    tracing::info!(
                        "[agent] complete turns={} output_len={}",
                        turn + 1,
                        assistant_text.len()
                    );
                }
                return Ok((assistant_text, messages));
            }

            // Execute tools
            if self.verbose {
                tracing::info!(
                    "[agent] turn={} executing {} tools: {}",
                    turn,
                    tool_calls.len(),
                    tool_calls
                        .iter()
                        .map(|t| t.function.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            self.execute_tools(turn, &tool_calls, &mut messages).await;

            if let Some(ref bus) = self.event_bus {
                bus.emit(events::turn_end(turn));
            }

            last_error = None;
            turn += 1;
        }
    }

    /// Sleep for `dur`, returning early with `true` if an interrupt arrives —
    /// either signalled on `interrupt_rx` or via the shared interrupt flag.
    /// Returns `false` if the full duration elapsed without interruption.
    ///
    /// The flag is polled every 50ms (matching the bash tool) so an `abort()`
    /// that only sets the flag — without a channel send — is still caught
    /// promptly.
    async fn sleep_or_interrupt(
        &self,
        dur: Duration,
        mut interrupt_rx: Option<&mut tokio::sync::mpsc::Receiver<()>>,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + dur;
        let poll = Duration::from_millis(50);
        loop {
            if self.is_interrupted() {
                return true;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return false;
            }
            let step = poll.min(deadline - now);
            match interrupt_rx {
                Some(ref mut rx) => {
                    tokio::select! {
                        _ = sleep(step) => {}
                        _ = rx.recv() => return true,
                    }
                }
                None => sleep(step).await,
            }
        }
    }

    /// Await `fut`, bailing out early with `None` if an interrupt arrives on
    /// `interrupt_rx` or via the shared interrupt flag; returns `Some(output)`
    /// if the future completed first. The flag is polled every 50ms as a
    /// fallback for aborts that only set the flag without a channel send.
    async fn await_or_interrupt<F, T>(
        &self,
        fut: F,
        mut interrupt_rx: Option<&mut tokio::sync::mpsc::Receiver<()>>,
    ) -> Option<T>
    where
        F: std::future::Future<Output = T>,
    {
        tokio::pin!(fut);
        let poll = Duration::from_millis(50);
        loop {
            if self.is_interrupted() {
                return None;
            }
            match interrupt_rx {
                Some(ref mut rx) => {
                    tokio::select! {
                        out = &mut fut => return Some(out),
                        _ = rx.recv() => return None,
                        _ = sleep(poll) => {}
                    }
                }
                None => {
                    tokio::select! {
                        out = &mut fut => return Some(out),
                        _ = sleep(poll) => {}
                    }
                }
            }
        }
    }

    fn process_usage_event(
        &self,
        u: &crate::types::Usage,
        total_usage: &mut Option<crate::types::Usage>,
    ) {
        use std::sync::atomic::Ordering;
        self.cumulative_input_tokens
            .fetch_add(u.prompt_tokens, Ordering::Relaxed);
        self.last_prompt_tokens
            .store(u.prompt_tokens + u.completion_tokens, Ordering::Relaxed);
        self.cumulative_output_tokens
            .fetch_add(u.completion_tokens, Ordering::Relaxed);
        if let Some(cache_r) = u.cache_read_tokens {
            self.cumulative_cache_read_tokens
                .fetch_add(cache_r, Ordering::Relaxed);
        }
        if let Some(cache_w) = u.cache_write_tokens {
            self.cumulative_cache_write_tokens
                .fetch_add(cache_w, Ordering::Relaxed);
        }
        *total_usage = Some(u.clone());
        if let Some(ref bus) = self.event_bus {
            bus.emit(usage_event(u));
        }
    }
}

fn tool_call_args_complete(tool_call: &AgentToolCall) -> bool {
    match &tool_call.args {
        serde_json::Value::String(raw) => serde_json::from_str::<serde_json::Value>(raw)
            .map(|value| value.is_object())
            .unwrap_or(false),
        serde_json::Value::Object(_) => true,
        _ => false,
    }
}

fn finalize_agent_tool_call(mut tool_call: AgentToolCall) -> ToolCall {
    repair_partial_tool_args(&mut tool_call.args);
    ToolCall {
        id: tool_call.id,
        call_type: "function".to_string(),
        function: crate::types::ToolCallFn {
            name: tool_call.name,
            arguments: tool_call.args,
        },
    }
}

fn repair_partial_tool_args(args: &mut serde_json::Value) {
    let serde_json::Value::String(raw) = args else {
        return;
    };
    // Empty string is never valid tool-call arguments — treat as empty object
    // so the tool handler gets a proper "missing field" error instead of
    // "invalid type: string \"\", expected struct BashParams".
    if raw.is_empty() {
        *raw = String::from("{}");
        return;
    }
    if serde_json::from_str::<serde_json::Value>(raw).is_ok() {
        return;
    }
    let Some(repaired) = repair_partial_json_object(raw) else {
        return;
    };
    if serde_json::from_str::<serde_json::Value>(&repaired).is_ok() {
        *raw = repaired;
    }
}

fn repair_partial_json_object(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if !trimmed.starts_with('{') {
        return None;
    }

    let mut repaired = trimmed.to_string();
    if has_unclosed_string(&repaired) {
        repaired.push('"');
    }

    let open_braces = repaired.chars().filter(|c| *c == '{').count();
    let close_braces = repaired.chars().filter(|c| *c == '}').count();
    if open_braces > close_braces {
        for _ in 0..(open_braces - close_braces) {
            repaired.push('}');
        }
    }

    Some(repaired)
}

fn has_unclosed_string(value: &str) -> bool {
    let mut in_string = false;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            _ => {}
        }
    }
    in_string
}

/// Returns true when the error message indicates the request was rejected
/// because the body is too large — either exceeding the model's context window
/// or hitting a reverse-proxy / gateway body-size limit.
///
/// These errors are retryable if we compact the conversation history first.
fn is_retryable_size_error(err_msg: &str) -> bool {
    // ── Explicit context-length errors from the LLM provider ──────────
    if err_msg.contains("maximum context")
        || err_msg.contains("context_length")
        || err_msg.contains("reduce the length")
        || err_msg.contains("too long")
    {
        return true;
    }

    // ── Empty-body HTTP 400 — typical of reverse-proxy / gateway ─────
    //     rejection (nginx client_max_body_size, Cloudflare WAF, etc.)
    if err_msg.contains("No response body") {
        return true;
    }

    // ── Our improved diagnostic messages from llm/mod.rs ─────────────
    if err_msg.contains("reverse-proxy or gateway") || err_msg.contains("request body too large") {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_error_matches_context_length() {
        assert!(is_retryable_size_error("maximum context length exceeded"));
        assert!(is_retryable_size_error("context_length_exceeded"));
        assert!(is_retryable_size_error("reduce the length of the messages"));
        assert!(is_retryable_size_error("request too long"));
    }

    #[test]
    fn size_error_matches_empty_body_400() {
        assert!(is_retryable_size_error(
            "API request failed (HTTP 400). No response body."
        ));
    }

    #[test]
    fn size_error_matches_gateway_rejection() {
        assert!(is_retryable_size_error(
            "API request failed (HTTP 400). No response body. This usually indicates a reverse-proxy or gateway issue"
        ));
        assert!(is_retryable_size_error(
            "request body too large for nginx client_max_body_size"
        ));
    }

    #[test]
    fn size_error_ignores_unrelated_errors() {
        assert!(!is_retryable_size_error(
            "Authentication failed (401). Check your API key."
        ));
        assert!(!is_retryable_size_error(
            "Rate limited (429). The API is throttling requests."
        ));
        assert!(!is_retryable_size_error("Connection timed out"));
        assert!(!is_retryable_size_error(""));
    }

    #[test]
    fn duplicate_id_fallback_prefers_longer_args() {
        /// Simulates the duplicate-id fallback: merge `new_args` into `existing`.
        fn merge_args(existing: &str, new_args: &str) -> String {
            if new_args.len() > existing.len() {
                if new_args.starts_with(existing) {
                    format!("{}{}", existing, &new_args[existing.len()..])
                } else {
                    new_args.to_string()
                }
            } else {
                existing.to_string()
            }
        }

        // Scenario 1: Normal incremental (each chunk extends previous)
        assert_eq!(merge_args("", "{\""), "{\"");
        assert_eq!(merge_args("{\"", "{\"path"), "{\"path");
        assert_eq!(
            merge_args("{\"path", "{\"path\":\"/etc/hosts\"}"),
            "{\"path\":\"/etc/hosts\"}"
        );

        // Scenario 2: Shorter/equal — keep existing (prevents data loss)
        let good = "{\"path\":\"/Users/ace/.future/agent/skills/future-web/SKILL.md\"}";
        assert_eq!(merge_args(good, "\"}"), good);
        assert_eq!(merge_args(good, good), good);
        assert_eq!(merge_args(good, ""), good);

        // Scenario 3: Longer non-prefix replacement
        assert_eq!(
            merge_args("{\"key2\":\"val2\"}", "{\"key\":\"val\",\"key2\":\"val2\"}"),
            "{\"key\":\"val\",\"key2\":\"val2\"}"
        );

        // Scenario 4: Partial → complete replacement
        assert_eq!(
            merge_args("{\"pa", "{\"path\":\"/etc/hosts\"}"),
            "{\"path\":\"/etc/hosts\"}"
        );
    }

    #[test]
    fn toolcall_delta_detects_full_replacement() {
        /// Simulates the toolcall_delta handler: merge `delta` into `existing`,
        /// detecting full-state replacements (Anthropic partial_json style).
        fn accumulate(existing: &str, delta: &str) -> String {
            if delta.starts_with('{')
                && (existing.is_empty() || existing == "{}" || delta.starts_with(existing))
            {
                delta.to_string()
            } else {
                format!("{}{}", existing, delta)
            }
        }

        // Scenario 1: Standard OpenAI — first fragment starts with {, empty args
        assert_eq!(
            accumulate("", "{\"path\": \"/file.txt\""),
            "{\"path\": \"/file.txt\""
        );

        // Scenario 2: Standard OpenAI — incremental fragment (starts with comma)
        assert_eq!(
            accumulate("{\"path\": \"/file.txt\"", ", \"content\": \"hello\"}"),
            "{\"path\": \"/file.txt\", \"content\": \"hello\"}"
        );

        // Scenario 3: Anthropic full replacement — delta extends current
        assert_eq!(
            accumulate(
                "{\"path\": \"/file.txt\", \"content\": \"hello",
                "{\"path\": \"/file.txt\", \"content\": \"hello world\"}"
            ),
            "{\"path\": \"/file.txt\", \"content\": \"hello world\"}"
        );

        // Scenario 4: Anthropic — first delta after toolcall_start with "{}" args
        // (e.g. proxy emits toolcall_start with empty-object args, then delta
        // carries the full partial_json). Delta starts with { but does NOT start
        // with "{}" — must still replace, not concatenate.
        assert_eq!(
            accumulate("{}", "{\"path\": \"/file.txt\", \"content\": \"hello\"}"),
            "{\"path\": \"/file.txt\", \"content\": \"hello\"}"
        );

        // Scenario 5: Anthropic — first delta from empty args
        assert_eq!(
            accumulate(
                "",
                "{\"path\": \"/file.txt\", \"content\": \"hello world\"}"
            ),
            "{\"path\": \"/file.txt\", \"content\": \"hello world\"}"
        );

        // Scenario 6: Nested JSON in content value — delta starts with { but
        // doesn't match the accumulated prefix and s is not empty/{}
        assert_eq!(
            accumulate(
                "{\"path\": \"/file.txt\", \"content\": \"",
                "{\"nested\": \"value\"}\"}"
            ),
            "{\"path\": \"/file.txt\", \"content\": \"{\"nested\": \"value\"}\"}"
        );

        // Scenario 7: Corrupted accumulated state — delta doesn't match prefix,
        // s is non-empty and not "{}" → append (best effort)
        assert_eq!(
            accumulate(
                "{\"path\": \"/file.txt\"}{\"path\": \"/file.txt\", \"content\": \"hello",
                "{\"path\": \"/file.txt\", \"content\": \"hello world\"}"
            ),
            "{\"path\": \"/file.txt\"}{\"path\": \"/file.txt\", \"content\": \"hello{\"path\": \"/file.txt\", \"content\": \"hello world\"}"
        );
    }
}
