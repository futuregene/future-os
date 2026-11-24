//! A standalone (manual) context compaction, detached from the session's lock.
//!
//! `compact` acknowledges and then summarises on its own worker thread — minutes
//! of model time on a long conversation. That worker used to run *inside* a read
//! guard on `ServerSession`, which had two consequences the product could not
//! live with: every mutating command that took the write lock waited behind the
//! summary (a client-side deadline expired while the command sat in the queue),
//! and so the dispatcher had to refuse prompts outright with `session_busy` for
//! the whole window. The window is exactly when a reader, watching a long
//! conversation be summarised, writes their next message.
//!
//! This job therefore owns the handful of handles the summary and its commit
//! need — all of them `Arc`s the live session shares — so the worker can drop the
//! lock for the entire operation. The session stays addressable and its queue
//! keeps accepting prompts; they wait like any other prompt behind a run and
//! start when the fence opens (`ServerSession::wake_scheduler_after_compaction`).
//!
//! What the fence still protects is the *transcript*: the summary is computed
//! over the snapshot this job reads at the start, and the mutating commands that
//! would change it mid-summary (`set_model`, `set_tools`, a reset, a fork) are
//! still refused while `compaction_in_progress` is set.

use std::sync::Arc;

use anyhow::Result;

use crate::rpc::SseBroadcaster;

/// The owned inputs of one manual compaction.
pub(crate) struct StandaloneCompaction {
    session_id: String,
    model: String,
    thinking_level: String,
    cwd: String,
    no_context_files: bool,
    ephemeral: bool,
    /// The session-owned half of the summary request's system prompt.
    identity: super::session_prompt::PromptIdentity,
    model_registry: Arc<parking_lot::RwLock<crate::models::Registry>>,
    messages: Arc<parking_lot::RwLock<Vec<crate::types::AgentMessage>>>,
    active_checkpoint: Arc<parking_lot::Mutex<Option<crate::compaction::ContextCheckpoint>>>,
    checkpoint_loaded: Arc<std::sync::atomic::AtomicBool>,
    last_prompt_tokens: Arc<std::sync::atomic::AtomicI64>,
    agent_loop: Arc<tokio::sync::RwLock<crate::agent::Loop>>,
    session_manager: Arc<crate::session::Manager>,
    persistence: crate::session::SessionPersistence,
    broadcaster: Arc<SseBroadcaster>,
    tokens_in: Arc<std::sync::atomic::AtomicI64>,
    tokens_out: Arc<std::sync::atomic::AtomicI64>,
    tokens_cache_r: Arc<std::sync::atomic::AtomicI64>,
    tokens_cache_w: Arc<std::sync::atomic::AtomicI64>,
    cumulative_cost: Arc<parking_lot::Mutex<f64>>,
    cumulative_cost_split: Arc<parking_lot::Mutex<crate::models::CostSplit>>,
    cost_split_complete: Arc<std::sync::atomic::AtomicBool>,
    compaction_in_progress: Arc<std::sync::atomic::AtomicBool>,
}

impl StandaloneCompaction {
    /// Take everything the summary needs off the session, under a short lock.
    ///
    /// Called with the session read or write guard held; after it returns the
    /// guard can be dropped and the summary runs without it.
    pub(crate) fn capture(session: &super::ServerSession) -> Self {
        Self {
            session_id: session.session_id.clone(),
            model: session.model.clone(),
            thinking_level: session.thinking_level.clone(),
            cwd: session.cwd.clone(),
            no_context_files: session.no_context_files,
            ephemeral: session.ephemeral,
            identity: super::session_prompt::PromptIdentity::of(session),
            model_registry: session.model_registry.clone(),
            messages: session.messages.clone(),
            active_checkpoint: session.active_checkpoint.clone(),
            checkpoint_loaded: session.checkpoint_loaded.clone(),
            last_prompt_tokens: session.last_prompt_tokens.clone(),
            agent_loop: session.agent_loop.clone(),
            session_manager: session.session_manager.clone(),
            persistence: session.persistence.clone(),
            broadcaster: session.broadcaster.clone(),
            tokens_in: session.tokens_in.clone(),
            tokens_out: session.tokens_out.clone(),
            tokens_cache_r: session.tokens_cache_r.clone(),
            tokens_cache_w: session.tokens_cache_w.clone(),
            cumulative_cost: session.cumulative_cost.clone(),
            cumulative_cost_split: session.cumulative_cost_split.clone(),
            cost_split_complete: session.cost_split_complete.clone(),
            compaction_in_progress: session.compaction_in_progress.clone(),
        }
    }

