//! Dump the projection produced by the real C3 path in this checkout.
//!
//! C3 is the runtime default: C's projection (protected user and assistant originals,
//! deterministic tool-evidence index, recent tail) **plus** a sticky model-written
//! handoff summary. `prepare_evidence_with_summary` is the production entry point, so
//! this driver exercises the production selection function on reduced frozen records.
//! It is NOT a live agent and does not reproduce the original system prompt, tools,
//! media or provider metadata. `--system-prompt-file` and `--thinking-level` make
//! those experiment choices explicit. Logical model requests are returned for audit;
//! no production prefix-cache guarantee is implied.
//!
//! Pass `--previous CHECKPOINT_JSON` to chain compactly: the previous summary is
//! handed to the next one. This permits recursive retention but does not guarantee
//! that individual facts survive every rewrite.
//!
//! Usage:
//!   abc_c3_probe --records FILE --model MODEL --window N [--previous CHECKPOINT_JSON]

use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
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

static TOTALS: OnceLock<Arc<Totals>> = OnceLock::new();

/// Wraps the Agent's own client so usage and cache counters are reported without
/// changing streaming behaviour or reading credentials.
struct Observed {
    inner: future_agent::llm::Client,
    handles: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    requests: Mutex<Vec<Value>>,
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

impl Observed {
    async fn observe(
        &self,
        request: ModelRequest,
        max_output_tokens: Option<i32>,
    ) -> Result<ReceiverStream<ModelStreamEvent>> {
        let totals = TOTALS.get().context("totals uninitialized")?.clone();
        // Bound retries before sending, so the driver can be reserved as one
        // transaction in the experiment ledger.
        anyhow::ensure!(
            totals
                .calls
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| (n < 3)
                    .then_some(n + 1))
                .is_ok(),
            "experiment summary request limit reached"
        );
        self.requests.lock().push(json!({
            "model": request.model,
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
    let mut window: i32 = 128_000;
    let mut previous: Option<Value> = None;
    let mut no_summary = false;
    let mut dump_messages = false;
    let mut thinking_level: Option<String> = None;
    let mut system_prompt: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--records" => records_path = args.next(),
            "--model" => model = args.next(),
            "--window" => window = args.next().context("window")?.parse()?,
            "--previous" => {
                previous = Some(serde_json::from_str(&args.next().context("previous")?)?)
            }
            "--no-summary" => no_summary = true,
            "--dump-messages" => dump_messages = true,
            "--thinking-level" => thinking_level = Some(args.next().context("thinking level")?),
            "--system-prompt-file" => {
                system_prompt = Some(std::fs::read_to_string(
                    args.next().context("system prompt file")?,
                )?);
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    let records: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(
        records_path.context("--records is required")?,
    )?)?;
    let model = model.context("--model is required")?;
    // A real session record is not one message: several tool calls can live in one
    // entry, and their results arrive in later entries. Emitting one message per
    // record produces `assistant(callA), assistant(callB), tool(resultA)`, which the
    // provider rejects. Group by journal position the way the agent does, then drop
    // any call whose result never arrived and any result whose call never appeared —
    // the same repair the runtime performs before sending.
    // Rebuild the array the way a provider requires it, not the way the journal stores
    // it. A real session interleaves calls and results across entries: several calls can
    // share one entry, results arrive later, and an assistant message with `tool_calls`
    // must be followed *immediately* by the tool messages answering those ids. Merging by
    // position and then dropping dangling ids is not enough -- the results have to be
    // moved up to sit behind their call.
    let mut grouped: Vec<(i64, Vec<AgentMessage>)> = Vec::new();
    for record in &records {
        // Real sessions carry `position`; the synthetic fixtures carry `order`. Using a
        // single missing-value default put every synthetic record in one group, which
        // produced a user message containing tool_calls and a provider rejection.
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

    // Split every group into text/calls (the "speaker" turn) and tool results.
    struct Turn {
        role: String,
        entry_id: String,
        text: String,
        calls: Vec<ContentBlock>,
    }
    let mut turns: Vec<Turn> = Vec::new();
    // (body, the entry id of the record that carried it)
    let mut results: std::collections::HashMap<String, Vec<(String, String, bool)>> =
        std::collections::HashMap::new();
    // Pass 1: collect every tool result, wherever it appears. A result arrives in a
    // later entry than its call, so collecting results here -- and not in the same loop
    // that emits turns -- is what lets pass 2 attach them. Doing both in one pass
    // dropped every tool call and result, which emptied the evidence index.
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

    // Pass 2: the speaker turns.
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

    // Emit each turn, and immediately after a turn with calls, emit the results that
    // answer them. A call with no stored result is dropped, since keeping it makes the
    // whole array invalid.
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
    let prompt = project_prompt_context(&messages, checkpoint.as_ref(), None, window as u64);
    let (reserve_tokens, keep_recent_tokens) = context_token_budgets(window);
    let manager = ContextManager {
        enabled: true,
        reserve_tokens,
        keep_recent_tokens,
        context_window: window,
        model: model.clone(),
    };
    let registry = Arc::new(parking_lot::RwLock::new(
        future_agent::models::Registry::new(),
    ));
    let mut inner = future_agent::llm::Client::from_live_model(model.clone(), registry);
    if let Some(level) = &thinking_level {
        inner = inner.with_thinking_level(level);
    }
    let provider = Observed {
        inner,
        handles: Mutex::new(Vec::new()),
        requests: Mutex::new(Vec::new()),
    };
    let before = prompt.usage.estimated_input_tokens;

    // The same system prompt and tools a real turn sends. Reusing them is what lets
    // the summary request be served from the provider's prefix cache; the driver does
    // not otherwise know the session's prompt, so it passes an empty one and an empty
    // tool list for the no-summary path.
    let prepared = if no_summary {
        manager.prepare_evidence(
            prompt,
            &messages,
            CompactionTrigger::Manual,
            CompactionPhase::Standalone,
            None,
            &AtomicBool::new(false),
            None,
        )?
    } else {
        manager
            .prepare_evidence_with_summary(
                prompt,
                &messages,
                CompactionTrigger::Manual,
                CompactionPhase::Standalone,
                None,
                &AtomicBool::new(false),
                None,
                Some(&provider),
                system_prompt.as_deref(),
                &[],
                None,
                // Surface the reason a summary was discarded: without this the
                // fallback is silent in a driver that installs no tracing subscriber.
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
            "projection": projection,
            "checkpoint": checkpoint,
            "estimated_before": before,
            "estimated_after": after,
            "window": window,
            "model_requests": totals.calls.load(Ordering::Relaxed),
            "thinking_level": thinking_level,
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

#[cfg(test)]
mod fidelity_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    #[tokio::test]
    async fn observed_forwards_off_and_output_cap_on_the_wire() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let (header_end, length) = loop {
                let mut buffer = [0_u8; 4096];
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..at]);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    break (at + 4, length);
                }
            };
            while bytes.len() < header_end + length {
                let mut buffer = [0_u8; 4096];
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            let request: Value =
                serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"summary\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            request
        });
        let model = future_agent::models::Model {
            id: "test".into(),
            provider: "deepseek".into(),
            api: "openai-completions".into(),
            base_url: format!("http://{address}"),
            reasoning: true,
            max_tokens: 32000,
            context_window: 128000,
            ..Default::default()
        };
        let mut target = future_agent::llm::schema::ResolvedModelTarget::from_model(
            &model,
            "test-key".into(),
            None,
            Some(32000),
        )
        .unwrap();
        target.protocol = future_agent::llm::schema::ProtocolConfig::OpenAiChat(
            future_agent::llm::schema::OpenAiChatConfig {
                reasoning: future_agent::llm::schema::ChatReasoningFormat::DeepSeek,
                ..Default::default()
            },
        );
        TOTALS.get_or_init(|| Arc::new(Totals::default()));
        let provider = Observed {
            inner: future_agent::llm::Client::from_target(target).with_thinking_level("off"),
            handles: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
        };
        let request = ModelRequest {
            model: "test".into(),
            system_prompt: "FIXED_BASE".into(),
            messages: vec![AgentMessage::new_user("user", json!("question"))],
            tools: vec![],
        };
        let mut stream = provider
            .stream_model_with_output_limit(request, 8192)
            .await
            .unwrap();
        while stream.next().await.is_some() {}
        let body = server.join().unwrap();
        assert_eq!(body["max_tokens"], 8192);
        assert_eq!(body["thinking"], json!({"type":"disabled"}));
        assert_eq!(body["messages"][0]["content"], "FIXED_BASE");
        assert_eq!(provider.requests.lock().len(), 1);
    }
}
