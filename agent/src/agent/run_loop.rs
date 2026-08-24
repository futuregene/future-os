use crate::llm::schema::{FinishReason, ModelStreamEvent};
use crate::types::{AgentMessage, AgentToolCall, ContentBlock, ConvertToLLM, Message, ToolCall};
use anyhow::{anyhow, Result};
use std::time::Duration;
use tokio::time::sleep;
use tokio_stream::StreamExt;

use super::{Loop, RunEvent, C_GREEN, C_MAGENTA, C_RESET, DEFAULT_MAX_TURNS};

const STREAM_EVENT_IDLE_TIMEOUT: Duration = Duration::from_secs(45);
const COMPLETE_TOOL_CALL_IDLE_TIMEOUT: Duration = Duration::from_secs(15);

impl Loop {
    pub async fn run_streaming_with_messages(
        &self,
        mut messages: Vec<AgentMessage>,
        ctx: &super::StreamContext,
        on_text: impl Fn(String) + Send + 'static,
        on_event: impl Fn(RunEvent) + Send + Sync + 'static,
        mut interrupt_rx: Option<tokio::sync::mpsc::Receiver<()>>,
    ) -> Result<(String, Vec<AgentMessage>)> {
        // Every model-visible message is bound to a stable journal identity.
        // Loaded messages already carry it; fresh/ephemeral callers receive one
        // here so compaction provenance never depends on vector indices.
        for message in &mut messages {
            message.ensure_journal_entry_id();
        }
        let mut active_checkpoint = self.active_checkpoint.lock().clone();
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
        on_event(RunEvent::AgentStart { started_at_ms });

        let tool_defs: Vec<_> = self.tools.iter().map(|t| t.def.clone()).collect();
        let mut retry_attempt = 0;
        let mut provider_limit_checkpoint_id: Option<String> = None;

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
            // Check max turn limit (0 = unlimited). The limit can only be
            // crossed right after a SUCCESSFUL turn: a failed LLM call either
            // returns its error immediately (retries exhausted) or `continue`s
            // without incrementing `turn`, so no failure state can be pending
            // here (that dead `last_error` arm was removed).
            if max_turns > 0 && turn >= max_turns {
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

            // Build a model-only projection from the immutable journal view.
            // A committed checkpoint changes this request's prompt but never
            // replaces `messages`, which remains the complete transcript.
            let context_window = self
                .context_manager
                .as_ref()
                .map(|manager| manager.context_window.max(1) as u64)
                .unwrap_or(1_000_000);
            let reported_input = self
                .last_prompt_tokens
                .load(std::sync::atomic::Ordering::Relaxed)
                .try_into()
                .ok()
                .filter(|tokens: &u64| *tokens > 0);
            let projected = crate::compaction::project_prompt_context(
                &messages,
                active_checkpoint.as_ref(),
                reported_input,
                context_window,
            );
            let automatic_phase = if turn == 0 {
                crate::compaction::CompactionPhase::PreTurn
            } else {
                crate::compaction::CompactionPhase::MidTurn
            };
            let automatic_operation_id = format!("cmp_{}", crate::utils::generate_entry_id());
            let emit_automatic_started = || {
                on_event(RunEvent::CompactionStarted {
                    operation_id: automatic_operation_id.clone(),
                    trigger: crate::compaction::CompactionTrigger::Automatic,
                    phase: automatic_phase,
                });
            };
            let prepared = if let Some(manager) = &self.context_manager {
                if provider_limit_checkpoint_id.is_some() {
                    // The retry below must use exactly the checkpoint produced
                    // for this failed model step. A second automatic checkpoint
                    // here could hide a non-context provider failure in a loop.
                    Ok(crate::compaction::ContextPreparation::Unchanged { prompt: projected })
                } else {
                    manager
                        .prepare_semantic_with_lifecycle(
                            projected,
                            crate::compaction::CompactionTrigger::Automatic,
                            automatic_phase,
                            None,
                            self.provider.as_ref(),
                            self.interrupt_flag.as_ref(),
                            None,
                            Some(&emit_automatic_started),
                        )
                        .await
                }
            } else {
                Ok(crate::compaction::ContextPreparation::Unchanged { prompt: projected })
            };
            let prompt = match prepared {
                Ok(crate::compaction::ContextPreparation::Unchanged { prompt }) => prompt,
                Ok(crate::compaction::ContextPreparation::Compacted { prompt, checkpoint }) => {
                    if let Some(commit) = &ctx.on_checkpoint {
                        if let Err(error) = commit(&checkpoint) {
                            on_event(RunEvent::CompactionFailed {
                                operation_id: automatic_operation_id.clone(),
                                trigger: crate::compaction::CompactionTrigger::Automatic,
                                phase: automatic_phase,
                                error: error.to_string(),
                            });
                            return Err(error);
                        }
                    }
                    active_checkpoint = Some((*checkpoint).clone());
                    *self.active_checkpoint.lock() = Some((*checkpoint).clone());
                    on_event(RunEvent::CompactionCommitted {
                        operation_id: automatic_operation_id.clone(),
                        checkpoint: *checkpoint,
                    });
                    prompt
                }
                Err(error) => {
                    on_event(RunEvent::CompactionFailed {
                        operation_id: automatic_operation_id.clone(),
                        trigger: crate::compaction::CompactionTrigger::Automatic,
                        phase: automatic_phase,
                        error: error.to_string(),
                    });
                    return Err(anyhow!(error));
                }
            };
            let work_messages: Vec<AgentMessage> = prompt
                .messages
                .into_iter()
                .map(|projected| projected.message)
                .collect();

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
                    self.provider
                        .stream_model(crate::llm::schema::ModelRequest {
                            model: ctx.model.clone(),
                            messages: work_messages.clone(),
                            tools: tool_defs.clone(),
                            system_prompt: step_system_prompt,
                        }),
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
                Err(last_error) => {
                    if self.config.max_retries > 0
                        && retry_attempt < self.config.max_retries as usize
                    {
                        // If this looks like a context-length or body-size
                        // error, compact before retrying. Auto-compaction
                        // only runs BEFORE a turn (based on last turn's token
                        // count), so it can't help on the first call.
                        let err_msg = format!("{last_error}");
                        if is_retryable_size_error(&err_msg) {
                            if let Some(checkpoint_id) = provider_limit_checkpoint_id.as_deref() {
                                return Err(anyhow!(
                                    "provider still rejected context after checkpoint {checkpoint_id}; refusing repeated compaction for the same model step"
                                ));
                            }
                            // Resolve the model's actual context window so we don't
                            // over-compact large-context models (1M+).
                            // Use the cached registry from the loop to avoid
                            // re-deserialising the model catalog; loops not
                            // derived from the app template (model_registry =
                            // None, e.g. tests) fall back to a fresh Registry
                            // so behaviour matches the pre-cache code.
                            let provider_limit_phase = crate::compaction::CompactionPhase::MidTurn;
                            let provider_limit_operation_id =
                                format!("cmp_{}", crate::utils::generate_entry_id());
                            let Some(manager) = &self.context_manager else {
                                let error = anyhow!(
                                    "context compaction is unavailable for provider-limit recovery"
                                );
                                on_event(RunEvent::CompactionStarted {
                                    operation_id: provider_limit_operation_id.clone(),
                                    trigger:
                                        crate::compaction::CompactionTrigger::ProviderContextLimit,
                                    phase: provider_limit_phase,
                                });
                                on_event(RunEvent::CompactionFailed {
                                    operation_id: provider_limit_operation_id,
                                    trigger:
                                        crate::compaction::CompactionTrigger::ProviderContextLimit,
                                    phase: provider_limit_phase,
                                    error: error.to_string(),
                                });
                                return Err(error);
                            };
                            let projected = crate::compaction::project_prompt_context(
                                &messages,
                                active_checkpoint.as_ref(),
                                None,
                                manager.context_window.max(1) as u64,
                            );
                            let emit_provider_limit_started = || {
                                on_event(RunEvent::CompactionStarted {
                                    operation_id: provider_limit_operation_id.clone(),
                                    trigger:
                                        crate::compaction::CompactionTrigger::ProviderContextLimit,
                                    phase: provider_limit_phase,
                                });
                            };
                            match manager
                                .prepare_semantic_with_lifecycle(
                                    projected,
                                    crate::compaction::CompactionTrigger::ProviderContextLimit,
                                    provider_limit_phase,
                                    None,
                                    self.provider.as_ref(),
                                    self.interrupt_flag.as_ref(),
                                    None,
                                    Some(&emit_provider_limit_started),
                                )
                                .await
                            {
                                Ok(crate::compaction::ContextPreparation::Compacted {
                                    checkpoint,
                                    ..
                                }) => {
                                    if let Some(commit) = &ctx.on_checkpoint {
                                        if let Err(error) = commit(&checkpoint) {
                                            on_event(RunEvent::CompactionFailed {
                                                operation_id: provider_limit_operation_id.clone(),
                                                trigger: crate::compaction::CompactionTrigger::ProviderContextLimit,
                                                phase: provider_limit_phase,
                                                error: error.to_string(),
                                            });
                                            return Err(error);
                                        }
                                    }
                                    active_checkpoint = Some((*checkpoint).clone());
                                    provider_limit_checkpoint_id =
                                        Some(checkpoint.checkpoint_id.clone());
                                    *self.active_checkpoint.lock() = Some((*checkpoint).clone());
                                    on_event(RunEvent::CompactionCommitted {
                                        operation_id: provider_limit_operation_id.clone(),
                                        checkpoint: *checkpoint,
                                    });
                                }
                                Ok(crate::compaction::ContextPreparation::Unchanged { .. }) => {
                                    let error = anyhow!(
                                        "context compaction made no progress after provider limit"
                                    );
                                    on_event(RunEvent::CompactionFailed {
                                        operation_id: provider_limit_operation_id.clone(),
                                        trigger: crate::compaction::CompactionTrigger::ProviderContextLimit,
                                        phase: provider_limit_phase,
                                        error: error.to_string(),
                                    });
                                    return Err(error);
                                }
                                Err(error) => {
                                    on_event(RunEvent::CompactionFailed {
                                        operation_id: provider_limit_operation_id.clone(),
                                        trigger: crate::compaction::CompactionTrigger::ProviderContextLimit,
                                        phase: provider_limit_phase,
                                        error: error.to_string(),
                                    });
                                    return Err(anyhow!(error));
                                }
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
                    tracing::error!("LLM call failed: {:#}", last_error);
                    return Err(last_error);
                }
            };

            // Reset retry on successful stream
            provider_limit_checkpoint_id = None;
            if retry_attempt > 0 {
                retry_attempt = 0;
            }

            // Process stream events
            let mut assistant_text = String::new();
            let mut reasoning_text = String::new();
            let mut reasoning_provider_metadata = crate::types::ProviderMetadata::new();
            let mut agent_tool_calls: Vec<AgentToolCall> = vec![];
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

                let model_event = match event {
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
                on_event(RunEvent::Model(model_event.clone()));

                // Close the text-output block before switching to a different
                // event type — text_end may never arrive from the LLM.
                let is_text = matches!(model_event, ModelStreamEvent::TextDelta { .. });
                if self.verbose && was_outputting && !is_text {
                    crate::eprintln_log!();
                    was_outputting = false;
                }
                if is_text && self.verbose {
                    was_outputting = true;
                }

                match model_event {
                    ModelStreamEvent::ReasoningStart { .. } => {
                        if self.verbose {
                            crate::eprint_log!("\n{}[thinking]{} ", C_MAGENTA, C_RESET);
                        }
                    }
                    ModelStreamEvent::ReasoningDelta { text, .. } => {
                        reasoning_text.push_str(&text);
                        if self.verbose {
                            crate::eprint_log!("{}", text);
                        }
                    }
                    ModelStreamEvent::ReasoningEnd {
                        provider_metadata, ..
                    } => {
                        reasoning_provider_metadata = provider_metadata;
                        if self.verbose {
                            crate::eprintln_log!(); // blank line after thinking
                        }
                    }
                    ModelStreamEvent::TextDelta { text, .. } => {
                        assistant_text.push_str(&text);
                        if self.verbose && !output_started {
                            output_started = true;
                            crate::eprint_log!("\n{}[output]{} ", C_GREEN, C_RESET);
                        }
                        if self.verbose {
                            crate::eprint_log!("{}", text);
                        }
                        on_text(text);
                    }
                    ModelStreamEvent::TextStart { .. } => {}
                    ModelStreamEvent::TextEnd { .. } => {
                        if self.verbose {
                            crate::eprintln_log!(); // blank line after output
                        }
                    }
                    ModelStreamEvent::ToolInputStart {
                        index,
                        id,
                        name,
                        arguments,
                        provider_metadata,
                    } => {
                        // Some providers (e.g. GLM/Z.AI without tool_stream)
                        // send id+name in every argument chunk instead of just
                        // the first — a repeat is merged into the pending call
                        // rather than starting a new one.
                        if merge_repeated_tool_input_start(
                            &mut current_tool_calls,
                            index,
                            &id,
                            &name,
                            arguments.as_ref(),
                            &provider_metadata,
                        ) {
                            continue;
                        }
                        if index >= current_tool_calls.len() {
                            current_tool_calls.resize(index + 1, None);
                        }

                        // Finalize any existing tool call at this index (different id)
                        if let Some(tc) = current_tool_calls[index].take() {
                            agent_tool_calls.push(finalize_agent_tool_call(tc));
                        }

                        current_tool_calls[index] = Some(AgentToolCall {
                            id,
                            name,
                            args: arguments.unwrap_or(serde_json::Value::Null),
                            provider_metadata,
                        });
                    }
                    ModelStreamEvent::ToolInputDelta {
                        index,
                        id,
                        delta,
                        snapshot,
                    } => {
                        if index >= current_tool_calls.len() {
                            current_tool_calls.resize(index + 1, None);
                        }
                        if let Some(tc_ref) = &mut current_tool_calls[index] {
                            if tc_ref.id.is_empty() && !id.is_empty() {
                                tc_ref.id = id;
                            }
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
                                if snapshot {
                                    *s = delta;
                                } else {
                                    s.push_str(&delta);
                                }
                            } else {
                                tc_ref.args = serde_json::Value::String(delta);
                            }
                        } else {
                            // Delta arrived before start — create placeholder
                            current_tool_calls[index] = Some(AgentToolCall {
                                id,
                                name: String::new(),
                                args: serde_json::Value::String(delta),
                                provider_metadata: Default::default(),
                            });
                        }
                    }
                    ModelStreamEvent::ToolInputEnd {
                        index,
                        id,
                        name,
                        arguments,
                        provider_metadata,
                    } => {
                        if index >= current_tool_calls.len() {
                            current_tool_calls.resize(index + 1, None);
                        }
                        let slot = current_tool_calls[index].get_or_insert_with(|| AgentToolCall {
                            id: id.clone(),
                            name: name.clone(),
                            args: arguments.clone(),
                            provider_metadata: provider_metadata.clone(),
                        });
                        if !id.is_empty() {
                            slot.id = id;
                        }
                        if !name.is_empty() {
                            slot.name = name;
                        }
                        if !arguments.is_null() {
                            slot.args = arguments;
                        }
                        if !provider_metadata.is_empty() {
                            slot.provider_metadata = provider_metadata;
                        }
                        if let Some(tc) = current_tool_calls[index].take() {
                            agent_tool_calls.push(finalize_agent_tool_call(tc));
                        }
                    }
                    ModelStreamEvent::Usage(usage) => {
                        self.process_usage_event(&usage, &mut total_usage);
                    }
                    ModelStreamEvent::Finish { reason, usage } => {
                        saw_terminal_event = true;
                        if reason == FinishReason::Incomplete {
                            stream_truncated = true;
                        }
                        if let Some(ref usage) = usage {
                            self.process_usage_event(usage, &mut total_usage);
                        }
                        for tc_opt in current_tool_calls.iter_mut() {
                            if let Some(tc) = tc_opt.take() {
                                agent_tool_calls.push(finalize_agent_tool_call(tc));
                            }
                        }
                    }
                    ModelStreamEvent::Error { message } => {
                        saw_terminal_event = true;
                        stream_error = Some(anyhow!(message));
                    }
                }
            }

            for tc_opt in current_tool_calls.iter_mut() {
                if let Some(tc) = tc_opt.take() {
                    agent_tool_calls.push(finalize_agent_tool_call(tc));
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
                 reasoning_provider_metadata: &crate::types::ProviderMetadata,
                 tool_calls: &[AgentToolCall]| {
                    let first_new_message = messages.len();
                    let msg = AgentMessage {
                        role: "assistant".to_string(),
                        content: {
                            let mut content = Vec::new();
                            if !reasoning_text.is_empty() || !reasoning_provider_metadata.is_empty()
                            {
                                content.push(ContentBlock::reasoning(
                                    reasoning_text,
                                    reasoning_provider_metadata.clone(),
                                ));
                            }
                            if !assistant_text.is_empty() {
                                content.push(ContentBlock::text(assistant_text));
                            }
                            for tc in tool_calls {
                                content.push(ContentBlock::tool_call(
                                    tc.id.clone(),
                                    tc.name.clone(),
                                    tc.args.clone(),
                                    tc.provider_metadata.clone(),
                                ));
                            }
                            content
                        },
                        ..Default::default()
                    };
                    // Don't push an empty assistant — the LLM API rejects
                    // messages with neither content nor tool_calls.
                    if !msg.content.is_empty() {
                        messages.push(msg);
                    }
                    // Append placeholder tool-result for every unexecuted
                    // tool call so the conversation remains API-valid.
                    for tc in tool_calls {
                        let cancelled = format!(
                            "[Tool execution cancelled — {} was not executed due to interrupt]",
                            tc.name
                        );
                        messages.push(AgentMessage {
                            role: "tool".to_string(),
                            content: vec![ContentBlock::tool_result(
                                tc.id.clone(),
                                &cancelled,
                                false,
                            )],
                            name: tc.name.clone(),
                            ..Default::default()
                        });
                    }
                    // Partial output is part of the authoritative conversation,
                    // just like a normally completed assistant/tool message.
                    // Persist every entry added by this interrupt path so the
                    // append-only terminal commit cannot leave memory and JSONL
                    // permanently divergent after abort/restart.
                    if let Some(ref save) = ctx.save_callback {
                        for message in &mut messages[first_new_message..] {
                            save(message);
                        }
                    }
                };

            // Check for stream errors, and for a pending interrupt that
            // arrived during the API call or last stream event (tokio::select!
            // can pick stream end over the interrupt channel). Both land on
            // the same partial-assistant exit; the single-line closure keeps
            // the test-unreproducible race edge off its own line.
            let interrupted_after_stream = interrupt_rx
                .as_mut()
                .is_some_and(|irx| irx.try_recv().is_ok());
            if stream_error.is_some() || interrupted_after_stream {
                build_partial_assistant(
                    &mut messages,
                    &assistant_text,
                    &reasoning_text,
                    &reasoning_provider_metadata,
                    &agent_tool_calls,
                );
                return Ok((String::new(), messages));
            }

            // Close any open output block (text_end may not have been emitted).
            if self.verbose && output_started {
                crate::eprintln_log!();
            }

            // Emit message_end

            // Build assistant message
            let assistant_msg = AgentMessage {
                role: "assistant".to_string(),
                content: {
                    let mut content = Vec::new();
                    if !reasoning_text.is_empty() || !reasoning_provider_metadata.is_empty() {
                        content.push(ContentBlock::reasoning(
                            &reasoning_text,
                            reasoning_provider_metadata.clone(),
                        ));
                    }
                    if !assistant_text.is_empty() {
                        content.push(ContentBlock::text(&assistant_text));
                    }
                    for tc in &agent_tool_calls {
                        content.push(ContentBlock::tool_call(
                            tc.id.clone(),
                            tc.name.clone(),
                            tc.args.clone(),
                            tc.provider_metadata.clone(),
                        ));
                    }
                    content
                },
                ..Default::default()
            };

            // Skip truly empty assistant messages — the LLM API rejects them
            // ("content or tool_calls must be set"). Reasoning and tool calls
            // are content blocks, so a message carrying either is non-empty.
            if !assistant_msg.content.is_empty() {
                messages.push(assistant_msg);
                // Persist the assistant response immediately so it survives a
                // crash mid-run, even if no tools were called in this turn.
                if let Some(ref save) = ctx.save_callback {
                    save(messages.last_mut().unwrap());
                }
            }

            let tool_calls: Vec<ToolCall> = agent_tool_calls
                .iter()
                .cloned()
                .map(agent_tool_call_to_tool_call)
                .collect();

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

            // Check stop condition. The is_some_and closure keeps the None
            // edge branchless (a nested if's closing brace here collected a
            // phantom zero-count coverage region).
            let stop_hit = self.config.stop_condition.as_ref().is_some_and(|stop_fn| {
                let llm_msgs: Vec<Message> = ConvertToLLM(&messages);
                stop_fn(llm_msgs, &assistant_text)
            });
            if stop_hit {
                return Ok((assistant_text, messages));
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
                &on_event,
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

/// Merge a repeated tool-input start (same tool id at the same stream index)
/// into the pending call's args, returning true when the event was consumed
/// as a repeat. Always prefers the longer args string — it's more complete:
/// some gateways (e.g. Aliyun MaaS) send chunks out of prefix order, or a
/// trailing fragment shorter than the accumulated args, and overwriting
/// longer data with shorter data is the primary cause of argument loss.
fn merge_repeated_tool_input_start(
    current_tool_calls: &mut [Option<AgentToolCall>],
    index: usize,
    id: &str,
    name: &str,
    arguments: Option<&serde_json::Value>,
    provider_metadata: &crate::types::ProviderMetadata,
) -> bool {
    let Some(Some(existing)) = current_tool_calls.get_mut(index) else {
        return false;
    };
    if existing.id != id {
        return false;
    }
    if !name.is_empty() {
        existing.name = name.to_string();
    }
    if !provider_metadata.is_empty() {
        existing.provider_metadata = provider_metadata.clone();
    }
    if let Some(arguments) = arguments {
        if let serde_json::Value::String(new_args) = arguments {
            let mut updated = false;
            if let serde_json::Value::String(ref mut s) = existing.args {
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
                existing.args = serde_json::Value::String(new_args.clone());
            }
        } else if !arguments.is_null() {
            existing.args = arguments.clone();
        }
    }
    true
}

fn finalize_agent_tool_call(mut tool_call: AgentToolCall) -> AgentToolCall {
    repair_partial_tool_args(&mut tool_call.args);
    tool_call
}

fn agent_tool_call_to_tool_call(tool_call: AgentToolCall) -> ToolCall {
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
        Events(Vec<ModelStreamEvent>),
        /// Fail the stream_model call itself.
        Fail(String),
        /// Send the events, then go silent forever (channel stays open).
        PartialThenStall(Vec<ModelStreamEvent>),
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
        async fn stream_model(
            &self,
            request: crate::llm::schema::ModelRequest,
        ) -> Result<ReceiverStream<ModelStreamEvent>> {
            if request
                .system_prompt
                .contains("context summarization agent")
            {
                let events = vec![
                    ev_text("## Objective\n- Continue the test.\n\n## Important Details\n- Preserve history.\n\n## Work State\n### Completed\n- Earlier work.\n\n### Active\n- Current run.\n\n### Blocked\n- (none)\n\n## Next Move\n1. Continue.\n\n## Relevant Files\n- (none)"),
                    ev_stop(),
                ];
                let (tx, rx) = mpsc::channel(events.len());
                for event in events {
                    let _ = tx.try_send(event);
                }
                return Ok(ReceiverStream::new(rx));
            }
            self.system_prompts.lock().push(request.system_prompt);
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

    struct TypedProvider {
        events: parking_lot::Mutex<Option<Vec<ModelStreamEvent>>>,
    }

    #[async_trait::async_trait]
    impl LLMProvider for TypedProvider {
        async fn stream_model(
            &self,
            _request: crate::llm::schema::ModelRequest,
        ) -> Result<ReceiverStream<ModelStreamEvent>> {
            let events = self.events.lock().take().unwrap_or_default();
            let (tx, rx) = mpsc::channel(events.len().max(1));
            for event in events {
                let _ = tx.try_send(event);
            }
            drop(tx);
            Ok(ReceiverStream::new(rx))
        }
    }

    fn ev_text(text: &str) -> ModelStreamEvent {
        ModelStreamEvent::TextDelta {
            id: "text".to_string(),
            text: text.to_string(),
        }
    }

    fn ev_stop() -> ModelStreamEvent {
        ModelStreamEvent::Finish {
            reason: FinishReason::Stop,
            usage: None,
        }
    }

    fn ev_usage(prompt: i64, completion: i64, credit_cost: Option<f64>) -> ModelStreamEvent {
        ModelStreamEvent::Usage(crate::types::Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
            cache_read_tokens: Some(3),
            cache_write_tokens: Some(2),
            reasoning_tokens: None,
            credit_cost,
            provider_metadata: None,
        })
    }

    fn ev_toolcall_start(index: usize, id: &str, name: &str, args: &str) -> ModelStreamEvent {
        ModelStreamEvent::ToolInputStart {
            index,
            id: id.to_string(),
            name: name.to_string(),
            arguments: Some(serde_json::Value::String(args.to_string())),
            provider_metadata: Default::default(),
        }
    }

    fn ev_toolcall_delta(index: usize, text: &str) -> ModelStreamEvent {
        ModelStreamEvent::ToolInputDelta {
            index,
            id: String::new(),
            delta: text.to_string(),
            snapshot: false,
        }
    }

    fn ev_toolcall_end() -> ModelStreamEvent {
        ModelStreamEvent::ToolInputEnd {
            index: 0,
            id: String::new(),
            name: String::new(),
            arguments: serde_json::Value::Null,
            provider_metadata: Default::default(),
        }
    }

    fn echo_tool() -> AgentTool {
        fn handler(
            args: serde_json::Value,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>> {
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

    #[tokio::test(flavor = "current_thread")]
    async fn run_consumes_typed_tool_state_and_emits_typed_events() {
        let mut start_metadata = crate::types::ProviderMetadata::new();
        start_metadata.insert("openai".into(), serde_json::json!({"item_id": "fc_1"}));
        let mut end_metadata = crate::types::ProviderMetadata::new();
        end_metadata.insert("openai".into(), serde_json::json!({"item_id": "fc_1_done"}));
        let provider = Arc::new(TypedProvider {
            events: parking_lot::Mutex::new(Some(vec![
                ModelStreamEvent::ToolInputStart {
                    index: 0,
                    id: "call_1".into(),
                    name: "lookup".into(),
                    arguments: Some(serde_json::json!("{}")),
                    provider_metadata: start_metadata,
                },
                ModelStreamEvent::ToolInputStart {
                    index: 1,
                    id: "call_2".into(),
                    name: "other".into(),
                    arguments: None,
                    provider_metadata: Default::default(),
                },
                ModelStreamEvent::ToolInputDelta {
                    index: 0,
                    id: "call_1".into(),
                    delta: "{\"q\":\"rust\"}".into(),
                    snapshot: true,
                },
                ModelStreamEvent::ToolInputDelta {
                    index: 1,
                    id: "call_2".into(),
                    delta: "{\"n\":2}".into(),
                    snapshot: false,
                },
                ModelStreamEvent::ToolInputEnd {
                    index: 0,
                    id: "call_1".into(),
                    name: "lookup".into(),
                    arguments: serde_json::json!({"q": "rust"}),
                    provider_metadata: end_metadata,
                },
                ModelStreamEvent::ToolInputEnd {
                    index: 1,
                    id: "call_2".into(),
                    name: "other".into(),
                    arguments: serde_json::json!({"n": 2}),
                    provider_metadata: Default::default(),
                },
                ModelStreamEvent::Finish {
                    reason: FinishReason::ToolCalls,
                    usage: None,
                },
            ])),
        });
        let config = crate::types::AgentConfig {
            stop_condition: Some(Arc::new(|_, _| true)),
            ..Default::default()
        };
        let loop_ = Loop::new(provider, "mock").with_config(config);
        let projected = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let (_, messages) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                {
                    let projected = projected.clone();
                    move |event| projected.lock().push(event)
                },
                None,
            )
            .await
            .unwrap();

        assert_eq!(messages[1].tool_calls().len(), 2);
        assert_eq!(
            messages[1].tool_calls()[0].args,
            serde_json::json!({"q": "rust"})
        );
        assert_eq!(
            messages[1].tool_calls()[0].provider_metadata["openai"]["item_id"],
            "fc_1_done"
        );
        let projected = projected.lock();
        assert!(projected.iter().any(|event| {
            matches!(
                event,
                RunEvent::Model(ModelStreamEvent::ToolInputDelta { snapshot: true, .. })
            )
        }));
    }

    fn noop_on_event(_: RunEvent) {}

    /// Install a thread-local tracing subscriber that discards output. The
    /// verbose log macros only evaluate their arguments when a subscriber
    /// enables the callsite — without one, argument lines never execute.
    fn tracing_sink() -> tracing::subscriber::DefaultGuard {
        tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .with_ansi(false)
                .finish(),
        )
    }

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
            ModelStreamEvent::ReasoningStart {
                id: "reasoning".into(),
            },
            ModelStreamEvent::ReasoningDelta {
                id: "reasoning".into(),
                text: "deep ".to_string(),
            },
            ModelStreamEvent::ReasoningDelta {
                id: "reasoning".into(),
                text: "thought".to_string(),
            },
            ModelStreamEvent::ReasoningEnd {
                id: "reasoning".into(),
                provider_metadata: Default::default(),
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
        assert_eq!(messages[1].reasoning_text(), "deep thought");
        assert_eq!(
            loop_
                .cumulative_input_tokens
                .load(std::sync::atomic::Ordering::Relaxed),
            10
        );
        assert_eq!(
            loop_
                .cumulative_output_tokens
                .load(std::sync::atomic::Ordering::Relaxed),
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
            loop_
                .last_prompt_tokens
                .load(std::sync::atomic::Ordering::Relaxed),
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
                              // Sink subscriber so the verbose tracing regions are evaluated.
        let _sink = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .with_ansi(false)
                .finish(),
        );
        let saved = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let tool_events = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let ctx = StreamContext {
            model: "mock".to_string(),
            system_prompt: "sys".to_string(),
            save_callback: Some({
                let saved = saved.clone();
                Arc::new(move |m: &mut AgentMessage| saved.lock().push(m.role.clone()))
            }),
            ..Default::default()
        };
        let (text, messages) = loop_
            .run_streaming_with_messages(
                user_messages("go"),
                &ctx,
                noop_on_text,
                {
                    let tool_events = tool_events.clone();
                    move |event| match event {
                        RunEvent::ToolExecutionStarted { .. } => {
                            tool_events.lock().push("tool_start")
                        }
                        RunEvent::ToolExecutionFinished { .. } => {
                            tool_events.lock().push("tool_end")
                        }
                        _ => {}
                    }
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "done");
        // user, assistant(tool_calls), tool result, assistant(text)
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[1].tool_calls().len(), 1);
        assert_eq!(messages[1].tool_calls()[0].name, "echo");
        assert_eq!(messages[2].role, "tool");
        assert!(messages[2].text().contains("echo:"));
        let tool_events = tool_events.lock().clone();
        assert!(tool_events.contains(&"tool_start"));
        assert!(tool_events.contains(&"tool_end"));
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
            .run_streaming_with_messages(
                messages,
                &StreamContext::default(),
                noop_on_text,
                noop_on_event,
                None,
            )
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
        assert!(result.unwrap_err().to_string().contains("turn limit"));
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
            ModelStreamEvent::Error {
                message: "upstream exploded".to_string(),
            },
        ])]);
        let loop_ = Loop::new(provider, "mock");
        let saved = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let ctx = StreamContext {
            save_callback: Some({
                let saved = saved.clone();
                Arc::new(move |m: &mut AgentMessage| saved.lock().push(m.role.clone()))
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
            ModelStreamEvent::Finish {
                reason: FinishReason::Incomplete,
                usage: None,
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
        let provider =
            ScriptedProvider::new(vec![Script::PartialThenStall(vec![ev_text("stuck")])]);
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
        let mut loop_ = Loop::new(provider, "mock").with_config(config);
        loop_.context_manager = Some(crate::compaction::ContextManager {
            enabled: false,
            reserve_tokens: 1,
            keep_recent_tokens: 100,
            context_window: 100,
            model: "mock".into(),
        });
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
        let mut loop_ = Loop::new(provider, "mock").with_config(config);
        loop_.context_manager = Some(crate::compaction::ContextManager {
            enabled: false,
            reserve_tokens: 1,
            keep_recent_tokens: 1,
            context_window: 1,
            model: "mock".into(),
        });
        // A single short user message cannot be compacted → hard failure.
        let mut messages = user_messages("hi");
        messages[0].ensure_journal_entry_id();
        let result = loop_
            .run_streaming_with_messages(
                messages,
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
            .contains("no valid journal boundary"));
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
        let mut loop_ = Loop::new(provider, "mock").with_config(config);
        loop_.context_manager = Some(crate::compaction::ContextManager {
            enabled: false,
            reserve_tokens: 1,
            keep_recent_tokens: 1,
            context_window: 1,
            model: "mock".into(),
        });
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
        messages.push(AgentMessage::new_user(
            "user",
            serde_json::json!("fresh question"),
        ));
        for message in &mut messages {
            message.ensure_journal_entry_id();
        }
        let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let (text, _) = loop_
            .run_streaming_with_messages(
                messages,
                &StreamContext::default(),
                noop_on_text,
                {
                    let events = events.clone();
                    move |event| match event {
                        RunEvent::CompactionStarted { .. } => {
                            events.lock().push("compaction_started")
                        }
                        RunEvent::CompactionCommitted { .. } => {
                            events.lock().push("compaction_committed")
                        }
                        RunEvent::CompactionFailed { .. } => {
                            events.lock().push("compaction_failed")
                        }
                        _ => {}
                    }
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "recovered");
        assert_eq!(
            events.lock().as_slice(),
            ["compaction_started", "compaction_committed"]
        );
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn provider_limit_retry_commits_only_one_checkpoint_for_the_model_step() {
        let provider = ScriptedProvider::new(vec![
            Script::Fail("[CTX_LIMIT] maximum context length".to_string()),
            Script::Fail("[CTX_LIMIT] maximum context length".to_string()),
        ]);
        let config = crate::types::AgentConfig {
            max_retries: 2,
            ..Default::default()
        };
        let mut loop_ = Loop::new(provider, "mock").with_config(config);
        loop_.context_manager = Some(crate::compaction::ContextManager {
            enabled: false,
            reserve_tokens: 1,
            keep_recent_tokens: 1,
            context_window: 1,
            model: "mock".into(),
        });
        let mut messages = Vec::new();
        for index in 0..6 {
            let role = if index % 2 == 0 { "user" } else { "assistant" };
            let mut message = AgentMessage {
                role: role.to_string(),
                content: vec![ContentBlock::text(format!("history {index}"))],
                ..Default::default()
            };
            message.ensure_journal_entry_id();
            messages.push(message);
        }
        messages.push(AgentMessage::new_user("user", serde_json::json!("latest")));
        messages.last_mut().unwrap().ensure_journal_entry_id();
        let committed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let result = loop_
            .run_streaming_with_messages(
                messages,
                &StreamContext {
                    on_checkpoint: Some(Arc::new(|_| Ok(()))),
                    ..Default::default()
                },
                noop_on_text,
                {
                    let committed = committed.clone();
                    move |event| {
                        if matches!(event, RunEvent::CompactionCommitted { .. }) {
                            committed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                },
                None,
            )
            .await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("refusing repeated compaction"));
        assert_eq!(committed.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_folds_steering_notes_into_system_prompt() {
        let provider = ScriptedProvider::new(vec![Script::Events(vec![ev_text("ok"), ev_stop()])]);
        let loop_ = Loop::new(provider.clone(), "mock");
        loop_.steering_notes.lock().push("be brief".to_string());
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
    async fn run_commits_checkpoint_without_replacing_history() {
        let provider = ScriptedProvider::new(vec![Script::Events(vec![ev_text("ok"), ev_stop()])]);
        let mut loop_ = Loop::new(provider, "mock");
        loop_.context_manager = Some(crate::compaction::ContextManager {
            enabled: true,
            reserve_tokens: 1,
            keep_recent_tokens: 1,
            context_window: 1,
            model: "mock".into(),
        });
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
                &StreamContext {
                    on_checkpoint: Some(Arc::new(|_| Ok(()))),
                    ..Default::default()
                },
                noop_on_text,
                {
                    let events = events.clone();
                    move |event| {
                        if matches!(event, RunEvent::CompactionCommitted { .. }) {
                            events.lock().push("compaction_committed")
                        }
                    }
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "ok");
        assert!(events.lock().contains(&"compaction_committed"));
        assert_eq!(final_messages.len(), 4, "full journal history is retained");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn checkpoint_commit_failure_emits_compaction_failed_after_started() {
        let provider = ScriptedProvider::new(vec![]);
        let mut loop_ = Loop::new(provider, "mock");
        loop_.context_manager = Some(crate::compaction::ContextManager {
            enabled: true,
            reserve_tokens: 1,
            keep_recent_tokens: 1,
            context_window: 1,
            model: "mock".into(),
        });
        let mut messages = user_messages("old");
        messages.push(AgentMessage {
            role: "assistant".to_string(),
            content: vec![ContentBlock::text("older reply")],
            ..Default::default()
        });
        messages.push(AgentMessage::new_user("user", serde_json::json!("latest")));
        let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let result = loop_
            .run_streaming_with_messages(
                messages,
                &StreamContext {
                    on_checkpoint: Some(Arc::new(|_| Err(anyhow!("checkpoint commit failed")))),
                    ..Default::default()
                },
                noop_on_text,
                {
                    let events = events.clone();
                    move |event| match event {
                        RunEvent::CompactionStarted { .. } => {
                            events.lock().push("compaction_started")
                        }
                        RunEvent::CompactionFailed { .. } => {
                            events.lock().push("compaction_failed")
                        }
                        RunEvent::CompactionCommitted { .. } => {
                            events.lock().push("compaction_committed")
                        }
                        _ => {}
                    }
                },
                None,
            )
            .await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("checkpoint commit failed"));
        assert_eq!(
            events.lock().as_slice(),
            ["compaction_started", "compaction_failed"]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_errors_when_compaction_has_no_journal_boundary() {
        let provider = ScriptedProvider::new(vec![Script::Events(vec![ev_text("ok"), ev_stop()])]);
        let mut loop_ = Loop::new(provider, "mock");
        loop_.context_manager = Some(crate::compaction::ContextManager {
            enabled: true,
            reserve_tokens: 1,
            keep_recent_tokens: 1,
            context_window: 1,
            model: "mock".into(),
        });
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
            .contains("no valid journal boundary"));
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
        let provider = ScriptedProvider::new(vec![Script::PartialThenStall(vec![ev_text(
            "before interrupt",
        )])]);
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
                ev_toolcall_end(),
                ModelStreamEvent::Usage(crate::types::Usage {
                    prompt_tokens: 7,
                    completion_tokens: 3,
                    total_tokens: 10,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                    reasoning_tokens: None,
                    credit_cost: None,
                    provider_metadata: None,
                }),
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
        assert_eq!(assistant.tool_calls().len(), 3);
        assert_eq!(assistant.tool_calls()[0].name, "");
        assert_eq!(assistant.tool_calls()[1].id, "call-1");
        assert_eq!(assistant.tool_calls()[2].id, "call-2");
        // call-1's args were concatenated verbatim (same-id start append +
        // non-'{' delta): `{"pre":1}` + `, "post":2}`.
        let calls = assistant.tool_calls();
        let args = calls[1].args.as_str().unwrap();
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
        assert!(tool_messages[1]
            .text()
            .contains("was skipped due to user interrupt"));
    }

    // ── pure helpers ────────────────────────────────────────────────────────

    #[test]
    fn tool_call_args_complete_checks_json_balance() {
        let mut call = AgentToolCall {
            id: "c1".to_string(),
            name: "echo".to_string(),
            args: serde_json::Value::String("{\"a\":1}".to_string()),
            provider_metadata: Default::default(),
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
            provider_metadata: Default::default(),
        };
        let finalized = finalize_agent_tool_call(complete);
        assert_eq!(finalized.id, "c1");
        assert_eq!(finalized.name, "echo");
        assert_eq!(
            finalized.args,
            serde_json::Value::String("{\"a\":1}".to_string())
        );

        let partial = AgentToolCall {
            id: "c2".to_string(),
            name: "echo".to_string(),
            args: serde_json::Value::String("{\"a\":\"x".to_string()),
            provider_metadata: Default::default(),
        };
        let finalized = finalize_agent_tool_call(partial);
        // The truncated JSON was repaired into something parseable.
        let args = finalized.args;
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
                ModelStreamEvent::ReasoningStart {
                    id: "reasoning".into(),
                },
                ModelStreamEvent::ReasoningDelta {
                    id: "reasoning".into(),
                    text: "ponder".to_string(),
                },
                ModelStreamEvent::ReasoningEnd {
                    id: "reasoning".into(),
                    provider_metadata: Default::default(),
                },
                ModelStreamEvent::TextStart { id: "text".into() },
                ev_text("chunk"),
                ModelStreamEvent::TextEnd { id: "text".into() },
                ev_toolcall_start(0, "c1", "echo", "{}"),
                // Usage piggy-backed on the terminal stop.
                ModelStreamEvent::Finish {
                    reason: FinishReason::ToolCalls,
                    usage: Some(crate::types::Usage {
                        prompt_tokens: 5,
                        completion_tokens: 1,
                        total_tokens: 6,
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                        reasoning_tokens: None,
                        credit_cost: Some(0.1),
                        provider_metadata: None,
                    }),
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

    #[tokio::test(flavor = "current_thread")]
    async fn run_with_empty_messages_validates_ok() {
        let provider = ScriptedProvider::new(vec![Script::Events(vec![ev_text("hi"), ev_stop()])]);
        let loop_ = Loop::new(provider, "mock");
        let (text, _) = loop_
            .run_streaming_with_messages(
                vec![],
                &StreamContext::default(),
                noop_on_text,
                |_| {},
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "hi");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_stop_event_finalizes_pending_tool_calls_and_usage() {
        // toolcall_start followed DIRECTLY by a usage-carrying stop: the stop
        // arm both finalizes the pending call and processes usage.
        let provider = ScriptedProvider::new(vec![
            Script::Events(vec![
                ev_toolcall_start(0, "c1", "echo", "{}"),
                ModelStreamEvent::Finish {
                    reason: FinishReason::ToolCalls,
                    usage: Some(crate::types::Usage {
                        prompt_tokens: 9,
                        completion_tokens: 4,
                        total_tokens: 13,
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                        reasoning_tokens: None,
                        credit_cost: None,
                        provider_metadata: None,
                    }),
                },
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
        assert_eq!(messages[1].tool_calls().len(), 1);
        assert_eq!(
            loop_
                .cumulative_input_tokens
                .load(std::sync::atomic::Ordering::Relaxed),
            9
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_verbose_truncated_stream_warns() {
        let _tracing = tracing_sink();
        let provider = ScriptedProvider::new(vec![Script::Events(vec![
            ev_text("cut off"),
            ModelStreamEvent::Finish {
                reason: FinishReason::Incomplete,
                usage: None,
            },
        ])]);
        let mut loop_ = Loop::new(provider, "mock");
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
        assert_eq!(text, "cut off");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_verbose_completion_logs_usage_details() {
        let _tracing = tracing_sink();
        let provider = ScriptedProvider::new(vec![Script::Events(vec![
            ev_text("final"),
            ModelStreamEvent::Finish {
                reason: FinishReason::Stop,
                usage: Some(crate::types::Usage {
                    prompt_tokens: 11,
                    completion_tokens: 7,
                    total_tokens: 18,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                    reasoning_tokens: None,
                    credit_cost: Some(0.01),
                    provider_metadata: None,
                }),
            },
        ])]);
        let mut loop_ = Loop::new(provider, "mock");
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
            async fn stream_model(
                &self,
                _request: crate::llm::schema::ModelRequest,
            ) -> Result<ReceiverStream<ModelStreamEvent>> {
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
            async fn stream_model(
                &self,
                _request: crate::llm::schema::ModelRequest,
            ) -> Result<ReceiverStream<ModelStreamEvent>> {
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
        let provider =
            ScriptedProvider::new(vec![Script::PartialThenStall(vec![ev_text("stuck")])]);
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
                Arc::new(move |m: &mut AgentMessage| saved.lock().push(m.role.clone()))
            }),
            ..Default::default()
        };
        let (_, messages) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &ctx,
                noop_on_text,
                |_| {},
                Some(interrupt_rx),
            )
            .await
            .unwrap();
        // Partial assistant carries the finalized tool call, followed by its
        // cancellation placeholder.
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].tool_calls().len(), 1);
        assert_eq!(messages[2].role, "tool");
        assert!(messages[2]
            .text()
            .contains("was not executed due to interrupt"));
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
                ModelStreamEvent::ToolInputStart {
                    index: 1,
                    id: "c2".into(),
                    name: "echo".into(),
                    arguments: None,
                    provider_metadata: Default::default(),
                },
                // …then a same-id start with real args → replace the Null.
                ev_toolcall_start(1, "c2", "echo", "{\"b\":2"),
                // Delta on non-String args → set to String.
                ModelStreamEvent::ToolInputStart {
                    index: 2,
                    id: "c3".into(),
                    name: "echo".into(),
                    arguments: Some(serde_json::Value::Null),
                    provider_metadata: Default::default(),
                },
                ev_toolcall_delta(2, "{\"c\":3"),
                // Full-state replacement delta (starts with '{' and extends).
                ev_toolcall_start(3, "c4", "echo", "{\"d\":"),
                ModelStreamEvent::ToolInputDelta {
                    index: 3,
                    id: "c4".into(),
                    delta: "{\"d\":4}".into(),
                    snapshot: true,
                },
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
        let calls = messages[1].tool_calls();
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
            async fn stream_model(
                &self,
                _request: crate::llm::schema::ModelRequest,
            ) -> Result<ReceiverStream<ModelStreamEvent>> {
                tokio::time::sleep(self.delay).await;
                let (tx, rx) = mpsc::channel(2);
                let _ = tx.try_send(ev_text("slow"));
                let _ = tx.try_send(ev_stop());
                drop(tx);
                Ok(ReceiverStream::new(rx))
            }
        }
        // Without an interrupt channel (None arm of await_or_interrupt).
        let loop_ = Loop::new(
            Arc::new(SlowProvider {
                delay: Duration::from_millis(120),
            }),
            "mock",
        );
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
        let loop_ = Loop::new(
            Arc::new(SlowProvider {
                delay: Duration::from_millis(120),
            }),
            "mock",
        );
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

    // ── coverage batch 14: residual run-loop arms ───────────────────────────

    /// Fails the LLM call after tripping the loop's interrupt flag, so the
    /// post-failure interrupt check (not the turn-top one) takes the exit.
    struct FailAndInterruptProvider {
        flag: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait::async_trait]
    impl LLMProvider for FailAndInterruptProvider {
        async fn stream_model(
            &self,
            _request: crate::llm::schema::ModelRequest,
        ) -> Result<ReceiverStream<ModelStreamEvent>> {
            self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
            Err(anyhow!("boom"))
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn run_interrupt_during_error_retry_returns_partial() {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let provider = FailAndInterruptProvider { flag: flag.clone() };
        let config = crate::types::AgentConfig {
            max_retries: 3,
            ..Default::default()
        };
        let mut loop_ = Loop::new(Arc::new(provider), "mock").with_config(config);
        loop_.interrupt_flag = flag;
        let (text, _) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                noop_on_event,
                None,
            )
            .await
            .unwrap();
        assert!(text.is_empty(), "interrupted runs return no final text");
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn run_reports_last_error_when_turn_limit_hit_after_failed_retry() {
        // Turn 0 fails once, the retry succeeds with a tool call (turn becomes
        // 1); the loop top then hits the turn limit. (The stale-last_error arm
        // was removed as dead by construction — see the limit check comment.)
        let provider = ScriptedProvider::new(vec![
            Script::Fail("late boom".to_string()),
            Script::Events(vec![
                ev_toolcall_start(0, "t1", "echo", "{}"),
                ev_toolcall_end(),
                ev_stop(),
            ]),
        ]);
        let config = crate::types::AgentConfig {
            max_turns: 1,
            max_retries: 1,
            ..Default::default()
        };
        let loop_ = Loop::new(provider, "mock")
            .with_tools(vec![echo_tool()])
            .with_config(config);
        let result = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                noop_on_event,
                None,
            )
            .await;
        let error = result.unwrap_err();
        assert!(error.to_string().contains("turn limit"), "{error}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_verbose_handles_typed_text_events() {
        let _sink = tracing_sink();
        let provider =
            ScriptedProvider::new(vec![Script::Events(vec![ev_text("done"), ev_stop()])]);
        let mut loop_ = Loop::new(provider, "mock");
        loop_.verbose = true;
        let (text, _) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                noop_on_event,
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "done");
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn run_finalizes_pending_tool_call_when_stream_ends_early() {
        // toolcall_start with no toolcall_end and no stop: the post-loop sweep
        // finalizes the pending call, then the turn ends as incomplete.
        let provider = ScriptedProvider::new(vec![Script::Events(vec![ev_toolcall_start(
            0, "t1", "echo", "{}",
        )])]);
        let loop_ = Loop::new(provider, "mock");
        loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                noop_on_event,
                None,
            )
            .await
            .unwrap();
        assert!(loop_
            .stream_incomplete
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_merges_repeated_toolcall_start_over_null_args() {
        // First chunk carries no tool_call payload (args stay Null); the
        // same-id repeat upgrades Null args to the real string via the
        // overwrite arm (the merge arm needs a String on both sides).
        let provider = ScriptedProvider::new(vec![
            Script::Events(vec![
                ModelStreamEvent::ToolInputStart {
                    index: 0,
                    id: "t1".into(),
                    name: "echo".into(),
                    arguments: None,
                    provider_metadata: Default::default(),
                },
                ev_toolcall_start(0, "t1", "echo", "{\"a\":1}"),
                // Repeat whose payload args are NOT a string: skipped over.
                ModelStreamEvent::ToolInputStart {
                    index: 0,
                    id: "t1".into(),
                    name: "echo".into(),
                    arguments: Some(serde_json::json!({"a": 1})),
                    provider_metadata: Default::default(),
                },
                // Repeat with no payload at all: also skipped over.
                ModelStreamEvent::ToolInputStart {
                    index: 0,
                    id: "t1".into(),
                    name: "echo".into(),
                    arguments: None,
                    provider_metadata: Default::default(),
                },
                ev_toolcall_end(),
                ev_stop(),
            ]),
            Script::Events(vec![ev_text("done"), ev_stop()]),
        ]);
        let loop_ = Loop::new(provider, "mock").with_tools(vec![echo_tool()]);
        let (_, messages) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                noop_on_event,
                None,
            )
            .await
            .unwrap();
        let tool_msg = messages
            .iter()
            .find(|m| m.role == "tool")
            .expect("tool result message");
        assert!(tool_msg.text().contains("echo"), "{}", tool_msg.text());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_stop_condition_ends_run_with_text() {
        let provider =
            ScriptedProvider::new(vec![Script::Events(vec![ev_text("stop here"), ev_stop()])]);
        let config = crate::types::AgentConfig {
            stop_condition: Some(Arc::new(|_msgs, text| text.contains("stop"))),
            ..Default::default()
        };
        let loop_ = Loop::new(provider, "mock").with_config(config);
        let (text, _) = loop_
            .run_streaming_with_messages(
                user_messages("hi"),
                &StreamContext::default(),
                noop_on_text,
                noop_on_event,
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "stop here");
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn sleep_or_interrupt_channel_variants() {
        let loop_ = Loop::new(ScriptedProvider::new(vec![]), "mock");
        // Sender alive but silent: the step sleep elapses, then the deadline.
        let (_tx, mut rx) = mpsc::channel::<()>(1);
        let slept_through = loop_
            .sleep_or_interrupt(Duration::from_millis(5), Some(&mut rx))
            .await;
        assert!(!slept_through);
        // A queued interrupt wakes the sleep immediately.
        let (tx, mut rx) = mpsc::channel::<()>(1);
        tx.try_send(()).unwrap();
        let interrupted = loop_
            .sleep_or_interrupt(Duration::from_secs(60), Some(&mut rx))
            .await;
        assert!(interrupted);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn await_or_interrupt_returns_none_when_flag_pre_set() {
        let loop_ = Loop::new(ScriptedProvider::new(vec![]), "mock");
        loop_.abort();
        let out = loop_.await_or_interrupt(async { 42 }, None).await;
        assert_eq!(out, None);
    }
}
