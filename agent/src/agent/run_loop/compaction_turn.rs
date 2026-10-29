//! Preparing one turn's prompt: the model's context budget, the projected
//! message list, and any automatic checkpoint the turn needs to fit.
//!
//! A turn's prompt is not the transcript. A committed checkpoint replaces the
//! conversation the model sees, while the message vector stays the complete
//! record — so the projection, the budget it is measured against, and the
//! compaction decision that may precede it are one unit, prepared together.

use crate::agent::{Loop, RunEvent, StreamContext};
use crate::compaction::{
    CompactionPhase, CompactionTrigger, ContextCheckpoint, ContextManager, ContextPreparation,
    PromptContext,
};
use crate::types::{AgentMessage, ToolDef};
use anyhow::{anyhow, Result};
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use super::billing::add_request_usage;

/// What the compaction summary request cost.
///
/// The summary is billed to the session but is not the user's turn, so its
/// usage is accumulated here and charged once from the final chunk — the same
/// one-request-one-record rule the run loop follows for a model call.
pub(super) struct SummaryUsage {
    reported: parking_lot::Mutex<Option<crate::types::Usage>>,
    input: Arc<AtomicI64>,
    output: Arc<AtomicI64>,
    cache_read: Arc<AtomicI64>,
    cache_write: Arc<AtomicI64>,
}

impl SummaryUsage {
    fn for_run(run: &Loop) -> Self {
        Self {
            reported: parking_lot::Mutex::new(None),
            input: run.cumulative_input_tokens.clone(),
            output: run.cumulative_output_tokens.clone(),
            cache_read: run.cumulative_cache_read_tokens.clone(),
            cache_write: run.cumulative_cache_write_tokens.clone(),
        }
    }

    /// Called once per attempt with that attempt's final usage (a billed
    /// request each), never per progressive chunk.
    pub(super) fn record(&self, reported: &crate::types::Usage) {
        add_request_usage(
            &self.input,
            &self.output,
            &self.cache_read,
            &self.cache_write,
            reported,
        );
        // Keep only the final chunk: providers send progressive cost.
        *self.reported.lock() = Some(reported.clone());
    }

    /// The summary request's final usage, at most once. `None` when the turn
    /// ran no summary request.
    pub(super) fn take(&self) -> Option<crate::types::Usage> {
        self.reported.lock().take()
    }
}

/// What one turn needs from the run while its prompt is prepared.
pub(super) struct TurnInputs<'a, OnEvent> {
    pub(super) ctx: &'a StreamContext,
    /// The full transcript. Entries without a journal identity get one here.
    pub(super) messages: &'a mut Vec<AgentMessage>,
    pub(super) tool_defs: &'a [ToolDef],
    pub(super) turn: usize,
    /// Set when the previous model step was already rejected for context size:
    /// the checkpoint produced for it must be reused rather than replaced.
    pub(super) provider_limit_checkpoint_id: Option<&'a str>,
    pub(super) on_event: &'a OnEvent,
}

/// The prompt for one turn, plus what the caller still needs from preparing it.
pub(super) struct TurnPreparation {
    pub(super) prompt: PromptContext,
    /// Output tokens reserved out of the window. The provider-limit recovery
    /// path re-applies the same budget, so it has to travel with the prompt.
    pub(super) output_reserve: i32,
    pub(super) summary: SummaryUsage,
}

impl Loop {
    /// Project this turn's prompt, budget it, and compact it if it will not fit.
    ///
    /// Returns a prompt that is within the model's capacity: the caller can send
    /// it without a further size check.
    pub(super) async fn prepare_turn<OnEvent>(
        &self,
        inputs: TurnInputs<'_, OnEvent>,
        context_manager: &mut Option<ContextManager>,
        active_checkpoint: &mut Option<ContextCheckpoint>,
    ) -> Result<TurnPreparation>
    where
        OnEvent: Fn(RunEvent) + Sync,
    {
        let TurnInputs {
            ctx,
            messages,
            tool_defs,
            turn,
            provider_limit_checkpoint_id,
            on_event,
        } = inputs;

        // Model limits are provider authority, not run/session state. A
        // catalog edit can change context limits while a multi-turn agent
        // run is active, so refresh the compaction budget from the same
        // shared Registry before every actual LLM turn. The dynamic LLM
        // client independently resolves protocol, route, headers,
        // modalities and max-output tokens for the request itself.
        let mut output_reserve = 0;
        if let (Some(registry), Some(manager)) = (&self.model_registry, context_manager.as_mut()) {
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
            messages.as_slice(),
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
            tool_defs,
            output_reserve,
        );
        let automatic_phase = if turn == 0 {
            CompactionPhase::PreTurn
        } else {
            CompactionPhase::MidTurn
        };
        // Model selection is a control-plane change only: it must never
        // compact history when the user clicks the selector. A real session
        // enables this preflight guard on its run snapshot, so the first LLM
        // request checks the selected model's actual context window here.
        // `ModelContextDownshift` is threshold-gated but bypasses the user's
        // automatic mid-run compaction preference; no checkpoint is written
        // unless the request truly needs one to fit.
        let automatic_trigger = if turn == 0 && self.preflight_context_check {
            CompactionTrigger::ModelContextDownshift
        } else {
            CompactionTrigger::Automatic
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
        let summary = SummaryUsage::for_run(self);
        let mut compaction_ticket = None;
        let prepared = if let Some(manager) = &*context_manager {
            if provider_limit_checkpoint_id.is_some() {
                // The retry below must use exactly the checkpoint produced
                // for this failed model step. A second automatic checkpoint
                // here could hide a non-context provider failure in a loop.
                Ok(ContextPreparation::Unchanged {
                    prompt: projected.clone(),
                })
            } else {
                let record = |reported: &crate::types::Usage| summary.record(reported);
                crate::compaction::prepare_with_journal_and_summary(
                    manager,
                    projected.clone(),
                    messages.as_slice(),
                    automatic_trigger,
                    automatic_phase,
                    None,
                    self.interrupt_flag.as_ref(),
                    Some(&emit_automatic_started),
                    ctx.compaction_journal.as_ref(),
                    &automatic_operation_id,
                    Some(self.provider.as_ref()),
                    Some(compaction_system_prompt),
                    tool_defs,
                    Some(&record),
                    None,
                )
                .await
                .map(|(prompt, ticket)| {
                    compaction_ticket = ticket;
                    prompt
                })
            }
        } else {
            Ok(ContextPreparation::Unchanged {
                prompt: projected.clone(),
            })
        };
        let prompt = match prepared {
            Ok(ContextPreparation::Unchanged { prompt }) => {
                if let Some(ticket) = &compaction_ticket {
                    ticket.finish(None, serde_json::json!({"alreadyCompacted":active_checkpoint.is_some(),"tokensBefore":prompt.usage.estimated_input_tokens,"tokensAfter":prompt.usage.estimated_input_tokens,"summary":"","messagesRemoved":0}))?;
                }
                prompt
            }
            Ok(ContextPreparation::Compacted { prompt, checkpoint }) => {
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
                *active_checkpoint = Some((*checkpoint).clone());
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
        if let Some(manager) = &*context_manager {
            if prompt
                .usage
                .estimated_input_tokens
                .max(prompt.usage.input_tokens.unwrap_or(0))
                > manager.input_limit(&prompt)
            {
                return Err(anyhow!("context input exceeds model capacity after reserving system/tools and output; reduce input or compact history"));
            }
        }
        Ok(TurnPreparation {
            prompt,
            output_reserve,
            summary,
        })
    }
}
