//! Measurement driver for the strategy a checkout already deploys.
//!
//! Copy it into the checkout whose default compaction you want to measure and build it
//! there. It calls the checkout's own default entry point with the same call shape the
//! runtime uses: the window comes from the model registry, and whatever budget helper the
//! checkout exposes is used as-is rather than approximated from another checkout's rules.
//!
//! Build in the target checkout:
//!   cp scripts/abc_experiment/abc_main_probe.rs <checkout>/agent/examples/
//!   (cd <checkout> && cargo build -p future-agent --example abc_main_probe)
//!
//! Only `prepare_semantic` and `context_token_budgets` are required, so it builds against
//! a checkout from before the evidence-index work as well as the measurement checkout.

use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

use future_agent::compaction::{
    context_token_budgets, project_prompt_context, CompactionPhase, CompactionTrigger,
    ContextManager, ContextPreparation,
};
use future_agent::llm::schema::{ModelRequest, ModelStreamEvent};
use future_agent::types::{AgentMessage, ContentBlock, LLMProvider};

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

struct Observed {
    inner: future_agent::llm::Client,
    handles: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    requests: Mutex<Vec<Value>>,
}

impl Observed {
    async fn observe(&self, request: ModelRequest) -> Result<ReceiverStream<ModelStreamEvent>> {
        let totals = TOTALS.get().context("totals uninitialized")?.clone();
        self.requests.lock().push(json!({
            "system_prompt": request.system_prompt,
            "messages": request.messages,
            "tools": request.tools,
        }));
        let stream = self.inner.stream_model(request).await?;
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
    async fn stream_model(&self, request: ModelRequest) -> Result<ReceiverStream<ModelStreamEvent>> {
        self.observe(request).await
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
            })
        })
        .collect()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let totals = TOTALS.get_or_init(|| Arc::new(Totals::default())).clone();
    let mut args = std::env::args().skip(1);
    let mut records_path = None;
    let mut model = None;
    let mut window: Option<i32> = None;
    let mut previous: Option<Value> = None;
    let mut dump_messages = false;
    let mut thinking_level: Option<String> = None;
    let mut base_system_prompt: Option<String> = None;
    let mut session_id = String::new();
    // The runtime compacts an ordinary turn before it sends it, and uses MidTurn once the
    // turn has already produced tool calls (`turn == 0` is PreTurn).
    let mut trigger = CompactionTrigger::Automatic;
    let mut phase = CompactionPhase::PreTurn;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--records" => records_path = args.next(),
            "--model" => model = args.next(),
            "--window" => window = Some(args.next().context("window")?.parse()?),
            "--previous" => {
                previous = Some(serde_json::from_str(&args.next().context("previous")?)?)
            }
            "--dump-messages" => dump_messages = true,
            "--thinking-level" => thinking_level = Some(args.next().context("thinking level")?),
            "--base-system-prompt-file" => {
                base_system_prompt = Some(std::fs::read_to_string(
                    args.next().context("base system prompt file")?,
                )?);
            }
            "--session-id" => session_id = args.next().context("session id")?,
            "--trigger" => match args.next().context("trigger")?.as_str() {
                "automatic" => trigger = CompactionTrigger::Automatic,
                "manual" => trigger = CompactionTrigger::Manual,
                other => anyhow::bail!("unknown trigger {other}"),
            },
            "--phase" => match args.next().context("phase")?.as_str() {
                "preturn" => phase = CompactionPhase::PreTurn,
                "midturn" => phase = CompactionPhase::MidTurn,
                "standalone" => phase = CompactionPhase::Standalone,
                other => anyhow::bail!("unknown phase {other}"),
            },
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    let records: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(
        records_path.context("--records is required")?,
    )?)?;
    let model_id = model.context("--model is required")?;
    let registry = Arc::new(parking_lot::RwLock::new(
        future_agent::models::Registry::new(),
    ));
    let resolved = registry
        .read()
        .resolve(&model_id)
        .with_context(|| format!("model {model_id} is not in the registry"))?;
    let window = window.unwrap_or(resolved.context_window.max(1));

    let messages = messages_from_records(&records);
    anyhow::ensure!(!messages.is_empty(), "no messages built from records");
    if dump_messages {
        println!("{}", serde_json::to_string(&messages)?);
        return Ok(());
    }
    let checkpoint = match &previous {
        Some(value) if !value.is_null() => {
            Some(serde_json::from_value(value.clone()).context("previous checkpoint")?)
        }
        _ => None,
    };
    let _ = (&base_system_prompt, &session_id);

    let prompt = project_prompt_context(&messages, checkpoint.as_ref(), None, window as u64);
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

    let prepared = manager
        .prepare_semantic_with_phase(
            prompt,
            trigger,
            phase,
            None,
            &provider,
            &AtomicBool::new(false),
        )
        .await?;
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
            "strategy": "main",
            "projection": projection,
            "checkpoint": checkpoint,
            "estimated_before": before,
            "estimated_after": after,
            "window": window,
            "reserve": reserve_tokens,
            "keep_recent": keep_recent_tokens,
            "trigger": trigger,
            "phase": phase,
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