    /// Run the summary. Called from a plain thread (the RPC worker or a test), so
    /// a temporary current-thread runtime drives the async path; the summary
    /// request itself is only issued when the provider is reachable.
    pub(crate) fn run(
        &self,
        instructions: &str,
        operation_id: String,
    ) -> Result<serde_json::Value> {
        let context_window = self
            .model_registry
            .read()
            .resolve(&self.model)
            .map(|m| m.context_window)
            .unwrap_or(1_000_000);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| anyhow::anyhow!("compaction runtime unavailable: {error}"))?;
        runtime.block_on(self.compact_with_policy(
            instructions,
            crate::compaction::CompactionTrigger::Manual,
            crate::compaction::CompactionPhase::Standalone,
            context_window,
            operation_id,
        ))
    }

    fn broadcast_compaction_event(
        &self,
        phase: crate::compaction::CompactionPhase,
        event: crate::agent::RunEvent,
    ) {
        broadcast_compaction_event(&self.broadcaster, phase, event);
    }
}

/// Emit one compaction lifecycle signal on the right scope.
///
/// A standalone (manual) compaction is not part of any run — `compact`
/// refuses a busy session, so it only ever runs with no active run. Between
/// runs the broadcaster still holds the run that just settled, and that run's
/// journal is already retired: run completion hands it to the compactor, which
/// publishes a snapshot and deletes its raw rows. Stamping the signal with
/// that run made the append permanently impossible, so a standalone
/// compaction's signals go to the session scope instead: never compacted,
/// durable, and already the path clients replay for the signals that belong
/// to no run. An in-run compaction keeps its run stamp — its progress belongs
/// to that run's stream — and never comes through here; it goes through
/// `ServerSession::broadcast_compaction_event`.
pub(crate) fn broadcast_compaction_event(
    broadcaster: &SseBroadcaster,
    phase: crate::compaction::CompactionPhase,
    event: crate::agent::RunEvent,
) {
    let Some(event) = super::prompt_helpers::run_event_to_sse(event) else {
        return;
    };
    if phase == crate::compaction::CompactionPhase::Standalone {
        broadcaster.broadcast_session(event);
    } else {
        broadcaster.broadcast(event);
    }
}

