//! The agent run loop: one conversation driven turn by turn.
//!
//! `run_streaming_with_messages` is the entry point and `run_streaming_inner`
//! is the driver. Everything it delegates to lives in a sibling module:
//!
//! - [`blocks`] — accumulates a turn's content blocks and keeps their order
//! - [`stream_events`] — drains one model stream into an accumulated turn
//! - [`tool_stream`] — normalizes provider tool-call streaming into calls
//! - [`placeholders`] — keeps the transcript API-valid when calls are skipped
//! - [`retry`] — interruptible waits and the stream-retry budget
//! - [`billing`] — per-request usage and cost accounting

mod billing;
mod blocks;
mod placeholders;
mod retry;
mod stream_events;
mod tool_stream;

#[cfg(test)]
mod tests;

use crate::llm::schema::ModelStreamEvent;
use crate::types::{AgentMessage, AgentToolCall, ContentBlock, ConvertToLLM, Message, ToolCall};
use anyhow::{anyhow, Result};
use std::time::Duration;

use super::{Loop, RunEvent, DEFAULT_MAX_TURNS};

pub(crate) use self::billing::{
    add_request_usage, estimate_usage_cost_with, usage_cost_split_with,
};
use self::blocks::{
    assemble_assistant_parts, bind_stream_message, AccumulatedReasoningBlock, AccumulatedTextBlock,
    AssistantBlockOrder,
};
use self::placeholders::append_skipped_tool_placeholders;
use self::retry::{is_retryable_size_error, MAX_STREAM_RETRIES};
use self::stream_events::StreamState;
use self::tool_stream::{
    agent_tool_call_to_tool_call, clear_unfinished_provider_identity, push_finalized_tool_call,
};

impl Loop {
    pub async fn run_streaming_with_messages(
        &self,
        mut messages: Vec<AgentMessage>,
        ctx: &super::StreamContext,
        on_text: impl Fn(String) + Send + 'static,
        on_event: impl Fn(RunEvent) + Send + Sync + 'static,
        interrupt_rx: Option<tokio::sync::mpsc::Receiver<()>>,
    ) -> Result<(String, Vec<AgentMessage>)> {
        match self
            .run_streaming_inner(&mut messages, ctx, on_text, on_event, interrupt_rx)
            .await
        {
            Ok(text) => Ok((text, messages)),
            Err(error) => {
                if let Some(callback) = &ctx.on_failure_messages {
                    callback(messages);
                }
                Err(error)
            }
        }
    }

    async fn run_streaming_inner(
        &self,
        messages: &mut Vec<AgentMessage>,
        ctx: &super::StreamContext,
        mut on_text: impl Fn(String) + Send + 'static,
        on_event: impl Fn(RunEvent) + Send + Sync + 'static,
        mut interrupt_rx: Option<tokio::sync::mpsc::Receiver<()>>,
    ) -> Result<String> {
        // Every model-visible message is bound to a stable journal identity.
        // Loaded messages already carry it; fresh/ephemeral callers receive one
        // here so compaction provenance never depends on vector indices.
        for message in messages.iter_mut() {
            message.ensure_journal_entry_id();
        }
        let mut active_checkpoint = self.active_checkpoint.lock().clone();
        let mut context_manager = self.context_manager.clone();
        // Validate: a conversation must never reach the model ending on an
        // assistant turn. That shape means the previous run was cut off, and
        // since nothing continues an interrupted run by itself, the caller is
        // a bug upstream.
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
        let max_stream_retries = if self.config.max_retries > 0 {
            MAX_STREAM_RETRIES
        } else {
            0
        };
        let mut stream_retry_attempt = 0;
        let mut reconnecting = false;
        let mut retry_text = String::new();
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
                return Ok(String::new());
            }

            // Emit turn_start

            // Model limits are provider authority, not run/session state. A
            // catalog edit can change context limits while a multi-turn agent
            // run is active, so refresh the compaction budget from the same
            // shared Registry before every actual LLM turn. The dynamic LLM
            // client independently resolves protocol, route, headers,
            // modalities and max-output tokens for the request itself.
            let mut output_reserve = 0;
            if let (Some(registry), Some(manager)) =
                (&self.model_registry, context_manager.as_mut())
            {
                let model_ref = if self.model_ref.is_empty() {
                    ctx.model.as_str()
                } else {
                    self.model_ref.as_str()
                };
                if let Some((_identity, model, _api_key)) =
                    registry.read().resolve_request_target(model_ref)
                {
                    output_reserve = crate::models::effective_max_tokens(&model);
                    manager.context_window = model.context_window.max(1);
                    let (reserve_tokens, keep_recent_tokens) =
                        crate::compaction::context_token_budgets(manager.context_window);
                    manager.reserve_tokens = reserve_tokens;
                    manager.keep_recent_tokens = keep_recent_tokens;
                    manager.model = model.id;
                }
            }

