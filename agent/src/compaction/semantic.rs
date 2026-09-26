pub(super) mod evidence;

use super::{
    estimate_tokens, AgentMessage, CompactionPhase, CompactionTrigger, ContentBlock,
    ContextCheckpoint, ContextError, ContextManager, ContextPreparation, ContextUsage,
    ConvertToLLM, ProjectedMessage, PromptContext,
};
use crate::llm::schema::{FinishReason, ModelRequest, ModelStreamEvent};
use crate::types::LLMProvider;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
// Only the tests queue scripts (the legacy fold, which also used this, is gone).
#[cfg(test)]
use std::collections::VecDeque;
use tokio_stream::StreamExt;

const TOOL_OUTPUT_LIMIT: usize = 2_000;
const REASONING_LIMIT: usize = 2_000;
// Strict truncation belonged to the legacy-A fold; one test still exercises the limits.
#[cfg(test)]
const STRICT_TOOL_OUTPUT_LIMIT: usize = 512;
#[cfg(test)]
const STRICT_REASONING_LIMIT: usize = 512;
const SUMMARY_EVENT_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_millis(100)
} else {
    Duration::from_secs(45)
};
const MAX_TRANSIENT_RETRIES: usize = 2;
// A very large model window must not turn an explicit manual compaction into
// an almost-no-op. OpenCode uses a similarly bounded recent-tail budget. Keep
// smaller configured values intact, but cap the manual tail at a useful size.
const MANUAL_RECENT_TAIL_MAX_TOKENS: u64 = 15_000;

pub(super) const SUMMARY_SYSTEM_PROMPT: &str = r#"You are a context summarization agent. Produce a structured handoff summary so another coding agent can continue the work. Do not continue the conversation or answer its questions. Output only the requested structure, using the conversation's primary language.
Evidence completeness: tool results may be partial excerpts. Describe only what the visible excerpt establishes; omitted content remains unknown. Never infer that the full result contains no relevant data, no errors, or only filler because its middle is omitted. Preserve this qualification and the history entry reference. A successful tool execution is not proof that all requested validation passed."#;

const SUMMARY_TEMPLATE: &str = r#"Output exactly this Markdown structure and keep every section:

## Objective
- [the user's unresolved objective, or (none)]

## Important Details
- [constraints, decisions and why, important facts, or (none)]

## Work State
### Completed
- [finished and verified work, or (none)]

### Active
- [current or partially completed work, or (none)]

### Blocked
- [blockers, failed commands, and unknowns, or (none)]

## Next Move
1. [immediate concrete action, or wait for the user's next instruction]

## Relevant Files
- [exact path and why it matters, or (none)]

Rules:
- Keep every section, even when empty.
- Use terse bullets, not prose paragraphs.
- Preserve exact paths, symbols, commands, error strings, URLs, and identifiers when known.
- Reflect the current state: completed requests belong in Completed; only unresolved work belongs in Objective, Active, and Next Move; remove resolved blockers.
- Do not mention compaction or the summary process."#;

#[derive(Clone, Copy)]
enum SerializationMode {
    Normal,
    /// Only the legacy-A fold asked for strict serialization; one test still checks the
    /// truncation limits it selected.
    #[cfg(test)]
    Strict,
}

struct CompactionPlan {
    removed: Vec<ProjectedMessage>,
    retained: Vec<ProjectedMessage>,
    protected: Vec<ProjectedMessage>,
    target_tokens: u64,
    summary_budget: u64,
    summarized_outputs: usize,
    previous_summary: Option<String>,
    covered_from_entry_id: String,
    cutoff_entry_id: String,
    tokens_before: u64,
    usage: ContextUsage,
    trigger: CompactionTrigger,
    phase: CompactionPhase,
    instructions: Option<String>,
}

enum PlannedPreparation {
    Unchanged(PromptContext),
    Compact(CompactionPlan),
}

#[derive(Debug)]
enum SummaryCallError {
    ContextLimit(String),
    Cancelled,
    Other(String),
}

impl std::fmt::Display for SummaryCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ContextLimit(reason) => {
                write!(f, "summary request exceeded the context limit: {reason}")
            }
            Self::Cancelled => write!(f, "summary request cancelled"),
            Self::Other(reason) => write!(f, "{reason}"),
        }
    }
}

fn plan(
    manager: &ContextManager,
    prompt: PromptContext,
    trigger: CompactionTrigger,
    phase: CompactionPhase,
    custom_instructions: Option<&str>,
    on_started: Option<&(dyn Fn() + Sync)>,
    summary_cap: u64,
) -> Result<PlannedPreparation, ContextError> {
    if prompt.messages.is_empty() {
        return Ok(PlannedPreparation::Unchanged(prompt));
    }
    if !manager.enabled && trigger == CompactionTrigger::Automatic {
        return Ok(PlannedPreparation::Unchanged(prompt));
    }

    let estimated = prompt
        .messages
        .iter()
        .map(projected_token_cost)
        .sum::<u64>()
        .saturating_add(prompt.usage.fixed_input_tokens)
        .max(prompt.usage.estimated_input_tokens);
    let tokens_before = estimated.max(prompt.usage.input_tokens.unwrap_or(0));
    let window = manager.context_window.max(1) as u64;
    let hard_limit = manager.input_limit(&prompt);
    if prompt.usage.fixed_input_tokens >= hard_limit {
        return Err(ContextError::BudgetExceeded(
            "system/tools and output reservation leave no input room".into(),
        ));
    }
    let threshold = if !manager.enabled && trigger == CompactionTrigger::ModelContextDownshift {
        hard_limit
    } else {
        manager.effective_trigger(&prompt)
    };
    // A user-selected manual compaction deliberately bypasses the automatic
    // threshold: `/压缩` is an explicit request to compact history, not a
    // suggestion to wait until the next context-limit guard. It still needs a
    // real journal boundary below; unlike automatic compaction, it may cover
    // the entire committed conversation and retain only the new summary.
    let threshold_gated = matches!(
        trigger,
        CompactionTrigger::Automatic | CompactionTrigger::ModelContextDownshift
    );
    if threshold_gated && tokens_before < threshold {
        return Ok(PlannedPreparation::Unchanged(prompt));
    }
    let costs = prompt
        .messages
        .iter()
        .map(projected_token_cost)
        .collect::<Vec<_>>();
    let compact_all_when_every_turn_fits = trigger == CompactionTrigger::Manual;
    let keep_recent_tokens = if compact_all_when_every_turn_fits {
        (manager.keep_recent_tokens.max(1) as u64).min(MANUAL_RECENT_TAIL_MAX_TOKENS)
    } else {
        manager.keep_recent_tokens.max(1) as u64
    };
    // Once a task is already in progress, retaining an oversized latest tool
    // result can leave the "compacted" prompt above the model limit. In that
    // case it is safer to summarize the complete active turn and continue from
    // the structured handoff. Do not do this before the first model request:
    // a lone, oversized user prompt has not been acted on yet and must not be
    // silently replaced by a lossy summary.
    let allow_full_active_turn = trigger == CompactionTrigger::Manual
        || (phase == CompactionPhase::MidTurn
            && matches!(
                trigger,
                CompactionTrigger::Automatic | CompactionTrigger::ProviderContextLimit
            ));
    let boundary = turn_aware_cut(
        &prompt.messages,
        &costs,
        keep_recent_tokens,
        compact_all_when_every_turn_fits,
    )
    .or_else(|| allow_full_active_turn.then_some(prompt.messages.len()));
    let Some(mut cut) = boundary else {
        // A lone new user input may exceed the economic trigger while still
        // fitting the model. Do not summarize unseen instructions or reject
        // them merely because no older history can be compacted.
        if threshold_gated && tokens_before <= hard_limit {
            return Ok(PlannedPreparation::Unchanged(prompt));
        }
        return Err(ContextError::NoValidBoundary);
    };
    // A later checkpoint must never move coverage behind the checkpoint
    // already represented in this projection (including a v2 -> S2 upgrade).
    if let Some(index) = prompt
        .messages
        .iter()
        .rposition(|item| internal_summary(&item.message).is_some())
    {
        cut = cut.max(index + 1);
    }
    if allow_full_active_turn && costs[cut..].iter().copied().sum::<u64>() > keep_recent_tokens {
        cut = prompt.messages.len();
    }
    if cut == 0
        || cut > prompt.messages.len()
        || (cut == prompt.messages.len() && !allow_full_active_turn)
    {
        return Err(ContextError::NoValidBoundary);
    }

    // A projected prompt may consist solely of the existing internal
    // checkpoint when the user compacts twice without adding another turn.
    // That checkpoint is useful model context but is deliberately excluded
    // from the next summary input. Treat this as a clean no-op instead of
    // emitting started/failed lifecycle events for an empty summary request.
    let has_compactable_content = prompt.messages[..cut]
        .iter()
        .filter(|item| {
            internal_summary(&item.message).is_none() && !super::is_protected(&item.message)
        })
        .any(|item| {
            !serialize_message(&item.message, SerializationMode::Normal)
                .trim()
                .is_empty()
        });
    if !has_compactable_content {
        return Ok(PlannedPreparation::Unchanged(prompt));
    }
    let removed = prompt.messages[..cut].to_vec();
    let retained = prompt.messages[cut..].to_vec();
    let mut seen = std::collections::HashSet::new();
    let mut protected = removed
        .iter()
        .filter_map(|item| super::protected_text(&item.message))
        .filter(|message| seen.insert(message.journal_entry_id().unwrap().to_string()))
        .map(|message| ProjectedMessage {
            source_entry_ids: vec![message.journal_entry_id().unwrap().to_string()],
            message,
        })
        .collect::<Vec<_>>();
    let summary_budget = (window / 8).clamp(1, summary_cap);
    let history_room = hard_limit.saturating_sub(prompt.usage.fixed_input_tokens);
    // 32K is a soft target. Expand up to MAX_EXPANDED_HISTORY for protected originals
    // while preserving headroom; never silently truncate user/assistant text.
    let maximum_target =
        super::budget::MAX_EXPANDED_HISTORY.min(history_room.saturating_mul(3) / 4);
    let base_required = protected
        .iter()
        .filter(|p| p.message.role == "user")
        .chain(retained.iter())
        .map(projected_token_cost)
        .sum::<u64>()
        .saturating_add(summary_budget)
        .saturating_add(64);
    let mut output_room = maximum_target.saturating_sub(base_required);
    let mut summarized_outputs = 0;
    // Preserve all user directives. If originals outgrow the expanded budget,
    // keep the newest assistant texts that fit and explicitly summarize the rest.
    protected.reverse();
    protected.retain(|item| {
        if item.message.role == "user" {
            return true;
        }
        let cost = projected_token_cost(item);
        if cost <= output_room {
            output_room -= cost;
            true
        } else {
            summarized_outputs += 1;
            false
        }
    });
    protected.reverse();
    let required = protected
        .iter()
        .chain(retained.iter())
        .map(projected_token_cost)
        .sum::<u64>()
        .saturating_add(summary_budget)
        .saturating_add(64);
    if required > maximum_target {
        return Err(ContextError::BudgetExceeded(format!("protected text and recent history need at least {required} tokens, but compacted history can use {maximum_target}; split large input material or use a new session")));
    }
    let target_tokens = super::budget::TARGET_HISTORY
        .min(maximum_target)
        .max(required);
    if let Some(on_started) = on_started {
        on_started();
    }
    let covered_from_entry_id = removed
        .iter()
        .flat_map(|message| message.source_entry_ids.iter())
        .find(|id| !id.is_empty())
        .cloned()
        .ok_or(ContextError::NoValidBoundary)?;
    let cutoff_entry_id = removed
        .iter()
        .rev()
        .flat_map(|message| message.source_entry_ids.iter().rev())
        .find(|id| !id.is_empty())
        .cloned()
        .ok_or(ContextError::NoValidBoundary)?;
    let previous_summary = removed
        .iter()
        .find_map(|item| internal_summary(&item.message));

    Ok(PlannedPreparation::Compact(CompactionPlan {
        removed,
        retained,
        protected,
        target_tokens,
        summary_budget,
        summarized_outputs,
        previous_summary,
        covered_from_entry_id,
        cutoff_entry_id,
        tokens_before,
        usage: prompt.usage,
        trigger,
        phase,
        instructions: custom_instructions
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    }))
}

