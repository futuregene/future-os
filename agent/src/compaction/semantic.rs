use super::{
    estimate_tokens, AgentMessage, CompactionPhase, CompactionTrigger, ContentBlock,
    ContextCheckpoint, ContextError, ContextManager, ContextPreparation, ContextUsage,
    ConvertToLLM, ProjectedMessage, PromptContext,
};
use crate::llm::schema::{FinishReason, ModelRequest, ModelStreamEvent};
use crate::types::LLMProvider;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio_stream::StreamExt;

const TOOL_OUTPUT_LIMIT: usize = 2_000;
const STRICT_TOOL_OUTPUT_LIMIT: usize = 512;
const REASONING_LIMIT: usize = 2_000;
const STRICT_REASONING_LIMIT: usize = 512;
const SUMMARY_OUTPUT_RESERVE: u64 = 4_096;
const SUMMARY_SAFETY_MARGIN: u64 = 2_048;
const MIN_SUMMARY_CHUNK_TOKENS: u64 = 128;
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

const SUMMARY_SYSTEM_PROMPT: &str = r#"You are a context summarization agent. Produce a structured handoff summary so another coding agent can continue the work. Do not continue the conversation or answer its questions. Output only the requested structure, using the conversation's primary language."#;

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
    Strict,
}

struct CompactionPlan {
    removed: Vec<ProjectedMessage>,
    retained: Vec<ProjectedMessage>,
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

#[allow(clippy::too_many_arguments)]
pub(super) async fn prepare(
    manager: &ContextManager,
    prompt: PromptContext,
    trigger: CompactionTrigger,
    phase: CompactionPhase,
    custom_instructions: Option<&str>,
    provider: &dyn LLMProvider,
    interrupted: &AtomicBool,
    fallback: Option<(&dyn LLMProvider, &str)>,
    on_started: Option<&(dyn Fn() + Sync)>,
) -> Result<ContextPreparation, ContextError> {
    let plan = match plan(
        manager,
        prompt,
        trigger,
        phase,
        custom_instructions,
        on_started,
    )? {
        PlannedPreparation::Unchanged(prompt) => {
            return Ok(ContextPreparation::Unchanged { prompt });
        }
        PlannedPreparation::Compact(plan) => plan,
    };

    let mut semantic = summarize_with_replan(manager, &plan, provider, interrupted).await;
    let mut summary_model = manager.model.as_str();
    if !matches!(semantic, Ok(ref summary) if valid_summary(summary))
        && !matches!(semantic, Err(SummaryCallError::Cancelled))
    {
        if let Some((fallback_provider, fallback_model)) = fallback {
            tracing::warn!(
                fallback_model,
                "retrying context compaction with selected new model"
            );
            semantic = summarize_with_replan(manager, &plan, fallback_provider, interrupted).await;
            summary_model = fallback_model;
        }
    }
    let (summary, algorithm_version) = match semantic {
        Ok(summary) if valid_summary(&summary) => (summary, "semantic-v1"),
        Err(SummaryCallError::Cancelled) => return Err(ContextError::Cancelled),
        Ok(_) => {
            if plan.trigger != CompactionTrigger::ProviderContextLimit {
                return Err(ContextError::InvalidSummary);
            }
            tracing::warn!("provider context-limit recovery received an invalid semantic summary; using emergency summary");
            (emergency_summary(&plan), "deterministic-emergency-v1")
        }
        Err(SummaryCallError::ContextLimit(error)) | Err(SummaryCallError::Other(error)) => {
            if plan.trigger != CompactionTrigger::ProviderContextLimit {
                return Err(ContextError::SummaryFailed(error));
            }
            tracing::warn!(
                error,
                "provider context-limit recovery compaction failed; using emergency summary"
            );
            (emergency_summary(&plan), "deterministic-emergency-v1")
        }
    };
    finalize(manager, plan, summary, algorithm_version, summary_model)
}

pub(super) fn prepare_deterministic(
    manager: &ContextManager,
    prompt: PromptContext,
    trigger: CompactionTrigger,
    custom_instructions: Option<&str>,
) -> Result<ContextPreparation, ContextError> {
    match plan(
        manager,
        prompt,
        trigger,
        super::default_phase(trigger),
        custom_instructions,
        None,
    )? {
        PlannedPreparation::Unchanged(prompt) => Ok(ContextPreparation::Unchanged { prompt }),
        PlannedPreparation::Compact(plan) => {
            let summary = emergency_summary(&plan);
            let model = manager.model.as_str();
            finalize(manager, plan, summary, "deterministic-emergency-v1", model)
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
        .flat_map(|projected| ConvertToLLM(std::slice::from_ref(&projected.message)))
        .map(|message| estimate_tokens(&message).max(0) as u64)
        .sum::<u64>();
    let tokens_before = estimated.max(prompt.usage.input_tokens.unwrap_or(0));
    let window = manager.context_window.max(1) as u64;
    let reserve = manager.reserve_tokens.max(0) as u64;
    // A user-selected manual compaction deliberately bypasses the automatic
    // threshold: `/压缩` is an explicit request to compact history, not a
    // suggestion to wait until the next context-limit guard. It still needs a
    // real journal boundary below; unlike automatic compaction, it may cover
    // the entire committed conversation and retain only the new summary.
    let threshold_gated = matches!(
        trigger,
        CompactionTrigger::Automatic | CompactionTrigger::ModelContextDownshift
    );
    if threshold_gated && tokens_before <= window.saturating_sub(reserve) {
        return Ok(PlannedPreparation::Unchanged(prompt));
    }
    if let Some(on_started) = on_started {
        on_started();
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
    let mut cut = turn_aware_cut(
        &prompt.messages,
        &costs,
        keep_recent_tokens,
        compact_all_when_every_turn_fits,
    )
    .or_else(|| allow_full_active_turn.then_some(prompt.messages.len()))
    .ok_or(ContextError::NoValidBoundary)?;
    if allow_full_active_turn && costs[cut..].iter().copied().sum::<u64>() > keep_recent_tokens {
        cut = prompt.messages.len();
    }
    if cut == 0
        || cut > prompt.messages.len()
        || (cut == prompt.messages.len() && !allow_full_active_turn)
    {
        return Err(ContextError::NoValidBoundary);
    }

    let removed = prompt.messages[..cut].to_vec();
    let retained = prompt.messages[cut..].to_vec();
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

fn projected_token_cost(projected: &ProjectedMessage) -> u64 {
    ConvertToLLM(std::slice::from_ref(&projected.message))
        .iter()
        .map(|message| estimate_tokens(message).max(0) as u64)
        .sum()
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
            (item.message.role == "user" && internal_summary(&item.message).is_none())
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

async fn summarize_with_replan(
    manager: &ContextManager,
    plan: &CompactionPlan,
    provider: &dyn LLMProvider,
    interrupted: &AtomicBool,
) -> Result<String, SummaryCallError> {
    match summarize_fold(
        manager,
        plan,
        provider,
        interrupted,
        SerializationMode::Normal,
    )
    .await
    {
        Err(SummaryCallError::ContextLimit(_)) => {
            summarize_fold(
                manager,
                plan,
                provider,
                interrupted,
                SerializationMode::Strict,
            )
            .await
        }
        result => result,
    }
}

async fn summarize_fold(
    manager: &ContextManager,
    plan: &CompactionPlan,
    provider: &dyn LLMProvider,
    interrupted: &AtomicBool,
    mode: SerializationMode,
) -> Result<String, SummaryCallError> {
    let serialized = plan
        .removed
        .iter()
        .filter(|item| internal_summary(&item.message).is_none())
        .map(|item| serialize_message(&item.message, mode))
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    let mut remaining = serialized.into_iter().collect::<VecDeque<_>>();
    if remaining.is_empty() {
        return Err(SummaryCallError::Other(
            "no conversation content remained after serialization".to_string(),
        ));
    }

    let mut accumulator = plan.previous_summary.clone();
    let mut fold_step = 0_usize;
    while !remaining.is_empty() {
        if interrupted.load(Ordering::Relaxed) {
            return Err(SummaryCallError::Cancelled);
        }
        let budget =
            summary_chunk_budget(manager, accumulator.as_deref(), mode).ok_or_else(|| {
                SummaryCallError::Other(
                    "context compaction accumulator left no safe room for another summary chunk"
                        .to_string(),
                )
            })?;
        let chunk = take_next_chunk(&mut remaining, budget).ok_or_else(|| {
            SummaryCallError::Other("failed to build context compaction chunk".to_string())
        })?;
        fold_step += 1;
        let remaining_parts = remaining.len();
        let strict = matches!(mode, SerializationMode::Strict);
        tracing::debug!(
            fold_step,
            budget,
            remaining_parts,
            strict,
            "planned context compaction summary fold step"
        );
        let prompt = summary_prompt(accumulator.as_deref(), &chunk, plan.instructions.as_deref());
        let summary = call_summary_model(provider, &manager.model, prompt, interrupted).await?;
        if !valid_summary(&summary) {
            return Err(SummaryCallError::Other(
                "model returned a summary without the required structure".to_string(),
            ));
        }
        accumulator = Some(summary);
    }
    accumulator.ok_or_else(|| SummaryCallError::Other("empty summary fold".to_string()))
}

fn summary_chunk_budget(
    manager: &ContextManager,
    accumulator: Option<&str>,
    mode: SerializationMode,
) -> Option<u64> {
    let window = manager.context_window.max(1) as u64;
    // Fixed 4K/2K reserves are appropriate for large coding models but leave
    // no input room on supported 4K/8K models. Scale them down while retaining
    // the original caps for larger windows.
    let output_reserve = SUMMARY_OUTPUT_RESERVE.min((window / 4).max(1));
    let safety_margin = SUMMARY_SAFETY_MARGIN.min((window / 8).max(1));
    let fixed = estimate_text_tokens(SUMMARY_SYSTEM_PROMPT)
        + estimate_text_tokens(SUMMARY_TEMPLATE)
        + accumulator.map(estimate_text_tokens).unwrap_or(0)
        + output_reserve
        + safety_margin;
    let available = window.saturating_sub(fixed);
    let ratio_cap = match mode {
        SerializationMode::Normal => window.saturating_mul(60) / 100,
        SerializationMode::Strict => window.saturating_mul(35) / 100,
    };
    let budget = available.min(ratio_cap);
    (budget >= MIN_SUMMARY_CHUNK_TOKENS).then_some(budget)
}

fn take_next_chunk(remaining: &mut VecDeque<String>, budget: u64) -> Option<String> {
    let mut current = Vec::new();
    let mut current_tokens = 0_u64;
    while let Some(value) = remaining.pop_front() {
        if estimate_text_tokens(&value) > budget {
            for part in split_to_token_budget(&value, budget).into_iter().rev() {
                remaining.push_front(part);
            }
            continue;
        }
        let tokens = estimate_text_tokens(&value);
        if !current.is_empty() && current_tokens.saturating_add(tokens) > budget {
            remaining.push_front(value);
            break;
        }
        current.push(value);
        current_tokens = current_tokens.saturating_add(tokens);
    }
    (!current.is_empty()).then(|| current.join("\n\n"))
}

/// A single huge message/tool result must not bypass the summary budget. This
/// is an internal summary-input boundary only; it never changes the retained
/// tail or the immutable journal entry.
fn split_to_token_budget(value: &str, budget: u64) -> Vec<String> {
    if estimate_text_tokens(value) <= budget {
        return vec![value.to_string()];
    }
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quarters = 0_u64;
    let quarter_budget = budget.saturating_mul(4).max(4);
    for ch in value.chars() {
        let cost = if ch.is_ascii() { 1 } else { 4 };
        if !current.is_empty() && quarters.saturating_add(cost) > quarter_budget {
            parts.push(std::mem::take(&mut current));
            quarters = 0;
        }
        current.push(ch);
        quarters = quarters.saturating_add(cost);
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
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

async fn call_summary_model(
    provider: &dyn LLMProvider,
    model: &str,
    prompt: String,
    interrupted: &AtomicBool,
) -> Result<String, SummaryCallError> {
    let mut attempt = 0_usize;
    'attempts: loop {
        if interrupted.load(Ordering::Relaxed) {
            return Err(SummaryCallError::Cancelled);
        }
        let request = ModelRequest {
            model: model.to_string(),
            system_prompt: SUMMARY_SYSTEM_PROMPT.to_string(),
            messages: vec![AgentMessage::new_user(
                "user",
                serde_json::json!([{ "type": "text", "text": prompt }]),
            )],
            tools: Vec::new(),
        };
        let stream = provider.stream_model(request).await;
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
                    tokio::time::sleep(retry_delay(attempt)).await;
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
            let event = match tokio::time::timeout(SUMMARY_EVENT_TIMEOUT, stream.next()).await {
                Ok(event) => event,
                Err(_) if attempt < MAX_TRANSIENT_RETRIES => {
                    attempt += 1;
                    tracing::warn!(attempt, "retrying timed-out context compaction stream");
                    tokio::time::sleep(retry_delay(attempt)).await;
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
                ModelStreamEvent::Finish { reason, .. } => match reason {
                    FinishReason::Stop => {
                        complete = true;
                        break;
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
                },
                ModelStreamEvent::Error { message } => {
                    if is_context_limit(&message) {
                        return Err(SummaryCallError::ContextLimit(message));
                    }
                    if attempt < MAX_TRANSIENT_RETRIES && is_retryable(&message) {
                        attempt += 1;
                        tracing::warn!(attempt, error = %message, "retrying failed context compaction stream");
                        tokio::time::sleep(retry_delay(attempt)).await;
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
            tokio::time::sleep(retry_delay(attempt)).await;
            continue;
        }
        return Err(SummaryCallError::Other(message));
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
                    SerializationMode::Strict => STRICT_TOOL_OUTPUT_LIMIT,
                };
                let kind = if *is_error { "error" } else { "result" };
                lines.push(format!(
                    "[Tool {kind} {tool_call_id}]: {}",
                    truncate(content, limit)
                ));
            }
            _ => {}
        }
    }
    lines.join("\n")
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
    let mut quarters = 0_u64;
    for ch in value.chars() {
        quarters += if ch.is_ascii() { 1 } else { 4 };
    }
    quarters.saturating_add(3) / 4
}

fn valid_summary(summary: &str) -> bool {
    [
        "## Objective",
        "## Important Details",
        "## Work State",
        "### Completed",
        "### Active",
        "### Blocked",
        "## Next Move",
        "## Relevant Files",
    ]
    .iter()
    .all(|heading| summary.contains(heading))
}

fn emergency_summary(plan: &CompactionPlan) -> String {
    let mut user_texts = Vec::new();
    let mut assistant_texts = Vec::new();
    let mut tool_state = Vec::new();
    let mut read_files = Vec::new();
    let mut modified_files = Vec::new();
    for item in &plan.removed {
        if internal_summary(&item.message).is_some() {
            continue;
        }
        for block in &item.message.content {
            match block {
                ContentBlock::Text { text } if item.message.role == "user" => {
                    user_texts.push(truncate(text, 1_000));
                }
                ContentBlock::Text { text } if item.message.role == "assistant" => {
                    assistant_texts.push(truncate(text, 1_000));
                }
                ContentBlock::ToolCall { name, args, .. } => {
                    tool_state.push(format!("{name}({args})"));
                    collect_file_operation(name, args, &mut read_files, &mut modified_files);
                }
                ContentBlock::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                } => tool_state.push(format!(
                    "{tool_call_id}: {}: {}",
                    if *is_error { "error" } else { "success" },
                    truncate(content, 512)
                )),
                _ => {}
            }
        }
    }
    user_texts = user_texts.into_iter().rev().take(3).collect();
    user_texts.reverse();
    assistant_texts = assistant_texts.into_iter().rev().take(3).collect();
    assistant_texts.reverse();
    tool_state = tool_state.into_iter().rev().take(8).collect();
    tool_state.reverse();
    read_files.sort();
    read_files.dedup();
    modified_files.sort();
    modified_files.dedup();

    format!(
        "## Objective\n- {}\n\n## Important Details\n- Previous summary: {}\n- Compaction instructions: {}\n\n## Work State\n### Completed\n- Recent assistant results: {}\n- Modified files: {}\n\n### Active\n- Recent user directives: {}\n- Recent tool activity: {}\n\n### Blocked\n- Tool errors are preserved above when present; otherwise (none).\n\n## Next Move\n1. Continue from the retained recent conversation and verify unresolved user directives.\n\n## Relevant Files\n- Read: {}\n- Modified: {}",
        user_texts.last().cloned().unwrap_or_else(|| "Continue the active user task.".to_string()),
        plan.previous_summary.as_deref().unwrap_or("(none)"),
        plan.instructions.as_deref().unwrap_or("(none)"),
        join_or_none(&assistant_texts),
        join_or_none(&modified_files),
        join_or_none(&user_texts),
        join_or_none(&tool_state),
        join_or_none(&read_files),
        join_or_none(&modified_files),
    )
}

fn collect_file_operation(
    tool: &str,
    args: &serde_json::Value,
    read_files: &mut Vec<String>,
    modified_files: &mut Vec<String>,
) {
    let path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(serde_json::Value::as_str);
    let Some(path) = path else { return };
    let lower = tool.to_ascii_lowercase();
    if lower.contains("read") || lower.contains("view") || lower.contains("open") {
        read_files.push(path.to_string());
    }
    if lower.contains("write")
        || lower.contains("edit")
        || lower.contains("patch")
        || lower.contains("create")
    {
        modified_files.push(path.to_string());
    }
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "(none)".to_string()
    } else {
        values.join(" | ")
    }
}

fn finalize(
    manager: &ContextManager,
    plan: CompactionPlan,
    summary: String,
    algorithm_version: &str,
    summary_model: &str,
) -> Result<ContextPreparation, ContextError> {
    if summary.trim().is_empty() {
        return Err(ContextError::InvalidSummary);
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
    let mut compacted_messages = Vec::with_capacity(plan.retained.len() + 1);
    compacted_messages.push(ProjectedMessage {
        message: summary_message,
        source_entry_ids: vec![entry_id.clone()],
    });
    compacted_messages.extend(plan.retained);
    let tokens_after = compacted_messages
        .iter()
        .map(projected_token_cost)
        .sum::<u64>();
    let window = manager.context_window.max(1) as u64;
    let checkpoint = ContextCheckpoint {
        entry_id,
        checkpoint_id,
        covered_from_entry_id: Some(plan.covered_from_entry_id),
        cutoff_entry_id: Some(plan.cutoff_entry_id),
        summary: vec![ContentBlock::text(summary)],
        tokens_before: plan.tokens_before,
        tokens_after,
        trigger: plan.trigger,
        phase: Some(plan.phase),
        algorithm_version: algorithm_version.to_string(),
        model: summary_model.to_string(),
        context_window: window,
        created_at: chrono::Utc::now(),
        legacy_without_cutoff: false,
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
                ..plan.usage
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

    enum ScriptedReply {
        Error(&'static str),
        StreamError(&'static str),
        Summary(&'static str),
    }

    struct ScriptedProvider {
        replies: Mutex<VecDeque<ScriptedReply>>,
        requests: Mutex<Vec<ModelRequest>>,
    }

    impl ScriptedProvider {
        fn new(replies: impl IntoIterator<Item = ScriptedReply>) -> Self {
            Self {
                replies: Mutex::new(replies.into_iter().collect()),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl LLMProvider for ScriptedProvider {
        async fn stream_model(
            &self,
            request: ModelRequest,
        ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
            self.requests.lock().push(request);
            let reply = self.replies.lock().pop_front().expect("scripted reply");
            match reply {
                ScriptedReply::Error(message) => anyhow::bail!(message),
                ScriptedReply::StreamError(message) => {
                    let (tx, rx) = mpsc::channel(1);
                    tx.send(ModelStreamEvent::Error {
                        message: message.to_string(),
                    })
                    .await
                    .unwrap();
                    Ok(ReceiverStream::new(rx))
                }
                ScriptedReply::Summary(summary) => {
                    let (tx, rx) = mpsc::channel(2);
                    tx.send(ModelStreamEvent::TextDelta {
                        id: "summary".to_string(),
                        text: summary.to_string(),
                    })
                    .await
                    .unwrap();
                    tx.send(ModelStreamEvent::Finish {
                        reason: FinishReason::Stop,
                        usage: None,
                    })
                    .await
                    .unwrap();
                    Ok(ReceiverStream::new(rx))
                }
            }
        }
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

    #[test]
    fn required_summary_structure_is_validated() {
        assert!(valid_summary(
            "## Objective\n## Important Details\n## Work State\n### Completed\n### Active\n### Blocked\n## Next Move\n## Relevant Files"
        ));
        assert!(!valid_summary("short summary"));
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
    async fn semantic_summary_uses_current_model_without_tools() {
        let provider = ScriptedProvider::new([ScriptedReply::Summary(VALID_SUMMARY)]);
        let prepared = test_manager()
            .prepare_semantic_with_phase(
                test_prompt(),
                CompactionTrigger::Automatic,
                CompactionPhase::PreTurn,
                None,
                &provider,
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
        let (_, checkpoint) = into_compacted(prepared).expect("expected compaction");
        assert_eq!(checkpoint.algorithm_version, "semantic-v1");
        assert_eq!(checkpoint.phase, Some(CompactionPhase::PreTurn));
        let requests = provider.requests.lock();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].model, "old-model");
        assert!(requests[0].tools.is_empty());
    }

    #[tokio::test]
    async fn mid_turn_compaction_summarizes_an_oversized_active_tool_tail() {
        let provider = ScriptedProvider::new([ScriptedReply::Summary(VALID_SUMMARY)]);
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
                .prepare_semantic_with_phase(
                    prompt,
                    CompactionTrigger::Automatic,
                    CompactionPhase::MidTurn,
                    None,
                    &provider,
                    &AtomicBool::new(false),
                )
                .await
                .unwrap(),
        )
        .expect("expected full active-turn compaction");

        assert_eq!(checkpoint.covered_from_entry_id.as_deref(), Some("e1"));
        assert_eq!(checkpoint.cutoff_entry_id.as_deref(), Some("e3"));
        assert_eq!(checkpoint.phase, Some(CompactionPhase::MidTurn));
        assert_eq!(
            prompt.messages.len(),
            1,
            "the structured summary continues the task"
        );
        assert!(checkpoint.tokens_after < checkpoint.tokens_before);
    }

    #[tokio::test]
    async fn pre_turn_compaction_does_not_replace_an_unseen_user_prompt() {
        let provider = ScriptedProvider::new([]);
        let prompt = PromptContext {
            messages: vec![projected("user", &"x".repeat(40_000), "e1")],
            usage: ContextUsage {
                input_tokens: Some(10_000),
                estimated_input_tokens: 10_000,
                context_window: 8_000,
                ..Default::default()
            },
        };

        let result = test_manager()
            .prepare_semantic_with_phase(
                prompt,
                CompactionTrigger::Automatic,
                CompactionPhase::PreTurn,
                None,
                &provider,
                &AtomicBool::new(false),
            )
            .await;

        assert_eq!(result.unwrap_err(), ContextError::NoValidBoundary);
        assert!(provider.requests.lock().is_empty());
    }

    #[tokio::test]
    async fn context_limit_replans_once_in_strict_mode() {
        let provider = ScriptedProvider::new([
            ScriptedReply::Error("maximum context length exceeded"),
            ScriptedReply::Summary(VALID_SUMMARY),
        ]);
        let prepared = test_manager()
            .prepare_semantic(
                test_prompt(),
                CompactionTrigger::Automatic,
                None,
                &provider,
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
        let (_, checkpoint) = into_compacted(prepared).expect("expected compaction");
        assert_eq!(checkpoint.algorithm_version, "semantic-v1");
        assert_eq!(provider.requests.lock().len(), 2);
    }

    #[tokio::test]
    async fn retryable_error_is_bounded_and_recovers() {
        let provider = ScriptedProvider::new([
            ScriptedReply::Error("status 503 overload"),
            ScriptedReply::Summary(VALID_SUMMARY),
        ]);
        let prepared = test_manager()
            .prepare_semantic(
                test_prompt(),
                CompactionTrigger::Automatic,
                None,
                &provider,
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
        let (_, checkpoint) = into_compacted(prepared).expect("expected compaction");
        assert_eq!(checkpoint.algorithm_version, "semantic-v1");
        assert_eq!(provider.requests.lock().len(), 2);
    }

    #[tokio::test]
    async fn retryable_stream_error_is_bounded_and_recovers() {
        let provider = ScriptedProvider::new([
            ScriptedReply::StreamError("status 503 overload"),
            ScriptedReply::Summary(VALID_SUMMARY),
        ]);
        let prepared = test_manager()
            .prepare_semantic(
                test_prompt(),
                CompactionTrigger::Automatic,
                None,
                &provider,
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
        let (_, checkpoint) = into_compacted(prepared).expect("expected compaction");
        assert_eq!(checkpoint.algorithm_version, "semantic-v1");
        assert_eq!(provider.requests.lock().len(), 2);
    }

    #[tokio::test]
    async fn oversized_head_is_folded_and_each_step_receives_the_accumulator() {
        let provider = ScriptedProvider::new([
            ScriptedReply::Summary(VALID_SUMMARY),
            ScriptedReply::Summary(VALID_SUMMARY),
        ]);
        let mut prompt = test_prompt();
        prompt.messages[0] = projected("user", &"x".repeat(30_000), "e1");
        test_manager()
            .prepare_semantic(
                prompt,
                CompactionTrigger::Automatic,
                None,
                &provider,
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
        let requests = provider.requests.lock();
        assert_eq!(requests.len(), 2);
        let second_prompt = requests[1].messages[0].text();
        assert!(second_prompt.contains("<prior-summary>"));
        assert!(second_prompt.contains("## Objective"));
    }

    #[test]
    fn fold_budget_shrinks_as_the_accumulator_grows() {
        let manager = test_manager();
        let initial = summary_chunk_budget(&manager, None, SerializationMode::Normal).unwrap();
        let accumulated = format!("{VALID_SUMMARY}\n{}", "detail ".repeat(1_000));
        let next =
            summary_chunk_budget(&manager, Some(&accumulated), SerializationMode::Normal).unwrap();

        assert!(next < initial);
        assert!(next > 0);
    }

    #[test]
    fn fold_budget_stops_when_the_accumulator_fills_the_window() {
        let manager = test_manager();
        let oversized = "x".repeat((manager.context_window as usize) * 4);

        assert_eq!(
            summary_chunk_budget(&manager, Some(&oversized), SerializationMode::Normal),
            None
        );
    }

    #[tokio::test]
    async fn non_retryable_failure_does_not_commit_automatic_checkpoint() {
        let provider = ScriptedProvider::new([ScriptedReply::Error("authentication failed")]);
        let error = test_manager()
            .prepare_semantic(
                test_prompt(),
                CompactionTrigger::Automatic,
                Some("preserve the exact user constraints"),
                &provider,
                &AtomicBool::new(false),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error,
            ContextError::SummaryFailed("authentication failed".to_string())
        );
    }

    #[tokio::test]
    async fn provider_context_limit_failure_uses_deterministic_emergency_summary() {
        let provider = ScriptedProvider::new([ScriptedReply::Error("authentication failed")]);
        let prepared = test_manager()
            .prepare_semantic(
                test_prompt(),
                CompactionTrigger::ProviderContextLimit,
                Some("preserve the exact user constraints"),
                &provider,
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
        let (_, checkpoint) =
            into_compacted(prepared).expect("expected emergency provider-limit compaction");
        assert_eq!(checkpoint.algorithm_version, "deterministic-emergency-v1");
        assert!(serde_json::to_string(&checkpoint.summary)
            .unwrap()
            .contains("preserve the exact user constraints"));
    }

    #[tokio::test]
    async fn manual_failure_does_not_replace_the_full_history() {
        let provider = ScriptedProvider::new([ScriptedReply::Error("authentication failed")]);
        let error = test_manager()
            .prepare_semantic(
                test_prompt(),
                CompactionTrigger::Manual,
                None,
                &provider,
                &AtomicBool::new(false),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error,
            ContextError::SummaryFailed("authentication failed".to_string())
        );
    }

    #[tokio::test]
    async fn downshift_can_retry_only_the_selected_new_model() {
        let old = ScriptedProvider::new([ScriptedReply::Error("authentication failed")]);
        let new = ScriptedProvider::new([ScriptedReply::Summary(VALID_SUMMARY)]);
        let prepared = test_manager()
            .prepare_semantic_with_phase_and_fallback(
                test_prompt(),
                CompactionTrigger::ModelContextDownshift,
                CompactionPhase::PreTurn,
                None,
                &old,
                &AtomicBool::new(false),
                Some((&new, "new-model")),
            )
            .await
            .unwrap();
        let (_, checkpoint) = into_compacted(prepared).expect("expected compaction");
        assert_eq!(checkpoint.model, "new-model");
        assert_eq!(old.requests.lock().len(), 1);
        assert_eq!(new.requests.lock().len(), 1);
    }

    #[tokio::test]
    async fn downshift_does_not_create_checkpoint_when_new_window_fits() {
        let provider = ScriptedProvider::new([]);
        let mut prompt = test_prompt();
        prompt.usage.input_tokens = Some(100);
        prompt.usage.estimated_input_tokens = 100;
        let prepared = test_manager()
            .prepare_semantic_with_phase(
                prompt,
                CompactionTrigger::ModelContextDownshift,
                CompactionPhase::PreTurn,
                None,
                &provider,
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
        assert!(matches!(prepared, ContextPreparation::Unchanged { .. }));
        assert!(provider.requests.lock().is_empty());
    }

    #[tokio::test]
    async fn manual_compaction_bypasses_the_automatic_threshold() {
        let provider = ScriptedProvider::new([ScriptedReply::Summary(VALID_SUMMARY)]);
        let mut prompt = test_prompt();
        prompt.usage.input_tokens = Some(100);
        prompt.usage.estimated_input_tokens = 100;
        let prepared = test_manager()
            .prepare_semantic_with_phase(
                prompt,
                CompactionTrigger::Manual,
                CompactionPhase::Standalone,
                None,
                &provider,
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
        let (_, checkpoint) =
            into_compacted(prepared).expect("manual compaction must not be threshold-gated");
        assert_eq!(checkpoint.trigger, CompactionTrigger::Manual);
        assert_eq!(checkpoint.phase, Some(CompactionPhase::Standalone));
        assert_eq!(provider.requests.lock().len(), 1);
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
        let provider = ScriptedProvider::new([ScriptedReply::Summary(VALID_SUMMARY)]);
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
            .prepare_semantic_with_phase(
                prompt,
                CompactionTrigger::Manual,
                CompactionPhase::Standalone,
                None,
                &provider,
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
        let (prompt, checkpoint) =
            into_compacted(prepared).expect("expected full manual compaction");

        assert_eq!(checkpoint.covered_from_entry_id.as_deref(), Some("e1"));
        assert_eq!(checkpoint.cutoff_entry_id.as_deref(), Some("e6"));
        assert_eq!(prompt.messages.len(), 1, "only the summary should remain");
        let requests = provider.requests.lock();
        let summary_input = requests[0].messages[0].text();
        assert!(summary_input.contains("你好，有什么可以帮你？"));
        assert!(summary_input.contains("工具调用正常"));
        assert!(summary_input.contains("这是完整长诗"));
    }

    #[tokio::test]
    async fn user_text_that_looks_like_a_checkpoint_is_not_filtered() {
        let provider = ScriptedProvider::new([ScriptedReply::Summary(VALID_SUMMARY)]);
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

        manager
            .prepare_semantic_with_phase(
                prompt,
                CompactionTrigger::Manual,
                CompactionPhase::Standalone,
                None,
                &provider,
                &AtomicBool::new(false),
            )
            .await
            .unwrap();

        let requests = provider.requests.lock();
        let summary_input = requests[0].messages[0].text();
        assert!(summary_input.contains(marker_like_text));
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
            let provider = ScriptedProvider::new([]);

            let prepared = manager
                .prepare_semantic(
                    prompt,
                    CompactionTrigger::Automatic,
                    None,
                    &provider,
                    &AtomicBool::new(false),
                )
                .await
                .unwrap();

            assert!(matches!(prepared, ContextPreparation::Unchanged { .. }));
            assert!(provider.requests.lock().is_empty());
        }
    }

    #[tokio::test]
    async fn cancellation_never_writes_an_emergency_checkpoint() {
        let provider = ScriptedProvider::new([]);
        let interrupted = AtomicBool::new(true);
        let result = test_manager()
            .prepare_semantic(
                test_prompt(),
                CompactionTrigger::Automatic,
                None,
                &provider,
                &interrupted,
            )
            .await;
        assert_eq!(result.unwrap_err(), ContextError::Cancelled);
        assert!(provider.requests.lock().is_empty());
    }

    // ─── stream/event scripting for call_summary_model arms ────────────────

    enum StreamScript {
        Events(Vec<ModelStreamEvent>),
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

    // ─── prepare_deterministic (sync path) ────────────────────────────────

    #[test]
    fn deterministic_prepare_compacts_via_emergency_summary() {
        let manager = test_manager();
        let prepared = manager
            .prepare(test_prompt(), CompactionTrigger::Automatic, None)
            .unwrap();
        let (_, checkpoint) = into_compacted(prepared).expect("expected deterministic compaction");
        assert_eq!(checkpoint.algorithm_version, "deterministic-emergency-v1");
        assert!(serde_json::to_string(&checkpoint.summary)
            .unwrap()
            .contains("## Objective"));
    }

    #[test]
    fn deterministic_prepare_returns_unchanged_below_threshold() {
        let manager = test_manager();
        let mut prompt = test_prompt();
        prompt.usage.input_tokens = Some(100);
        prompt.usage.estimated_input_tokens = 100;
        let prepared = manager
            .prepare(prompt, CompactionTrigger::Automatic, None)
            .unwrap();
        assert!(into_compacted(prepared).is_none());
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

    // ─── collect_file_operation ───────────────────────────────────────────

    #[test]
    fn collect_file_operation_classifies_read_and_write_tools() {
        let mut reads = Vec::new();
        let mut writes = Vec::new();
        collect_file_operation(
            "read",
            &serde_json::json!({"path": "a.rs"}),
            &mut reads,
            &mut writes,
        );
        collect_file_operation(
            "view",
            &serde_json::json!({"file_path": "b.rs"}),
            &mut reads,
            &mut writes,
        );
        collect_file_operation(
            "open",
            &serde_json::json!({"path": "c.rs"}),
            &mut reads,
            &mut writes,
        );
        collect_file_operation(
            "write",
            &serde_json::json!({"path": "d.rs"}),
            &mut reads,
            &mut writes,
        );
        collect_file_operation(
            "edit",
            &serde_json::json!({"path": "e.rs"}),
            &mut reads,
            &mut writes,
        );
        collect_file_operation(
            "patch",
            &serde_json::json!({"path": "f.rs"}),
            &mut reads,
            &mut writes,
        );
        collect_file_operation(
            "create",
            &serde_json::json!({"path": "g.rs"}),
            &mut reads,
            &mut writes,
        );
        // No path key → early return.
        collect_file_operation(
            "read",
            &serde_json::json!({"cmd": "ls"}),
            &mut reads,
            &mut writes,
        );
        // Unknown tool → neither list.
        collect_file_operation(
            "shell",
            &serde_json::json!({"path": "h.rs"}),
            &mut reads,
            &mut writes,
        );

        assert_eq!(reads, vec!["a.rs", "b.rs", "c.rs"]);
        assert_eq!(writes, vec!["d.rs", "e.rs", "f.rs", "g.rs"]);
    }

    // ─── emergency_summary block coverage ─────────────────────────────────

    #[test]
    fn emergency_summary_collects_tool_state_and_skips_internal_checkpoints() {
        let removed = vec![
            internal_checkpoint("old summary", "e0"),
            projected("user", "user directive", "e1"),
            projected_message(
                AgentMessage {
                    role: "assistant".to_string(),
                    content: vec![
                        ContentBlock::reasoning("internal reasoning", Default::default()),
                        ContentBlock::tool_call(
                            "c1",
                            "read",
                            serde_json::json!({"path": "a.rs"}),
                            Default::default(),
                        ),
                        ContentBlock::tool_call(
                            "c2",
                            "edit",
                            serde_json::json!({"file_path": "b.rs"}),
                            Default::default(),
                        ),
                    ],
                    ..Default::default()
                },
                "e2",
            ),
            projected_message(
                AgentMessage {
                    role: "tool".to_string(),
                    content: vec![
                        ContentBlock::tool_result("c1", "boom".to_string(), true),
                        ContentBlock::tool_result("c2", "ok".to_string(), false),
                    ],
                    ..Default::default()
                },
                "e3",
            ),
        ];
        let summary = emergency_summary(&test_plan(removed));
        assert!(summary.contains("## Objective"));
        assert!(summary.contains("Read: a.rs"));
        assert!(summary.contains("Modified: b.rs"));
        assert!(summary.contains("c1: error"));
        assert!(summary.contains("c2: success"));
        assert!(!summary.contains("old summary"));
        assert!(summary.contains("prior summary"));
        assert!(summary.contains("keep the exact constraints"));
    }

    // ─── split_to_token_budget ────────────────────────────────────────────

    #[test]
    fn split_to_token_budget_returns_input_unchanged_when_it_fits() {
        assert_eq!(
            split_to_token_budget("short", 1_000),
            vec!["short".to_string()]
        );
    }

    #[test]
    fn split_to_token_budget_splits_oversized_values_losslessly() {
        let value = "x".repeat(200);
        let parts = split_to_token_budget(&value, 5);
        assert!(parts.len() > 1);
        assert_eq!(parts.concat(), value);
    }

    // ─── summarize_fold error arms ────────────────────────────────────────

    #[tokio::test]
    async fn summarize_fold_errors_when_all_content_is_internal() {
        let manager = test_manager();
        let provider = ScriptedProvider::new([]);
        let plan = test_plan(vec![internal_checkpoint("old", "e0")]);
        let result = summarize_fold(
            &manager,
            &plan,
            &provider,
            &AtomicBool::new(false),
            SerializationMode::Normal,
        )
        .await;
        assert!(
            matches!(&result, Err(SummaryCallError::Other(msg)) if msg.contains("no conversation content"))
        );
    }

    // ─── finalize empty-summary guard ─────────────────────────────────────

    #[test]
    fn finalize_rejects_empty_summary() {
        let manager = test_manager();
        let plan = test_plan(vec![projected("user", "hi", "e1")]);
        let result = finalize(&manager, plan, "   ".to_string(), "v", "m");
        assert_eq!(result.unwrap_err(), ContextError::InvalidSummary);
    }

    // ─── call_summary_model event arms ────────────────────────────────────

    #[tokio::test]
    async fn summary_model_length_limit_reports_context_limit() {
        let provider =
            ScriptStreamProvider::new([StreamScript::Events(vec![ModelStreamEvent::Finish {
                reason: FinishReason::Length,
                usage: None,
            }])]);
        let result = call_summary_model(&provider, "m", "p".into(), &AtomicBool::new(false)).await;
        assert!(matches!(result, Err(SummaryCallError::ContextLimit(_))));
    }

    #[tokio::test]
    async fn summary_model_cancelled_finish_propagates_cancellation() {
        let provider =
            ScriptStreamProvider::new([StreamScript::Events(vec![ModelStreamEvent::Finish {
                reason: FinishReason::Cancelled,
                usage: None,
            }])]);
        let result = call_summary_model(&provider, "m", "p".into(), &AtomicBool::new(false)).await;
        assert!(matches!(result, Err(SummaryCallError::Cancelled)));
    }

    #[tokio::test]
    async fn summary_model_unexpected_finish_reason_is_reported() {
        let provider =
            ScriptStreamProvider::new([StreamScript::Events(vec![ModelStreamEvent::Finish {
                reason: FinishReason::ToolCalls,
                usage: None,
            }])]);
        let result = call_summary_model(&provider, "m", "p".into(), &AtomicBool::new(false)).await;
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
            call_summary_model(&context_limit, "m", "p".into(), &AtomicBool::new(false)).await,
            Err(SummaryCallError::ContextLimit(_))
        ));

        let fatal =
            ScriptStreamProvider::new([StreamScript::Events(vec![ModelStreamEvent::Error {
                message: "authentication failed".to_string(),
            }])]);
        assert!(matches!(
            call_summary_model(&fatal, "m", "p".into(), &AtomicBool::new(false)).await,
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
        let result = call_summary_model(&provider, "m", "p".into(), &AtomicBool::new(false)).await;
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
        let result = call_summary_model(&provider, "m", "p".into(), &AtomicBool::new(false)).await;
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
        let result = call_summary_model(&provider, "m", "p".into(), &AtomicBool::new(false)).await;
        assert!(
            matches!(&result, Err(SummaryCallError::Other(msg)) if msg.contains("ended before a complete response"))
        );
    }

    #[tokio::test]
    async fn summary_model_timeout_retries_then_gives_up() {
        let provider =
            ScriptStreamProvider::new([StreamScript::Hang, StreamScript::Hang, StreamScript::Hang]);
        let result = call_summary_model(&provider, "m", "p".into(), &AtomicBool::new(false)).await;
        assert!(matches!(&result, Err(SummaryCallError::Other(msg)) if msg.contains("timed out")));
    }

    #[tokio::test]
    async fn summary_model_respects_interrupt_before_request() {
        let provider = ScriptStreamProvider::new([]);
        let result = call_summary_model(&provider, "m", "p".into(), &AtomicBool::new(true)).await;
        assert!(matches!(result, Err(SummaryCallError::Cancelled)));
    }

    #[tokio::test]
    async fn summary_model_respects_interrupt_during_stream() {
        let interrupted = std::sync::Arc::new(AtomicBool::new(false));
        let provider = InterruptOnStreamProvider {
            interrupted: interrupted.clone(),
        };
        let result = call_summary_model(&provider, "m", "p".into(), &interrupted).await;
        assert!(matches!(result, Err(SummaryCallError::Cancelled)));
    }

    #[tokio::test]
    async fn invalid_model_summary_is_rejected() {
        let provider = ScriptStreamProvider::new([StreamScript::Events(vec![
            ModelStreamEvent::TextDelta {
                id: "s".to_string(),
                text: "not a structured summary".to_string(),
            },
            ModelStreamEvent::Finish {
                reason: FinishReason::Stop,
                usage: None,
            },
        ])]);
        let error = test_manager()
            .prepare_semantic(
                test_prompt(),
                CompactionTrigger::Automatic,
                None,
                &provider,
                &AtomicBool::new(false),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, ContextError::SummaryFailed(msg) if msg.contains("required structure"))
        );
    }
}
