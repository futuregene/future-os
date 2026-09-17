//! Experiment driver for the two production compaction strategies.
//!
//! It calls the same entry points the runtime calls, with the same call shape:
//!
//! * the window defaults to the model registry's declared `context_window`, not a
//!   simulated one;
//! * the output reservation defaults to `effective_max_tokens`, as a real turn reserves;
//! * `set_request_budget` runs before the strategy, with the session's system prompt and
//!   the real tool definitions — the runtime calls it at the call site, not inside the
//!   strategy, so a driver that omits it sees a budget with no fixed input and no output
//!   reserve and therefore compacts later than production does;
//! * the trigger/phase default to what an ordinary pre-turn compaction uses.
//!
//! The system prompt and tools are inputs rather than guesses because only the session
//! knows them; capture them from a real turn (see `scripts/abc_experiment/capture_shape.py`)
//! and pass them with `--system-prompt-file` / `--tools-file`. Without them the driver
//! still runs, but the budget and the summary request are not the shape production sends.
//!
//! Strategies:
//!   * `deterministic` — `prepare_evidence`; no model call at all.
//!   * `summarized`    — `prepare_evidence_with_summary`; the same projection plus a
//!     model-written handoff summary, and the runtime default.

use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

use future_agent::agent::history_recall;
use future_agent::compaction::{
    context_token_budgets, project_prompt_context, set_request_budget, CompactionPhase,
    CompactionTrigger, ContextManager, ContextPreparation,
};
use future_agent::llm::schema::{ModelRequest, ModelStreamEvent};
use future_agent::types::{AgentMessage, ContentBlock, LLMProvider, ToolDef};

#[derive(Default)]
struct Totals {
    calls: AtomicUsize,
    input: AtomicUsize,
    output: AtomicUsize,
    cache_read: AtomicUsize,
    cache_write: AtomicUsize,
    cost: Mutex<f64>,
}

static TOTALS: std::sync::OnceLock<Arc<Totals>> = std::sync::OnceLock::new();

/// Wraps the Agent's own client so the harness can report real usage without changing
/// streaming behaviour or reading credentials itself.
struct Observed {
    inner: future_agent::llm::Client,
    /// Forwarding tasks must be joined before usage is read: the last SSE frame may still
    /// be in flight when the summarizer returns.
    handles: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// The resolved requests this driver sent, for auditing.
    requests: Mutex<Vec<Value>>,
}

impl Observed {
    async fn observe(
        &self,
        request: ModelRequest,
        max_output_tokens: Option<i32>,
    ) -> Result<ReceiverStream<ModelStreamEvent>> {
        let totals = TOTALS.get().context("totals uninitialized")?.clone();
        self.requests.lock().push(json!({
            "system_prompt": request.system_prompt,
            "messages": request.messages,
            "tools": request.tools,
            "max_output_tokens": max_output_tokens,
        }));
        let stream = match max_output_tokens {
            Some(limit) => {
                self.inner
                    .stream_model_with_output_limit(request, limit)
                    .await?
            }
            None => self.inner.stream_model(request).await?,
        };
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let handle = tokio::spawn(async move {
            let mut stream = stream;
            while let Some(event) = stream.next().await {
                if let ModelStreamEvent::Finish {
                    usage: Some(usage), ..
                } = &event
                {
                    totals
                        .input
                        .fetch_add(usage.prompt_tokens.max(0) as usize, Ordering::Relaxed);
                    totals
                        .output
                        .fetch_add(usage.completion_tokens.max(0) as usize, Ordering::Relaxed);
                    totals.cache_read.fetch_add(
                        usage.cache_read_tokens.unwrap_or(0).max(0) as usize,
                        Ordering::Relaxed,
                    );
                    totals.cache_write.fetch_add(
                        usage.cache_write_tokens.unwrap_or(0).max(0) as usize,
                        Ordering::Relaxed,
                    );
                    if let Some(credit) = usage.credit_cost {
                        if credit.is_finite() && credit > 0.0 {
                            *totals.cost.lock() += credit;
                        }
                    }
                }
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });
        self.handles.lock().push(handle);
        Ok(ReceiverStream::new(rx))
    }
}

#[async_trait::async_trait]
impl LLMProvider for Observed {
    async fn stream_model(
        &self,
        request: ModelRequest,
    ) -> Result<ReceiverStream<ModelStreamEvent>> {
        self.observe(request, None).await
    }