/// Token cost of a bare message, for callers that hold messages rather than
/// projection entries.
pub(super) fn projected_token_cost_message(message: &AgentMessage) -> u64 {
    projected_token_cost(&ProjectedMessage {
        message: message.clone(),
        source_entry_ids: Vec::new(),
    })
}

pub(super) fn projected_token_cost(projected: &ProjectedMessage) -> u64 {
    let body: u64 = ConvertToLLM(std::slice::from_ref(&projected.message))
        .iter()
        .map(|message| estimate_tokens(message).max(0) as u64)
        .sum();
    let images = projected
        .message
        .content
        .iter()
        .filter(|b| matches!(b, ContentBlock::Image { .. }))
        .count() as u64;
    let attachment_images = if images == 0 {
        projected
            .message
            .metadata
            .as_ref()
            .and_then(|m| m.get("attachments"))
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|a| a.get("kind").and_then(serde_json::Value::as_str) == Some("image"))
                    .count() as u64
            })
            .unwrap_or(0)
    } else {
        0
    };
    let reasoning = projected
        .message
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Reasoning { text, .. } => Some(estimate_text_tokens(text)),
            _ => None,
        })
        .sum::<u64>();
    body + reasoning + 6 + (images + attachment_images) * 2_048
}

fn turn_aware_cut(
    messages: &[ProjectedMessage],
    costs: &[u64],
    keep_recent_tokens: u64,
    compact_all_when_every_turn_fits: bool,
) -> Option<usize> {
    let user_starts = messages
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            (item.message.role == "user"
                && internal_summary(&item.message).is_none()
                && !super::is_protected(&item.message))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    if user_starts.is_empty() {
        if compact_all_when_every_turn_fits
            && !messages.is_empty()
            && costs.iter().copied().sum::<u64>() <= keep_recent_tokens
        {
            return Some(messages.len());
        }
        return fallback_cut(messages, costs, keep_recent_tokens);
    }

    let mut retained_tokens = 0_u64;
    let mut retained_start = None;
    for (position, start) in user_starts.iter().copied().enumerate().rev() {
        let end = user_starts
            .get(position + 1)
            .copied()
            .unwrap_or(messages.len());
        let turn_tokens = costs[start..end].iter().copied().sum::<u64>();
        if retained_tokens.saturating_add(turn_tokens) > keep_recent_tokens {
            break;
        }
        retained_tokens = retained_tokens.saturating_add(turn_tokens);
        retained_start = Some(start);
    }

    match retained_start {
        // Manual compaction of a short conversation should summarize the
        // complete committed history. Retaining every turn verbatim would
        // provide no useful compaction; the old fallback forced cut=1 and
        // produced misleading summaries of only the first message.
        Some(start)
            if compact_all_when_every_turn_fits && user_starts.first().copied() == Some(start) =>
        {
            Some(messages.len())
        }
        // A single provider-reported oversized turn may look small to the
        // local estimator (for example, hidden attachment/tool overhead).
        // Preserve progress by finding an atomic boundary inside that turn.
        Some(0) => fallback_cut(messages, costs, keep_recent_tokens),
        Some(start) => Some(start),
        None => fallback_cut(messages, costs, keep_recent_tokens),
    }
}

fn fallback_cut(
    messages: &[ProjectedMessage],
    costs: &[u64],
    keep_recent_tokens: u64,
) -> Option<usize> {
    if messages.len() < 2 {
        return None;
    }
    let mut retained = 0_u64;
    let mut cut = messages.len() - 1;
    for index in (0..messages.len()).rev() {
        let next = retained.saturating_add(costs[index]);
        if next > keep_recent_tokens && index < messages.len() - 1 {
            break;
        }
        retained = next;
        cut = index;
    }
    if cut == 0 {
        cut = 1;
    }
    while cut > 0 && cut < messages.len() && messages[cut].message.role == "tool" {
        cut -= 1;
        if messages[cut].message.role == "assistant" && messages[cut].message.has_tool_calls() {
            break;
        }
    }
    (cut > 0 && cut < messages.len()).then_some(cut)
}

fn summary_prompt(
    previous_summary: Option<&str>,
    conversation: &str,
    instructions: Option<&str>,
) -> String {
    let prior = previous_summary.map(|summary| {
        format!(
            "The <prior-summary> covers everything before this conversation and will be discarded after this update. Carry forward its still-relevant objectives, constraints, user directives, decisions, and workstreams. The newer conversation wins conflicts. Update completed work, resolved blockers, Objective, and Next Move to reflect the current state.\n\n<prior-summary>\n{summary}\n</prior-summary>\n\n"
        )
    }).unwrap_or_default();
    let instructions = instructions
        .map(|value| format!("Additional user instructions for this compaction:\n{value}\n\n"))
        .unwrap_or_default();
    format!(
        "{prior}<conversation>\n{conversation}\n</conversation>\n\n{instructions}{SUMMARY_TEMPLATE}"
    )
}

struct AttemptUsage<'a> {
    latest: Option<crate::types::Usage>,
    observer: Option<&'a (dyn Fn(&crate::types::Usage) + Sync)>,
}
impl Drop for AttemptUsage<'_> {
    fn drop(&mut self) {
        if let (Some(usage), Some(observer)) = (&self.latest, self.observer) {
            observer(usage);
        }
    }
}

async fn summary_interruptible<T>(
    future: impl std::future::Future<Output = T>,
    interrupted: &AtomicBool,
) -> Result<T, SummaryCallError> {
    let cancelled = async {
        while !interrupted.load(Ordering::Relaxed) {
            tokio::time::sleep(if cfg!(test) {
                Duration::from_millis(1)
            } else {
                Duration::from_millis(50)
            })
            .await;
        }
    };
    tokio::select! { result = future => Ok(result), _ = cancelled => Err(SummaryCallError::Cancelled) }
}