impl StandaloneCompaction {
    async fn compact_with_policy(
        &self,
        instructions: &str,
        trigger: crate::compaction::CompactionTrigger,
        phase: crate::compaction::CompactionPhase,
        context_window: i32,
        operation_id: String,
    ) -> Result<serde_json::Value> {
        use std::sync::atomic::Ordering;
        // A standalone manual compaction can arrive immediately after the
        // session was reattached/recovered. Rehydrate from the append-only
        // journal if its in-memory cache has not been populated yet; otherwise
        // `/压缩` would incorrectly report "nothing to compact" for a visible
        // conversation. Keep the restored cache for the next prompt as well.
        let mut messages = self.messages.read().clone();
        let mut loaded_checkpoint = None;
        if !self.ephemeral || messages.is_empty() {
            if let Ok(session) = self.session_manager.load(&self.session_id) {
                let supports_images = self
                    .model_registry
                    .read()
                    .request_model_accepts_images(&self.model);
                messages =
                    crate::session::entries_to_agent_messages(&session.entries, supports_images);
                loaded_checkpoint = crate::session::latest_context_checkpoint(&session.entries);
                *self.active_checkpoint.lock() = loaded_checkpoint.clone();
                self.checkpoint_loaded
                    .store(true, std::sync::atomic::Ordering::Release);
                if !messages.is_empty() {
                    *self.messages.write() = messages.clone();
                }
            }
        }
        let (reserve_tokens, keep_tokens) =
            crate::compaction::context_token_budgets(context_window);
        let active_checkpoint = loaded_checkpoint.or_else(|| self.active_checkpoint.lock().clone());
        let reported = self
            .last_prompt_tokens
            .load(Ordering::Relaxed)
            .try_into()
            .ok()
            .filter(|tokens: &u64| *tokens > 0);
        let mut prompt = crate::compaction::project_prompt_context(
            &messages,
            active_checkpoint.as_ref(),
            reported,
            context_window.max(1) as u64,
        );
        let manager = crate::compaction::ContextManager {
            enabled: true,
            reserve_tokens,
            keep_recent_tokens: keep_tokens,
            context_window,
            model: self.model.clone(),
        };
        let (interrupted, current_model, request_tools, provider) = {
            let loop_ = match self.agent_loop.try_read() {
                Ok(loop_) => loop_,
                Err(_) => {
                    let error = anyhow::anyhow!("session configuration is busy; retry /compact");
                    self.compaction_in_progress.store(false, Ordering::Release);
                    self.broadcast_compaction_event(
                        phase,
                        crate::agent::RunEvent::CompactionFailed {
                            operation_id,
                            trigger,
                            phase,
                            error: error.to_string(),
                        },
                    );
                    return Err(error);
                }
            };
            (
                loop_.interrupt_flag.clone(),
                loop_.model.clone(),
                loop_.tools.clone(),
                loop_.provider.clone(),
            )
        };
        let (budget_system, _) = self.identity.build(
            &self.cwd,
            request_tools.clone(),
            &self.model,
            &self.thinking_level,
            self.no_context_files,
        );
        let tool_defs = request_tools
            .iter()
            .map(|tool| tool.def.clone())
            .collect::<Vec<_>>();
        // The summary request carries the system prompt this session's turns send. Providers
        // cache on the request prefix, so a summary that differs from the turn that already
        // paid for those tokens shares no prefix with it and the whole conversation is billed
        // again. The automatic path does the same thing with the same expression
        // (`run_loop.rs`); keeping them identical is the point. There is no longer a
        // post-checkpoint addition to keep in step, so both simply send the session's prompt.
        let budget_system = budget_system.as_str();
        let summary_system_prompt = budget_system.to_string();
        let max_output = self
            .model_registry
            .read()
            .resolve(&self.model)
            .map(|model| crate::models::effective_max_tokens(&model))
            .unwrap_or(0);
        crate::compaction::set_request_budget(&mut prompt, budget_system, &tool_defs, max_output);
        let journal = (!self.ephemeral).then(|| crate::compaction::CompactionJournal::new(
            self.session_manager.clone(), self.persistence.clone(), self.session_id.clone(),
            serde_json::json!({"model":self.model,"thinking":self.thinking_level,"cwd":self.cwd,"tools":tool_defs,
                "protocol":self.model_registry.read().resolve(&self.model).map(|m| serde_json::json!({"api":m.api,"baseUrl":m.base_url,"compat":m.compat}))}),
        ));
        let projected_messages_before = prompt.messages.len();
        let manager = crate::compaction::ContextManager {
            model: current_model,
            ..manager
        };
        // Charge the session for the summary request. The compaction is synchronous here
        // and the caller only receives an acknowledgement, so the cost is folded into the
        // session's cumulative counters as it is incurred rather than reported per event.
        let summary_usage: Arc<parking_lot::Mutex<Option<crate::types::Usage>>> =
            Arc::new(parking_lot::Mutex::new(None));
        let record_summary_usage = {
            let usage = summary_usage.clone();
            let tokens_in = self.tokens_in.clone();
            let tokens_out = self.tokens_out.clone();
            let cache_r = self.tokens_cache_r.clone();
            let cache_w = self.tokens_cache_w.clone();
            let cumulative_cost = self.cumulative_cost.clone();
            let cumulative_cost_split = self.cumulative_cost_split.clone();
            let registry = self.model_registry.clone();
            let model = self.model.clone();
            move |reported: &crate::types::Usage| {
                // One billed request per attempt, so one record per attempt: the
                // same rule the run loop applies to a model call.
                crate::agent::add_request_usage(
                    &tokens_in,
                    &tokens_out,
                    &cache_r,
                    &cache_w,
                    reported,
                );
                // The observer receives the final usage of EACH attempt, including
                // charged retries and cancelled/failed summary streams.
                let cost = reported.credit_cost.unwrap_or_else(|| {
                    crate::agent::estimate_usage_cost_with(Some(&registry), &model, reported)
                });
                if cost > 0.0 {
                    *cumulative_cost.lock() += cost;
                }
                // Priced with this request's model, like the run loop does.
                let parts = crate::agent::usage_cost_split_with(Some(&registry), &model, reported);
                if parts.iter().any(|amount| *amount > 0.0) {
                    cumulative_cost_split.lock().add(parts);
                }
                *usage.lock() = Some(reported.clone());
            }
        };

        // Summarised compaction: C's projection plus a handoff summary, so a
        // user-initiated compaction gets the same retention as an automatic one. The
        // system prompt is passed through so the summary request reuses the prefix the
        // session already sent and can be served from the provider cache. The prompt passed
        // here is the session's own, which is the base prompt at every point now that the
        // post-checkpoint recall guidance is gone.
        let mut prepared = crate::compaction::prepare_with_journal_and_summary(
            &manager,
            prompt,
            &messages,
            trigger,
            phase,
            Some(instructions),
            interrupted.as_ref(),
            Some(&|| {
                self.broadcast_compaction_event(
                    phase,
                    crate::agent::RunEvent::CompactionStarted {
                        operation_id: operation_id.clone(),
                        trigger,
                        phase,
                    },
                );
            }),
            journal.as_ref(),
            &operation_id,
            Some(provider.as_ref()),
            Some(summary_system_prompt.as_str()),
            &tool_defs,
            Some(&record_summary_usage),
            None,
        )
        .await
        .map_err(anyhow::Error::from);

        // Account for the summary request exactly as a model call is accounted for.
        // The usage is always reported, even when it prices at zero (an unpriced
        // model, or a provider that reports no credit_cost), so the request is never
        // invisible; only the cost is conditional.
        if let Some(usage) = summary_usage.lock().clone() {
            // Journal the summary as the billed request it is, through the same
            // `usage` event a model call produces: the session's per-category
            // split is rebuilt from the journal for a record that has none, and
            // the replay prices only `usage`/`model_changed` rows — a summary
            // charged here but never journaled would leave that rebuilt split
            // short by a whole request (h1-C2).
            self.broadcast_compaction_event(
                phase,
                crate::agent::RunEvent::Model(crate::llm::schema::ModelStreamEvent::Usage(
                    usage.clone(),
                )),
            );
            let cost = usage.credit_cost.unwrap_or_else(|| {
                crate::agent::estimate_usage_cost_with(
                    Some(&self.model_registry),
                    &self.model,
                    &usage,
                )
            });
            tracing::info!(
                cost,
                input = usage.prompt_tokens,
                output = usage.completion_tokens,
                cache_read = usage.cache_read_tokens.unwrap_or(0),
                cache_write = usage.cache_write_tokens.unwrap_or(0),
                "charged a manual compaction summary"
            );
            // Unlike a normal run, manual compaction has no run-end metadata
            // commit. Save all cumulative counters even when preparation failed
            // or was cancelled; a replay with no model call must not charge again.
            if !self.ephemeral {
                let fields = serde_json::json!({
                    "tokens_in": self.tokens_in.load(Ordering::Relaxed),
                    "tokens_out": self.tokens_out.load(Ordering::Relaxed),
                    "tokens_cache_r": self.tokens_cache_r.load(Ordering::Relaxed),
                    "tokens_cache_w": self.tokens_cache_w.load(Ordering::Relaxed),
                    "total_cost": *self.cumulative_cost.lock(),
                    "cost_split": serde_json::to_value(*self.cumulative_cost_split.lock())
                        .unwrap_or(serde_json::Value::Null),
                    // Only true when it covers the whole history; a load
                    // re-prices the journal otherwise.
                    "cost_split_complete": self
                        .cost_split_complete
                        .load(Ordering::Relaxed),
                });
                if let Err(error) = self.persistence.update_info_fields(
                    fields
                        .as_object()
                        .expect("usage fields are an object")
                        .clone(),
                ) {
                    if let Ok((_, Some(ticket))) = &prepared {
                        ticket.fail(&error.to_string());
                    }
                    prepared = Err(error.context("could not persist manual compaction usage"));
                }
            }
        }
        let (prepared, ticket) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.compaction_in_progress.store(false, Ordering::Release);
                self.broadcast_compaction_event(
                    phase,
                    crate::agent::RunEvent::CompactionFailed {
                        operation_id,
                        trigger,
                        phase,
                        error: error.to_string(),
                    },
                );
                return Err(error);
            }
        };
        if let Some(ticket) = &ticket {
            if let Some(mut result) = ticket.cached_result() {
                result["reused"] = serde_json::json!(true);
                if result.get("checkpointId").is_some() {
                    result["alreadyCompacted"] = serde_json::json!(true);
                }
                result["sourceOperationId"] = serde_json::json!(ticket.operation_id);
                return Ok(result);
            }
        }
        match prepared {
            crate::compaction::ContextPreparation::Unchanged { prompt } => {
                let result = serde_json::json!({
                    "alreadyCompacted": active_checkpoint.is_some(),
                    "tokensBefore": prompt.usage.estimated_input_tokens,
                    "tokensAfter": prompt.usage.estimated_input_tokens,
                    "summary": "",
                    "messagesRemoved": 0,
                });
                if let Some(ticket) = &ticket {
                    if let Err(error) = ticket.finish(None, result.clone()) {
                        ticket.fail(&error.to_string());
                        self.compaction_in_progress.store(false, Ordering::Release);
                        self.broadcast_compaction_event(
                            phase,
                            crate::agent::RunEvent::CompactionFailed {
                                operation_id,
                                trigger,
                                phase,
                                error: error.to_string(),
                            },
                        );
                        return Err(error);
                    }
                }
                Ok(result)
            }
            crate::compaction::ContextPreparation::Compacted { checkpoint, prompt } => {
                let summary = checkpoint
                    .summary
                    .iter()
                    .filter_map(|b| match b {
                        crate::types::ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let result = serde_json::json!({"checkpointId":checkpoint.checkpoint_id,"tokensBefore":checkpoint.tokens_before,
                    "tokensAfter":checkpoint.tokens_after,"summary":summary,
                    "algorithmVersion":checkpoint.algorithm_version,"summaryOutcome":checkpoint.summary_outcome,
                    "messagesRemoved":projected_messages_before.saturating_sub(prompt.messages.len()),
                    "tokensSaved":checkpoint.tokens_before.saturating_sub(checkpoint.tokens_after),"protectedEntries":checkpoint.protected_entry_ids.len()});
                let committed = if let Some(ticket) = &ticket {
                    ticket.finish(Some(&checkpoint), result.clone())
                } else {
                    self.persistence
                        .commit_checkpoint(crate::session::checkpoint_to_entry(&checkpoint))
                };
                if let Err(error) = committed {
                    if let Some(ticket) = &ticket {
                        ticket.fail(&error.to_string());
                    }
                    self.compaction_in_progress.store(false, Ordering::Release);
                    self.broadcast_compaction_event(
                        phase,
                        crate::agent::RunEvent::CompactionFailed {
                            operation_id,
                            trigger,
                            phase,
                            error: error.to_string(),
                        },
                    );
                    return Err(error);
                }
                self.last_prompt_tokens.store(0, Ordering::Relaxed);
                if let Err(error) = self
                    .persistence
                    .update_info("last_prompt_tokens", serde_json::json!(0))
                {
                    tracing::warn!(%error, "checkpoint committed but usage-baseline persistence failed");
                }
                *self.active_checkpoint.lock() = Some((*checkpoint).clone());
                if let Ok(loop_) = self.agent_loop.try_write() {
                    *loop_.active_checkpoint.lock() = Some((*checkpoint).clone());
                }
                // Publish the terminal event only after the session admission
                // fence is open again. Desktop enables Send when it observes
                // this event, so this ordering prevents a transient
                // `session_busy` on the very next command.
                self.compaction_in_progress.store(false, Ordering::Release);
                self.broadcast_compaction_event(
                    phase,
                    crate::agent::RunEvent::CompactionCommitted {
                        operation_id,
                        checkpoint: (*checkpoint).clone(),
                    },
                );
                Ok(result)
            }
        }
    }
}
