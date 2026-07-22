//! LLM Client — 1:1 compatible with internal/llm/
//!
//! Uses reqwest for HTTP + SSE streaming, matching Go's OpenAI SDK behavior.

mod helpers;
use crate::types::{Message, StreamEvent, ToolDef};
use anyhow::{anyhow, Result};
use futures::StreamExt;
use parking_lot::RwLock;
use reqwest::Client as HttpClient;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};

const DEFAULT_TIMEOUT_SECS: u64 = 600;
const STREAM_IDLE_TIMEOUT_SECS: u64 = 45;
const STREAM_TOOL_CALL_IDLE_TIMEOUT_SECS: u64 = 15;

// ─── LLM Client ────────────────────────────────────────────────────────────

pub struct Client {
    http: HttpClient,
    base_url: RwLock<String>,
    api_key: RwLock<String>,
    reasoning_effort: String,
    thinking_budget: RwLock<i32>,
    #[allow(clippy::type_complexity)]
    on_payload: Option<Arc<dyn Fn(&[u8]) + Send + Sync>>,
    #[allow(clippy::type_complexity)]
    on_response: Option<Arc<dyn Fn(u16, &HashMap<String, String>) + Send + Sync>>,
    thinking_level: RwLock<String>,
    thinking_level_map: RwLock<HashMap<String, String>>,
    compat_thinking_format: RwLock<String>,
    compat_supports_reasoning_effort: RwLock<bool>,
    compat_requires_reasoning_on_assistant: RwLock<bool>,
    max_tokens_field: RwLock<String>,
    temperature: Option<f32>,
    max_tokens: Option<i32>,
}

impl Client {
    pub fn new(
        base_url: &str,
        api_key: &str,
        temperature: Option<f32>,
        max_tokens: Option<i32>,
    ) -> Self {
        let http = HttpClient::builder()
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|_| HttpClient::new());

        Self {
            http,
            base_url: RwLock::new(base_url.to_string()),
            api_key: RwLock::new(api_key.to_string()),
            reasoning_effort: String::new(),
            thinking_budget: RwLock::new(0),
            on_payload: None,
            on_response: None,
            thinking_level: RwLock::new(String::new()),
            thinking_level_map: RwLock::new(HashMap::new()),
            compat_thinking_format: RwLock::new(String::new()),
            compat_supports_reasoning_effort: RwLock::new(false),
            compat_requires_reasoning_on_assistant: RwLock::new(false),
            max_tokens_field: RwLock::new("max_tokens".to_string()),
            temperature,
            max_tokens,
        }
    }

    pub fn with_thinking_level(self, level: &str) -> Self {
        *self.thinking_level.write() = level.to_string();
        self
    }

    pub fn with_thinking_budget(self, budget: i32) -> Self {
        *self.thinking_budget.write() = budget;
        self
    }

    pub fn with_compat(
        self,
        format: &str,
        supports_reasoning_effort: bool,
        requires_reasoning_on_assistant: bool,
    ) -> Self {
        *self.compat_thinking_format.write() = format.to_string();
        *self.compat_supports_reasoning_effort.write() = supports_reasoning_effort;
        *self.compat_requires_reasoning_on_assistant.write() = requires_reasoning_on_assistant;
        self
    }

    pub fn with_max_tokens_field(self, field: &str) -> Self {
        if !field.is_empty() {
            *self.max_tokens_field.write() = field.to_string();
        }
        self
    }

    pub fn with_thinking_level_map(self, map: HashMap<String, String>) -> Self {
        *self.thinking_level_map.write() = map;
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: i32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }
}