/// Send a summary request built from the live conversation as real messages, with
/// the instruction appended last.
///
/// Providers cache on the request prefix. A flattened request shares no prefix with
/// the turns that already paid for those tokens, so it is billed in full every time;
/// measured against the provider, the message-array shape reusing a live prefix hit
/// 99.9% of the cache and cost about 48x less for the same input. The system prompt
/// is supplied by the caller for the same reason: it must match what the agent turn
/// sent or the prefix diverges.
#[allow(clippy::too_many_arguments)]
async fn call_summary_model_with_messages(
    provider: &dyn LLMProvider,
    model: &str,
    system_prompt: &str,
    mut messages: Vec<AgentMessage>,
    instruction: String,
    tools: Vec<crate::types::ToolDef>,
    interrupted: &AtomicBool,
    max_output_tokens: i32,
    on_usage: Option<&(dyn Fn(&crate::types::Usage) + Sync)>,
) -> Result<String, SummaryCallError> {
    messages.push(AgentMessage::new_user(
        "user",
        serde_json::json!([{ "type": "text", "text": instruction }]),
    ));
    let system_prompt = system_prompt.to_string();
    call_summary_request(
        provider,
        move || ModelRequest {
            // The same tools as the agent turn: they are part of the cached prefix,
            // and omitting them is what stopped the summary from being cache-served.
            model: model.to_string(),
            system_prompt: system_prompt.clone(),
            messages: messages.clone(),
            tools: tools.clone(),
        },
        interrupted,
        max_output_tokens,
        on_usage,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn call_summary_request(
    provider: &dyn LLMProvider,
    make_request: impl Fn() -> ModelRequest,
    interrupted: &AtomicBool,
    max_output_tokens: i32,
    on_usage: Option<&(dyn Fn(&crate::types::Usage) + Sync)>,
) -> Result<String, SummaryCallError> {
    let mut attempt = 0_usize;
    'attempts: loop {
        if interrupted.load(Ordering::Relaxed) {
            return Err(SummaryCallError::Cancelled);
        }
        let mut attempt_usage = AttemptUsage {
            latest: None,
            observer: on_usage,
        };
        let request = make_request();
        let stream = match summary_interruptible(
            tokio::time::timeout(
                SUMMARY_EVENT_TIMEOUT,
                provider.stream_model_with_output_limit(request, max_output_tokens),
            ),
            interrupted,
        )
        .await?
        {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("summary request timed out")),
        };
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                let message = error.to_string();
                if is_context_limit(&message) {
                    return Err(SummaryCallError::ContextLimit(message));
                }
                if attempt < MAX_TRANSIENT_RETRIES && is_retryable(&message) {
                    attempt += 1;
                    tracing::warn!(attempt, error = %message, "retrying context compaction request");
                    await_or_interrupt(tokio::time::sleep(retry_delay(attempt)), interrupted)
                        .await?;
                    continue;
                }
                return Err(SummaryCallError::Other(message));
            }
        };
        let mut text = String::new();
        let mut complete = false;
        loop {
            if interrupted.load(Ordering::Relaxed) {
                return Err(SummaryCallError::Cancelled);
            }
            let event = match tokio::time::timeout(
                SUMMARY_EVENT_TIMEOUT,
                await_or_interrupt(stream.next(), interrupted),
            )
            .await
            {
                Ok(Ok(event)) => event,
                Ok(Err(error)) => return Err(error),
                Err(_) if attempt < MAX_TRANSIENT_RETRIES => {
                    attempt += 1;
                    tracing::warn!(attempt, "retrying timed-out context compaction stream");
                    await_or_interrupt(tokio::time::sleep(retry_delay(attempt)), interrupted)
                        .await?;
                    continue 'attempts;
                }
                Err(_) => {
                    return Err(SummaryCallError::Other(
                        "summary stream timed out".to_string(),
                    ));
                }
            };
            let Some(event) = event else { break };
            match event {
                ModelStreamEvent::TextDelta { text: delta, .. } => text.push_str(&delta),
                ModelStreamEvent::Usage(usage) => attempt_usage.latest = Some(usage),
                ModelStreamEvent::Finish { reason, usage } => {
                    if usage.is_some() {
                        attempt_usage.latest = usage;
                    }
                    match reason {
                        FinishReason::Stop => {
                            complete = true;
                        }
                        FinishReason::Length => {
                            return Err(SummaryCallError::ContextLimit(
                                "summary output reached the provider length limit".to_string(),
                            ));
                        }
                        FinishReason::Cancelled => return Err(SummaryCallError::Cancelled),
                        other => {
                            return Err(SummaryCallError::Other(format!(
                                "summary stream finished with {}",
                                other.as_str()
                            )));
                        }
                    }
                }
                ModelStreamEvent::Error { .. } if complete => break,
                ModelStreamEvent::Error { message } => {
                    if is_context_limit(&message) {
                        return Err(SummaryCallError::ContextLimit(message));
                    }
                    if attempt < MAX_TRANSIENT_RETRIES && is_retryable(&message) {
                        attempt += 1;
                        tracing::warn!(attempt, error = %message, "retrying failed context compaction stream");
                        await_or_interrupt(tokio::time::sleep(retry_delay(attempt)), interrupted)
                            .await?;
                        continue 'attempts;
                    }
                    return Err(SummaryCallError::Other(message));
                }
                ModelStreamEvent::ToolInputStart { .. }
                | ModelStreamEvent::ToolInputDelta { .. }
                | ModelStreamEvent::ToolInputEnd { .. } => {
                    return Err(SummaryCallError::Other(
                        "summary model attempted a tool call".to_string(),
                    ));
                }
                _ => {}
            }
        }
        if complete && !text.trim().is_empty() {
            return Ok(text.trim().to_string());
        }
        let message = "summary stream ended before a complete response".to_string();
        if attempt < MAX_TRANSIENT_RETRIES {
            attempt += 1;
            await_or_interrupt(tokio::time::sleep(retry_delay(attempt)), interrupted).await?;
            continue;
        }
        return Err(SummaryCallError::Other(message));
    }
}

