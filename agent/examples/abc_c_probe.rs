//! Dump the projection produced by the *real* C path in this checkout.
//!
//! The A/B/C experiment contexts were rendered by an approximation in the Python
//! harness (last N tool results, fixed character clipping). C is our own code, so it
//! can be measured directly: `prepare_evidence` is synchronous and takes no provider,
//! which means this driver makes **no model calls at all** and costs nothing.
//!
//! Reads the same fixture-shaped records the harness uses (each record carries an
//! `id`, which becomes the journal entry id) and prints the projection plus the
//! checkpoint, in the shape `projection_json` already emits.
//!
//! Usage:
//!   abc_c_probe --records FILE --model MODEL --window N [--previous CHECKPOINT_JSON]

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;

use future_agent::compaction::{
    context_token_budgets, project_prompt_context, CompactionPhase, CompactionTrigger,
    ContextManager, ContextPreparation,
};
use future_agent::types::{AgentMessage, ContentBlock};

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
                "sourceEntryIds": item.source_entry_ids,
            })
        })
        .collect()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut records_path = None;
    let mut model = None;
    let mut window: i32 = 32_000;
    let mut previous: Option<Value> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--records" => records_path = args.next(),
            "--model" => model = args.next(),
            "--window" => window = args.next().context("window")?.parse()?,
            "--previous" => previous = Some(serde_json::from_str(&args.next().context("previous")?)?),
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
    let prompt = project_prompt_context(&messages, checkpoint.as_ref(), None, window as u64);
    let (reserve_tokens, keep_recent_tokens) = context_token_budgets(window);
    let manager = ContextManager {
        enabled: true,
        reserve_tokens,
        keep_recent_tokens,
        context_window: window,
        model: model.clone(),
    };
    let before = prompt.usage.estimated_input_tokens;
    let prepared = manager.prepare_evidence(
        prompt,
        &messages,
        CompactionTrigger::Manual,
        CompactionPhase::Standalone,
        None,
        &AtomicBool::new(false),
        None,
    )?;
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
            "projection": projection,
            "checkpoint": checkpoint,
            "estimated_before": before,
            "estimated_after": after,
            "window": window,
            "model_requests": 0,
        }))?
    );
    Ok(())
}
