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
        ctx: &super::StreamContext,
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

        // Emit agent_start. Carry the run's wall-clock start so clients that
        // attach late (or replay a buffered event) can anchor their live elapsed
        // timer to the real run start instead of the event's arrival time.
        let started_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default();
        on_event(StreamEvent {
            event_type: "agent_start".to_string(),
            payload: Some(serde_json::json!({ "started_at_ms": started_at_ms })),
            ..Default::default()
        });

        let tool_defs: Vec<_> = self.tools.iter().map(|t| t.def.clone()).collect();
        let mut last_error: Option<anyhow::Error> = None;
        let mut retry_attempt = 0;

        if self.verbose {
            tracing::info!(
                "[agent] starting run model={} msgs={} tools={}",
                ctx.model,
                messages.len(),
                tool_defs.len()
            );
        }

        let mut turn: usize = 0;
        loop {
            // Check max turn limit (0 = unlimited)
            if max_turns > 0 && turn >= max_turns {
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
            // Cancellation always terminates this run. A later user submission
            // is a distinct scheduler-owned run and can never be injected here.
            if self.is_interrupted() {
                return Ok((String::new(), messages));
            }

            // Emit turn_start

            // Apply TransformContext if configured (e.g., compaction)
            let work_messages = if let Some(ref transform_fn) = self.config.transform_context {
                let before_len = messages.len();
                let llm_msgs: Vec<Message> = ConvertToLLM(&messages);
                let transformed = transform_fn(llm_msgs, String::new());
                let result = ConvertFromLLM(transformed);
                if result.len() < before_len {
                    // Compaction happened — replace in-memory messages with
                    // compacted ones so the save path persists the trimmed
                    // history instead of the full (now discarded) prefix.
                    messages = result.clone();
                    let compaction = self.last_compaction_result.lock().take();
                    let (tokens_before, summary) = compaction
                        .map(|result| (result.tokens_before, result.summary))
                        .unwrap_or((0, String::new()));
                    on_event(StreamEvent {
                        event_type: "compaction_end".to_string(),
                        payload: Some(serde_json::json!({
                            "tokens_before": tokens_before,
                            "summary": summary,
                            "aborted": false,
                            "reason": "auto",
                        })),
                        ..Default::default()
                    });
                }
                result
            } else {
                messages.clone()
            };

            // Auto-compaction was needed but failed — context is overflowing
            // the model's window. Stop instead of silently proceeding.
            if self
                .compaction_failed
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(anyhow!(
                    "context compaction failed: conversation overflows model context window"
                ));
            }

            // Emit message_start

            // Convert to LLM format
            let llm_messages: Vec<Message> = ConvertToLLM(&work_messages);

            if self.verbose {
                tracing::info!("[agent] turn={} calling LLM model={} msgs={} tools={} sys_prompt_len={} msg_chars={}",
                    turn,
                    ctx.model,
                    llm_messages.len(),
                    tool_defs.len(),
                    ctx.system_prompt.len(),
                    llm_messages.iter().map(|m| {
                        m.content.as_ref().map(|c| c.to_string().len()).unwrap_or(0)
                    }).sum::<usize>()
                );
            }

            // Stream chat — interruptible so a stop during connect / TLS /
            // time-to-first-byte takes effect immediately instead of blocking
            // on the request to return (especially noticeable on Windows where
            // flaky connections make this phase slow).
            // Drain any mid-turn steering notes and fold them into this step's
            // system prompt (the frozen snapshot semantics for model/tools/
            // config are unchanged — steering is an additive operator channel).
            let step_system_prompt =
                fold_steering_into_prompt(&ctx.system_prompt, &mut self.steering_notes.lock());
            let stream_result = match self
                .await_or_interrupt(
                    self.provider.stream_chat(
                        ctx.model.clone(),
                        llm_messages,
                        tool_defs.clone(),
                        step_system_prompt,
                    ),
                    interrupt_rx.as_mut(),
                )
                .await
            {
                Some(r) => r,
                None => {
                    return Ok((String::new(), messages));
                }
            };

            let mut rx = match stream_result {
                Ok(rx) => rx,
                Err(e) => {
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
                            // Resolve the model's actual context window so we don't
                            // over-compact large-context models (1M+).
                            // Use the cached registry from the loop to avoid
                            // re-deserialising the model catalog; loops not
                            // derived from the app template (model_registry =
                            // None, e.g. tests) fall back to a fresh Registry
                            // so behaviour matches the pre-cache code.
                            let context_window = self
                                .model_registry
                                .as_ref()
                                .and_then(|r| r.read().resolve(&ctx.model))
                                .or_else(|| crate::models::Registry::new().resolve(&ctx.model))
                                .map(|m| m.context_window)
                                .unwrap_or(1_000_000);
                            let reserve = ((context_window as f64 * 0.1) as i32).max(16384);
                            let keep_tokens = ((context_window as f64 * 0.2) as i32).max(reserve);
                            let (compacted, compact_result) = crate::compaction::compact(
                                ConvertToLLM(&messages),
                                &crate::compaction::CompactOptions {
                                    reserve_tokens: reserve,
                                    keep_recent_tokens: keep_tokens,
                                    context_window,
                                    tokens_before: 999999, // force compaction
                                },
                            );
                            messages = ConvertFromLLM(compacted);
                            if let Some(r) = compact_result {
                                on_event(StreamEvent {
                                    event_type: "compaction_end".to_string(),
                                    payload: Some(serde_json::json!({
                                        "tokens_before": r.tokens_before,
                                        "summary": r.summary.clone(),
                                        "aborted": false,
                                        "reason": "auto",
                                    })),
                                    ..Default::default()
                                });
                                *self.last_compaction_result.lock() = Some(r);
                            } else {
                                // Forced compaction (context-length error) failed to
                                // find any valid cut point. The conversation cannot
                                // continue safely — report the error and stop.
                                tracing::error!(
                                    "forced compaction after context-length error failed"
                                );
                                return Err(anyhow!(
                                    "context compaction failed: conversation overflows model context window"
                                ));
                            }
                        }
                        // Don't burn a retry (and its backoff) if the user
                        // already asked to stop.
                        if self.is_interrupted() {
                            return Ok((String::new(), messages));
                        }
                        retry_attempt += 1;
                        let delay_ms = 2000 * (1 << (retry_attempt - 1));
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
                            return Ok((String::new(), messages));
                        }
                        continue;
                    }
                    let err = last_error.unwrap();
                    tracing::error!("LLM call failed: {:#}", err);
                    return Err(err);
                }
            };

            // Reset retry on successful stream
            if retry_attempt > 0 {
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
            let mut saw_terminal_event = false;

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
                        if event_timed_out || !saw_terminal_event {
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
                    crate::eprintln_log!();
                    was_outputting = false;
                }
                if is_text && self.verbose {
                    was_outputting = true;
                }

                match event.event_type.as_str() {
                    "thinking_start" => {
                        if self.verbose {
                            crate::eprint_log!("\n{}[thinking]{} ", C_MAGENTA, C_RESET);
                        }
                    }
                    "thinking_delta" => {
                        reasoning_text.push_str(&event.text);
                        if self.verbose {
                            crate::eprint_log!("{}", event.text);
                        }
                    }
                    "thinking_end" => {
                        if self.verbose {
                            crate::eprintln_log!(); // blank line after thinking
                        }
                    }
                    "text" | "text_delta" => {
                        assistant_text.push_str(&event.text);
                        if self.verbose && !output_started {
                            output_started = true;
                            crate::eprint_log!("\n{}[output]{} ", C_GREEN, C_RESET);
                        }
                        if self.verbose {
                            crate::eprint_log!("{}", event.text);
                        }
                        on_text(event.text.clone());
                    }
                    "text_start" => {}
                    "text_end" => {
                        if self.verbose {
                            crate::eprintln_log!(); // blank line after output
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
                    }
                    "tool_call" | "toolcall_end" => {
                        if let Some(ref u) = event.usage {
                            self.process_usage_event(u, &mut total_usage);
                        }
                        for tc_opt in current_tool_calls.iter_mut() {
                            if let Some(tc) = tc_opt.take() {
                                tool_calls.push(finalize_agent_tool_call(tc));
                            }
                        }
                    }
                    "tool_start" => {
                        if self.verbose {
                            tracing::info!("[tool] {} → starting", event.tool_name);
                        }
                    }
                    "tool_end" => {
                        if self.verbose {
                            tracing::info!("[tool] {} ← done", event.tool_name);
                        }
                    }
                    "usage" => {
                        if let Some(ref u) = event.usage {
                            self.process_usage_event(u, &mut total_usage);
                        }
                    }
                    "stop" => {
                        saw_terminal_event = true;
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
                            }
                        }
                    }
                    "error" => {
                        stream_error = Some(anyhow!("{}", event.error_text));
                    }
                    _ => {}
                }
            }

            for tc_opt in current_tool_calls.iter_mut() {
                if let Some(tc) = tc_opt.take() {
                    tool_calls.push(finalize_agent_tool_call(tc));
                }
            }

            // Build a partial assistant message from whatever was accumulated
            // before the interrupt — reasoning, text, and tool calls should
            // survive an abort so the user doesn't lose generated content.
            // Tool calls that were never executed MUST be followed by
            // placeholder tool-result messages, otherwise the LLM API rejects
            // the conversation on resume (HTTP 400: "assistant message with
            // tool_calls must be followed by tool messages").
            let build_partial_assistant =
                |messages: &mut Vec<AgentMessage>,
                 assistant_text: &str,
                 reasoning_text: &str,
                 tool_calls: &[ToolCall]| {
                    let first_new_message = messages.len();
                    let mut msg = AgentMessage {
                        role: "assistant".to_string(),
                        content: if !assistant_text.is_empty() {
                            vec![ContentBlock::text(assistant_text)]
                        } else {
                            vec![]
                        },
                        thinking: reasoning_text.to_string(),
                        tool_calls: vec![],
                        ..Default::default()
                    };
                    for tc in tool_calls {
                        msg.tool_calls.push(AgentToolCall {
                            id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            args: tc.function.arguments.clone(),
                        });
                    }
                    // Don't push an empty assistant — the LLM API rejects
                    // messages with neither content nor tool_calls.
                    if !msg.content.is_empty() || !msg.tool_calls.is_empty() {
                        messages.push(msg);
                    }
                    // Append placeholder tool-result for every unexecuted
                    // tool call so the conversation remains API-valid.
                    for tc in tool_calls {
                        let cancelled = format!(
                            "[Tool execution cancelled — {} was not executed due to interrupt]",
                            tc.function.name
                        );
                        messages.push(AgentMessage {
                            role: "tool".to_string(),
                            content: vec![ContentBlock::text(&cancelled)],
                            tool_call_id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            ..Default::default()
                        });
                    }
                    // Partial output is part of the authoritative conversation,
                    // just like a normally completed assistant/tool message.
                    // Persist every entry added by this interrupt path so the
                    // append-only terminal commit cannot leave memory and JSONL
                    // permanently divergent after abort/restart.
                    if let Some(ref save) = ctx.save_callback {
                        for message in &messages[first_new_message..] {
                            save(message);
                        }
                    }
                };

            // Check for stream errors before processing results
            if let Some(_err) = stream_error {
                build_partial_assistant(
                    &mut messages,
                    &assistant_text,
                    &reasoning_text,
                    &tool_calls,
                );
                return Ok((String::new(), messages));
            }

            // Check for pending interrupt (may have arrived during API call
            // or last stream event — tokio::select! can pick stream end over
            // the interrupt channel)
            if let Some(ref mut irx) = interrupt_rx {
                if irx.try_recv().is_ok() {
                    // Same interrupt path as above
                    build_partial_assistant(
                        &mut messages,
                        &assistant_text,
                        &reasoning_text,
                        &tool_calls,
                    );
                    return Ok((String::new(), messages));
                }
            }

            // Close any open output block (text_end may not have been emitted).
            if self.verbose && output_started {
                crate::eprintln_log!();
            }

            // Emit message_end

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
            // Skip truly empty assistant messages — the LLM API rejects them
            // ("content or tool_calls must be set").  However, a message that
            // has reasoning_content (thinking) is NOT empty: the thinking
            // content was already streamed to the client, and dropping it here
            // loses the entire response (the GUI shows "没有返回文本").
            // convert_messages_to_openai sends reasoning_content even when the
            // content field is omitted, matching the tool_calls-only pattern.
            if !assistant_msg.content.is_empty()
                || !assistant_msg.tool_calls.is_empty()
                || !assistant_msg.thinking.is_empty()
            {
                messages.push(assistant_msg);
                // Persist the assistant response immediately so it survives a
                // crash mid-run, even if no tools were called in this turn.
                if let Some(ref save) = ctx.save_callback {
                    save(messages.last().unwrap());
                }
            }

            // Apply the final credit_cost from this LLM call to cumulative_cost.
            // The upstream API sends progressive credit_cost updates in each
            // usage chunk; total_usage holds the LAST (complete) value. Adding
            // it here — once per LLM call — avoids the N× inflation that
            // would result from accumulating every intermediate chunk.
            if let Some(ref u) = total_usage {
                if let Some(cost) = u.credit_cost {
                    *self.cumulative_cost.lock() += cost;
                }
            }

            // Stream was truncated mid-reply: the assistant text is a prefix,
            // not a finished answer. End the turn as `incomplete` (keeping the
            // partial text so it isn't lost) rather than presenting a cut-off
            // reply as `complete`. Tool calls, if
            // any, are left unexecuted — their arguments may be partial.
            if stream_truncated {
                self.stream_incomplete
                    .store(true, std::sync::atomic::Ordering::SeqCst);
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
                    return Ok((assistant_text, messages));
                }
            }

            // No tool calls means this run is complete. Follow-up submissions
            // are separate queued runs and are started only by the scheduler.
            if tool_calls.is_empty() {
                if self.verbose {
                    tracing::info!(
                        "[agent] complete turns={} output_len={}",
                        turn + 1,
                        assistant_text.len()
                    );
                }
                if let Some(ref u) = total_usage {
                    let cost = u.credit_cost.unwrap_or(0.0);
                    let cum_cost = *self.cumulative_cost.lock();
                    tracing::info!(
                        "[agent] usage tokens_in={} tokens_out={} cache_read={} cache_write={} cost={:.6} cumulative_cost={:.6}",
                        u.prompt_tokens,
                        u.completion_tokens,
                        u.cache_read_tokens.unwrap_or(0),
                        u.cache_write_tokens.unwrap_or(0),
                        cost,
                        cum_cost,
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
            self.execute_tools(
                turn,
                &tool_calls,
                &mut messages,
                &ctx.tool_event_callback,
                &ctx.on_tool_result,
            )
            .await;

            // Log per-turn usage: tokens and cost.
            if let Some(ref u) = total_usage {
                let cost = u.credit_cost.unwrap_or(0.0);
                let cum_cost = *self.cumulative_cost.lock();
                tracing::info!(
                    "[agent] turn={} tokens_in={} tokens_out={} cache_read={} cache_write={} cost={:.6} cumulative_cost={:.6}",
                    turn,
                    u.prompt_tokens,
                    u.completion_tokens,
                    u.cache_read_tokens.unwrap_or(0),
                    u.cache_write_tokens.unwrap_or(0),
                    cost,
                    cum_cost,
                );
            }

            last_error = None;
            turn += 1;
        }
    }

    /// Sleep for `dur`, returning early with `true` if an interrupt arrives —
    /// either signalled on `interrupt_rx` or via the shared interrupt flag.
    /// Returns `false` if the full duration elapsed without interruption.
    ///
    /// The flag is polled every 50ms (matching the shell tool) so an `abort()`
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
        // NOTE: credit_cost is NOT accumulated here. The upstream API sends
        // progressive credit_cost updates in each usage chunk (each value is
        // the cumulative cost of the request so far). Adding every chunk
        // inflates the total by N×. Instead, credit_cost is applied once at
        // the end of the LLM call using the final value from total_usage.
        *total_usage = Some(u.clone());
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
    // "invalid type: string \"\", expected struct ShellParams".
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

/// Returns true when the LLM error was caused by request body exceeding
/// the provider's limit (context window or proxy body-size cap).  The LLM
/// client annotates these errors with a `[CTX_LIMIT]` prefix so we can
/// detect them reliably without fragile keyword matching.
fn is_retryable_size_error(err_msg: &str) -> bool {
    err_msg.starts_with("[CTX_LIMIT]")
}

/// Drain pending mid-turn steering notes and fold them into the system prompt
/// for the next LLM call. Notes are consumed exactly once; an empty queue
/// returns the base prompt unchanged.
fn fold_steering_into_prompt(
    base: &str,
    notes: &mut parking_lot::MutexGuard<'_, Vec<String>>,
) -> String {
    if notes.is_empty() {
        return base.to_string();
    }
    let drained = std::mem::take(&mut **notes);
    format!(
        "{}\n\n── steering update (injected mid-turn by the orchestrator) ──\n{}",
        base,
        drained.join("\n\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::StreamContext;
    use crate::types::{AgentTool, LLMProvider, ToolDef};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    // ── scripted provider infrastructure ────────────────────────────────────

    enum Script {
        /// Send all events, then close the channel (stream end).
        Events(Vec<StreamEvent>),
        /// Fail the stream_chat call itself.
        Fail(String),
        /// Send the events, then go silent forever (channel stays open).
        PartialThenStall(Vec<StreamEvent>),
    }

    struct ScriptedProvider {
        scripts: parking_lot::Mutex<std::collections::VecDeque<Script>>,
        system_prompts: parking_lot::Mutex<Vec<String>>,
    }

    impl ScriptedProvider {
        fn new(scripts: Vec<Script>) -> Arc<Self> {
            Arc::new(Self {
                scripts: parking_lot::Mutex::new(scripts.into()),
                system_prompts: parking_lot::Mutex::new(vec![]),
            })
        }
    }

    #[async_trait::async_trait]
    impl LLMProvider for ScriptedProvider {
        async fn stream_chat(
            &self,
            _model: String,
            _messages: Vec<Message>,
            _tools: Vec<ToolDef>,
            system_prompt: String,
        ) -> Result<ReceiverStream<StreamEvent>> {
            self.system_prompts.lock().push(system_prompt);
            let script = self
                .scripts
                .lock()
                .pop_front()
                .expect("test ran out of scripted responses");
            match script {
                Script::Events(events) => {
                    let (tx, rx) = mpsc::channel(events.len().max(1));
                    for event in events {
                        let _ = tx.try_send(event);
                    }
                    drop(tx);
                    Ok(ReceiverStream::new(rx))
                }
                Script::PartialThenStall(events) => {
                    let (tx, rx) = mpsc::channel(events.len().max(1));
                    for event in events {
                        let _ = tx.try_send(event);
                    }
                    std::mem::forget(tx); // keep the stream open forever
                    Ok(ReceiverStream::new(rx))
                }
                Script::Fail(error) => Err(anyhow!(error)),
            }
        }
    }

    fn ev_text(text: &str) -> StreamEvent {
        StreamEvent {
            event_type: "text_delta".to_string(),
            text: text.to_string(),
            ..Default::default()
        }
    }

    fn ev_stop() -> StreamEvent {
        StreamEvent {
            event_type: "stop".to_string(),
            stop_reason: "end_turn".to_string(),
            ..Default::default()
        }
    }

    fn ev_usage(prompt: i64, completion: i64, credit_cost: Option<f64>) -> StreamEvent {
        StreamEvent {
            event_type: "usage".to_string(),
            usage: Some(crate::types::Usage {
                prompt_tokens: prompt,
                completion_tokens: completion,
                total_tokens: prompt + completion,
                cache_read_tokens: Some(3),
                cache_write_tokens: Some(2),
                credit_cost,
            }),
            ..Default::default()
        }
    }

    fn ev_toolcall_start(index: usize, id: &str, name: &str, args: &str) -> StreamEvent {
        StreamEvent {
            event_type: "toolcall_start".to_string(),
            tool_id: id.to_string(),
            tool_name: name.to_string(),
            tc_index: index,
            tool_call: Some(ToolCall {
                id: id.to_string(),
                call_type: "function".to_string(),
                function: crate::types::ToolCallFn {
                    name: name.to_string(),
                    arguments: serde_json::Value::String(args.to_string()),
                },
            }),
            ..Default::default()
        }
    }

    fn ev_toolcall_delta(index: usize, text: &str) -> StreamEvent {
        StreamEvent {
            event_type: "toolcall_delta".to_string(),
            tc_index: index,
            text: text.to_string(),
            ..Default::default()
        }
    }

    fn ev_toolcall_end() -> StreamEvent {
        StreamEvent {
            event_type: "toolcall_end".to_string(),
            ..Default::default()
        }
    }

    fn echo_tool() -> AgentTool {
        fn handler(args: serde_json::Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>> {
            Box::pin(async move { Ok(format!("echo: {args}")) })
        }
        AgentTool {
            def: ToolDef {
                tool_type: "function".to_string(),
                function: crate::types::FunctionDef {
                    name: "echo".to_string(),
                    description: "echo the args".to_string(),
                    parameters: serde_json::json!({}),
                },
            },
            handler,
            guidelines: vec![],
        }
    }

    fn user_messages(text: &str) -> Vec<AgentMessage> {
        vec![AgentMessage::new_user("user", serde_json::json!(text))]
    }

    fn noop_on_text(_: String) {}

    // ── run_streaming_with_messages ─────────────────────────────────────────

    #[tokio::test(flavor = "current_thread")]
    async fn run_streams_simple_text_reply() {
        let provider = ScriptedProvider::new(vec![Script::Events(vec![
            ev_text("Hello"),
            ev_text(" world"),
            ev_stop(),
        ])]);
        let loop_ = Loop::new(provider, "mock");
        let collected = Arc::new(parking_lot::Mutex::new(String::new()));
        let collected2 = collected.clone();
        let (text, messages) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                move |chunk| {
                    collected2.lock().push_str(&chunk);
                },
                |_| {},
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "Hello world");
        assert_eq!(*collected.lock(), "Hello world");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].text(), "Hello world");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_records_thinking_and_usage() {
        let provider = ScriptedProvider::new(vec![Script::Events(vec![
            StreamEvent {
                event_type: "thinking_start".to_string(),
                ..Default::default()
            },
            StreamEvent {
                event_type: "thinking_delta".to_string(),
                text: "deep ".to_string(),
                ..Default::default()
            },
            StreamEvent {
                event_type: "thinking_delta".to_string(),
                text: "thought".to_string(),
                ..Default::default()
            },
            StreamEvent {
                event_type: "thinking_end".to_string(),
                ..Default::default()
            },
            ev_usage(10, 5, Some(0.25)),
            ev_text("answer"),
            ev_stop(),
        ])]);
        let loop_ = Loop::new(provider.clone(), "mock");
        let (text, messages) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "answer");
        assert_eq!(messages[1].thinking, "deep thought");
        assert_eq!(
            loop_.cumulative_input_tokens.load(std::sync::atomic::Ordering::Relaxed),
            10
        );
        assert_eq!(
            loop_.cumulative_output_tokens.load(std::sync::atomic::Ordering::Relaxed),
            5
        );
        assert_eq!(
            loop_
                .cumulative_cache_read_tokens
                .load(std::sync::atomic::Ordering::Relaxed),
            3
        );
        assert_eq!(
            loop_
                .cumulative_cache_write_tokens
                .load(std::sync::atomic::Ordering::Relaxed),
            2
        );
        // credit_cost is applied once from the final usage, not per chunk.
        assert!((*loop_.cumulative_cost.lock() - 0.25).abs() < f64::EPSILON);
        assert_eq!(
            loop_.last_prompt_tokens.load(std::sync::atomic::Ordering::Relaxed),
            15
        );
        drop(provider);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_executes_tool_then_answers() {
        let provider = ScriptedProvider::new(vec![
            Script::Events(vec![
                ev_toolcall_start(0, "call-1", "echo", "{\"text\":\"hi\""),
                ev_toolcall_delta(0, ", \"more\":1}"),
                ev_toolcall_end(),
                ev_stop(),
            ]),
            Script::Events(vec![ev_text("done"), ev_stop()]),
        ]);
        let mut loop_ = Loop::new(provider, "mock").with_tools(vec![echo_tool()]);
        loop_.verbose = true; // exercise the verbose logging arms
        let saved = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let tool_events = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let ctx = StreamContext {
            model: "mock".to_string(),
            system_prompt: "sys".to_string(),
            save_callback: Some({
                let saved = saved.clone();
                Arc::new(move |m: &AgentMessage| saved.lock().push(m.role.clone()))
            }),
            tool_event_callback: Some({
                let tool_events = tool_events.clone();
                Arc::new(move |e: StreamEvent| tool_events.lock().push(e.event_type.clone()))
            }),
            ..Default::default()
        };
        let (text, messages) = loop_
            .run_streaming_with_messages(user_messages("go"), &ctx, noop_on_text, |_| {}, None)
            .await
            .unwrap();
        assert_eq!(text, "done");
        // user, assistant(tool_calls), tool result, assistant(text)
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[1].tool_calls.len(), 1);
        assert_eq!(messages[1].tool_calls[0].name, "echo");
        assert_eq!(messages[2].role, "tool");
        assert!(messages[2].text().contains("echo:"));
        let tool_events = tool_events.lock().clone();
        assert!(tool_events.contains(&"tool_start".to_string()));
        assert!(tool_events.contains(&"tool_end".to_string()));
        // save_callback fired for assistant + tool messages.
        assert!(saved.lock().len() >= 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_rejects_assistant_last_message() {
        let provider = ScriptedProvider::new(vec![]);
        let loop_ = Loop::new(provider, "mock");
        let mut messages = user_messages("hi");
        messages.push(AgentMessage {
            role: "assistant".to_string(),
            content: vec![ContentBlock::text("stale")],
            ..Default::default()
        });
        let result = loop_
            .run_streaming_with_messages(messages, &StreamContext::default(), noop_on_text, |_| {}, None)
            .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("conversation ended with an assistant message"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_respects_max_turns() {
        let provider = ScriptedProvider::new(vec![
            Script::Events(vec![
                ev_toolcall_start(0, "c1", "echo", "{}"),
                ev_toolcall_end(),
                ev_stop(),
            ]),
            Script::Events(vec![
                ev_toolcall_start(0, "c2", "echo", "{}"),
                ev_toolcall_end(),
                ev_stop(),
            ]),
        ]);
        let config = crate::types::AgentConfig {
            max_turns: 1,
            ..Default::default()
        };
        let loop_ = Loop::new(provider, "mock")
            .with_tools(vec![echo_tool()])
            .with_config(config);
        let result = loop_
            .run_streaming_with_messages(
                user_messages("loop forever"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("turn limit"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_returns_early_when_interrupt_flag_set() {
        let provider = ScriptedProvider::new(vec![]);
        let loop_ = Loop::new(provider, "mock");
        loop_
            .interrupt_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let (text, messages) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "");
        assert_eq!(messages.len(), 1, "no assistant reply was produced");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_handles_stream_error_event() {
        let provider = ScriptedProvider::new(vec![Script::Events(vec![
            ev_text("partial"),
            StreamEvent {
                event_type: "error".to_string(),
                error_text: "upstream exploded".to_string(),
                ..Default::default()
            },
        ])]);
        let loop_ = Loop::new(provider, "mock");
        let saved = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let ctx = StreamContext {
            save_callback: Some({
                let saved = saved.clone();
                Arc::new(move |m: &AgentMessage| saved.lock().push(m.role.clone()))
            }),
            ..Default::default()
        };
        let (text, messages) = loop_
            .run_streaming_with_messages(user_messages("hi"), &ctx, noop_on_text, |_| {}, None)
            .await
            .unwrap();
        assert_eq!(text, "");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].text(), "partial");
        assert!(!saved.lock().is_empty(), "partial reply persisted");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_marks_truncated_stop_as_incomplete() {
        let provider = ScriptedProvider::new(vec![Script::Events(vec![
            ev_text("cut off"),
            StreamEvent {
                event_type: "stop".to_string(),
                stop_reason: "truncated".to_string(),
                ..Default::default()
            },
        ])]);
        let loop_ = Loop::new(provider, "mock");
        let (text, _) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "cut off");
        assert!(loop_
            .stream_incomplete
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn run_times_out_a_stalled_stream() {
        let provider = ScriptedProvider::new(vec![Script::PartialThenStall(vec![ev_text("stuck")])]);
        let loop_ = Loop::new(provider, "mock");
        let (text, _) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "stuck");
        assert!(loop_
            .stream_incomplete
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn run_retries_then_succeeds() {
        let provider = ScriptedProvider::new(vec![
            Script::Fail("transient boom".to_string()),
            Script::Events(vec![ev_text("recovered"), ev_stop()]),
        ]);
        let config = crate::types::AgentConfig {
            max_retries: 2,
            ..Default::default()
        };
        let loop_ = Loop::new(provider, "mock").with_config(config);
        let (text, _) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "recovered");
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn run_returns_error_after_retries_exhausted() {
        let provider = ScriptedProvider::new(vec![
            Script::Fail("boom 1".to_string()),
            Script::Fail("boom 2".to_string()),
        ]);
        let config = crate::types::AgentConfig {
            max_retries: 1,
            ..Default::default()
        };
        let loop_ = Loop::new(provider, "mock").with_config(config);
        let result = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("boom 2"));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn run_fails_when_forced_compaction_finds_no_cut() {
        let provider = ScriptedProvider::new(vec![Script::Fail(
            "[CTX_LIMIT] Request exceeds the model's maximum context length (HTTP 400)."
                .to_string(),
        )]);
        let config = crate::types::AgentConfig {
            max_retries: 1,
            ..Default::default()
        };
        let loop_ = Loop::new(provider, "mock").with_config(config);
        // A single short user message cannot be compacted → hard failure.
        let result = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("context compaction failed"));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn run_compacts_then_retries_on_context_length_error() {
        let provider = ScriptedProvider::new(vec![
            Script::Fail(
                "[CTX_LIMIT] Request exceeds the model's maximum context length (HTTP 400)."
                    .to_string(),
            ),
            Script::Events(vec![ev_text("recovered"), ev_stop()]),
        ]);
        let config = crate::types::AgentConfig {
            max_retries: 1,
            ..Default::default()
        };
        let loop_ = Loop::new(provider, "mock").with_config(config);
        // A long multi-turn history gives compaction a valid cut point.
        let mut messages = Vec::new();
        for i in 0..12 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            messages.push(AgentMessage {
                role: role.to_string(),
                content: vec![ContentBlock::text(format!("message {i} ").repeat(200))],
                ..Default::default()
            });
        }
        messages.push(AgentMessage::new_user("user", serde_json::json!("fresh question")));
        let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let (text, _) = loop_
            .run_streaming_with_messages(
                messages,
                &StreamContext::default(),
                noop_on_text,
                {
                    let events = events.clone();
                    move |e: StreamEvent| events.lock().push(e.event_type.clone())
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "recovered");
        assert!(events.lock().contains(&"compaction_end".to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_folds_steering_notes_into_system_prompt() {
        let provider = ScriptedProvider::new(vec![Script::Events(vec![ev_text("ok"), ev_stop()])]);
        let loop_ = Loop::new(provider.clone(), "mock");
        loop_
            .steering_notes
            .lock()
            .push("be brief".to_string());
        let ctx = StreamContext {
            system_prompt: "base".to_string(),
            ..Default::default()
        };
        let _ = loop_
            .run_streaming_with_messages(user_messages("hi"), &ctx, noop_on_text, |_| {}, None)
            .await
            .unwrap();
        let prompts = provider.system_prompts.lock().clone();
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("base"));
        assert!(prompts[0].contains("be brief"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_applies_transform_context_and_emits_compaction_event() {
        let provider = ScriptedProvider::new(vec![Script::Events(vec![ev_text("ok"), ev_stop()])]);
        let config = crate::types::AgentConfig {
            transform_context: Some(Arc::new(|messages: Vec<Message>, _| {
                // Keep only the last message — a fake compaction.
                messages.into_iter().last().into_iter().collect()
            })),
            ..Default::default()
        };
        let loop_ = Loop::new(provider, "mock").with_config(config);
        let mut messages = user_messages("old");
        messages.push(AgentMessage {
            role: "assistant".to_string(),
            content: vec![ContentBlock::text("older reply")],
            ..Default::default()
        });
        messages.push(AgentMessage::new_user("user", serde_json::json!("latest")));
        let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let (text, final_messages) = loop_
            .run_streaming_with_messages(
                messages,
                &StreamContext::default(),
                noop_on_text,
                {
                    let events = events.clone();
                    move |e: StreamEvent| events.lock().push(e.event_type.clone())
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "ok");
        assert!(events.lock().contains(&"compaction_end".to_string()));
        // The in-memory history was replaced by the compacted transform result.
        assert!(final_messages.len() <= 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_errors_when_compaction_failed_flag_set() {
        let provider = ScriptedProvider::new(vec![Script::Events(vec![ev_text("ok"), ev_stop()])]);
        let config = crate::types::AgentConfig {
            transform_context: Some(Arc::new(|messages: Vec<Message>, _| messages)),
            ..Default::default()
        };
        let loop_ = Loop::new(provider, "mock").with_config(config);
        loop_
            .compaction_failed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let result = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("context compaction failed"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_stop_condition_short_circuits_the_loop() {
        let provider = ScriptedProvider::new(vec![Script::Events(vec![
            ev_toolcall_start(0, "c1", "echo", "{}"),
            ev_toolcall_end(),
            ev_stop(),
        ])]);
        let config = crate::types::AgentConfig {
            stop_condition: Some(Arc::new(|_, _| true)),
            ..Default::default()
        };
        let loop_ = Loop::new(provider, "mock")
            .with_tools(vec![echo_tool()])
            .with_config(config);
        let (text, _) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "");
        // The scripted tool turn was never followed by a second LLM call
        // (provider had exactly one script; a second call would panic).
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn run_interrupt_mid_stream_keeps_partial_reply() {
        let provider = ScriptedProvider::new(vec![Script::PartialThenStall(vec![
            ev_text("before interrupt"),
        ])]);
        let loop_ = Loop::new(provider, "mock");
        let (interrupt_tx, interrupt_rx) = mpsc::channel::<()>(1);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let _ = interrupt_tx.send(()).await;
        });
        let (text, messages) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                Some(interrupt_rx),
            )
            .await
            .unwrap();
        assert_eq!(text, "");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].text(), "before interrupt");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_handles_toolcall_streaming_edge_cases() {
        let provider = ScriptedProvider::new(vec![
            Script::Events(vec![
                // Delta before any start → placeholder that the later start replaces.
                ev_toolcall_delta(0, "{\"pre"),
                ev_toolcall_start(0, "call-1", "echo", "{\"pre"),
                // Same-id start at the same index appends longer args.
                ev_toolcall_start(0, "call-1", "echo", "{\"pre\":1}"),
                // Delta on a placeholder-less slot after the real start appends.
                ev_toolcall_delta(0, ", \"post\":2}"),
                // A second tool call at index 1.
                ev_toolcall_start(1, "call-2", "echo", "{}"),
                // Usage piggy-backed on the end event.
                StreamEvent {
                    event_type: "toolcall_end".to_string(),
                    usage: Some(crate::types::Usage {
                        prompt_tokens: 7,
                        completion_tokens: 3,
                        total_tokens: 10,
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                        credit_cost: None,
                    }),
                    ..Default::default()
                },
                ev_stop(),
            ]),
            Script::Events(vec![ev_text("done"), ev_stop()]),
        ]);
        let loop_ = Loop::new(provider, "mock").with_tools(vec![echo_tool()]);
        let (text, messages) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "done");
        let assistant = &messages[1];
        // The pre-start delta created a placeholder (empty id/name) that was
        // finalized when the real start arrived; then call-1 and call-2.
        assert_eq!(assistant.tool_calls.len(), 3);
        assert_eq!(assistant.tool_calls[0].name, "");
        assert_eq!(assistant.tool_calls[1].id, "call-1");
        assert_eq!(assistant.tool_calls[2].id, "call-2");
        // call-1's args were concatenated verbatim (same-id start append +
        // non-'{' delta): `{"pre":1}` + `, "post":2}`.
        let args = assistant.tool_calls[1].args.as_str().unwrap();
        assert_eq!(args, "{\"pre\":1}, \"post\":2}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_skips_remaining_tools_after_interrupt() {
        let provider = ScriptedProvider::new(vec![
            Script::Events(vec![
                ev_toolcall_start(0, "c1", "echo", "{}"),
                ev_toolcall_start(1, "c2", "echo", "{}"),
                ev_toolcall_end(),
                ev_stop(),
            ]),
            Script::Events(vec![ev_text("after"), ev_stop()]),
        ]);
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let config = crate::types::AgentConfig {
            after_tool_call: Some({
                let flag = flag.clone();
                Arc::new(move |_, _, _, _, _| {
                    flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    None
                })
            }),
            ..Default::default()
        };
        let mut loop_ = Loop::new(provider, "mock")
            .with_tools(vec![echo_tool()])
            .with_config(config);
        loop_.interrupt_flag = flag;
        let (_, messages) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await
            .unwrap();
        // The second tool call was never executed: it got a cancellation
        // placeholder instead of an echo result.
        let tool_messages: Vec<_> = messages.iter().filter(|m| m.role == "tool").collect();
        assert_eq!(tool_messages.len(), 2);
        assert!(tool_messages[0].text().contains("echo:"));
        assert!(tool_messages[1].text().contains("was skipped due to user interrupt"));
    }

    // ── pure helpers ────────────────────────────────────────────────────────

    #[test]
    fn tool_call_args_complete_checks_json_balance() {
        let mut call = AgentToolCall {
            id: "c1".to_string(),
            name: "echo".to_string(),
            args: serde_json::Value::String("{\"a\":1}".to_string()),
        };
        assert!(tool_call_args_complete(&call));
        call.args = serde_json::Value::String("{\"a\":1".to_string());
        assert!(!tool_call_args_complete(&call));
        call.args = serde_json::Value::Null;
        assert!(!tool_call_args_complete(&call));
    }

    #[test]
    fn finalize_agent_tool_call_parses_and_repairs_args() {
        let complete = AgentToolCall {
            id: "c1".to_string(),
            name: "echo".to_string(),
            args: serde_json::Value::String("{\"a\":1}".to_string()),
        };
        let finalized = finalize_agent_tool_call(complete);
        assert_eq!(finalized.id, "c1");
        assert_eq!(finalized.function.name, "echo");
        assert_eq!(
            finalized.function.arguments,
            serde_json::Value::String("{\"a\":1}".to_string())
        );

        let partial = AgentToolCall {
            id: "c2".to_string(),
            name: "echo".to_string(),
            args: serde_json::Value::String("{\"a\":\"x".to_string()),
        };
        let finalized = finalize_agent_tool_call(partial);
        // The truncated JSON was repaired into something parseable.
        let args = finalized.function.arguments;
        let args_str = args.as_str().unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(args_str).is_ok());
    }

    #[test]
    fn repair_partial_json_object_handles_common_truncations() {
        assert_eq!(repair_partial_json_object("not an object"), None);
        assert_eq!(repair_partial_json_object("{}"), Some("{}".to_string()));
        let repaired = repair_partial_json_object("{\"key\": \"value").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&repaired).unwrap();
        assert_eq!(parsed["key"], "value");
        let repaired = repair_partial_json_object("{\"a\": {\"b\": 1").unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&repaired).is_ok());
        // Unclosed arrays are NOT repaired (best-effort brace balancing only).
        let attempted = repair_partial_json_object("{\"a\": [1, 2").unwrap();
        assert!(attempted.ends_with('}'));
        assert!(serde_json::from_str::<serde_json::Value>(&attempted).is_err());
    }

    #[test]
    fn has_unclosed_string_detects_open_quotes() {
        assert!(!has_unclosed_string("\"closed\""));
        assert!(has_unclosed_string("\"open"));
        assert!(!has_unclosed_string("\"escaped \\\" quote\""));
        assert!(has_unclosed_string("\"trailing escape\\"));
    }

    // ── run_loop batch 2: interrupts, verbose arms, merge variants ─────────

    #[tokio::test(flavor = "current_thread")]
    async fn run_verbose_covers_logging_arms() {
        let provider = ScriptedProvider::new(vec![
            Script::Events(vec![
                StreamEvent {
                    event_type: "thinking_start".to_string(),
                    ..Default::default()
                },
                StreamEvent {
                    event_type: "thinking_delta".to_string(),
                    text: "ponder".to_string(),
                    ..Default::default()
                },
                StreamEvent {
                    event_type: "thinking_end".to_string(),
                    ..Default::default()
                },
                StreamEvent {
                    event_type: "text_start".to_string(),
                    ..Default::default()
                },
                ev_text("chunk"),
                StreamEvent {
                    event_type: "text_end".to_string(),
                    ..Default::default()
                },
                ev_toolcall_start(0, "c1", "echo", "{}"),
                StreamEvent {
                    event_type: "tool_start".to_string(),
                    tool_name: "echo".to_string(),
                    ..Default::default()
                },
                StreamEvent {
                    event_type: "tool_end".to_string(),
                    tool_name: "echo".to_string(),
                    ..Default::default()
                },
                // Usage piggy-backed on the terminal stop.
                StreamEvent {
                    event_type: "stop".to_string(),
                    stop_reason: "tool_calls".to_string(),
                    usage: Some(crate::types::Usage {
                        prompt_tokens: 5,
                        completion_tokens: 1,
                        total_tokens: 6,
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                        credit_cost: Some(0.1),
                    }),
                    ..Default::default()
                },
            ]),
            Script::Events(vec![ev_text("final"), ev_stop()]),
        ]);
        let mut loop_ = Loop::new(provider, "mock").with_tools(vec![echo_tool()]);
        loop_.verbose = true;
        let (text, _) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "final");
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn run_interrupt_while_connecting_returns_early() {
        struct PendProvider;
        #[async_trait::async_trait]
        impl LLMProvider for PendProvider {
            async fn stream_chat(
                &self,
                _model: String,
                _messages: Vec<Message>,
                _tools: Vec<ToolDef>,
                _system_prompt: String,
            ) -> Result<ReceiverStream<StreamEvent>> {
                std::future::pending::<()>().await;
                unreachable!("never resolves");
            }
        }
        let loop_ = Loop::new(Arc::new(PendProvider), "mock");
        let (interrupt_tx, interrupt_rx) = mpsc::channel::<()>(1);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let _ = interrupt_tx.send(()).await;
        });
        let (text, _) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                Some(interrupt_rx),
            )
            .await
            .unwrap();
        assert_eq!(text, "");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_interrupt_during_retry_backoff_returns_early() {
        struct FailOnSignalProvider {
            release: tokio::sync::Notify,
        }
        #[async_trait::async_trait]
        impl LLMProvider for FailOnSignalProvider {
            async fn stream_chat(
                &self,
                _model: String,
                _messages: Vec<Message>,
                _tools: Vec<ToolDef>,
                _system_prompt: String,
            ) -> Result<ReceiverStream<StreamEvent>> {
                self.release.notified().await;
                Err(anyhow!("signalled failure"))
            }
        }
        let provider = Arc::new(FailOnSignalProvider {
            release: tokio::sync::Notify::new(),
        });
        let config = crate::types::AgentConfig {
            max_retries: 3,
            ..Default::default()
        };
        let loop_ = Loop::new(provider.clone(), "mock").with_config(config);
        let flag = loop_.interrupt_flag.clone();
        let runner = tokio::spawn(async move {
            loop_
                .run_streaming_with_messages(
                    user_messages("hi"),
                    &StreamContext::default(),
                    noop_on_text,
                    |_| {},
                    None,
                )
                .await
        });
        // Let the call start, then release the failure and interrupt while the
        // 2s backoff sleep is in flight.
        tokio::time::sleep(Duration::from_millis(50)).await;
        provider.release.notify_one();
        tokio::time::sleep(Duration::from_millis(100)).await;
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        let (text, _) = runner.await.unwrap().unwrap();
        assert_eq!(text, "");
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn run_stream_idle_timeout_with_interrupt_channel() {
        let provider = ScriptedProvider::new(vec![Script::PartialThenStall(vec![ev_text("stuck")])]);
        let loop_ = Loop::new(provider, "mock");
        let (_interrupt_tx, interrupt_rx) = mpsc::channel::<()>(1);
        let (text, _) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                Some(interrupt_rx),
            )
            .await
            .unwrap();
        assert_eq!(text, "stuck");
        assert!(loop_
            .stream_incomplete
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn run_interrupt_with_finalized_tool_call_writes_placeholders() {
        let provider = ScriptedProvider::new(vec![Script::PartialThenStall(vec![
            ev_toolcall_start(0, "c1", "echo", "{}"),
            ev_toolcall_end(),
        ])]);
        let loop_ = Loop::new(provider, "mock").with_tools(vec![echo_tool()]);
        let (interrupt_tx, interrupt_rx) = mpsc::channel::<()>(1);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let _ = interrupt_tx.send(()).await;
        });
        let saved = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let ctx = StreamContext {
            save_callback: Some({
                let saved = saved.clone();
                Arc::new(move |m: &AgentMessage| saved.lock().push(m.role.clone()))
            }),
            ..Default::default()
        };
        let (_, messages) = loop_
            .run_streaming_with_messages(user_messages("hi"), &ctx, noop_on_text, |_| {}, Some(interrupt_rx))
            .await
            .unwrap();
        // Partial assistant carries the finalized tool call, followed by its
        // cancellation placeholder.
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].tool_calls.len(), 1);
        assert_eq!(messages[2].role, "tool");
        assert!(messages[2].text().contains("was not executed due to interrupt"));
        assert_eq!(saved.lock().len(), 2, "assistant + placeholder persisted");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_toolcall_merge_and_delta_variants() {
        let provider = ScriptedProvider::new(vec![
            Script::Events(vec![
                // Same-id start with a longer non-prefix arg string → replace.
                ev_toolcall_start(0, "c1", "echo", "{\"a\""),
                ev_toolcall_start(0, "c1", "echo", "zz{\"longer\""),
                // Same-id start with shorter args → keep existing.
                ev_toolcall_start(0, "c1", "echo", "z"),
                // Start with no tool_call payload (args Null)…
                StreamEvent {
                    event_type: "toolcall_start".to_string(),
                    tool_id: "c2".to_string(),
                    tool_name: "echo".to_string(),
                    tc_index: 1,
                    tool_call: None,
                    ..Default::default()
                },
                // …then a same-id start with real args → replace the Null.
                ev_toolcall_start(1, "c2", "echo", "{\"b\":2"),
                // Delta on non-String args → set to String.
                StreamEvent {
                    event_type: "toolcall_start".to_string(),
                    tool_id: "c3".to_string(),
                    tool_name: "echo".to_string(),
                    tc_index: 2,
                    tool_call: Some(ToolCall {
                        id: "c3".to_string(),
                        call_type: "function".to_string(),
                        function: crate::types::ToolCallFn {
                            name: "echo".to_string(),
                            arguments: serde_json::Value::Null,
                        },
                    }),
                    ..Default::default()
                },
                ev_toolcall_delta(2, "{\"c\":3"),
                // Full-state replacement delta (starts with '{' and extends).
                ev_toolcall_start(3, "c4", "echo", "{\"d\":"),
                ev_toolcall_delta(3, "{\"d\":4}"),
                ev_toolcall_end(),
                ev_stop(),
            ]),
            Script::Events(vec![ev_text("done"), ev_stop()]),
        ]);
        let loop_ = Loop::new(provider, "mock").with_tools(vec![echo_tool()]);
        let (text, messages) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "done");
        let calls = &messages[1].tool_calls;
        assert_eq!(calls.len(), 4);
        assert_eq!(
            calls[0].args,
            serde_json::Value::String("zz{\"longer\"".to_string())
        );
        assert_eq!(
            calls[2].args,
            serde_json::Value::String("{\"c\":3}".to_string()),
            "delta-set args get brace-repaired at finalize"
        );
        assert_eq!(
            calls[3].args,
            serde_json::Value::String("{\"d\":4}".to_string())
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_slow_provider_exercises_await_polling() {
        struct SlowProvider {
            delay: Duration,
        }
        #[async_trait::async_trait]
        impl LLMProvider for SlowProvider {
            async fn stream_chat(
                &self,
                _model: String,
                _messages: Vec<Message>,
                _tools: Vec<ToolDef>,
                _system_prompt: String,
            ) -> Result<ReceiverStream<StreamEvent>> {
                tokio::time::sleep(self.delay).await;
                let (tx, rx) = mpsc::channel(2);
                let _ = tx.try_send(StreamEvent {
                    event_type: "text_delta".to_string(),
                    text: "slow".to_string(),
                    ..Default::default()
                });
                let _ = tx.try_send(StreamEvent {
                    event_type: "stop".to_string(),
                    stop_reason: "end_turn".to_string(),
                    ..Default::default()
                });
                drop(tx);
                Ok(ReceiverStream::new(rx))
            }
        }
        // Without an interrupt channel (None arm of await_or_interrupt).
        let loop_ = Loop::new(Arc::new(SlowProvider {
            delay: Duration::from_millis(120),
        }), "mock");
        let (text, _) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "slow");

        // With an interrupt channel that never fires (Some arm + poll sleep).
        let loop_ = Loop::new(Arc::new(SlowProvider {
            delay: Duration::from_millis(120),
        }), "mock");
        let (_tx, interrupt_rx) = mpsc::channel::<()>(1);
        let (text, _) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                Some(interrupt_rx),
            )
            .await
            .unwrap();
        assert_eq!(text, "slow");
    }

    #[test]
    fn repair_partial_tool_args_normalizes_values() {
        // Empty string becomes an empty object.
        let mut args = serde_json::Value::String(String::new());
        repair_partial_tool_args(&mut args);
        assert_eq!(args, serde_json::Value::String("{}".to_string()));
        // Already valid → unchanged.
        let mut args = serde_json::Value::String("{\"a\":1}".to_string());
        repair_partial_tool_args(&mut args);
        assert_eq!(args, serde_json::Value::String("{\"a\":1}".to_string()));
        // Non-string values pass through.
        let mut args = serde_json::json!({"a": 1});
        repair_partial_tool_args(&mut args);
        assert_eq!(args, serde_json::json!({"a": 1}));
        // Unrepairable (not an object) → unchanged.
        let mut args = serde_json::Value::String("[1,2".to_string());
        repair_partial_tool_args(&mut args);
        assert_eq!(args, serde_json::Value::String("[1,2".to_string()));
        // Repaired in place.
        let mut args = serde_json::Value::String("{\"a\":1".to_string());
        repair_partial_tool_args(&mut args);
        assert_eq!(args, serde_json::Value::String("{\"a\":1}".to_string()));
    }

    #[test]
    fn steering_fold_returns_base_when_empty() {
        let cell = std::sync::Arc::new(parking_lot::Mutex::new(vec![]));
        let mut guard = cell.lock();
        assert_eq!(
            fold_steering_into_prompt("base prompt", &mut guard),
            "base prompt"
        );
    }

    #[test]
    fn steering_fold_drains_once_and_appends() {
        let cell = std::sync::Arc::new(parking_lot::Mutex::new(vec!["do X instead".to_string()]));
        let mut guard = cell.lock();
        let out = fold_steering_into_prompt("base", &mut guard);
        assert!(out.contains("base"));
        assert!(out.contains("do X instead"));
        assert!(guard.is_empty(), "notes drained exactly once");
        let out2 = fold_steering_into_prompt("base", &mut guard);
        assert_eq!(out2, "base", "second call sees an empty queue");
    }

    #[test]
    fn size_error_matches_context_limit_prefix() {
        assert!(is_retryable_size_error(
            "[CTX_LIMIT] Request exceeds the model's maximum context length (HTTP 400)."
        ));
        assert!(is_retryable_size_error(
            "[CTX_LIMIT] API request failed (HTTP 400). No response body."
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
                if let Some(suffix) = new_args.strip_prefix(existing) {
                    format!("{}{}", existing, suffix)
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