/// Await provider work while polling the run's shared cancellation flag. This
/// covers connection setup, a silent stream, and retry backoff, so compaction
/// cannot outlive the runtime's 30-second cancellation acknowledgement window.
async fn await_or_interrupt<F, T>(
    future: F,
    interrupted: &AtomicBool,
) -> Result<T, SummaryCallError>
where
    F: Future<Output = T>,
{
    tokio::pin!(future);
    loop {
        if interrupted.load(Ordering::Relaxed) {
            return Err(SummaryCallError::Cancelled);
        }
        tokio::select! {
            output = &mut future => return Ok(output),
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
}

fn retry_delay(attempt: usize) -> Duration {
    if cfg!(test) {
        Duration::from_millis(1)
    } else {
        Duration::from_millis(250_u64.saturating_mul(1_u64 << attempt.saturating_sub(1)))
    }
}

fn is_context_limit(message: &str) -> bool {
    let value = message.to_ascii_lowercase();
    [
        "context length",
        "context window",
        "maximum context",
        "too many tokens",
        "request too large",
        "body too large",
        "payload too large",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn is_retryable(message: &str) -> bool {
    let value = message.to_ascii_lowercase();
    [
        "timeout",
        "timed out",
        "connection",
        "overload",
        "rate limit",
        "too many requests",
        "status 429",
        "status 500",
        "status 502",
        "status 503",
        "status 504",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn serialize_message(message: &AgentMessage, mode: SerializationMode) -> String {
    let label = match message.role.as_str() {
        "user" => "User",
        "assistant" => "Assistant",
        "tool" => "Tool",
        "system" => "System update",
        other => other,
    };
    let mut lines = Vec::new();
    for block in &message.content {
        match block {
            ContentBlock::Text { text } if !text.trim().is_empty() => {
                lines.push(format!("[{label}]: {text}"));
            }
            ContentBlock::Reasoning { text, .. } if !text.trim().is_empty() => {
                let limit = match mode {
                    SerializationMode::Normal => REASONING_LIMIT,
                    #[cfg(test)]
                    SerializationMode::Strict => STRICT_REASONING_LIMIT,
                };
                lines.push(format!("[Assistant reasoning]: {}", truncate(text, limit)));
            }
            ContentBlock::Image { image_url } => {
                let description = image_url
                    .url
                    .as_deref()
                    .and_then(|url| (!url.starts_with("data:")).then_some(url))
                    .unwrap_or("embedded image");
                lines.push(format!("[Attached image: {description}]"));
            }
            ContentBlock::ToolCall { id, name, args, .. } => {
                lines.push(format!("[Assistant tool call {id}]: {name}({args})"));
            }
            ContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => {
                let limit = match mode {
                    SerializationMode::Normal => TOOL_OUTPUT_LIMIT,
                    #[cfg(test)]
                    SerializationMode::Strict => STRICT_TOOL_OUTPUT_LIMIT,
                };
                let kind = if *is_error { "error" } else { "result" };
                lines.push(format!(
                    "[Tool {kind} {tool_call_id}]: {}",
                    tool_excerpt(content, limit)
                ));
            }
            _ => {}
        }
    }
    let text = lines.join("\n");
    if text.is_empty() {
        return text;
    }
    message.journal_entry_id().map_or_else(
        || text.clone(),
        |id| format!("[History entry {id}]\n{text}"),
    )
}

fn tool_excerpt(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let head = value.chars().take(max_chars / 2).collect::<String>();
    let tail = value
        .chars()
        .rev()
        .take(max_chars - max_chars / 2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{head}\n[truncated for compaction] Middle omitted; only head and tail are shown. Missing content is unknown, not evidence of absence. Query the history entry for the full stored output.\n{tail}")
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let truncated = value.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n[truncated for compaction]")
}

fn internal_summary(message: &AgentMessage) -> Option<String> {
    let explicitly_internal = message
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(super::INTERNAL_CHECKPOINT_METADATA_KEY))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if message.role != "user" || !explicitly_internal {
        return None;
    }
    message.content.iter().find_map(|block| match block {
        ContentBlock::Text { text } => text
            .strip_prefix("[Context compaction:")
            .and_then(|value| value.strip_suffix(']'))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        _ => None,
    })
}

fn estimate_text_tokens(value: &str) -> u64 {
    super::estimate_text_tokens(value) as u64
}

#[cfg(test)]
#[test]
fn text_estimators_agree_on_unicode_costs() {
    // The legacy fold used `estimate_text_tokens`; the planner and the evidence builder use
    // the shared `mod::estimate_text_tokens`. Both must classify the same characters the
    // same way, or a budget would mean two different things in one projection.
    for text in ["ПриветПривет", "🙂🙂🙂🙂", "漢字漢字", "hello世界"] {
        assert_eq!(
            estimate_text_tokens(text),
            super::estimate_text_tokens(text) as u64
        );
    }
}

fn retention_note(count: usize, algorithm: &str) -> String {
    let action = if algorithm == evidence::ALGORITHM_DETERMINISTIC {
        "omitted from the active context (not summarized)"
    } else {
        "summarized"
    };
    format!("\n\n[Retention note: {count} older assistant outputs were {action} to fit. Query original history for their exact text.]")
}

fn finalize(
    manager: &ContextManager,
    plan: &CompactionPlan,
    mut summary: String,
    algorithm_version: &str,
    summary_model: &str,
) -> Result<ContextPreparation, ContextError> {
    if summary.trim().is_empty() {
        return Err(ContextError::InvalidSummary);
    }
    if plan.summarized_outputs > 0 {
        summary.push_str(&retention_note(plan.summarized_outputs, algorithm_version));
    }
    let entry_id = crate::utils::generate_entry_id();
    let checkpoint_id = format!("cp_{entry_id}");
    let mut summary_message = AgentMessage::new_user(
        "user",
        serde_json::json!([{
            "type": "text",
            "text": format!("[Context compaction: {summary}]")
        }]),
    );
    super::stamp_internal_checkpoint_message(&mut summary_message, &entry_id);
    let protected_entry_ids = plan
        .protected
        .iter()
        .flat_map(|item| item.source_entry_ids.iter().cloned())
        .collect::<Vec<_>>();
    let mut compacted_messages = plan.protected.clone();
    compacted_messages.push(ProjectedMessage {
        message: summary_message,
        source_entry_ids: vec![entry_id.clone()],
    });
    compacted_messages.extend(plan.retained.iter().cloned());
    let history_after = compacted_messages
        .iter()
        .map(projected_token_cost)
        .sum::<u64>();
    let tokens_after = history_after.saturating_add(plan.usage.fixed_input_tokens);
    if history_after > plan.target_tokens
        || tokens_after > manager.input_limit_for_usage(&plan.usage)
    {
        return Err(ContextError::BudgetExceeded(format!("result is {history_after} history tokens ({tokens_after} with overhead), exceeding its admitted budget {}", plan.target_tokens)));
    }
    if plan.trigger != CompactionTrigger::Manual && tokens_after >= plan.tokens_before {
        return Err(ContextError::NoProgress);
    }
    let window = manager.context_window.max(1) as u64;
    let checkpoint = ContextCheckpoint {
        entry_id,
        checkpoint_id,
        covered_from_entry_id: Some(plan.covered_from_entry_id.clone()),
        cutoff_entry_id: Some(plan.cutoff_entry_id.clone()),
        summary: vec![ContentBlock::text(summary)],
        protected_entry_ids,
        tokens_before: plan.tokens_before,
        tokens_after,
        trigger: plan.trigger,
        phase: Some(plan.phase),
        algorithm_version: algorithm_version.to_string(),
        summary_outcome: None,
        model: summary_model.to_string(),
        context_window: window,
        created_at: chrono::Utc::now(),
    };
    tracing::info!(
        trigger = ?checkpoint.trigger,
        phase = ?checkpoint.phase,
        algorithm_version = checkpoint.algorithm_version,
        tokens_before = checkpoint.tokens_before,
        tokens_after = checkpoint.tokens_after,
        "prepared context compaction checkpoint"
    );
    Ok(ContextPreparation::Compacted {
        prompt: PromptContext {
            messages: compacted_messages,
            usage: ContextUsage {
                input_tokens: None,
                estimated_input_tokens: tokens_after,
                context_window: window,
                ..plan.usage.clone()
            },
        },
        checkpoint: Box::new(checkpoint),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    const VALID_SUMMARY: &str = "## Objective\n- ship it\n\n## Important Details\n- preserve data\n\n## Work State\n### Completed\n- audit\n\n### Active\n- implementation\n\n### Blocked\n- (none)\n\n## Next Move\n1. test\n\n## Relevant Files\n- agent/src/compaction/semantic.rs";

    /// The raw journal a projection was built from. These tests used to reach the legacy
    /// semantic entry points, which took only the projection; `prepare_evidence` takes the
    /// raw messages as well, and for a projection with no checkpoint that is exactly the
    /// projection's own messages.
    fn raw_of(prompt: &PromptContext) -> Vec<AgentMessage> {
        prompt
            .messages
            .iter()
            .map(|item| item.message.clone())
            .collect()
    }

    /// The phase `prepare_semantic` used to derive from the trigger.
    fn phase_of(trigger: CompactionTrigger) -> CompactionPhase {
        super::super::default_phase(trigger)
    }

    fn projected_message(mut message: AgentMessage, id: &str) -> ProjectedMessage {
        message
            .metadata
            .get_or_insert_with(serde_json::Map::new)
            .insert(
                AgentMessage::JOURNAL_ENTRY_ID_KEY.to_string(),
                serde_json::Value::String(id.to_string()),
            );
        ProjectedMessage {
            message,
            source_entry_ids: vec![id.to_string()],
        }
    }

    fn projected(role: &str, text: &str, id: &str) -> ProjectedMessage {
        projected_message(
            AgentMessage::new_user(role, serde_json::json!([{ "type": "text", "text": text }])),
            id,
        )
    }

    /// A length of ASCII text that alone exceeds `MAX_EXPANDED_HISTORY` (4 chars per
    /// estimated token). Sized from the constant so the two tests below keep testing the
    /// mechanism — the plan refuses rather than truncating a user original; an assistant
    /// original that does not fit is demoted and counted — instead of a ceiling value they
    /// happen to hard-code.
    fn oversized_original_len() -> usize {
        (super::super::budget::MAX_EXPANDED_HISTORY as usize) * 4 + 8_192
    }

    fn test_prompt() -> PromptContext {
        PromptContext {
            messages: vec![
                projected("user", "old goal", "e1"),
                projected("assistant", "old result", "e2"),
                projected("user", "recent instruction", "e3"),
            ],
            usage: ContextUsage {
                input_tokens: Some(10_000),
                estimated_input_tokens: 10_000,
                context_window: 8_000,
                ..Default::default()
            },
        }
    }

    fn test_manager() -> ContextManager {
        ContextManager {
            enabled: true,
            reserve_tokens: 1_000,
            keep_recent_tokens: 10,
            context_window: 8_000,
            model: "old-model".to_string(),
        }
    }

    #[tokio::test]
    async fn s2_counts_request_overhead_and_preserves_originals_across_checkpoints() {
        // The window is chosen so its 80% trigger (256 000) sits just above the history
        // alone (~247 K): what pushes this compaction over the line is the counted
        // request overhead, which is exactly what the test is about. The history is not
        // scaled up instead, because a 1M-token window would need ~3 MB of filler to
        // cross its trigger and this test does not measure anything size-dependent.
        let window = 320_000;
        let (reserve_tokens, keep_recent_tokens) = super::super::context_token_budgets(window);
        let manager = ContextManager {
            enabled: true,
            reserve_tokens,
            keep_recent_tokens,
            context_window: window,
            model: "m".into(),
        };
        let mut original = vec![
            projected("user", "first exact requirement", "u1").message,
            projected("assistant", "first exact answer", "a1").message,
            projected_message(
                AgentMessage {
                    role: "tool".into(),
                    content: vec![ContentBlock::tool_result("c1", "x".repeat(990_000), false)],
                    ..Default::default()
                },
                "t1",
            )
            .message,
            projected("user", "new requirement", "u2").message,
        ];
        let before = super::super::project_prompt_context(&original, None, None, 1_000_000);
        assert!(matches!(
            manager
                .prepare_evidence(
                    before.clone(),
                    &raw_of(&before),
                    CompactionTrigger::Automatic,
                    phase_of(CompactionTrigger::Automatic),
                    None,
                    &AtomicBool::new(false),
                    None,
                )
                .unwrap(),
            ContextPreparation::Unchanged { .. }
        ));
        let mut prompt = super::super::project_prompt_context(&original, None, None, 1_000_000);
        super::super::set_request_budget(&mut prompt, &"s".repeat(40_000), &[], 16_000);
        let (first, checkpoint) = into_compacted(
            manager
                .prepare_evidence(
                    prompt.clone(),
                    &raw_of(&prompt),
                    CompactionTrigger::Automatic,
                    phase_of(CompactionTrigger::Automatic),
                    None,
                    &AtomicBool::new(false),
                    None,
                )
                .unwrap(),
        )
        .unwrap();
        assert!(checkpoint.tokens_after < 32_000 + 10_100);
        assert_eq!(checkpoint.protected_entry_ids, vec!["u1", "a1"]);
        assert_eq!(first.messages[0].message.text(), "first exact requirement");
        assert_eq!(first.messages[1].message.text(), "first exact answer");
        assert_eq!(
            checkpoint.algorithm_version,
            evidence::ALGORITHM_DETERMINISTIC
        );
        let replay =
            super::super::project_prompt_context(&original, Some(&checkpoint), None, 1_000_000);
        assert_eq!(
            replay
                .messages
                .iter()
                .map(|m| m.message.text())
                .collect::<Vec<_>>(),
            first
                .messages
                .iter()
                .map(|m| m.message.text())
                .collect::<Vec<_>>()
        );
        original.push(projected("assistant", "new exact answer", "a2").message);
        original.push(
            projected_message(
                AgentMessage {
                    role: "tool".into(),
                    content: vec![ContentBlock::tool_result(
                        "c2",
                        "tail".repeat(10_000),
                        false,
                    )],
                    ..Default::default()
                },
                "t2",
            )
            .message,
        );
        let second =
            super::super::project_prompt_context(&original, Some(&checkpoint), None, 1_000_000);
        let (second, cp2) = into_compacted(
            manager
                .prepare_evidence(
                    second.clone(),
                    &raw_of(&second),
                    CompactionTrigger::Manual,
                    phase_of(CompactionTrigger::Manual),
                    None,
                    &AtomicBool::new(false),
                    None,
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(cp2.protected_entry_ids, vec!["u1", "a1", "u2", "a2"]);
        for text in [
            "first exact requirement",
            "first exact answer",
            "new requirement",
            "new exact answer",
        ] {
            assert!(second.messages.iter().any(|m| m.message.text() == text));
        }
        assert!(
            matches!(&original[2].content[0], ContentBlock::ToolResult { content, .. } if content.len() == 990_000)
        );
    }

    #[tokio::test]
    async fn s2_rejects_oversized_protected_text_before_calling_model() {
        let (reserve_tokens, keep_recent_tokens) = super::super::context_token_budgets(1_000_000);
        let manager = ContextManager {
            enabled: true,
            reserve_tokens,
            keep_recent_tokens,
            context_window: 1_000_000,
            model: "m".into(),
        };
        let prompt = PromptContext {
            messages: vec![
                projected("user", &"x".repeat(oversized_original_len()), "u"),
                projected("assistant", "answer", "a"),
            ],
            usage: ContextUsage::default(),
        };
        let result = manager.prepare_evidence(
            prompt.clone(),
            &raw_of(&prompt),
            CompactionTrigger::Manual,
            phase_of(CompactionTrigger::Manual),
            None,
            &AtomicBool::new(false),
            None,
        );
        assert!(matches!(result, Err(ContextError::BudgetExceeded(_))));
    }

    #[tokio::test]
    async fn s2_summarizes_oversized_assistant_output_with_an_explicit_notice() {
        let (reserve_tokens, keep_recent_tokens) = super::super::context_token_budgets(1_000_000);
        let manager = ContextManager {
            enabled: true,
            reserve_tokens,
            keep_recent_tokens,
            context_window: 1_000_000,
            model: "m".into(),
        };
        let prompt = PromptContext {
            messages: vec![
                projected("user", "must retain this requirement", "u"),
                projected("assistant", &"x".repeat(oversized_original_len()), "a"),
                projected("user", "latest question", "new"),
            ],
            usage: ContextUsage::default(),
        };
        let (prompt, checkpoint) = into_compacted(
            manager
                .prepare_evidence(
                    prompt.clone(),
                    &raw_of(&prompt),
                    CompactionTrigger::Manual,
                    phase_of(CompactionTrigger::Manual),
                    None,
                    &AtomicBool::new(false),
                    None,
                )
                .unwrap(),
        )
        .unwrap();
        assert!(checkpoint.protected_entry_ids.contains(&"u".to_string()));
        assert!(!checkpoint.protected_entry_ids.contains(&"a".to_string()));
        assert!(prompt
            .messages
            .iter()
            .any(|m| m.message.text() == "must retain this requirement"));
        assert!(checkpoint
            .summary
            .iter()
            .any(|b| matches!(b,ContentBlock::Text{text} if text.contains("Retention note: 1"))));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn summary_connect_wait_is_interruptible() {
        struct HangingProvider;
        #[async_trait::async_trait]
        impl LLMProvider for HangingProvider {
            async fn stream_model(
                &self,
                _: ModelRequest,
            ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
                std::future::pending().await
            }
        }
        let interrupted = std::sync::Arc::new(AtomicBool::new(false));
        let flag = interrupted.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1)).await;
            flag.store(true, Ordering::Relaxed);
        });
        let result = call_summary_model_with_messages(
            &HangingProvider,
            "m",
            "",
            Vec::new(),
            "prompt".into(),
            Vec::new(),
            &interrupted,
            8192,
            None,
        )
        .await;
        assert!(matches!(result, Err(SummaryCallError::Cancelled)));
        // The caller sees the cancellation as its own message, never as the
        // provider's text: a cancelled summary must not be retried or reported
        // as a provider failure.
        assert_eq!(
            result.unwrap_err().to_string(),
            "summary request cancelled",
            "the Cancelled arm owns its message"
        );
    }

    #[tokio::test]
    async fn summary_usage_observes_final_trailing_usage_once_per_attempt() {
        let usage = |input, cost| crate::types::Usage {
            prompt_tokens: input,
            completion_tokens: 2,
            credit_cost: Some(cost),
            ..Default::default()
        };
        let provider = ScriptStreamProvider::new([
            StreamScript::Events(vec![
                ModelStreamEvent::Usage(usage(10, 0.1)),
                ModelStreamEvent::Error {
                    message: "status 503 overload".into(),
                },
            ]),
            StreamScript::Events(vec![
                ModelStreamEvent::TextDelta {
                    id: "s".into(),
                    text: VALID_SUMMARY.into(),
                },
                ModelStreamEvent::Usage(usage(20, 0.15)),
                ModelStreamEvent::Finish {
                    reason: FinishReason::Stop,
                    usage: None,
                },
                ModelStreamEvent::Usage(usage(25, 0.2)),
            ]),
        ]);
        let observed = Mutex::new(Vec::new());
        let summary = call_summary_model_with_messages(
            &provider,
            "m",
            "",
            Vec::new(),
            "prompt".into(),
            Vec::new(),
            &AtomicBool::new(false),
            8192,
            Some(&|u| observed.lock().push(u.clone())),
        )
        .await
        .unwrap();
        assert_eq!(summary, VALID_SUMMARY);
        let observed = observed.lock();
        assert_eq!(observed.len(), 2);
        assert_eq!(observed[0].prompt_tokens, 10);
        assert_eq!(observed[1].prompt_tokens, 25);
        assert_eq!(observed[1].credit_cost, Some(0.2));
    }

    #[test]
    fn tool_excerpt_keeps_error_tail_and_source_id() {
        let text = format!("HEAD{}TAIL_ERROR", "x".repeat(5000));
        let message = projected_message(
            AgentMessage {
                role: "tool".into(),
                content: vec![ContentBlock::tool_result("c", text, false)],
                ..Default::default()
            },
            "entry-source",
        );
        let serialized = serialize_message(&message.message, SerializationMode::Normal);
        assert!(serialized.contains("HEAD"));
        assert!(serialized.contains("TAIL_ERROR"));
        assert!(serialized.contains("History entry entry-source"));
        assert!(serialized.contains("not evidence of absence"));
        assert!(SUMMARY_SYSTEM_PROMPT.contains("omitted content remains unknown"));
    }

    #[test]
    fn summary_prompt_distinguishes_completed_requests_from_active_objectives() {
        let prompt = summary_prompt(
            None,
            "[User]: explain the result\n[Assistant]: explained",
            None,
        );

        assert!(prompt.contains("unresolved objective"));
        assert!(prompt.contains("completed requests belong in Completed"));
        assert!(prompt.contains("remove resolved blockers"));
    }

    #[test]
    fn summary_serialization_truncates_tool_output_without_mutating_message() {
        let content = "x".repeat(3_000);
        let message = AgentMessage {
            role: "tool".to_string(),
            content: vec![ContentBlock::tool_result("call", content.clone(), false)],
            ..Default::default()
        };
        let serialized = serialize_message(&message, SerializationMode::Normal);
        assert!(serialized.contains("[truncated for compaction]"));
        // The original message is untouched by the compaction serialization.
        assert_eq!(message.text(), content);
    }

    #[tokio::test]
    async fn mid_turn_compaction_summarizes_an_oversized_active_tool_tail() {
        let prompt = PromptContext {
            messages: vec![
                projected("user", "audit the repository", "e1"),
                projected_message(
                    AgentMessage {
                        role: "assistant".to_string(),
                        content: vec![ContentBlock::tool_call(
                            "call-1",
                            "read",
                            serde_json::json!({"path": "large.rs"}),
                            Default::default(),
                        )],
                        ..Default::default()
                    },
                    "e2",
                ),
                projected_message(
                    AgentMessage {
                        role: "tool".to_string(),
                        content: vec![ContentBlock::tool_result(
                            "call-1",
                            "x".repeat(20_000),
                            false,
                        )],
                        ..Default::default()
                    },
                    "e3",
                ),
            ],
            usage: ContextUsage {
                input_tokens: Some(10_000),
                estimated_input_tokens: 10_000,
                context_window: 8_000,
                ..Default::default()
            },
        };

        let (prompt, checkpoint) = into_compacted(
            test_manager()
                .prepare_evidence(
                    prompt.clone(),
                    &raw_of(&prompt),
                    CompactionTrigger::Automatic,
                    CompactionPhase::MidTurn,
                    None,
                    &AtomicBool::new(false),
                    None,
                )
                .unwrap(),
        )
        .expect("expected full active-turn compaction");

        assert_eq!(checkpoint.covered_from_entry_id.as_deref(), Some("e1"));
        assert_eq!(checkpoint.cutoff_entry_id.as_deref(), Some("e3"));
        assert_eq!(checkpoint.phase, Some(CompactionPhase::MidTurn));
        assert_eq!(
            prompt.messages.len(),
            2,
            "the user original and structured summary continue the task"
        );
        assert!(checkpoint.tokens_after < checkpoint.tokens_before);
    }

    #[tokio::test]
    async fn pre_turn_compaction_does_not_replace_an_unseen_user_prompt() {
        let prompt = PromptContext {
            messages: vec![projected("user", &"x".repeat(40_000), "e1")],
            usage: ContextUsage {
                input_tokens: Some(10_000),
                estimated_input_tokens: 10_000,
                context_window: 8_000,
                ..Default::default()
            },
        };

        let result = test_manager().prepare_evidence(
            prompt.clone(),
            &raw_of(&prompt),
            CompactionTrigger::Automatic,
            CompactionPhase::PreTurn,
            None,
            &AtomicBool::new(false),
            None,
        );

        assert_eq!(result.unwrap_err(), ContextError::NoValidBoundary);
    }

    #[tokio::test]
    async fn downshift_does_not_create_checkpoint_when_new_window_fits() {
        let mut prompt = test_prompt();
        prompt.usage.input_tokens = Some(100);
        prompt.usage.estimated_input_tokens = 100;
        let prepared = test_manager()
            .prepare_evidence(
                prompt.clone(),
                &raw_of(&prompt),
                CompactionTrigger::ModelContextDownshift,
                CompactionPhase::PreTurn,
                None,
                &AtomicBool::new(false),
                None,
            )
            .unwrap();
        assert!(matches!(prepared, ContextPreparation::Unchanged { .. }));
    }

    #[tokio::test]
    async fn manual_compaction_bypasses_the_automatic_threshold() {
        let mut prompt = test_prompt();
        prompt.usage.input_tokens = Some(100);
        prompt.usage.estimated_input_tokens = 100;
        let prepared = test_manager()
            .prepare_evidence(
                prompt.clone(),
                &raw_of(&prompt),
                CompactionTrigger::Manual,
                CompactionPhase::Standalone,
                None,
                &AtomicBool::new(false),
                None,
            )
            .unwrap();
        let (_, checkpoint) =
            into_compacted(prepared).expect("manual compaction must not be threshold-gated");
        assert_eq!(checkpoint.trigger, CompactionTrigger::Manual);
        assert_eq!(checkpoint.phase, Some(CompactionPhase::Standalone));
        assert_eq!(
            checkpoint.algorithm_version,
            evidence::ALGORITHM_DETERMINISTIC
        );
    }

    #[tokio::test]
    async fn repeated_manual_compaction_without_new_content_is_unchanged() {
        let mut checkpoint = projected(
            "user",
            "[Context compaction: existing summary]",
            "checkpoint-entry",
        );
        crate::compaction::stamp_internal_checkpoint_message(
            &mut checkpoint.message,
            "checkpoint-entry",
        );
        let mut prompt = test_prompt();
        prompt.messages = vec![checkpoint];

        let prepared = test_manager()
            .prepare_evidence(
                prompt.clone(),
                &raw_of(&prompt),
                CompactionTrigger::Manual,
                CompactionPhase::Standalone,
                None,
                &AtomicBool::new(false),
                None,
            )
            .unwrap();

        assert!(matches!(prepared, ContextPreparation::Unchanged { .. }));
    }

    #[test]
    fn manual_cut_summarizes_all_when_every_turn_fits() {
        let messages = vec![
            projected("user", "hello", "e1"),
            projected("assistant", "hi", "e2"),
            projected("user", "write a poem", "e3"),
            projected("assistant", "poem", "e4"),
        ];
        let costs = vec![1, 1, 1, 1];

        assert_eq!(turn_aware_cut(&messages, &costs, 10, true), Some(4));
        assert_eq!(turn_aware_cut(&messages, &costs, 10, false), Some(1));
    }

    #[test]
    fn manual_cut_retains_complete_recent_turn_when_history_exceeds_budget() {
        let messages = vec![
            projected("user", "old request", "e1"),
            projected("assistant", "large old result", "e2"),
            projected("user", "recent request", "e3"),
            projected("assistant", "recent result", "e4"),
        ];
        let costs = vec![10_000, 10_000, 500, 500];

        assert_eq!(turn_aware_cut(&messages, &costs, 15_000, true), Some(2));
    }

    #[tokio::test]
    async fn manual_compaction_of_short_history_replaces_every_turn_with_summary() {
        let mut manager = test_manager();
        manager.keep_recent_tokens = 200_000;
        manager.context_window = 1_000_000;
        let prompt = PromptContext {
            messages: vec![
                projected("user", "你好", "e1"),
                projected("assistant", "你好，有什么可以帮你？", "e2"),
                projected("user", "测试工具调用", "e3"),
                projected("assistant", "工具调用正常", "e4"),
                projected("user", "写一首长诗", "e5"),
                projected("assistant", "这是完整长诗", "e6"),
            ],
            usage: ContextUsage {
                input_tokens: Some(5_143),
                estimated_input_tokens: 5_143,
                context_window: 1_000_000,
                ..Default::default()
            },
        };

        let prepared = manager
            .prepare_evidence(
                prompt.clone(),
                &raw_of(&prompt),
                CompactionTrigger::Manual,
                CompactionPhase::Standalone,
                None,
                &AtomicBool::new(false),
                None,
            )
            .unwrap();
        let (prompt, checkpoint) =
            into_compacted(prepared).expect("expected full manual compaction");

        assert_eq!(checkpoint.covered_from_entry_id.as_deref(), Some("e1"));
        assert_eq!(checkpoint.cutoff_entry_id.as_deref(), Some("e6"));
        assert_eq!(
            prompt.messages.len(),
            7,
            "all six QA originals and one summary remain"
        );
        assert_eq!(
            checkpoint.protected_entry_ids,
            vec!["e1", "e2", "e3", "e4", "e5", "e6"]
        );
        assert_eq!(prompt.messages[0].message.text(), "你好");
        assert_eq!(prompt.messages[5].message.text(), "这是完整长诗");
        // Under C there is no summary request to inspect: every original is protected
        // verbatim (asserted above), and the index carries the tool evidence the model no
        // longer has to be told about. The legacy assertion that the *request input* held
        // the conversation is therefore replaced by the projection's own contents.
        assert_eq!(
            checkpoint.algorithm_version,
            evidence::ALGORITHM_DETERMINISTIC
        );
    }

    #[tokio::test]
    async fn user_text_that_looks_like_a_checkpoint_is_not_filtered() {
        let mut manager = test_manager();
        manager.keep_recent_tokens = 200_000;
        manager.context_window = 1_000_000;
        let marker_like_text = "[Context compaction: explain this literal syntax]";
        let prompt = PromptContext {
            messages: vec![
                projected("user", marker_like_text, "e1"),
                projected("assistant", "I will explain it", "e2"),
            ],
            usage: ContextUsage {
                input_tokens: Some(100),
                estimated_input_tokens: 100,
                context_window: 1_000_000,
                ..Default::default()
            },
        };

        let prepared = manager
            .prepare_evidence(
                prompt.clone(),
                &raw_of(&prompt),
                CompactionTrigger::Manual,
                CompactionPhase::Standalone,
                None,
                &AtomicBool::new(false),
                None,
            )
            .unwrap();

        let (_, checkpoint) = into_compacted(prepared).expect("expected compaction");
        // The marker-looking user text must be treated as an ordinary user original: it is
        // protected verbatim (the legacy string protocol would have swallowed it).
        assert!(checkpoint.protected_entry_ids.contains(&"e1".to_string()));
    }

    #[tokio::test]
    async fn first_turn_on_small_windows_does_not_require_a_compaction_boundary() {
        for window in [4_096, 8_192, 16_384] {
            let (reserve_tokens, keep_recent_tokens) =
                crate::compaction::context_token_budgets(window);
            let manager = ContextManager {
                enabled: true,
                reserve_tokens,
                keep_recent_tokens,
                context_window: window,
                model: "small-model".to_string(),
            };
            let prompt = PromptContext {
                messages: vec![projected("user", "hello", "e1")],
                usage: ContextUsage {
                    input_tokens: Some(10),
                    estimated_input_tokens: 10,
                    context_window: window as u64,
                    ..Default::default()
                },
            };
            let prepared = manager
                .prepare_evidence(
                    prompt.clone(),
                    &raw_of(&prompt),
                    CompactionTrigger::Automatic,
                    phase_of(CompactionTrigger::Automatic),
                    None,
                    &AtomicBool::new(false),
                    None,
                )
                .unwrap();

            assert!(matches!(prepared, ContextPreparation::Unchanged { .. }));
        }
    }

    /// The regression this bounds: Kimi's catalog declares an output limit equal
    /// to its context window (`max_tokens == context_length`, because input and
    /// output share one window). While that limit was reserved verbatim, the
    /// admission check rejected *every* request of such a session — the first
    /// one included — with "system/tools and output reservation leave no input
    /// room", without ever calling a provider.
    #[tokio::test]
    async fn a_declared_output_limit_equal_to_the_window_admits_the_first_turn() {
        let model = crate::models::Model {
            id: "kimi-k3".into(),
            context_window: 1_048_576,
            max_tokens: 1_048_576,
            reasoning: true,
            ..Default::default()
        };
        let mut prompt = PromptContext {
            messages: vec![projected("user", "hi", "e1")],
            usage: ContextUsage::default(),
        };
        super::super::set_request_budget(
            &mut prompt,
            &"s".repeat(120_000),
            &[],
            crate::models::effective_max_tokens(&model),
        );
        let (reserve_tokens, keep_recent_tokens) =
            crate::compaction::context_token_budgets(model.context_window);
        let manager = ContextManager {
            enabled: true,
            reserve_tokens,
            keep_recent_tokens,
            context_window: model.context_window,
            model: model.id.clone(),
        };
        let prepared = manager
            .prepare_evidence(
                prompt.clone(),
                &raw_of(&prompt),
                CompactionTrigger::ModelContextDownshift,
                CompactionPhase::PreTurn,
                None,
                &AtomicBool::new(false),
                None,
            )
            .unwrap();
        // Unchanged *and* no checkpoint: an untouched projection must not be
        // reported as a compaction, or the caller would commit a receipt for a
        // checkpoint that does not exist.
        assert!(matches!(prepared, ContextPreparation::Unchanged { .. }));
        assert!(into_compacted(prepared).is_none());
    }

    // ─── stream/event scripting for call_summary_model arms ────────────────

    enum StreamScript {
        Events(Vec<ModelStreamEvent>),
        /// The request itself is rejected (a provider-side error before any
        /// frame), e.g. an oversized request.
        Fail(String),
        /// A stream whose sender is leaked, so `next()` never resolves and the
        /// summary-event timeout fires (mirrors a hung provider connection).
        Hang,
    }

    struct ScriptStreamProvider {
        scripts: Mutex<VecDeque<StreamScript>>,
    }

    impl ScriptStreamProvider {
        fn new(scripts: impl IntoIterator<Item = StreamScript>) -> Self {
            Self {
                scripts: Mutex::new(scripts.into_iter().collect()),
            }
        }
    }

    #[async_trait::async_trait]
    impl LLMProvider for ScriptStreamProvider {
        async fn stream_model(
            &self,
            _request: ModelRequest,
        ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
            let script = self.scripts.lock().pop_front().expect("scripted stream");
            match script {
                StreamScript::Events(events) => {
                    let (tx, rx) = mpsc::channel(events.len().max(1));
                    for event in events {
                        tx.send(event).await.unwrap();
                    }
                    Ok(ReceiverStream::new(rx))
                }
                StreamScript::Fail(message) => Err(anyhow::anyhow!(message)),
                StreamScript::Hang => {
                    let (tx, rx) = mpsc::channel::<ModelStreamEvent>(1);
                    std::mem::forget(tx);
                    Ok(ReceiverStream::new(rx))
                }
            }
        }
    }

    struct InterruptOnStreamProvider {
        interrupted: std::sync::Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl LLMProvider for InterruptOnStreamProvider {
        async fn stream_model(
            &self,
            _request: ModelRequest,
        ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
            self.interrupted.store(true, Ordering::Relaxed);
            let (tx, rx) = mpsc::channel::<ModelStreamEvent>(1);
            drop(tx);
            Ok(ReceiverStream::new(rx))
        }
    }

    struct PendingRequestProvider;

    #[async_trait::async_trait]
    impl LLMProvider for PendingRequestProvider {
        async fn stream_model(
            &self,
            _request: ModelRequest,
        ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
            std::future::pending().await
        }
    }

    fn internal_checkpoint(summary: &str, id: &str) -> ProjectedMessage {
        let mut msg = AgentMessage::new_user(
            "user",
            serde_json::json!([{
                "type": "text",
                "text": format!("[Context compaction: {summary}]")
            }]),
        );
        super::super::stamp_internal_checkpoint_message(&mut msg, id);
        ProjectedMessage {
            message: msg,
            source_entry_ids: vec![id.to_string()],
        }
    }

    fn test_plan(removed: Vec<ProjectedMessage>) -> CompactionPlan {
        CompactionPlan {
            removed,
            retained: Vec::new(),
            protected: Vec::new(),
            target_tokens: 6000,
            summary_budget: 1024,
            summarized_outputs: 0,
            previous_summary: Some("prior summary".to_string()),
            covered_from_entry_id: "e1".to_string(),
            cutoff_entry_id: "e1".to_string(),
            tokens_before: 100,
            usage: ContextUsage::default(),
            trigger: CompactionTrigger::Automatic,
            phase: CompactionPhase::PreTurn,
            instructions: Some("keep the exact constraints".to_string()),
        }
    }

    /// Extracts the `Compacted` payload without a dead `let-else` panic arm:
    /// returning `Option` and `.expect()`-ing at the call site keeps the panic
    /// in `core` where it belongs instead of leaving an uncovered test line.
    fn into_compacted(
        prepared: ContextPreparation,
    ) -> Option<(PromptContext, Box<ContextCheckpoint>)> {
        match prepared {
            ContextPreparation::Compacted { prompt, checkpoint } => Some((prompt, checkpoint)),
            ContextPreparation::Unchanged { .. } => None,
        }
    }

    // ─── plan / turn_aware_cut / fallback_cut edge cases ──────────────────

    #[test]
    fn turn_aware_cut_without_user_messages_compacts_all_or_falls_back() {
        let messages = vec![
            projected("assistant", "thinking", "e1"),
            projected("tool", "result", "e2"),
        ];
        let costs = vec![1, 1];
        // No user starts, compact_all + fits → the full conversation.
        assert_eq!(turn_aware_cut(&messages, &costs, 100, true), Some(2));
        // Not compact_all → the plain fallback cut.
        assert_eq!(turn_aware_cut(&messages, &costs, 100, false), None);
    }

    #[test]
    fn fallback_cut_walks_back_from_tool_without_tool_call_owner() {
        let messages = vec![
            projected("user", "first", "e1"),
            projected("user", "second", "e2"),
            projected("tool", "result", "e3"),
        ];
        let costs = vec![100, 100, 1];
        assert_eq!(fallback_cut(&messages, &costs, 50), Some(1));
    }

    #[test]
    fn plan_downshift_mid_turn_keeps_full_active_turn_disabled() {
        let manager = test_manager();
        let result = plan(
            &manager,
            test_prompt(),
            CompactionTrigger::ModelContextDownshift,
            CompactionPhase::MidTurn,
            None,
            None,
            4096,
        );
        // Downshift + MidTurn leaves allow_full_active_turn false (the
        // `matches!` fallthrough arm), but the plan still resolves without
        // panicking — exercising the non-Automatic/non-ProviderContextLimit
        // branch of the trigger guard.
        assert!(result.is_ok() || matches!(result, Err(ContextError::NoValidBoundary)));
    }

    // ─── serialize_message content-block coverage ─────────────────────────

    #[test]
    fn serialize_message_covers_role_labels_and_block_variants() {
        let system = AgentMessage {
            role: "system".to_string(),
            content: vec![ContentBlock::text("sys")],
            ..Default::default()
        };
        assert!(serialize_message(&system, SerializationMode::Normal).contains("[System update]"));

        let unknown = AgentMessage {
            role: "mystery".to_string(),
            content: vec![ContentBlock::text("x")],
            ..Default::default()
        };
        assert!(serialize_message(&unknown, SerializationMode::Normal).contains("[mystery]"));

        let reasoning = AgentMessage {
            role: "assistant".to_string(),
            content: vec![ContentBlock::reasoning("think", Default::default())],
            ..Default::default()
        };
        assert!(serialize_message(&reasoning, SerializationMode::Normal)
            .contains("[Assistant reasoning]"));
        let long_reasoning = AgentMessage {
            role: "assistant".to_string(),
            content: vec![ContentBlock::reasoning("r".repeat(600), Default::default())],
            ..Default::default()
        };
        assert!(
            serialize_message(&long_reasoning, SerializationMode::Strict)
                .contains("[truncated for compaction]")
        );

        let img_http = AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::image("http://example.com/x.png")],
            ..Default::default()
        };
        assert!(serialize_message(&img_http, SerializationMode::Normal)
            .contains("http://example.com/x.png"));
        let img_data = AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::image("data:image/png;base64,abc")],
            ..Default::default()
        };
        assert!(serialize_message(&img_data, SerializationMode::Normal).contains("embedded image"));
        let img_none = AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::Image {
                image_url: crate::types::ImageUrlData { url: None },
            }],
            ..Default::default()
        };
        assert!(serialize_message(&img_none, SerializationMode::Normal).contains("embedded image"));

        let tool_result = AgentMessage {
            role: "tool".to_string(),
            content: vec![ContentBlock::tool_result("c", "x".repeat(600), false)],
            ..Default::default()
        };
        assert!(serialize_message(&tool_result, SerializationMode::Strict)
            .contains("[truncated for compaction]"));

        // A short result is carried verbatim (no ellipsis marker, no rewrite).
        let short_result = AgentMessage {
            role: "tool".to_string(),
            content: vec![ContentBlock::tool_result("c9", "cap=128MiB", false)],
            ..Default::default()
        };
        let short = serialize_message(&short_result, SerializationMode::Normal);
        assert!(short.contains("[Tool result c9]: cap=128MiB"), "{short}");
        assert!(!short.contains("[truncated for compaction]"));

        // An assistant tool call is named with its id and arguments, so the
        // summary model can see which file/command was attempted.
        let tool_call = AgentMessage {
            role: "assistant".to_string(),
            content: vec![ContentBlock::tool_call(
                "c9",
                "read",
                serde_json::json!({"path":"config.json"}),
                Default::default(),
            )],
            ..Default::default()
        };
        let call_text = serialize_message(&tool_call, SerializationMode::Normal);
        assert!(
            call_text.contains("[Assistant tool call c9]: read("),
            "{call_text}"
        );
        assert!(call_text.contains("config.json"), "{call_text}");

        // Empty text falls through to the catch-all arm and is ignored.
        let empty_text = AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::text("   ")],
            ..Default::default()
        };
        assert_eq!(
            serialize_message(&empty_text, SerializationMode::Normal),
            ""
        );
    }

    /// Attachment images are charged as a fixed allowance, and the two sources of
    /// them (an inline image block, an `attachments` metadata entry) must not be
    /// added together: a message that carries both would otherwise be billed as
    /// two images and shorten the projection the model is allowed to see.
    #[test]
    fn attachment_images_are_charged_once_whichever_way_they_are_declared() {
        let base = projected_token_cost(&projected("user", "hello world", "e1"));
        let attachments = serde_json::json!([{"kind": "image"}, {"kind": "file"}]);

        let mut inline = projected("user", "hello world", "e1");
        inline
            .message
            .content
            .push(ContentBlock::image("https://example.com/x.png"));
        let inline_cost = projected_token_cost(&inline);
        assert_eq!(
            inline_cost,
            base + 2_048,
            "one inline image is one image allowance"
        );

        let mut both = inline.clone();
        both.message
            .metadata
            .get_or_insert_with(Default::default)
            .insert("attachments".into(), attachments.clone());
        assert_eq!(
            projected_token_cost(&both),
            inline_cost,
            "metadata attachments must not be charged on top of an inline image"
        );

        let mut metadata_only = projected("user", "hello world", "e1");
        metadata_only
            .message
            .metadata
            .get_or_insert_with(Default::default)
            .insert("attachments".into(), attachments);
        assert_eq!(
            projected_token_cost(&metadata_only),
            base + 2_048,
            "exactly one of the two metadata entries names an image"
        );
    }

    /// Retention notes are what the next agent reads about material it can no
    /// longer see; the two outcomes (omitted vs summarized) must not be conflated.
    #[test]
    fn retention_note_distinguishes_omitted_outputs_from_summarized_ones() {
        assert_eq!(
            retention_note(3, evidence::ALGORITHM_DETERMINISTIC),
            "\n\n[Retention note: 3 older assistant outputs were omitted from the active \
             context (not summarized) to fit. Query original history for their exact text.]"
        );
        let summarized = retention_note(3, evidence::ALGORITHM_SUMMARIZED);
        assert!(
            summarized.contains("3 older assistant outputs were summarized to fit"),
            "{summarized}"
        );
        assert!(!summarized.contains("not summarized"), "{summarized}");
    }

    /// A projection whose only possible cut is the existing checkpoint itself has
    /// nothing left to compact: the boundary walk lands on the last message, and
    /// the plan must refuse rather than emit a checkpoint that covers no history.
    #[test]
    fn plan_refuses_a_boundary_that_lands_on_the_existing_checkpoint() {
        let manager = test_manager();
        let mut prompt = test_prompt();
        prompt
            .messages
            .push(internal_checkpoint("an earlier handoff", "cp1"));
        let result = plan(
            &manager,
            prompt,
            CompactionTrigger::Automatic,
            CompactionPhase::PreTurn,
            None,
            None,
            4096,
        );
        assert!(
            matches!(result, Err(ContextError::NoValidBoundary)),
            "a cut at the checkpoint covers nothing compactable"
        );
    }

    // ─── internal_summary ─────────────────────────────────────────────────

    #[test]
    fn internal_summary_extracts_only_internal_checkpoint_text() {
        let cp = internal_checkpoint("hello world", "e1");
        assert_eq!(
            internal_summary(&cp.message),
            Some("hello world".to_string())
        );

        let mut assistant = cp.message.clone();
        assistant.role = "assistant".to_string();
        assert_eq!(internal_summary(&assistant), None);

        let mut non_text = cp.message.clone();
        non_text.content = vec![ContentBlock::reasoning("r", Default::default())];
        assert_eq!(internal_summary(&non_text), None);

        let mut no_close = cp.message.clone();
        no_close.content = vec![ContentBlock::text("[Context compaction: unclosed")];
        assert_eq!(internal_summary(&no_close), None);

        let mut blank = cp.message.clone();
        blank.content = vec![ContentBlock::text("[Context compaction:   ]")];
        assert_eq!(internal_summary(&blank), None);

        let plain = projected("user", "[Context compaction: x]", "e2").message;
        assert_eq!(internal_summary(&plain), None);
    }

    // ─── finalize empty-summary guard ─────────────────────────────────────

    #[test]
    fn finalize_rejects_empty_summary() {
        let manager = test_manager();
        let plan = test_plan(vec![projected("user", "hi", "e1")]);
        let result = finalize(&manager, &plan, "   ".to_string(), "v", "m");
        assert_eq!(result.unwrap_err(), ContextError::InvalidSummary);
    }

    /// An autonomous compaction that does not shrink the context is a loss: it
    /// pays a model call and replaces history with a summary of the same size.
    /// `finalize` must refuse it, while the same plan with a summary that does
    /// shrink is admitted — so the refusal is about progress, not shape.
    #[test]
    fn finalize_refuses_a_summary_that_would_not_shrink_an_automatic_compaction() {
        let manager = test_manager();
        let plan = test_plan(vec![projected("user", "hi", "e1")]);
        assert_eq!(plan.tokens_before, 100);
        let grown = finalize(&manager, &plan, "s".repeat(400), "v", "m");
        assert_eq!(grown.unwrap_err(), ContextError::NoProgress);

        let shrunk = finalize(&manager, &plan, "s".repeat(40), "v", "m").unwrap();
        let (_, checkpoint) = into_compacted(shrunk)
            .expect("a shrinking summary is admitted and yields a checkpoint");
        assert!(checkpoint.tokens_after < checkpoint.tokens_before);
    }

    // ─── call_summary_model event arms ────────────────────────────────────

    #[tokio::test]
    async fn summary_model_length_limit_reports_context_limit() {
        let provider =
            ScriptStreamProvider::new([StreamScript::Events(vec![ModelStreamEvent::Finish {
                reason: FinishReason::Length,
                usage: None,
            }])]);
        let result = call_summary_model_with_messages(
            &provider,
            "m",
            "",
            Vec::new(),
            "p".into(),
            Vec::new(),
            &AtomicBool::new(false),
            8192,
            None,
        )
        .await;
        assert!(matches!(result, Err(SummaryCallError::ContextLimit(_))));
    }

    #[tokio::test]
    async fn summary_model_cancelled_finish_propagates_cancellation() {
        let provider =
            ScriptStreamProvider::new([StreamScript::Events(vec![ModelStreamEvent::Finish {
                reason: FinishReason::Cancelled,
                usage: None,
            }])]);
        let result = call_summary_model_with_messages(
            &provider,
            "m",
            "",
            Vec::new(),
            "p".into(),
            Vec::new(),
            &AtomicBool::new(false),
            8192,
            None,
        )
        .await;
        assert!(matches!(result, Err(SummaryCallError::Cancelled)));
    }

    #[tokio::test]
    async fn summary_model_unexpected_finish_reason_is_reported() {
        let provider =
            ScriptStreamProvider::new([StreamScript::Events(vec![ModelStreamEvent::Finish {
                reason: FinishReason::ToolCalls,
                usage: None,
            }])]);
        let result = call_summary_model_with_messages(
            &provider,
            "m",
            "",
            Vec::new(),
            "p".into(),
            Vec::new(),
            &AtomicBool::new(false),
            8192,
            None,
        )
        .await;
        assert!(
            matches!(&result, Err(SummaryCallError::Other(msg)) if msg.contains("finished with tool_calls"))
        );
    }

    #[tokio::test]
    async fn summary_model_stream_errors_distinguish_context_limit_and_fatal() {
        let context_limit =
            ScriptStreamProvider::new([StreamScript::Events(vec![ModelStreamEvent::Error {
                message: "maximum context length exceeded".to_string(),
            }])]);
        assert!(matches!(
            call_summary_model_with_messages(
                &context_limit,
                "m",
                "",
                Vec::new(),
                "p".into(),
                Vec::new(),
                &AtomicBool::new(false),
                8192,
                None,
            )
            .await,
            Err(SummaryCallError::ContextLimit(_))
        ));

        let fatal =
            ScriptStreamProvider::new([StreamScript::Events(vec![ModelStreamEvent::Error {
                message: "authentication failed".to_string(),
            }])]);
        assert!(matches!(
            call_summary_model_with_messages(
            &fatal,
            "m",
            "",
            Vec::new(),
            "p".into(),
            Vec::new(),
            &AtomicBool::new(false),
            8192,
            None,
        ).await,
            Err(SummaryCallError::Other(msg)) if msg == "authentication failed"
        ));
    }

    #[tokio::test]
    async fn summary_model_tool_input_is_rejected() {
        let provider = ScriptStreamProvider::new([StreamScript::Events(vec![
            ModelStreamEvent::ToolInputDelta {
                index: 0,
                id: "t".to_string(),
                delta: "x".to_string(),
                snapshot: false,
            },
        ])]);
        let result = call_summary_model_with_messages(
            &provider,
            "m",
            "",
            Vec::new(),
            "p".into(),
            Vec::new(),
            &AtomicBool::new(false),
            8192,
            None,
        )
        .await;
        assert!(matches!(&result, Err(SummaryCallError::Other(msg)) if msg.contains("tool call")));
    }

    #[tokio::test]
    async fn summary_model_ignores_unmatched_events_and_completes() {
        let provider = ScriptStreamProvider::new([StreamScript::Events(vec![
            ModelStreamEvent::Usage(crate::types::Usage::default()),
            ModelStreamEvent::TextDelta {
                id: "s".to_string(),
                text: VALID_SUMMARY.to_string(),
            },
            ModelStreamEvent::Finish {
                reason: FinishReason::Stop,
                usage: None,
            },
        ])]);
        let result = call_summary_model_with_messages(
            &provider,
            "m",
            "",
            Vec::new(),
            "p".into(),
            Vec::new(),
            &AtomicBool::new(false),
            8192,
            None,
        )
        .await;
        assert_eq!(result.unwrap(), VALID_SUMMARY);
    }

    #[tokio::test]
    async fn summary_model_incomplete_response_is_bounded_and_errors() {
        let partial = || {
            StreamScript::Events(vec![ModelStreamEvent::TextDelta {
                id: "s".to_string(),
                text: "partial".to_string(),
            }])
        };
        let provider = ScriptStreamProvider::new([partial(), partial(), partial()]);
        let result = call_summary_model_with_messages(
            &provider,
            "m",
            "",
            Vec::new(),
            "p".into(),
            Vec::new(),
            &AtomicBool::new(false),
            8192,
            None,
        )
        .await;
        assert!(
            matches!(&result, Err(SummaryCallError::Other(msg)) if msg.contains("ended before a complete response"))
        );
    }

    #[tokio::test]
    async fn summary_model_timeout_retries_then_gives_up() {
        let provider =
            ScriptStreamProvider::new([StreamScript::Hang, StreamScript::Hang, StreamScript::Hang]);
        let result = call_summary_model_with_messages(
            &provider,
            "m",
            "",
            Vec::new(),
            "p".into(),
            Vec::new(),
            &AtomicBool::new(false),
            8192,
            None,
        )
        .await;
        assert!(matches!(&result, Err(SummaryCallError::Other(msg)) if msg.contains("timed out")));
    }

    #[tokio::test]
    async fn summary_model_respects_interrupt_before_request() {
        let provider = ScriptStreamProvider::new([]);
        let result = call_summary_model_with_messages(
            &provider,
            "m",
            "",
            Vec::new(),
            "p".into(),
            Vec::new(),
            &AtomicBool::new(true),
            8192,
            None,
        )
        .await;
        assert!(matches!(result, Err(SummaryCallError::Cancelled)));
    }

    #[tokio::test]
    async fn summary_model_respects_interrupt_during_connection_setup() {
        let interrupted = std::sync::Arc::new(AtomicBool::new(false));
        let signal = interrupted.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            signal.store(true, Ordering::Relaxed);
        });
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            call_summary_model_with_messages(
                &PendingRequestProvider,
                "m",
                "",
                Vec::new(),
                "p".into(),
                Vec::new(),
                &interrupted,
                8192,
                None,
            ),
        )
        .await
        .expect("interrupt must beat the cancellation watchdog");
        assert!(matches!(result, Err(SummaryCallError::Cancelled)));
    }

    #[tokio::test]
    async fn summary_model_respects_interrupt_during_stream() {
        let interrupted = std::sync::Arc::new(AtomicBool::new(false));
        let provider = InterruptOnStreamProvider {
            interrupted: interrupted.clone(),
        };
        let result = call_summary_model_with_messages(
            &provider,
            "m",
            "",
            Vec::new(),
            "p".into(),
            Vec::new(),
            &interrupted,
            8192,
            None,
        )
        .await;
        assert!(matches!(result, Err(SummaryCallError::Cancelled)));
    }

    /// A provider that accepts the request and then never connects is a timeout,
    /// not a hang: the summary call is bounded, and the failure is reported as a
    /// timeout rather than as a context-limit failure, because the two have
    /// different recovery paths (the retry policy vs the emergency projection).
    #[tokio::test]
    async fn a_summary_request_that_never_connects_times_out() {
        let result = call_summary_model_with_messages(
            &PendingRequestProvider,
            "m",
            "",
            Vec::new(),
            "p".into(),
            Vec::new(),
            &AtomicBool::new(false),
            8192,
            None,
        )
        .await;
        assert_eq!(
            result.unwrap_err().to_string(),
            "summary request timed out",
            "the connect timeout owns its message"
        );
    }

    /// A provider that rejects the summary request for its size is different from
    /// a hung one: the rejection is classified as a context limit and is *not*
    /// retried, because the same oversized request can only fail again.
    /// `ScriptStreamProvider` holds a single script, so a retry would panic
    /// instead of quietly passing.
    #[tokio::test]
    async fn a_summary_request_rejected_for_its_size_is_not_retried() {
        let provider = ScriptStreamProvider::new([StreamScript::Fail(
            "maximum context length exceeded (HTTP 400)".to_string(),
        )]);
        let error = call_summary_model_with_messages(
            &provider,
            "m",
            "",
            Vec::new(),
            "p".into(),
            Vec::new(),
            &AtomicBool::new(false),
            8192,
            None,
        )
        .await
        .expect_err("an oversized summary request must fail");
        assert!(
            matches!(error, SummaryCallError::ContextLimit(_)),
            "{error:?}"
        );
        assert!(
            error
                .to_string()
                .contains("maximum context length exceeded"),
            "the provider's reason must reach the caller: {error}"
        );
    }

    /// Frame types the summary does not consume are ignored, and an error that
    /// arrives *after* the stream finished a complete answer must not throw that
    /// answer away — a trailing transport error is not a reason to recompute the
    /// handoff. Reasoning frames must not leak into the text either.
    #[tokio::test]
    async fn a_complete_summary_survives_unknown_frames_and_a_trailing_error() {
        let provider = ScriptStreamProvider::new([StreamScript::Events(vec![
            ModelStreamEvent::ReasoningDelta {
                id: "s".into(),
                text: "private thinking".into(),
            },
            ModelStreamEvent::TextDelta {
                id: "s".into(),
                text: "the summary".into(),
            },
            ModelStreamEvent::Finish {
                reason: FinishReason::Stop,
                usage: None,
            },
            ModelStreamEvent::Error {
                message: "connection reset after the finish".into(),
            },
        ])]);
        let text = call_summary_model_with_messages(
            &provider,
            "m",
            "",
            Vec::new(),
            "p".into(),
            Vec::new(),
            &AtomicBool::new(false),
            8192,
            None,
        )
        .await
        .unwrap();
        assert_eq!(text, "the summary");
        assert!(!text.contains("private thinking"));
    }
}