    async fn stream_model_with_output_limit(
        &self,
        request: ModelRequest,
        max_output_tokens: i32,
    ) -> Result<ReceiverStream<ModelStreamEvent>> {
        self.observe(request, Some(max_output_tokens)).await
    }
}

fn block(message: &AgentMessage) -> String {
    let mut parts = Vec::new();
    for content in &message.content {
        match content {
            ContentBlock::Text { text } => parts.push(text.clone()),
            ContentBlock::ToolCall { id, name, args, .. } => {
                parts.push(format!("[Assistant tool call {id}]: {name}({args})"))
            }
            ContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => parts.push(format!(
                "[Tool {} {tool_call_id}]: {content}",
                if *is_error { "error" } else { "result" }
            )),
            _ => {}
        }
    }
    parts.join("\n")
}

fn message(record: &Value) -> Option<AgentMessage> {
    let id = record.get("id")?.as_str()?;
    let kind = record.get("kind")?.as_str()?;
    let role = match kind {
        "tool_result" => "tool".to_string(),
        _ => record.get("role")?.as_str()?.to_string(),
    };
    let mut value = AgentMessage {
        role,
        ..Default::default()
    };
    value.content = match kind {
        "tool_call" => vec![ContentBlock::tool_call(
            record.get("call")?.as_str()?,
            record.get("tool").and_then(Value::as_str).unwrap_or("read"),
            json!({"path": record.get("path").cloned().unwrap_or(Value::Null)}),
            Default::default(),
        )],
        "tool_result" => vec![ContentBlock::tool_result(
            record.get("call")?.as_str()?,
            record.get("text")?.as_str()?,
            record
                .get("error")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        )],
        _ => vec![ContentBlock::text(record.get("text")?.as_str()?)],
    };
    value
        .metadata
        .get_or_insert_with(serde_json::Map::new)
        .insert(AgentMessage::JOURNAL_ENTRY_ID_KEY.to_string(), json!(id));
    Some(value)
}

/// Rebuild the message array the way a provider requires it, not the way the journal
/// stores it: several tool calls can share one entry, results arrive in later entries,
/// and an assistant message with `tool_calls` must be followed immediately by the tool
/// messages answering those ids.
fn messages_from_records(records: &[Value]) -> Vec<AgentMessage> {
    let mut grouped: Vec<(i64, Vec<AgentMessage>)> = Vec::new();
    for record in records {
        let position = record
            .get("position")
            .or_else(|| record.get("order"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if let Some(message) = message(record) {
            match grouped.last_mut() {
                Some((last, bucket)) if *last == position => bucket.push(message),
                _ => grouped.push((position, vec![message])),
            }
        }
    }

    struct Turn {
        role: String,
        entry_id: String,
        text: String,
        calls: Vec<ContentBlock>,
    }
    let mut turns: Vec<Turn> = Vec::new();
    let mut results: std::collections::HashMap<String, Vec<(String, String, bool)>> =
        std::collections::HashMap::new();
    // A result arrives in a later entry than its call, so collect every result first and
    // attach them in the second pass.
    for (_, group) in &grouped {
        for m in group {
            for c in &m.content {
                if let ContentBlock::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                } = c
                {
                    let source = m.journal_entry_id().map(str::to_string).unwrap_or_default();
                    results.entry(tool_call_id.clone()).or_default().push((
                        content.clone(),
                        source,
                        *is_error,
                    ));
                }
            }
        }
    }
    for (_, group) in grouped {
        let role = group[0].role.clone();
        let entry_id = group[0]
            .journal_entry_id()
            .map(str::to_string)
            .unwrap_or_default();
        if role == "tool" {
            continue;
        }
        let mut text = Vec::new();
        let mut calls = Vec::new();
        for m in &group {
            for c in &m.content {
                match c {
                    ContentBlock::Text { text: body } => text.push(body.clone()),
                    other @ ContentBlock::ToolCall { .. } => calls.push(other.clone()),
                    _ => {}
                }
            }
        }
        if text.is_empty() && calls.is_empty() {
            continue;
        }
        turns.push(Turn {
            role,
            entry_id,
            text: text.join("\n"),
            calls,
        });
    }

    let mut messages: Vec<AgentMessage> = Vec::new();
    for turn in turns {
        let mut content = Vec::new();
        if !turn.text.is_empty() {
            content.push(ContentBlock::text(turn.text));
        }
        let mut answered: Vec<(ContentBlock, String)> = Vec::new();
        for call in &turn.calls {
            if let ContentBlock::ToolCall { id, .. } = call {
                if let Some(bodies) = results.remove(id) {
                    content.push(call.clone());
                    for (body, source, is_error) in bodies {
                        answered.push((ContentBlock::tool_result(id, body, is_error), source));
                    }
                }
            }
        }
        let mut speaker = AgentMessage {
            role: turn.role,
            content,
            ..Default::default()
        };
        speaker
            .metadata
            .get_or_insert_with(serde_json::Map::new)
            .insert(
                AgentMessage::JOURNAL_ENTRY_ID_KEY.to_string(),
                json!(turn.entry_id),
            );
        if !speaker.content.is_empty() {
            messages.push(speaker);
        }
        messages.extend(answered.into_iter().map(|(block, source)| {
            let mut m = AgentMessage {
                role: "tool".to_string(),
                ..Default::default()
            };
            m.content = vec![block];
            if !source.is_empty() {
                m.metadata.get_or_insert_with(serde_json::Map::new).insert(
                    AgentMessage::JOURNAL_ENTRY_ID_KEY.to_string(),
                    json!(source),
                );
            }
            m
        }));
    }
    messages
}

fn projection_json(prompt: &future_agent::compaction::PromptContext) -> Vec<Value> {
    prompt
        .messages
        .iter()
        .map(|item| {
            json!({
                "role": item.message.role,
                "text": block(&item.message),
                "sourceEntryIds": item.source_entry_ids,
                "anchor": item.message.metadata.as_ref()
                    .and_then(|m| m.get("internal_context_anchor"))
                    .and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

fn parse_trigger(value: &str) -> Result<CompactionTrigger> {
    Ok(match value {
        "automatic" => CompactionTrigger::Automatic,
        "manual" => CompactionTrigger::Manual,
        "downshift" => CompactionTrigger::ModelContextDownshift,
        "provider-limit" => CompactionTrigger::ProviderContextLimit,
        other => anyhow::bail!("unknown trigger {other}"),
    })
}

fn parse_phase(value: &str) -> Result<CompactionPhase> {
    Ok(match value {
        "preturn" => CompactionPhase::PreTurn,
        "midturn" => CompactionPhase::MidTurn,
        "standalone" => CompactionPhase::Standalone,
        other => anyhow::bail!("unknown phase {other}"),
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let totals = TOTALS.get_or_init(|| Arc::new(Totals::default())).clone();
    let mut args = std::env::args().skip(1);
    let mut records_path = None;
    let mut model = None;
    let mut strategy = None;
    #[allow(unused_assignments)]
    let mut window: Option<i32> = None;
    let mut output_reserve: Option<i32> = None;
    let mut previous: Option<Value> = None;
    let mut dump_messages = false;
    let mut thinking_level: Option<String> = None;
    let mut base_system_prompt: Option<String> = None;
    let mut session_id = String::new();
    let mut recall_allowed = true;
    let mut tools: Vec<ToolDef> = Vec::new();
    let mut instructions: Option<String> = None;
    // The runtime compacts an ordinary turn before it sends it, in the pre-turn phase.
    let mut trigger = CompactionTrigger::Automatic;
    let mut phase = CompactionPhase::PreTurn;
    let mut print_limits = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--records" => records_path = args.next(),
            "--model" => model = args.next(),
            "--strategy" => strategy = args.next(),
            "--window" => window = Some(args.next().context("window")?.parse()?),
            "--output-reserve" => {
                output_reserve = Some(args.next().context("output reserve")?.parse()?)
            }
            "--previous" => {
                previous = Some(serde_json::from_str(&args.next().context("previous")?)?)
            }
            "--dump-messages" => dump_messages = true,
            "--print-limits" => print_limits = true,
            "--thinking-level" => thinking_level = Some(args.next().context("thinking level")?),
            "--instructions" => instructions = Some(args.next().context("instructions")?),
            "--trigger" => trigger = parse_trigger(&args.next().context("trigger")?)?,
            "--phase" => phase = parse_phase(&args.next().context("phase")?)?,
            "--base-system-prompt-file" => {
                base_system_prompt = Some(std::fs::read_to_string(
                    args.next().context("base system prompt file")?,
                )?);
            }
            "--session-id" => session_id = args.next().context("session id")?,
            "--no-recall" => recall_allowed = false,
            "--tools-file" => {
                tools = serde_json::from_str(&std::fs::read_to_string(
                    args.next().context("tools file")?,
                )?)
                .context("tools file must be a JSON array of tool definitions")?;
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    let model_id = model.context("--model is required")?;
    // `--print-limits` reports what the registry and the budget rules imply for this model
    // and tool set, so a harness can schedule against production's own numbers instead of
    // hard-coding a window.
    if print_limits {
        let registry = future_agent::models::Registry::new();
        let resolved = registry
            .resolve(&model_id)
            .with_context(|| format!("model {model_id} is not in the registry"))?;
        let window = resolved.context_window.max(1);
        let output_reserve = future_agent::models::effective_max_tokens(&resolved);
        let (reserve_tokens, keep_recent_tokens) = context_token_budgets(window);
        let margin = 2_048u64.min(window.max(1) as u64 / 16);
        let input_limit = (window.max(1) as u64)
            .saturating_sub(output_reserve.max(0) as u64)
            .saturating_sub(margin);
        println!(
            "{}",
            serde_json::to_string(&json!({
                "model": model_id,
                "window": window,
                "output_reserve": output_reserve,
                "reserve": reserve_tokens,
                "keep_recent": keep_recent_tokens,
                "effective_trigger": (window.max(1) as u64)
                    .saturating_sub(reserve_tokens.max(0) as u64)
                    .min(input_limit),
                "input_limit": input_limit,
                // Rates, so a cost analysis prices the run with the same numbers the
                // runtime bills with instead of hard-coding them.
                "rates_per_million": {
                    "input": resolved.cost.input,
                    "output": resolved.cost.output,
                    "cache_read": resolved.cost.cache_read,
                    "cache_write": resolved.cost.cache_write,
                },
            }))?
        );
        return Ok(());
    }
    let records: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(
        records_path.context("--records is required")?,
    )?)?;

    // Resolve the production limits from the same registry the agent reads, so a run
    // cannot silently use a window or an output reservation the runtime would not.
    let registry = Arc::new(parking_lot::RwLock::new(
        future_agent::models::Registry::new(),
    ));
    let resolved = registry
        .read()
        .resolve(&model_id)
        .with_context(|| format!("model {model_id} is not in the registry"))?;
    let registry_window = resolved.context_window.max(1);
    let registry_output = future_agent::models::effective_max_tokens(&resolved);
    let window = window.unwrap_or(registry_window);
    let output_reserve = output_reserve.unwrap_or(registry_output);

    let messages = messages_from_records(&records);
    anyhow::ensure!(!messages.is_empty(), "no messages built from records");
    // `--dump-messages` is the shared normalizer the harness runs every raw segment
    // through, so it must not require a strategy.
    if dump_messages {
        println!("{}", serde_json::to_string(&messages)?);
        return Ok(());
    }
    let strategy = strategy.context("--strategy is required")?;
    anyhow::ensure!(
        matches!(strategy.as_str(), "deterministic" | "summarized"),
        "strategy must be deterministic or summarized, not {strategy}"
    );
    let checkpoint = match &previous {
        Some(value) if !value.is_null() => {
            Some(serde_json::from_value(value.clone()).context("previous checkpoint")?)
        }
        _ => None,
    };

    let mut prompt = project_prompt_context(&messages, checkpoint.as_ref(), None, window as u64);
    // Reproduce the two prompts the runtime derives, through the same function:
    //   * the budget prompt always reserves the recall guidance, so committing the first
    //     checkpoint cannot make the next request exceed admission;
    //   * the outgoing prompt carries it only once a checkpoint exists.
    let base = base_system_prompt.as_deref().unwrap_or_default();
    let can_recall = recall_allowed && tools.iter().any(|tool| tool.function.name == "shell");
    let budget_system = history_recall::system_prompt(base, &session_id, can_recall);
    let request_system =
        history_recall::system_prompt(base, &session_id, can_recall && checkpoint.is_some());
    // The runtime calls this at the call site before invoking the strategy. Skipping it
    // leaves `fixed_input_tokens` and `output_reserve_tokens` at zero, which raises the
    // effective limit and compacts later than production.
    set_request_budget(&mut prompt, &budget_system, &tools, output_reserve);
    let (reserve_tokens, keep_recent_tokens) = context_token_budgets(window);
    let manager = ContextManager {
        enabled: true,
        reserve_tokens,
        keep_recent_tokens,
        context_window: window,
        model: model_id.clone(),
    };
    let mut inner = future_agent::llm::Client::from_live_model(model_id.clone(), registry);
    if let Some(level) = &thinking_level {
        inner = inner.with_thinking_level(level);
    }
    let provider = Observed {
        inner,
        handles: Mutex::new(Vec::new()),
        requests: Mutex::new(Vec::new()),
    };
    let before = prompt.usage.estimated_input_tokens;
    let fixed_before = prompt.usage.fixed_input_tokens;
    let output_before = prompt.usage.output_reserve_tokens;
    // Mirrors `ContextManager::effective_trigger`, which is crate-private: the caller
    // only needs it to explain why a run did or did not compact.
    let margin = 2_048u64.min(window.max(1) as u64 / 16);
    let input_limit = (window.max(1) as u64)
        .saturating_sub(output_reserve.max(0) as u64)
        .saturating_sub(margin);
    let effective_trigger = (window.max(1) as u64)
        .saturating_sub(reserve_tokens.max(0) as u64)
        .min(input_limit);

    let prepared = if strategy == "deterministic" {
        manager.prepare_evidence(
            prompt,
            &messages,
            trigger,
            phase,
            instructions.as_deref(),
            &AtomicBool::new(false),
            None,
        )?
    } else {
        manager
            .prepare_evidence_with_summary(
                prompt,
                &messages,
                trigger,
                phase,
                instructions.as_deref(),
                &AtomicBool::new(false),
                None,
                Some(&provider),
                Some(&request_system),
                &tools,
                None,
                Some(&|reason: &str| eprintln!("SUMMARY_FALLBACK: {reason}")),
            )
            .await?
    };
    let pending = std::mem::take(&mut *provider.handles.lock());
    for handle in pending {
        let _ = handle.await;
    }
    let (projection, checkpoint, after) = match prepared {
        ContextPreparation::Compacted { prompt, checkpoint } => (
            projection_json(&prompt),
            Some(serde_json::to_value(&*checkpoint)?),
            prompt.usage.estimated_input_tokens,
        ),
        ContextPreparation::Unchanged { prompt } => (
            projection_json(&prompt),
            None,
            prompt.usage.estimated_input_tokens,
        ),
    };
    println!(
        "{}",
        serde_json::to_string(&json!({
            "strategy": strategy,
            "projection": projection,
            "checkpoint": checkpoint,
            "estimated_before": before,
            "estimated_after": after,
            "window": window,
            "registry_window": registry_window,
            "output_reserve": output_reserve,
            "registry_output_reserve": registry_output,
            "reserve": reserve_tokens,
            "keep_recent": keep_recent_tokens,
            "effective_trigger": effective_trigger,
            "trigger": trigger,
            "phase": phase,
            "budget": {
                "fixed_input_tokens": fixed_before,
                "output_reserve_tokens": output_before,
            },
            "base_system_prompt_chars": base.len(),
            "budget_system_prompt_chars": budget_system.len(),
            "request_system_prompt_chars": request_system.len(),
            "session_id": session_id,
            "recall_guidance_in_request": request_system.len() != base.len(),
            "tool_count": tools.len(),
            "thinking_level": thinking_level,
            "model_requests": totals.calls.load(Ordering::Relaxed),
            "logical_requests": *provider.requests.lock(),
            "usage": {
                "input_tokens": totals.input.load(Ordering::Relaxed),
                "output_tokens": totals.output.load(Ordering::Relaxed),
                "cache_read_tokens": totals.cache_read.load(Ordering::Relaxed),
                "cache_write_tokens": totals.cache_write.load(Ordering::Relaxed),
                "cost": *totals.cost.lock(),
            },
        }))?
    );
    Ok(())
}