#[async_trait::async_trait]
impl crate::types::LLMProvider for Client {
    async fn stream_chat(
        &self,
        model: String,
        messages: Vec<Message>,
        tools: Vec<ToolDef>,
        system_prompt: String,
    ) -> Result<ReceiverStream<StreamEvent>> {
        let (tx, rx) = mpsc::channel(16);

        let base_url = self.base_url.read().clone();
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

        let mut body = serde_json::json!({
            "model": model,
            "messages": Self::convert_messages_to_openai(messages, system_prompt, *self.compat_requires_reasoning_on_assistant.read()),
            "stream": true,
        });

        // Add tools
        if !tools.is_empty() {
            let openai_tools: Vec<Value> = tools
                .into_iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.function.name,
                            "description": t.function.description,
                            "parameters": t.function.parameters,
                        }
                    })
                })
                .collect();
            body["tools"] = serde_json::json!(openai_tools);

            // Z.AI (GLM) models require tool_stream=true for incremental
            // tool-call argument streaming when connecting directly to
            // ZhipuAI's API. Without it, every chunk repeats id+name,
            // causing parse_sse_chunk to emit toolcall_start for each
            // fragment instead of toolcall_delta.
            //
            // Only enable for direct ZhipuAI connections (bigmodel.cn / z.ai).
            // When GLM models are accessed through third-party gateways
            // (Alibaba Cloud MaaS, Vercel AI Gateway, etc.), tool_stream
            // is either unsupported or handled differently, and the
            // run-loop's duplicate-id fallback handles streaming correctly.
            let base_url_lower = base_url.to_lowercase();
            if base_url_lower.contains("bigmodel") || base_url_lower.contains("z.ai") {
                body["tool_stream"] = serde_json::json!(true);
            }
        }

        // Add temperature
        if let Some(temp) = self.temperature {
            body["temperature"] = serde_json::json!(temp);
        }

        // Use model-specific max_tokens field name (from compat.maxTokensField)
        // Open AI SDK sets maxTokensField to "max_completion_tokens" for reasoning models
        if let Some(mt) = self.max_tokens {
            let field = self.max_tokens_field.read();
            body[field.as_str()] = serde_json::json!(mt);
        }

        // Add stream_options for usage stats in streaming
        body["stream_options"] = serde_json::json!({"include_usage": true});

        // Add thinking/reasoning parameters (compat format)
        self.apply_thinking_params(&mut body);

        let req = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key.read()))
            .header("Content-Type", "application/json")
            .header(
                "User-Agent",
                concat!("future-agent/", env!("FUTURE_VERSION")),
            )
            .json(&body)
            .build()?;

        let msg_count = body["messages"].as_array().map(|a| a.len()).unwrap_or(0);
        let body_bytes = serde_json::to_string(&body).unwrap_or_default().len();
        info!(
            model = %body["model"], msgs = %msg_count, body_kb = body_bytes / 1024,
            "LLM request"
        );

        let resp = self.http.execute(req).await?;

        let status = resp.status();
        let headers: HashMap<String, String> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        if let Some(ref cb) = self.on_response {
            cb(status.as_u16(), &headers);
        }

        if !status.is_success() {
            let status_code = status.as_u16();
            let text = resp.text().await.unwrap_or_default();

            // Diagnostic: log request size and model on failure
            let body_str = serde_json::to_string(&body).unwrap_or_default();
            let msg_count = body["messages"].as_array().map(|a| a.len()).unwrap_or(0);
            let body_kb = body_str.len() / 1024;
            warn!(
                model = %body["model"], status = %status_code,
                msgs = %msg_count, body_kb = body_kb,
                "LLM API error"
            );
            if text.len() < 500 && !text.is_empty() {
                warn!("LLM error body: {}", text);
            }

            // Parse Azure/OpenAI error body for a user-friendly message
            if let Ok(err_body) = serde_json::from_str::<serde_json::Value>(&text) {
                let code = err_body
                    .get("error")
                    .and_then(|e| e.get("code"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                let message = err_body
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("");
                match (status_code, code) {
                    (404, "DeploymentNotFound") => {
                        return Err(anyhow!(
                            "Azure deployment not found. Check that the model deployment exists \
                             in your Azure OpenAI resource and the deployment name matches the \
                             model ID. If you just created the deployment, wait a few minutes \
                             and try again."
                        ));
                    }
                    (401, _) => {
                        return Err(anyhow!(
                            "Authentication failed (401). Check your API key is correct and \
                             has access to this Azure OpenAI resource."
                        ));
                    }
                    (429, _) => {
                        return Err(anyhow!(
                            "Rate limited (429). The API is throttling requests. \
                             Try again in a few seconds."
                        ));
                    }
                    (400, "content_filter") | (400, "content_policy_violation") => {
                        return Err(anyhow!(
                            "Content was flagged by the provider's safety system (HTTP 400). \
                             Try rephrasing the request or reducing potentially sensitive content.{}",
                            if message.is_empty() {
                                String::new()
                            } else {
                                format!(" Detail: {}", message)
                            }
                        ));
                    }
                    (400, "context_length_exceeded") | (400, "invalid_request_error")
                        if message.contains("maximum context")
                            || message.contains("context_length")
                            || message.contains("too long")
                            || message.contains("reduce") =>
                    {
                        return Err(anyhow!(
                            "[CTX_LIMIT] Request exceeds the model's maximum context length (HTTP 400). \
                             The conversation history may be too long. Consider starting a \
                             new session or reducing the message count (current: {} messages, \
                             {} KB).{}",
                            msg_count,
                            body_kb,
                            if message.is_empty() {
                                String::new()
                            } else {
                                format!(" Detail: {}", message)
                            }
                        ));
                    }
                    (400, _) if !code.is_empty() => {
                        return Err(anyhow!(
                            "API request failed (HTTP 400): code={}, message=\"{}\". \
                             Request: {} messages, {} KB.",
                            code,
                            if message.is_empty() {
                                "(none)"
                            } else {
                                message
                            },
                            msg_count,
                            body_kb,
                        ));
                    }
                    _ => {}
                }
            }

            // If body is empty, the 400 likely comes from a reverse proxy /
            // gateway (e.g. nginx body size limit, Cloudflare challenge page).
            // The run-loop retry will back off and re-send, but the request
            // size is the more likely culprit when this happens repeatedly.
            if text.is_empty() {
                return Err(anyhow!(
                    "[CTX_LIMIT] API request failed (HTTP 400). No response body. \
                     This usually indicates a reverse-proxy or gateway issue \
                     (e.g. request body too large for nginx client_max_body_size, \
                     or Cloudflare rejecting the connection). \
                     Request: {} messages, {} KB.",
                    msg_count,
                    body_kb,
                ));
            }

            return Err(anyhow!(
                "API request failed (HTTP {}).{} Request: {} messages, {} KB.",
                status_code,
                if text.is_empty() {
                    " No response body.".to_string()
                } else {
                    let truncated: String = text.chars().take(200).collect();
                    if truncated.len() < text.len() {
                        format!(" {}…", truncated)
                    } else {
                        format!(" {}", text)
                    }
                },
                msg_count,
                body_kb,
            ));
        }

        let stream = resp.bytes_stream();
        let on_payload = self.on_payload.clone();

        tokio::spawn(async move {
            let mut stream = stream;
            let tx = tx;
            let mut in_thinking = false;
            let mut in_tool_call = false;
            let mut buffer: Vec<u8> = Vec::new();
            let mut last_sse_event_at = std::time::Instant::now();
            // Tracks whether the provider sent a genuine terminal signal —
            // either `[DONE]` or a chunk carrying finish_reason stop/tool_calls.
            // If the read loop instead exits via idle timeout or a premature
            // connection close (`Ok(None)`), the response was cut off mid-flight
            // and must be flagged so the run loop doesn't present a truncated
            // prefix as a clean completion.
            let mut saw_terminal = false;
            // Diagnostics for premature stream termination: how long the stream
            // ran, how much it delivered, and which exit path fired. Logged only
            // when the stream ends without a terminal signal so recurring
            // upstream drops (gateway / proxy cutting the connection mid-reply)
            // leave an actionable trace instead of a silent truncation.
            let stream_started_at = std::time::Instant::now();
            let mut total_bytes: usize = 0;
            let mut idle_timed_out = false;

            // Helper to emit events from a parsed SSE data line, handling
            // thinking/tool-call bookending (matches original per-line logic).
            async fn process_data_line(
                data: &str,
                tx: &mpsc::Sender<StreamEvent>,
                in_thinking: &mut bool,
                in_tool_call: &mut bool,
                saw_terminal: &mut bool,
            ) -> bool {
                if data == "[DONE]" {
                    *saw_terminal = true;
                    if *in_tool_call {
                        let _ = tx
                            .send(StreamEvent {
                                event_type: "toolcall_end".to_string(),
                                ..Default::default()
                            })
                            .await;
                        *in_tool_call = false;
                    }
                    if *in_thinking {
                        let _ = tx
                            .send(StreamEvent {
                                event_type: "thinking_end".to_string(),
                                ..Default::default()
                            })
                            .await;
                        *in_thinking = false;
                    }
                    let _ = tx
                        .send(StreamEvent {
                            event_type: "stop".to_string(),
                            ..Default::default()
                        })
                        .await;
                    return false; // signal done
                }
                if let Ok(event) = Client::parse_sse_chunk(data) {
                    let stop_reason = event.stop_reason.clone();
                    let should_finish_response =
                        matches!(stop_reason.as_str(), "stop" | "tool_calls");
                    let should_emit_tool_end =
                        stop_reason == "tool_calls" && event.event_type != "toolcall_end";
                    let should_emit_thinking_end = should_finish_response
                        && *in_thinking
                        && event.event_type != "thinking_delta";

                    if event.event_type == "thinking_delta" {
                        if !*in_thinking {
                            *in_thinking = true;
                            let _ = tx
                                .send(StreamEvent {
                                    event_type: "thinking_start".to_string(),
                                    ..Default::default()
                                })
                                .await;
                        }
                    } else if *in_thinking
                        && event.event_type != "thinking_delta"
                        && event.event_type != "usage"
                    {
                        *in_thinking = false;
                        let _ = tx
                            .send(StreamEvent {
                                event_type: "thinking_end".to_string(),
                                ..Default::default()
                            })
                            .await;
                    }

                    if event.event_type == "toolcall_start" {
                        *in_tool_call = true;
                    } else if event.event_type == "toolcall_end" {
                        *in_tool_call = false;
                    }

                    let _ = tx.send(event).await;

                    if should_emit_tool_end && *in_tool_call {
                        let _ = tx
                            .send(StreamEvent {
                                event_type: "toolcall_end".to_string(),
                                stop_reason: "tool_calls".to_string(),
                                ..Default::default()
                            })
                            .await;
                        *in_tool_call = false;
                    }

                    if should_emit_thinking_end {
                        let _ = tx
                            .send(StreamEvent {
                                event_type: "thinking_end".to_string(),
                                ..Default::default()
                            })
                            .await;
                        *in_thinking = false;
                    }

                    if should_finish_response {
                        // A real finish_reason (stop/tool_calls) is a genuine
                        // terminal signal even when the provider never sends a
                        // trailing `[DONE]` (some close the socket right after).
                        *saw_terminal = true;
                        let _ = tx
                            .send(StreamEvent {
                                event_type: "stop".to_string(),
                                stop_reason,
                                ..Default::default()
                            })
                            .await;
                        // Do NOT stop reading here. Per the OpenAI streaming spec the
                        // stream ends at `[DONE]` (or connection close), not at the
                        // finish_reason chunk — and providers like dashscope/qwen send
                        // the `usage` chunk AFTER finish_reason. Returning false here
                        // dropped token usage for every reasoning turn. Keep reading;
                        // `[DONE]` / `Ok(None)` still terminate the stream below.
                    }
                }
                true // continue
            }

            loop {
                let idle_timeout_secs = if in_tool_call {
                    STREAM_TOOL_CALL_IDLE_TIMEOUT_SECS
                } else {
                    STREAM_IDLE_TIMEOUT_SECS
                };
                let chunk_result = tokio::select! {
                    // The consumer dropped the receiver — e.g. the user hit stop
                    // and the run loop abandoned this stream. Stop reading right
                    // away instead of draining the HTTP body until the idle
                    // timeout, which leaked a live connection for up to 45s on
                    // every interrupt.
                    _ = tx.closed() => break,
                    res = tokio::time::timeout(
                        std::time::Duration::from_secs(idle_timeout_secs),
                        stream.next(),
                    ) => match res {
                        Ok(Some(chunk_result)) => chunk_result,
                        Ok(None) => break,
                        Err(_) => {
                            idle_timed_out = true;
                            break;
                        }
                    },
                };

                match chunk_result {
                    Ok(bytes) => {
                        if let Some(ref cb) = on_payload {
                            cb(&bytes);
                        }
                        total_bytes += bytes.len();
                        buffer.extend_from_slice(&bytes);

                        // Guard against malformed streams (no \n\n delimiter).
                        // 1 MiB is far larger than any legitimate single SSE event.
                        if buffer.len() > 1_048_576 {
                            warn!("SSE buffer exceeded 1 MiB without \\n\\n, discarding");
                            buffer.clear();
                        }

                        // Process complete SSE events (delimited by b"\n\n").
                        // Byte-level search avoids corrupting multi-byte UTF-8
                        // chars split across chunks.  We only decode once we have
                        // a complete event (all multi-byte chars within it are
                        // guaranteed to be fully assembled).
                        while let Some(pos) = buffer.windows(2).position(|w| w == b"\n\n") {
                            let event_bytes: Vec<u8> = buffer.drain(..pos).collect();
                            buffer.drain(..2); // consume the \n\n delimiter
                            let event_block = String::from_utf8_lossy(&event_bytes);
                            let mut done = false;
                            for line in event_block.lines() {
                                let line = line.trim();
                                if !line.starts_with("data: ") {
                                    continue;
                                }
                                let data = &line[6..];
                                last_sse_event_at = std::time::Instant::now();
                                if !process_data_line(
                                    data,
                                    &tx,
                                    &mut in_thinking,
                                    &mut in_tool_call,
                                    &mut saw_terminal,
                                )
                                .await
                                {
                                    done = true;
                                    break;
                                }
                            }
                            if done {
                                return Ok::<_, ()>(());
                            }
                        }

                        if in_tool_call
                            && last_sse_event_at.elapsed()
                                >= std::time::Duration::from_secs(
                                    STREAM_TOOL_CALL_IDLE_TIMEOUT_SECS,
                                )
                        {
                            let _ = tx
                                .send(StreamEvent {
                                    event_type: "toolcall_end".to_string(),
                                    stop_reason: "tool_calls".to_string(),
                                    ..Default::default()
                                })
                                .await;
                            let _ = tx
                                .send(StreamEvent {
                                    event_type: "stop".to_string(),
                                    stop_reason: "tool_calls".to_string(),
                                    ..Default::default()
                                })
                                .await;
                            return Ok::<_, ()>(());
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(StreamEvent {
                                event_type: "error".to_string(),
                                error_text: e.to_string(),
                                ..Default::default()
                            })
                            .await;
                    }
                }
            }

            // Close any open blocks at stream end
            if in_tool_call {
                let _ = tx
                    .send(StreamEvent {
                        event_type: "toolcall_end".to_string(),
                        ..Default::default()
                    })
                    .await;
            }
            if in_thinking {
                let _ = tx
                    .send(StreamEvent {
                        event_type: "thinking_end".to_string(),
                        ..Default::default()
                    })
                    .await;
            }

            // If we never saw a genuine terminal signal, the read loop exited
            // via idle timeout or a premature EOF — the response is a truncated
            // prefix. Mark the synthetic stop so the run loop can distinguish it
            // from a clean completion instead of persisting a cut-off reply as a
            // success.
            let stop_reason = if saw_terminal {
                String::new()
            } else {
                warn!(
                    elapsed_ms = stream_started_at.elapsed().as_millis() as u64,
                    bytes = total_bytes,
                    in_tool_call = in_tool_call,
                    cause = if idle_timed_out {
                        "idle_timeout"
                    } else {
                        "upstream_eof"
                    },
                    "LLM stream ended without a terminal signal ([DONE]/finish_reason \
                     missing) — response truncated mid-flight"
                );
                "truncated".to_string()
            };
            let _ = tx
                .send(StreamEvent {
                    event_type: "stop".to_string(),
                    stop_reason,
                    ..Default::default()
                })
                .await;

            Ok::<_, ()>(())
        });

        Ok(ReceiverStream::new(rx))
    }

    fn set_api_key(&self, api_key: &str) {
        *self.api_key.write() = api_key.to_string();
    }

    fn update_thinking(&self, level: &str, budget: i32) {
        *self.thinking_level.write() = level.to_string();
        *self.thinking_budget.write() = budget;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Client construction ────────────────────────────────────────────────

    #[test]
    fn client_new() {
        let c = Client::new("https://api.openai.com", "sk-test", None, None);
        assert_eq!(*c.base_url.read(), "https://api.openai.com");
        assert_eq!(*c.api_key.read(), "sk-test");
        assert!(c.temperature.is_none());
        assert!(c.max_tokens.is_none());
    }

    #[test]
    fn client_new_with_params() {
        let c = Client::new("https://api.openai.com", "sk-test", Some(0.7), Some(4096));
        assert_eq!(c.temperature, Some(0.7));
        assert_eq!(c.max_tokens, Some(4096));
    }

    // ─── Builder pattern ────────────────────────────────────────────────────

    #[test]
    fn with_thinking_level() {
        let c = Client::new("https://api.test", "key", None, None)
            .with_thinking_level("high");
        assert_eq!(*c.thinking_level.read(), "high");
    }

    #[test]
    fn with_thinking_budget() {
        let c = Client::new("https://api.test", "key", None, None)
            .with_thinking_budget(16000);
        assert_eq!(*c.thinking_budget.read(), 16000);
    }

    #[test]
    fn with_compat() {
        let c = Client::new("https://api.test", "key", None, None)
            .with_compat("deepseek", true, false);
        assert_eq!(*c.compat_thinking_format.read(), "deepseek");
        assert!(*c.compat_supports_reasoning_effort.read());
        assert!(!*c.compat_requires_reasoning_on_assistant.read());
    }

    #[test]
    fn with_max_tokens_field() {
        let c = Client::new("https://api.test", "key", None, None)
            .with_max_tokens_field("max_completion_tokens");
        assert_eq!(*c.max_tokens_field.read(), "max_completion_tokens");
    }

    #[test]
    fn with_max_tokens_field_empty_keeps_default() {
        let c = Client::new("https://api.test", "key", None, None)
            .with_max_tokens_field("");
        assert_eq!(*c.max_tokens_field.read(), "max_tokens");
    }

    #[test]
    fn with_thinking_level_map() {
        let mut map = HashMap::new();
        map.insert("high".to_string(), "high".to_string());
        map.insert("xhigh".to_string(), "max".to_string());
        let c = Client::new("https://api.test", "key", None, None)
            .with_thinking_level_map(map);
        assert_eq!(c.thinking_level_map.read().len(), 2);
        assert_eq!(
            c.thinking_level_map.read().get("xhigh").unwrap(),
            "max"
        );
    }

    #[test]
    fn with_temperature() {
        let c = Client::new("https://api.test", "key", None, None)
            .with_temperature(0.5);
        assert_eq!(c.temperature, Some(0.5));
    }

    #[test]
    fn with_max_tokens() {
        let c = Client::new("https://api.test", "key", None, None)
            .with_max_tokens(8192);
        assert_eq!(c.max_tokens, Some(8192));
    }

    #[test]
    fn builder_chaining() {
        let c = Client::new("https://api.test", "key", None, None)
            .with_thinking_level("medium")
            .with_thinking_budget(8000)
            .with_compat("qwen", true, false)
            .with_max_tokens_field("max_tokens")
            .with_temperature(0.3)
            .with_max_tokens(2048);
        assert_eq!(*c.thinking_level.read(), "medium");
        assert_eq!(*c.thinking_budget.read(), 8000);
        assert_eq!(c.temperature, Some(0.3));
        assert_eq!(c.max_tokens, Some(2048));
    }

    // ─── set_api_key / update_thinking ──────────────────────────────────────

    #[test]
    fn set_api_key_updates() {
        let c = Client::new("https://api.test", "old_key", None, None);
        assert_eq!(*c.api_key.read(), "old_key");
        crate::types::LLMProvider::set_api_key(&c, "new_key");
        assert_eq!(*c.api_key.read(), "new_key");
    }

    #[test]
    fn update_thinking_changes_level_and_budget() {
        let c = Client::new("https://api.test", "key", None, None)
            .with_thinking_level("off");
        assert_eq!(*c.thinking_level.read(), "off");
        crate::types::LLMProvider::update_thinking(&c, "high", 16000);
        assert_eq!(*c.thinking_level.read(), "high");
        assert_eq!(*c.thinking_budget.read(), 16000);
    }

    #[test]
    fn default_max_tokens_field() {
        let c = Client::new("https://api.test", "key", None, None);
        assert_eq!(*c.max_tokens_field.read(), "max_tokens");
    }

    #[test]
    fn default_thinking_level_empty() {
        let c = Client::new("https://api.test", "key", None, None);
        assert!(c.thinking_level.read().is_empty());
    }

    #[test]
    fn default_thinking_budget_zero() {
        let c = Client::new("https://api.test", "key", None, None);
        assert_eq!(*c.thinking_budget.read(), 0);
    }

    #[test]
    fn default_compat_fields() {
        let c = Client::new("https://api.test", "key", None, None);
        assert!(c.compat_thinking_format.read().is_empty());
        assert!(!*c.compat_supports_reasoning_effort.read());
        assert!(!*c.compat_requires_reasoning_on_assistant.read());
    }
}