            // Ephemeral/direct Loop users may have no persistence callback.
            // New assistant/tool messages still need stable in-memory IDs for
            // checkpoint ranges across subsequent turns.
            for message in messages.iter_mut() {
                if message.journal_entry_id().is_none() {
                    message.ensure_journal_entry_id();
                }
            }
            // Build a model-only projection from the immutable journal view.
            // A committed checkpoint changes this request's prompt but never
            // replaces `messages`, which remains the complete transcript.
            let context_window = context_manager
                .as_ref()
                .map(|manager| manager.context_window.max(1) as u64)
                .unwrap_or(1_000_000);
            let reported_input = self
                .last_prompt_tokens
                .load(std::sync::atomic::Ordering::Relaxed)
                .try_into()
                .ok()
                .filter(|tokens: &u64| *tokens > 0);
            let mut projected = crate::compaction::project_prompt_context(
                messages,
                active_checkpoint.as_ref(),
                reported_input,
                context_window,
            );
            // The turn's system prompt is the session's own, unchanged. It used to grow a
            // post-checkpoint recall guidance, which the budget had to reserve in advance so
            // committing a checkpoint could not push the next request past admission; with
            // the guidance gone there is nothing to reserve and nothing to keep in step.
            let budget_system = ctx.system_prompt.as_str();
            crate::compaction::set_request_budget(
                &mut projected,
                budget_system,
                &tool_defs,
                output_reserve,
            );
            let automatic_phase = if turn == 0 {
                crate::compaction::CompactionPhase::PreTurn
            } else {
                crate::compaction::CompactionPhase::MidTurn
            };
            // Model selection is a control-plane change only: it must never
            // compact history when the user clicks the selector. A real session
            // enables this preflight guard on its run snapshot, so the first LLM
            // request checks the selected model's actual context window here.
            // `ModelContextDownshift` is threshold-gated but bypasses the user's
            // automatic mid-run compaction preference; no checkpoint is written
            // unless the request truly needs one to fit.
            let automatic_trigger = if turn == 0 && self.preflight_context_check {
                crate::compaction::CompactionTrigger::ModelContextDownshift
            } else {
                crate::compaction::CompactionTrigger::Automatic
            };
            let automatic_operation_id = crate::utils::generate_entry_id();
            let emit_automatic_started = || {
                on_event(RunEvent::CompactionStarted {
                    operation_id: automatic_operation_id.clone(),
                    trigger: automatic_trigger,
                    phase: automatic_phase,
                });
            };
            // The summary request carries the session's own system prompt, so its prefix
            // matches the turn that was just sent and the provider serves it from cache.
            let compaction_system_prompt = ctx.system_prompt.as_str();
            // The summary request is billed to the session, but it is not the user's
            // turn: `last_prompt_tokens` drives the compaction threshold and must keep
            // describing the conversation the agent actually sent.
            let compaction_summary_usage: std::sync::Arc<
                parking_lot::Mutex<Option<crate::types::Usage>>,
            > = std::sync::Arc::new(parking_lot::Mutex::new(None));
            let record_summary_usage = {
                let usage = compaction_summary_usage.clone();
                let input = self.cumulative_input_tokens.clone();
                let output = self.cumulative_output_tokens.clone();
                let cache_read = self.cumulative_cache_read_tokens.clone();
                let cache_write = self.cumulative_cache_write_tokens.clone();
                move |reported: &crate::types::Usage| {
                    // Called once per attempt with that attempt's final usage (a
                    // billed request each), never per progressive chunk — the
                    // same one-request-one-record rule the main loop follows.
                    add_request_usage(&input, &output, &cache_read, &cache_write, reported);
                    // Keep only the final chunk: providers send progressive cost.
                    *usage.lock() = Some(reported.clone());
                }
            };
            let mut compaction_ticket = None;
            let prepared = if let Some(manager) = &context_manager {
                if provider_limit_checkpoint_id.is_some() {
                    // The retry below must use exactly the checkpoint produced
                    // for this failed model step. A second automatic checkpoint
                    // here could hide a non-context provider failure in a loop.
                    Ok(crate::compaction::ContextPreparation::Unchanged {
                        prompt: projected.clone(),
                    })
                } else {
                    crate::compaction::prepare_with_journal_and_summary(
                        manager,
                        projected.clone(),
                        messages,
                        automatic_trigger,
                        automatic_phase,
                        None,
                        self.interrupt_flag.as_ref(),
                        Some(&emit_automatic_started),
                        ctx.compaction_journal.as_ref(),
                        &automatic_operation_id,
                        Some(self.provider.as_ref()),
                        Some(compaction_system_prompt),
                        &tool_defs,
                        Some(&record_summary_usage),
                        None,
                    )
                    .await
                    .map(|(prompt, ticket)| {
                        compaction_ticket = ticket;
                        prompt
                    })
                }
            } else {
                Ok(crate::compaction::ContextPreparation::Unchanged {
                    prompt: projected.clone(),
                })
            };
            let prompt = match prepared {
                Ok(crate::compaction::ContextPreparation::Unchanged { prompt }) => {
                    if let Some(ticket) = &compaction_ticket {
                        ticket.finish(None, serde_json::json!({"alreadyCompacted":active_checkpoint.is_some(),"tokensBefore":prompt.usage.estimated_input_tokens,"tokensAfter":prompt.usage.estimated_input_tokens,"summary":"","messagesRemoved":0}))?;
                    }
                    prompt
                }
                Ok(crate::compaction::ContextPreparation::Compacted { prompt, checkpoint }) => {
                    let commit_result = if let Some(ticket) = &compaction_ticket {
                        ticket.finish(Some(&checkpoint),serde_json::json!({"checkpointId":checkpoint.checkpoint_id,"tokensBefore":checkpoint.tokens_before,"tokensAfter":checkpoint.tokens_after,"algorithmVersion":checkpoint.algorithm_version,"summaryOutcome":checkpoint.summary_outcome}))
                    } else if let Some(commit) = &ctx.on_checkpoint {
                        commit(&checkpoint)
                    } else {
                        Ok(())
                    };
                    if let Err(error) = commit_result {
                        if let Some(ticket) = &compaction_ticket {
                            ticket.fail(&error.to_string());
                        }
                        {
                            on_event(RunEvent::CompactionFailed {
                                operation_id: automatic_operation_id.clone(),
                                trigger: automatic_trigger,
                                phase: automatic_phase,
                                error: error.to_string(),
                            });
                            return Err(error);
                        }
                    }
                    self.last_prompt_tokens
                        .store(0, std::sync::atomic::Ordering::Relaxed);
                    active_checkpoint = Some((*checkpoint).clone());
                    *self.active_checkpoint.lock() = Some((*checkpoint).clone());
                    if compaction_ticket
                        .as_ref()
                        .is_none_or(|t| t.cached_result().is_none())
                    {
                        on_event(RunEvent::CompactionCommitted {
                            operation_id: automatic_operation_id.clone(),
                            checkpoint: *checkpoint,
                        });
                    }
                    prompt
                }
                Err(error) => {
                    on_event(RunEvent::CompactionFailed {
                        operation_id: automatic_operation_id.clone(),
                        trigger: automatic_trigger,
                        phase: automatic_phase,
                        error: error.to_string(),
                    });
                    if matches!(
                        error,
                        crate::compaction::ContextError::InvalidSummary
                            | crate::compaction::ContextError::SummaryFailed(_)
                    ) {
                        // Automatic semantic summarization is opportunistic.
                        // Keep the current projection and let the model request
                        // run; a real provider context-limit response below has
                        // its own emergency recovery path.
                        projected
                    } else {
                        return Err(anyhow!(error));
                    }
                }
            };
            if let Some(manager) = &context_manager {
                if prompt
                    .usage
                    .estimated_input_tokens
                    .max(prompt.usage.input_tokens.unwrap_or(0))
                    > manager.input_limit(&prompt)
                {
                    return Err(anyhow!("context input exceeds model capacity after reserving system/tools and output; reduce input or compact history"));
                }
            }
            let mut work_messages: Vec<AgentMessage> = prompt
                .messages
                .into_iter()
                .map(|projected| projected.message)
                .collect();

