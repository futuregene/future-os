//! Durable, content-addressed admission shared by manual and automatic compaction.
#[cfg(test)]
mod tests;
use super::*;
use crate::session::{compaction_ops::CompactionClaim, Manager, SessionPersistence};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Clone)]
pub struct CompactionJournal {
    manager: Arc<Manager>,
    persistence: SessionPersistence,
    session: String,
    policy: Value,
}

impl CompactionJournal {
    pub fn new(
        manager: Arc<Manager>,
        persistence: SessionPersistence,
        session: String,
        policy: Value,
    ) -> Self {
        Self {
            manager,
            persistence,
            session,
            policy,
        }
    }
}

pub(crate) struct CompactionTicket {
    journal: CompactionJournal,
    key: String,
    input: String,
    cached: Option<Value>,
    pub operation_id: String,
}

impl CompactionTicket {
    pub fn cached_result(&self) -> Option<Value> {
        self.cached.as_ref().map(|r| r["result"].clone())
    }
    pub fn finish(
        &self,
        checkpoint: Option<&ContextCheckpoint>,
        result: Value,
    ) -> anyhow::Result<()> {
        if self.cached.is_some() {
            return Ok(());
        }
        self.journal.persistence.commit_compaction(
            self.key.clone(),
            self.input.clone(),
            checkpoint.map(crate::session::checkpoint_to_entry),
            json!({"checkpoint":checkpoint,"result":result}),
        )
    }
    pub fn fail(&self, error: &str) {
        if self.cached.is_none() {
            if let Err(write_error) =
                self.journal
                    .manager
                    .fail_compaction(&self.journal.session, &self.key, error)
            {
                tracing::error!(%write_error,"could not persist compaction failure; receipt remains indeterminate");
            }
        }
    }
}

/// Journal admission outcome, shared by the deterministic and summary paths.
enum Admission {
    /// A recorded identical compaction was replayed; nothing left to compute.
    Replayed(ContextPreparation, CompactionTicket),
    /// No usable record: compute now, then commit through the ticket when present.
    Fresh(Option<CompactionTicket>),
}

/// Decide whether this compaction was already recorded, and claim the operation
/// when it was not. `strategy` participates in the idempotency key, so a run that
/// adds a model summary and one that does not are distinct operations over the
/// same history and never replay each other's receipt.
#[allow(clippy::too_many_arguments)]
fn admit(
    manager: &ContextManager,
    prompt: PromptContext,
    raw: &[AgentMessage],
    trigger: CompactionTrigger,
    phase: CompactionPhase,
    instructions: Option<&str>,
    journal: Option<&CompactionJournal>,
    operation_id: &str,
    strategy: &str,
) -> Result<Admission, ContextError> {
    let estimate = prompt
        .messages
        .iter()
        .map(semantic::projected_token_cost)
        .sum::<u64>()
        .saturating_add(prompt.usage.fixed_input_tokens)
        .max(prompt.usage.estimated_input_tokens)
        .max(prompt.usage.input_tokens.unwrap_or(0));
    let threshold = if !manager.enabled && trigger == CompactionTrigger::ModelContextDownshift {
        manager.input_limit(&prompt)
    } else {
        manager.effective_trigger(&prompt)
    };
    let gated = (!manager.enabled && trigger == CompactionTrigger::Automatic)
        || (matches!(
            trigger,
            CompactionTrigger::Automatic | CompactionTrigger::ModelContextDownshift
        ) && estimate < threshold);
    let mut ticket = None;
    if let Some(journal) = journal.filter(|_| !gated && !prompt.messages.is_empty()) {
        journal
            .persistence
            .barrier()
            .map_err(|e| ContextError::PersistenceFailed(e.to_string()))?;
        let policy = json!({"version":"c-idempotency-v1","evidence":semantic::evidence::policy_identity(),
            "summaryStrategy":strategy,
            "sessionPolicy":journal.policy,"model":manager.model,"window":manager.context_window,
            "reserve":manager.reserve_tokens,"recent":manager.keep_recent_tokens,
            "fixedInput":prompt.usage.fixed_input_tokens,"outputReserve":prompt.usage.output_reserve_tokens,
            "trigger":trigger,"phase":phase,"instructions":instructions.unwrap_or("").trim()});
        let claim = journal
            .manager
            .claim_compaction(&journal.session, policy, operation_id)
            .map_err(|e| ContextError::PersistenceFailed(e.to_string()))?;
        match claim {
            CompactionClaim::Cached {
                value,
                operation_id: original,
            } => {
                let prepared = if value["checkpoint"].is_null() {
                    ContextPreparation::Unchanged { prompt }
                } else {
                    let checkpoint: ContextCheckpoint =
                        serde_json::from_value(value["checkpoint"].clone())
                            .map_err(|e| ContextError::PersistenceFailed(e.to_string()))?;
                    if let Some(cutoff) = checkpoint.cutoff_entry_id.as_deref() {
                        if !raw.iter().any(|m| m.journal_entry_id() == Some(cutoff)) {
                            return Err(ContextError::PersistenceFailed(
                                "cached checkpoint does not match the current raw snapshot".into(),
                            ));
                        }
                    }
                    let current = prompt
                        .messages
                        .iter()
                        .find(|item| {
                            item.message
                                .metadata
                                .as_ref()
                                .and_then(|m| m.get(INTERNAL_CHECKPOINT_METADATA_KEY))
                                .and_then(Value::as_bool)
                                == Some(true)
                        })
                        .and_then(|item| item.message.journal_entry_id());
                    // Idempotent replay must not undo a later operation made
                    // with different settings against the same raw history.
                    if current.is_some_and(|id| id != checkpoint.entry_id) {
                        let mut unchanged = prompt;
                        unchanged.usage.input_tokens = None;
                        return Ok(Admission::Replayed(
                            ContextPreparation::Unchanged { prompt: unchanged },
                            CompactionTicket {
                                journal: journal.clone(),
                                key: String::new(),
                                input: String::new(),
                                cached: Some(value),
                                operation_id: original,
                            },
                        ));
                    }
                    let mut replay = project_prompt_context(
                        raw,
                        Some(&checkpoint),
                        None,
                        manager.context_window.max(1) as u64,
                    );
                    replay.usage = ContextUsage {
                        input_tokens: None,
                        estimated_input_tokens: replay
                            .messages
                            .iter()
                            .map(semantic::projected_token_cost)
                            .sum::<u64>()
                            + prompt.usage.fixed_input_tokens,
                        ..prompt.usage
                    };
                    ContextPreparation::Compacted {
                        prompt: replay,
                        checkpoint: Box::new(checkpoint),
                    }
                };
                return Ok(Admission::Replayed(
                    prepared,
                    CompactionTicket {
                        journal: journal.clone(),
                        key: String::new(),
                        input: String::new(),
                        cached: Some(value),
                        operation_id: original,
                    },
                ));
            }
            CompactionClaim::New { key, input } => {
                ticket = Some(CompactionTicket {
                    journal: journal.clone(),
                    key,
                    input,
                    cached: None,
                    operation_id: operation_id.into(),
                })
            }
        }
    }
    Ok(Admission::Fresh(ticket))
}

