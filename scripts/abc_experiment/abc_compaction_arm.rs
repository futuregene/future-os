//! Experiment driver: dump the projection produced by *this checkout's*
//! compaction code, so the same harness can measure any branch's strategy.
//!
//! Build it in the worktree whose strategy you want to measure; the file itself
//! is strategy-agnostic. It performs real model calls for summary generation
//! using the local Agent registry (no credentials are printed or copied).
//!
//! The call below uses the legacy `prepare_semantic` entry point, so this driver only
//! builds against a checkout that still exposes it — that is, one from before the commit
//! that retired the legacy entry points (see `docs/compaction-prompts.md`). Measuring a
//! checkout whose default is C or C3 with `abc_c_probe` / `abc_c3_probe` is preferable:
//! they exercise the production entry points, and their recorded `driver_sha256` stays
//! meaningful.

use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

use future_agent::compaction::{
    project_prompt_context, CompactionTrigger, ContextManager, ContextPreparation,
};
use future_agent::llm::schema::{ModelRequest, ModelStreamEvent};
use future_agent::types::{AgentMessage, ContentBlock, LLMProvider};

#[derive(Default)]
struct Totals {
    calls: AtomicUsize,
    input: AtomicUsize,
    output: AtomicUsize,
    cost: Mutex<f64>,
}

static TOTALS: std::sync::OnceLock<Arc<Totals>> = std::sync::OnceLock::new();

/// Wraps the Agent's own client so the harness can report real usage/cost
/// without changing streaming behaviour or reading credentials itself.
struct Observed {
    inner: future_agent::llm::Client,
    /// Forwarding tasks must be joined before usage is read: the last SSE frame
    /// may still be in flight when the summarizer returns.
    handles: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

#[async_trait::async_trait]
impl LLMProvider for Observed {
    async fn stream_model(&self, request: ModelRequest) -> Result<ReceiverStream<ModelStreamEvent>> {
        let totals = TOTALS
            .get()
            .context("usage totals are not initialized")?
            .clone();
        totals.calls.fetch_add(1, Ordering::Relaxed);
        let stream = self.inner.stream_model(request).await?;
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let handle = tokio::spawn(async move {
            let mut stream = stream;
            while let Some(event) = stream.next().await {
                if std::env::var("ABC_DEBUG_EVENTS").is_ok() {
                    eprintln!("EVENT {event:?}");
                }
                if let ModelStreamEvent::Finish { usage: Some(usage), .. } = &event {
                    totals
                        .input
                        .fetch_add(usage.prompt_tokens.max(0) as usize, Ordering::Relaxed);
                    totals
                        .output
                        .fetch_add(usage.completion_tokens.max(0) as usize, Ordering::Relaxed);
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
            "read",
            json!({"path": record.get("path").cloned().unwrap_or(Value::Null)}),
            Default::default(),
        )],
        "tool_result" => vec![ContentBlock::tool_result(
            record.get("call")?.as_str()?,
            record.get("text")?.as_str()?,
            record.get("error").and_then(Value::as_bool).unwrap_or(false),
        )],
        _ => vec![ContentBlock::text(record.get("text")?.as_str()?)],
    };
    value
        .metadata
        .get_or_insert_with(serde_json::Map::new)
        .insert(AgentMessage::JOURNAL_ENTRY_ID_KEY.to_string(), json!(id));
    Some(value)
}

fn projection_json(prompt: &future_agent::compaction::PromptContext) -> Vec<Value> {
    prompt
        .messages
        .iter()
        .map(|item| {
            json!({
                "role": item.message.role,
                "text": block(&item.message),
            })
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut records_path = None;
    let mut model = None;
    let mut window: u64 = 32_000;
    let mut previous: Option<Value> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--records" => records_path = args.next(),
            "--model" => model = args.next(),
            "--window" => window = args.next().context("window")?.parse()?,
            "--previous" => {
                previous = Some(serde_json::from_str(&args.next().context("previous")?)?)
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    let records: Vec<Value> = serde_json::from_str(
        &std::fs::read_to_string(records_path.context("--records is required")?)?,
    )?;
    let model = model.context("--model is required")?;
    let messages = records.iter().filter_map(message).collect::<Vec<_>>();
    anyhow::ensure!(!messages.is_empty(), "no messages built from records");

    let checkpoint = match &previous {
        Some(value) if !value.is_null() => {
            Some(serde_json::from_value(value.clone()).context("previous checkpoint")?)
        }
        _ => None,
    };
    let prompt = project_prompt_context(&messages, checkpoint.as_ref(), None, window);
    let (reserve_tokens, keep_recent_tokens) =
        future_agent::compaction::context_token_budgets(window as i32);
    let manager = ContextManager {
        enabled: true,
        reserve_tokens,
        keep_recent_tokens,
        context_window: window as i32,
        model: model.clone(),
    };
    let registry = Arc::new(parking_lot::RwLock::new(
        future_agent::models::Registry::new(),
    ));
    let totals = TOTALS.get_or_init(|| Arc::new(Totals::default()));
    let provider = Observed {
        inner: future_agent::llm::Client::from_live_model(model.clone(), registry),
        handles: Mutex::new(Vec::new()),
    };
    let before = prompt.usage.estimated_input_tokens;
    let prepared = manager
        .prepare_semantic(
            prompt,
            CompactionTrigger::Manual,
            None,
            &provider,
            &AtomicBool::new(false),
        )
        .await?;
    // Join every forwarding task before reading usage: the final SSE frame may
    // still be in flight when the summarizer returns.
    let pending = std::mem::take(&mut *provider.handles.lock());
    for handle in pending {
        let _ = handle.await;
    }
    let (projection, checkpoint, after) = match prepared {        ContextPreparation::Compacted { prompt, checkpoint } => (
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
            "projection": projection,
            "checkpoint": checkpoint,
            "usage": {
                "input_tokens": totals.input.load(Ordering::Relaxed),
                "output_tokens": totals.output.load(Ordering::Relaxed),
                "credit_cost": *totals.cost.lock(),
            },
            "model_requests": totals.calls.load(Ordering::Relaxed),
            "estimated_before": before,
            "estimated_after": after,
            "window": window,
            "reserve": reserve_tokens,
            "keep_recent": keep_recent_tokens,
        }))?
    );
    Ok(())
}