            // Make continuation explicit rather than relying on assistant
            // prefill support. This is request-only recovery context, not a
            // fabricated user submission in the durable conversation or UI.
            if stream_retry_attempt > 0
                && work_messages
                    .last()
                    .is_some_and(|message| message.role == "assistant")
            {
                let mut recovery = AgentMessage {
                    role: "user".into(),
                    content: vec![ContentBlock::text(
                        "[Automatic recovery after a connection interruption. Continue the original task from the preserved partial response. Do not repeat text already delivered or rerun completed tool calls.]",
                    )],
                    ..Default::default()
                };
                recovery.ensure_journal_entry_id();
                work_messages.push(recovery);
            }

            // Emit message_start

            // Convert to LLM format
            if self.verbose {
                let llm_messages: Vec<Message> = ConvertToLLM(&work_messages);
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

            let request_system_prompt = ctx.system_prompt.as_str();

            // Stream chat — interruptible so a stop during connect / TLS /
            // time-to-first-byte takes effect immediately instead of blocking
            // on the request to return (especially noticeable on Windows where
            // flaky connections make this phase slow).
            let stream_result = match self
                .await_or_interrupt(
                    self.provider
                        .stream_model(crate::llm::schema::ModelRequest {
                            model: ctx.model.clone(),
                            messages: work_messages,
                            tools: tool_defs.clone(),
                            system_prompt: request_system_prompt.to_string(),
                        }),
                    interrupt_rx.as_mut(),
                )
                .await
            {
                Some(r) => r,
                None => {
                    return Ok(String::new());
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
                            // A rejected first prompt is still pre-turn: no
                            // model response or tool result exists yet, so do
                            // not replace that user input with a lossy
                            // checkpoint. Once a completed model/tool round
                            // exists, recovery may summarize the active turn.
                            let provider_limit_phase = if turn == 0 {
                                crate::compaction::CompactionPhase::PreTurn
                            } else {
                                crate::compaction::CompactionPhase::MidTurn
                            };
                            let provider_limit_operation_id = crate::utils::generate_entry_id();
                            let Some(manager) = &context_manager else {
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
                            let mut projected = crate::compaction::project_prompt_context(
                                messages,
                                active_checkpoint.as_ref(),
                                None,
                                manager.context_window.max(1) as u64,
                            );
                            crate::compaction::set_request_budget(
                                &mut projected,
                                budget_system,
                                &tool_defs,
                                output_reserve,
                            );
                            let emit_provider_limit_started = || {
                                on_event(RunEvent::CompactionStarted {
                                    operation_id: provider_limit_operation_id.clone(),
                                    trigger:
                                        crate::compaction::CompactionTrigger::ProviderContextLimit,
                                    phase: provider_limit_phase,
                                });
                            };
                            let mut recovery_ticket = None;
                            match crate::compaction::prepare_with_journal_and_summary(
                                manager,
                                projected,
                                messages,
                                crate::compaction::CompactionTrigger::ProviderContextLimit,
                                provider_limit_phase,
                                None,
                                self.interrupt_flag.as_ref(),
                                Some(&emit_provider_limit_started),
                                ctx.compaction_journal.as_ref(),
                                &provider_limit_operation_id,
                                Some(self.provider.as_ref()),
                                Some(compaction_system_prompt),
                                &tool_defs,
                                Some(&record_summary_usage),
                                None,
                            )
                            .await
                            .map(|(prompt, ticket)| {
                                recovery_ticket = ticket;
                                prompt
                            }) {
                                Ok(crate::compaction::ContextPreparation::Compacted {
                                    checkpoint,
                                    ..
                                }) => {
                                    let committed = if let Some(ticket) = &recovery_ticket {
                                        ticket.finish(Some(&checkpoint), serde_json::json!({"checkpointId":checkpoint.checkpoint_id,"tokensBefore":checkpoint.tokens_before,"tokensAfter":checkpoint.tokens_after,"algorithmVersion":checkpoint.algorithm_version,"summaryOutcome":checkpoint.summary_outcome}))
                                    } else if let Some(commit) = &ctx.on_checkpoint {
                                        commit(&checkpoint)
                                    } else {
                                        Ok(())
                                    };
                                    if let Err(error) = committed {
                                        if let Some(ticket) = &recovery_ticket {
                                            ticket.fail(&error.to_string());
                                        }
                                        {
                                            on_event(RunEvent::CompactionFailed {
                                                operation_id: provider_limit_operation_id.clone(),
                                                trigger: crate::compaction::CompactionTrigger::ProviderContextLimit,
                                                phase: provider_limit_phase,
                                                error: error.to_string(),
                                            });
                                            return Err(error);
                                        }
                                    }
                                    self.last_prompt_tokens
                                        .store(0, std::sync::atomic::Ordering::Relaxed);
                                    active_checkpoint = Some((*checkpoint).clone());
                                    provider_limit_checkpoint_id =
                                        Some(checkpoint.checkpoint_id.clone());
                                    *self.active_checkpoint.lock() = Some((*checkpoint).clone());
                                    if recovery_ticket
                                        .as_ref()
                                        .is_none_or(|t| t.cached_result().is_none())
                                    {
                                        on_event(RunEvent::CompactionCommitted {
                                            operation_id: provider_limit_operation_id.clone(),
                                            checkpoint: *checkpoint,
                                        });
                                    }
                                }
                                Ok(crate::compaction::ContextPreparation::Unchanged { .. }) => {
                                    if let Some(ticket) = &recovery_ticket {
                                        ticket.fail("no progress after provider limit");
                                    }
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
                            return Ok(String::new());
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
                            return Ok(String::new());
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

            let stream_entry_id = crate::utils::generate_entry_id();
            if ctx.segmented_storage {
                on_event(RunEvent::MessageStart {
                    entry_id: stream_entry_id.clone(),
                });
            }
            // Process stream events
            let mut stream = StreamState::new(&retry_text);
            self.consume_stream(
                &mut stream,
                &mut rx,
                ctx,
                &mut on_text,
                &on_event,
                &mut interrupt_rx,
                &mut reconnecting,
                stream_retry_attempt,
                max_stream_retries,
                turn,
            )
            .await;
            let StreamState {
                assistant_text,
                reasoning_blocks,
                text_blocks,
                block_order: mut assistant_block_order,
                mut agent_tool_calls,
                total_usage,
                mut current_tool_calls,
                output_started,
                stream_error,
                interrupted_during_stream,
                model_stream_failed,
                stream_truncated,
                truncation_detected_by,
                ..
            } = stream;

            // Release the HTTP pump before potentially slow persistence during
            // cancellation. Its tx.closed() branch drops the upstream response.
            drop(rx);

            for tc_opt in current_tool_calls.iter_mut() {
                if let Some(mut tc) = tc_opt.take() {
                    clear_unfinished_provider_identity(&mut tc);
                    push_finalized_tool_call(&mut agent_tool_calls, &mut assistant_block_order, tc);
                }
            }

            // Charge the compaction summary once, the same way a model call is
            // charged: from the final usage chunk, never per progressive chunk.
            if let Some(summary_usage) = compaction_summary_usage.lock().clone() {
                let cost = summary_usage
                    .credit_cost
                    .unwrap_or_else(|| self.estimate_usage_cost(&summary_usage));
                if cost > 0.0 {
                    *self.cumulative_cost.lock() += cost;
                }
                self.charge_cost_split(&summary_usage);
                // Always reported: a zero-priced summary must not look like no request.
                tracing::info!(
                    cost,
                    input = summary_usage.prompt_tokens,
                    output = summary_usage.completion_tokens,
                    cache_read = summary_usage.cache_read_tokens.unwrap_or(0),
                    cache_write = summary_usage.cache_write_tokens.unwrap_or(0),
                    "charged a compaction summary"
                );
            }

            // Apply this LLM call's usage to the session exactly once: tokens
            // and cost, from `total_usage` — the call's merged final block, not
            // any progressive chunk. Adding every chunk would inflate the
            // totals N× (this is what reported a 99.8%-cached prompt as a 46%
            // one, by counting a coarse synthesized frame plus the real block).
            // A provider that reports an authoritative `credit_cost` (the
            // Future platform) is charged that; otherwise the cost falls back to
            // a token×price estimate so mixed-model sessions still account for
            // every request.
            if let Some(ref u) = total_usage {
                self.charge_request_usage(u);
            }

            // Build a partial assistant message from whatever was accumulated
            // before the interrupt — reasoning, text, and tool calls should
            // survive an abort so the user doesn't lose generated content.
            // Tool calls that were never executed MUST be followed by
            // placeholder tool-result messages, otherwise the LLM API rejects
            // the conversation on resume (HTTP 400: "assistant message with
            // tool_calls must be followed by tool messages").
            if ctx.segmented_storage {
                on_event(RunEvent::MessageEnd {
                    entry_id: stream_entry_id.clone(),
                    complete: !interrupted_during_stream
                        && !model_stream_failed
                        && !stream_truncated,
                });
            }
            ctx.await_durability().await?;
            let build_partial_assistant = |messages: &mut Vec<AgentMessage>,
                                           assistant_block_order: &[AssistantBlockOrder],
                                           reasoning_blocks: &[AccumulatedReasoningBlock],
                                           text_blocks: &[AccumulatedTextBlock],
                                           tool_calls: &[AgentToolCall],
                                           reason: &str| {
                let first_new_message = messages.len();
                let (content, keys) = assemble_assistant_parts(
                    assistant_block_order,
                    reasoning_blocks,
                    text_blocks,
                    tool_calls,
                );
                let mut msg = AgentMessage {
                    role: "assistant".into(),
                    content,
                    ..Default::default()
                };
                if ctx.segmented_storage {
                    bind_stream_message(&mut msg, &stream_entry_id, keys);
                }
                // Don't push an empty assistant — the LLM API rejects
                // messages with neither content nor tool_calls.
                if !msg.content.is_empty() {
                    messages.push(msg);
                }
                // Append placeholder tool-result for every unexecuted
                // tool call so the conversation remains API-valid.
                for tc in tool_calls {
                    let cancelled = format!(
                        "[Tool execution cancelled — {} was not executed due to {reason}]",
                        tc.name
                    );
                    messages.push(AgentMessage {
                        role: "tool".to_string(),
                        content: vec![ContentBlock::tool_result(tc.id.clone(), &cancelled, false)],
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
            let interrupted_after_stream = interrupted_during_stream
                || self.is_interrupted()
                || interrupt_rx
                    .as_mut()
                    .is_some_and(|irx| irx.try_recv().is_ok());
            let disconnected = matches!(
                truncation_detected_by,
                Some("upstream_disconnected" | "eof_no_terminal")
            );
            if disconnected
                && !interrupted_after_stream
                && stream_retry_attempt < max_stream_retries
            {
                // Resume from durable history, never from the beginning of the
                // run. This step's tools have not executed: pair even partial
                // calls with skipped results before sending history upstream.
                build_partial_assistant(
                    messages,
                    &assistant_block_order,
                    &reasoning_blocks,
                    &text_blocks,
                    &agent_tool_calls,
                    "connection interruption (retrying)",
                );
                for tc in &agent_tool_calls {
                    let output = format!(
                        "[Tool execution cancelled — {} was not executed due to connection interruption (retrying)]",
                        tc.name
                    );
                    on_event(RunEvent::ToolExecutionFinished {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        output: output.clone(),
                        error: Some(output),
                        exit_code: None,
                        is_soft_fail: None,
                        target_path: None,
                        shell_result: None,
                    });
                }
                // Close a possibly unfinished reasoning display before the
                // next attempt. No synthetic reasoning metadata is persisted.
                for block in &reasoning_blocks {
                    on_event(RunEvent::Model(ModelStreamEvent::ReasoningEnd {
                        id: block.id.clone(),
                        provider_metadata: block.provider_metadata.clone(),
                    }));
                }
                retry_text = assistant_text;
                stream_retry_attempt += 1;
                let delay_ms = 2000_u64 << (stream_retry_attempt - 1);
                reconnecting = true;
                on_event(RunEvent::StreamRetry {
                    attempt: stream_retry_attempt,
                    max_retries: max_stream_retries,
                    delay_ms,
                });
                tracing::warn!(
                    turn,
                    attempt = stream_retry_attempt,
                    max_retries = max_stream_retries,
                    delay_ms,
                    error = ?stream_error,
                    "LLM stream disconnected; reconnecting"
                );
                if self
                    .sleep_or_interrupt(Duration::from_millis(delay_ms), interrupt_rx.as_mut())
                    .await
                {
                    return Ok(String::new());
                }
                // Opening another HTTP stream is not success: only a complete
                // model step resets this budget. Retries do not consume turns.
                continue;
            }
            if stream_error.is_some() || interrupted_after_stream {
                let reason = if interrupted_after_stream {
                    "interrupt"
                } else if disconnected {
                    "connection interruption (retries exhausted)"
                } else {
                    "provider stream error"
                };
                build_partial_assistant(
                    messages,
                    &assistant_block_order,
                    &reasoning_blocks,
                    &text_blocks,
                    &agent_tool_calls,
                    reason,
                );
                for tc in &agent_tool_calls {
                    let output = format!(
                        "[Tool execution cancelled — {} was not executed due to {reason}]",
                        tc.name
                    );
                    on_event(RunEvent::ToolExecutionFinished {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        output: output.clone(),
                        error: Some(output),
                        exit_code: None,
                        is_soft_fail: None,
                        target_path: None,
                        shell_result: None,
                    });
                }
                if model_stream_failed {
                    self.stream_incomplete
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    *self.stream_truncation.lock() = Some(crate::agent::StreamTruncation {
                        turns_so_far: turn as i32,
                        output_len: assistant_text.len(),
                        tool_calls_so_far: agent_tool_calls.len(),
                        detected_by: truncation_detected_by
                            .unwrap_or("model_response_error")
                            .to_string(),
                    });
                }
                return Ok(String::new());
            }

            // Close any open output block (text_end may not have been emitted).
            if self.verbose && output_started {
                crate::eprintln_log!();
            }

            // Emit message_end

            // Build assistant message
            let (content, keys) = assemble_assistant_parts(
                &assistant_block_order,
                &reasoning_blocks,
                &text_blocks,
                &agent_tool_calls,
            );
            let mut assistant_msg = AgentMessage {
                role: "assistant".into(),
                content,
                ..Default::default()
            };
            if ctx.segmented_storage {
                bind_stream_message(&mut assistant_msg, &stream_entry_id, keys);
            }

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

            // Stream was truncated mid-reply: the assistant text is a prefix,
            // not a finished answer. End the turn as `incomplete` (keeping the
            // partial text so it isn't lost) rather than presenting a cut-off
            // reply as `complete`. Tool calls, if
            // any, are left unexecuted — their arguments may be partial — but
            // every tool_call_id must still get a placeholder result or the
            // next LLM request is rejected with HTTP 400.
            if stream_truncated {
                self.stream_incomplete
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                // Capture how far the run had progressed and how the cut was
                // detected, so the run commit path can surface it on
                // `run_terminal` / `agent_end`. "Cut off mid-work" (turns or
                // tool calls > 0) is worth resuming; "silent immediately" is
                // more likely a prompt/connection problem.
                *self.stream_truncation.lock() = Some(crate::agent::StreamTruncation {
                    turns_so_far: turn as i32,
                    output_len: assistant_text.len(),
                    tool_calls_so_far: agent_tool_calls.len(),
                    detected_by: truncation_detected_by
                        .unwrap_or("eof_no_terminal")
                        .to_string(),
                });
                if self.verbose {
                    tracing::warn!(
                        "[agent] stream truncated turns={} output_len={}",
                        turn + 1,
                        assistant_text.len()
                    );
                }
                append_skipped_tool_placeholders(
                    messages,
                    &agent_tool_calls,
                    "the stream was truncated",
                    ctx.save_callback.as_ref(),
                );
                return Ok(assistant_text);
            }

            stream_retry_attempt = 0;
            retry_text.clear();

            // Check stop condition. The is_some_and closure keeps the None
            // edge branchless (a nested if's closing brace here collected a
            // phantom zero-count coverage region).
            let stop_hit = self.config.stop_condition.as_ref().is_some_and(|stop_fn| {
                let llm_msgs: Vec<Message> = ConvertToLLM(messages);
                stop_fn(llm_msgs, &assistant_text)
            });
            if stop_hit {
                // The stop condition fired while the model had emitted tool
                // calls; they are never executed, so append placeholder
                // results to keep the conversation API-valid for the next run.
                append_skipped_tool_placeholders(
                    messages,
                    &agent_tool_calls,
                    "the stop condition fired",
                    ctx.save_callback.as_ref(),
                );
                return Ok(assistant_text);
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
                return Ok(assistant_text);
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
            self.execute_tools(turn, &tool_calls, messages, &on_event, ctx)
                .await?;

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
}