/// Commit a freshly computed preparation through its ticket, failing the receipt
/// when the computation failed so a retry never replays a bad record.
fn commit(
    result: Result<ContextPreparation, ContextError>,
    ticket: Option<CompactionTicket>,
) -> Result<(ContextPreparation, Option<CompactionTicket>), ContextError> {
    match result {
        Ok(prepared) => Ok((prepared, ticket)),
        Err(error) => {
            if let Some(ticket) = ticket {
                ticket.fail(&error.to_string());
            }
            Err(error)
        }
    }
}

/// C3 admitted durably: C's projection plus a sticky model summary when a provider
/// is available. The strategy in the idempotency key reflects what will actually be
/// computed, so an unavailability downgrade is a distinct operation rather than a
/// silent replay of a different result.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn prepare_with_journal_summarized(
    manager: &ContextManager,
    prompt: PromptContext,
    raw: &[AgentMessage],
    trigger: CompactionTrigger,
    phase: CompactionPhase,
    instructions: Option<&str>,
    interrupted: &std::sync::atomic::AtomicBool,
    on_started: Option<&(dyn Fn() + Sync)>,
    journal: Option<&CompactionJournal>,
    operation_id: &str,
    provider: Option<&dyn crate::types::LLMProvider>,
    system_prompt: Option<&str>,
    tools: &[crate::types::ToolDef],
    on_usage: Option<&(dyn Fn(&crate::types::Usage) + Sync)>,
    on_fallback: Option<&(dyn Fn(&str) + Sync)>,
) -> Result<(ContextPreparation, Option<CompactionTicket>), ContextError> {
    if interrupted.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(ContextError::Cancelled);
    }
    let strategy = if provider.is_some() {
        semantic::evidence::ALGORITHM_STICKY
    } else {
        semantic::evidence::ALGORITHM
    };
    match admit(
        manager,
        prompt.clone(),
        raw,
        trigger,
        phase,
        instructions,
        journal,
        operation_id,
        strategy,
    )? {
        Admission::Replayed(prepared, ticket) => Ok((prepared, Some(ticket))),
        Admission::Fresh(ticket) => commit(
            manager
                .prepare_evidence_with_summary(
                    prompt,
                    raw,
                    trigger,
                    phase,
                    instructions,
                    interrupted,
                    on_started,
                    provider,
                    system_prompt,
                    tools,
                    on_usage,
                    on_fallback,
                )
                .await,
            ticket,
        ),
    }
}
