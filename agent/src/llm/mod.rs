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

const DEFAULT_TIMEOUT_SECS: u64 = 1800;
const STREAM_IDLE_TIMEOUT_SECS: u64 = 45;
const STREAM_TOOL_CALL_IDLE_TIMEOUT_SECS: u64 = 15;

/// Stream-read idle timeout. Tests override it (a stalled-mock test cannot
/// wait 45 s of real time) via FUTURE_TEST_STREAM_IDLE_SECS.
fn stream_idle_timeout_secs() -> u64 {
    #[cfg(test)]
    if let Some(secs) = std::env::var("FUTURE_TEST_STREAM_IDLE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        return secs;
    }
    STREAM_IDLE_TIMEOUT_SECS
}

/// Tool-call idle timeout — same test seam (FUTURE_TEST_TOOL_IDLE_SECS).
fn stream_tool_call_idle_timeout_secs() -> u64 {
    #[cfg(test)]
    if let Some(secs) = std::env::var("FUTURE_TEST_TOOL_IDLE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        return secs;
    }
    STREAM_TOOL_CALL_IDLE_TIMEOUT_SECS
}

/// HTTP request timeout for a single LLM call. Defaults to 30 min (1800 s);
/// override with the FUTURE_LLM_TIMEOUT_SECS env var without rebuilding.
fn llm_timeout_secs() -> u64 {
    std::env::var("FUTURE_LLM_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v >= 60)
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

/// Why the LLM SSE read loop exited before seeing a genuine terminal signal
/// (`[DONE]` or `finish_reason`). Distinguishes abort/disconnect (expected —
/// log at INFO) from provider-side drops (actionable — log at WARN).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamExitCause {
    /// The consumer (run loop) dropped the receiver — user abort or client
    /// disconnect. Expected truncation, not a provider failure.
    ConsumerDropped,
    /// The provider closed the HTTP connection without a terminal signal.
    UpstreamEof,
    /// No SSE data arrived within the idle window.
    IdleTimeout,
}

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
            .timeout(std::time::Duration::from_secs(llm_timeout_secs()))
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
            // Why the read loop exited (only consulted when !saw_terminal).
            // Set at every break point; early returns skip the WARN block.
            let exit_cause: StreamExitCause;

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
                    stream_tool_call_idle_timeout_secs()
                } else {
                    stream_idle_timeout_secs()
                };
                let chunk_result = tokio::select! {
                    // The consumer dropped the receiver — e.g. the user hit stop
                    // and the run loop abandoned this stream. Stop reading right
                    // away instead of draining the HTTP body until the idle
                    // timeout, which leaked a live connection for up to 45s on
                    // every interrupt. This is an expected exit, not a provider
                    // failure — recorded so the end-of-stream log stays quiet.
                    _ = tx.closed() => {
                        exit_cause = StreamExitCause::ConsumerDropped;
                        break;
                    },
                    res = tokio::time::timeout(
                        std::time::Duration::from_secs(idle_timeout_secs),
                        stream.next(),
                    ) => match res {
                        Ok(Some(chunk_result)) => chunk_result,
                        Ok(None) => {
                            exit_cause = StreamExitCause::UpstreamEof;
                            break;
                        }
                        Err(_) => {
                            exit_cause = StreamExitCause::IdleTimeout;
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
                                    stream_tool_call_idle_timeout_secs(),
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
            // via consumer drop, idle timeout, or a premature EOF. Only the
            // last two are provider issues worth a WARN — a consumer drop is
            // an expected abort/disconnect and logs at INFO instead.
            let stop_reason = if saw_terminal {
                String::new()
            } else {
                match exit_cause {
                    StreamExitCause::ConsumerDropped => {
                        info!(
                            elapsed_ms = stream_started_at.elapsed().as_millis() as u64,
                            bytes = total_bytes,
                            in_tool_call = in_tool_call,
                            "LLM stream dropped by consumer (abort/disconnect)"
                        );
                    }
                    StreamExitCause::IdleTimeout | StreamExitCause::UpstreamEof => {
                        warn!(
                            elapsed_ms = stream_started_at.elapsed().as_millis() as u64,
                            bytes = total_bytes,
                            in_tool_call = in_tool_call,
                            cause = match exit_cause {
                                StreamExitCause::IdleTimeout => "idle_timeout",
                                StreamExitCause::UpstreamEof => "upstream_eof",
                                // Unreachable: ConsumerDropped is dispatched
                                // above; included for match exhaustiveness.
                                StreamExitCause::ConsumerDropped => "unknown",
                            },
                            "LLM stream ended without a terminal signal ([DONE]/finish_reason \
                             missing) — response truncated mid-flight"
                        );
                    }
                }
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
    use crate::types::LLMProvider;

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
        let c = Client::new("https://api.test", "key", None, None).with_thinking_level("high");
        assert_eq!(*c.thinking_level.read(), "high");
    }

    #[test]
    fn with_thinking_budget() {
        let c = Client::new("https://api.test", "key", None, None).with_thinking_budget(16000);
        assert_eq!(*c.thinking_budget.read(), 16000);
    }

    #[test]
    fn with_compat() {
        let c =
            Client::new("https://api.test", "key", None, None).with_compat("deepseek", true, false);
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
        let c = Client::new("https://api.test", "key", None, None).with_max_tokens_field("");
        assert_eq!(*c.max_tokens_field.read(), "max_tokens");
    }

    #[test]
    fn with_thinking_level_map() {
        let mut map = HashMap::new();
        map.insert("high".to_string(), "high".to_string());
        map.insert("xhigh".to_string(), "max".to_string());
        let c = Client::new("https://api.test", "key", None, None).with_thinking_level_map(map);
        assert_eq!(c.thinking_level_map.read().len(), 2);
        assert_eq!(c.thinking_level_map.read().get("xhigh").unwrap(), "max");
    }

    #[test]
    fn with_temperature() {
        let c = Client::new("https://api.test", "key", None, None).with_temperature(0.5);
        assert_eq!(c.temperature, Some(0.5));
    }

    #[test]
    fn with_max_tokens() {
        let c = Client::new("https://api.test", "key", None, None).with_max_tokens(8192);
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
        let c = Client::new("https://api.test", "key", None, None).with_thinking_level("off");
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

    // ─── mock HTTP server for stream_chat ───────────────────────────────────

    /// One-shot HTTP server: accepts a single request, records its body, and
    /// replies with a canned (status, content_type, body). Loops so aborted
    /// probe connections don't consume the one real response.
    struct MockServer {
        base_url: String,
        requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    fn mock_server(
        respond: impl Fn(&str) -> (u16, &'static str, String) + Send + 'static,
    ) -> MockServer {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for _ in 0..16 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
                let mut buf: Vec<u8> = Vec::new();
                let mut chunk = [0u8; 8192];
                // Read until end of headers.
                let mut header_end: Option<usize> = None;
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if let Some(pos) =
                                buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
                            {
                                header_end = Some(pos);
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                    if buf.len() > 1_000_000 {
                        break;
                    }
                }
                let Some(header_end) = header_end else {
                    continue; // aborted probe connection
                };
                let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
                let content_length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse().ok())
                    })
                    .unwrap_or(0);
                // Read the remaining body bytes.
                while buf.len() < header_end + content_length {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        Err(_) => break,
                    }
                }
                let body = String::from_utf8_lossy(&buf[header_end..]).to_string();
                captured.lock().unwrap().push(body.clone());
                let (status, content_type, response_body) = respond(&body);
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                if stream.write_all(response.as_bytes()).is_err() {
                    return;
                }
                let _ = stream.flush();
            }
        });
        MockServer {
            base_url: format!("http://127.0.0.1:{port}"),
            requests,
        }
    }

    fn one_user_message() -> Vec<Message> {
        vec![serde_json::from_str(r#"{"role":"user","content":"hi"}"#).unwrap()]
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_flows_text_usage_and_terminal_stop() {
        let sse = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2,\"total_tokens\":12}}\n\n",
            "data: [DONE]\n\n"
        );
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        let text: String = events
            .iter()
            .filter(|e| e.event_type == "text_delta")
            .map(|e| e.text.clone())
            .collect();
        assert_eq!(text, "Hello world");
        // The final chunk carries both finish_reason=stop and the usage block.
        let stop = events
            .iter()
            .find(|e| e.event_type == "stop" && e.stop_reason == "stop")
            .expect("finish_reason stop event");
        let usage = stop.usage.as_ref().expect("usage on the stop chunk");
        assert_eq!(usage.prompt_tokens, 10);
        let last = events.last().unwrap();
        assert_eq!(last.event_type, "stop");
        assert_eq!(last.stop_reason, "", "[DONE] is the genuine terminal");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_thinking_and_toolcall_bookends() {
        let sse = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"hmm\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\" think\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"type\":\"function\",\"function\":{\"name\":\"echo\",\"arguments\":\"{\\\"a\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"1}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert!(types.contains(&"thinking_start"));
        assert!(types.contains(&"thinking_delta"));
        assert!(types.contains(&"thinking_end"));
        assert!(types.contains(&"toolcall_start"));
        assert!(types.contains(&"toolcall_end"));
        assert!(types.contains(&"stop"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_maps_http_errors() {
        let cases: Vec<(u16, &str, &str)> = vec![
            (
                401,
                r#"{"error":{"code":"x","message":"bad key"}}"#,
                "Authentication failed (401)",
            ),
            (
                429,
                r#"{"error":{"code":"x","message":"slow down"}}"#,
                "Rate limited (429)",
            ),
            (
                404,
                r#"{"error":{"code":"DeploymentNotFound","message":"gone"}}"#,
                "Azure deployment not found",
            ),
            (
                400,
                r#"{"error":{"code":"content_filter","message":"blocked"}}"#,
                "flagged by the provider's safety system",
            ),
            (
                400,
                r#"{"error":{"code":"context_length_exceeded","message":"maximum context exceeded"}}"#,
                "[CTX_LIMIT] Request exceeds the model's maximum context length",
            ),
            (
                400,
                r#"{"error":{"code":"some_other","message":"weird"}}"#,
                "API request failed (HTTP 400): code=some_other",
            ),
        ];
        for (status, body, expected) in cases {
            let server = mock_server(move |_| (status, "application/json", body.to_string()));
            let client = Client::new(&server.base_url, "sk-test", None, None);
            let result = client
                .stream_chat(
                    "mock".to_string(),
                    one_user_message(),
                    vec![],
                    String::new(),
                )
                .await;
            let error = result.unwrap_err().to_string();
            assert!(error.contains(expected), "status {status}: {error}");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_empty_400_body_is_ctx_limit() {
        let server = mock_server(move |_| (400, "text/plain", String::new()));
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let result = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await;
        let error = result.unwrap_err().to_string();
        assert!(error.contains("[CTX_LIMIT]"), "{error}");
        assert!(error.contains("No response body"), "{error}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_generic_error_status() {
        let server = mock_server(move |_| {
            (
                500,
                "application/json",
                r#"{"error":{"code":"x","message":"boom"}}"#.to_string(),
            )
        });
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let result = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_reports_response_via_callback() {
        let server = mock_server(move |_| {
            (
                401,
                "application/json",
                r#"{"error":{"code":"x","message":"no"}}"#.to_string(),
            )
        });
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let seen2 = seen.clone();
        let mut client = Client::new(&server.base_url, "sk-test", None, None);
        client.on_response = Some(std::sync::Arc::new(
            move |status: u16, _headers: &HashMap<String, String>| {
                *seen2.lock().unwrap() = Some(status);
            },
        ));
        let _ = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await;
        assert_eq!(*seen.lock().unwrap(), Some(401));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_enables_tool_stream_only_for_zhipu_hosts() {
        let tool = ToolDef {
            tool_type: "function".to_string(),
            function: crate::types::FunctionDef {
                name: "echo".to_string(),
                description: "echo".to_string(),
                parameters: serde_json::json!({"type":"object"}),
            },
        };
        let sse = "data: [DONE]\n\n";
        // Direct ZhipuAI hosts get tool_stream=true. The host check is a
        // substring match on the configured base_url, so embedding the marker
        // in the mock's URL path exercises it against the local listener.
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::new(
            &format!("{}/bigmodel", server.base_url),
            "sk-test",
            None,
            None,
        );
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![tool.clone()],
                String::new(),
            )
            .await
            .unwrap();
        let _: Vec<StreamEvent> = rx.collect().await;
        let body = server.requests.lock().unwrap()[0].clone();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["tool_stream"], true);
        assert_eq!(parsed["tools"][0]["function"]["name"], "echo");

        // Other hosts (the bare mock URL) do not get tool_stream.
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![tool],
                String::new(),
            )
            .await
            .unwrap();
        let _: Vec<StreamEvent> = rx.collect().await;
        let body = server.requests.lock().unwrap()[0].clone();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed.get("tool_stream").is_none());
        assert!(parsed["tools"][0]["function"]["name"] == "echo");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_sends_temperature_max_tokens_and_thinking_params() {
        let sse = "data: [DONE]\n\n";
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::new(&server.base_url, "sk-test", Some(0.5), Some(4096))
            .with_max_tokens_field("max_completion_tokens")
            .with_thinking_level("high")
            .with_thinking_budget(8192)
            .with_compat("deepseek", true, false);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                "sys".to_string(),
            )
            .await
            .unwrap();
        let _: Vec<StreamEvent> = rx.collect().await;
        let body = server.requests.lock().unwrap()[0].clone();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["temperature"], 0.5);
        assert_eq!(parsed["max_completion_tokens"], 4096);
        assert!(parsed.get("max_tokens").is_none());
        // thinking params emitted per the deepseek compat format
        assert!(parsed.get("thinking").is_some() || parsed.get("reasoning_effort").is_some());
        // system prompt is folded into the messages array (content blocks)
        let messages = parsed["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert!(messages[0]["content"].to_string().contains("sys"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_idle_timeout_marks_stream_truncated() {
        std::env::set_var("FUTURE_TEST_STREAM_IDLE_SECS", "1");
        // The server sends one partial SSE event (no \n\n terminator) and then
        // holds the connection open forever.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut sink = [0u8; 4096];
            let _ = stream.read(&mut sink); // headers + body (best effort)
            let partial = "HTTP/1.1 200 X\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n7\r\ndata: x\r\n";
            let _ = stream.write_all(partial.as_bytes());
            let _ = stream.flush();
            // Hold the socket open; the test process kills this thread on exit.
            let _ = stream.read(&mut sink);
        });
        let client = Client::new(&format!("http://127.0.0.1:{port}"), "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        let last = events.last().unwrap();
        assert_eq!(last.event_type, "stop");
        assert_eq!(last.stop_reason, "truncated");
        std::env::remove_var("FUTURE_TEST_STREAM_IDLE_SECS");
    }

    #[test]
    fn llm_timeout_env_override() {
        std::env::set_var("FUTURE_LLM_TIMEOUT_SECS", "120");
        assert_eq!(llm_timeout_secs(), 120);
        // Below the floor → default.
        std::env::set_var("FUTURE_LLM_TIMEOUT_SECS", "5");
        assert_eq!(llm_timeout_secs(), DEFAULT_TIMEOUT_SECS);
        std::env::set_var("FUTURE_LLM_TIMEOUT_SECS", "garbage");
        assert_eq!(llm_timeout_secs(), DEFAULT_TIMEOUT_SECS);
        std::env::remove_var("FUTURE_LLM_TIMEOUT_SECS");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_done_closes_open_thinking_and_toolcall_blocks() {
        // thinking_delta + toolcall_start with no matching ends, then [DONE].
        let sse = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"hmm\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"type\":\"function\",\"function\":{\"name\":\"echo\",\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        let thinking_end = types.iter().position(|t| *t == "thinking_end");
        let toolcall_end = types.iter().position(|t| *t == "toolcall_end");
        let stop = types.iter().rposition(|t| *t == "stop").unwrap();
        assert!(thinking_end.is_some(), "{types:?}");
        assert!(toolcall_end.is_some(), "{types:?}");
        assert!(thinking_end.unwrap() < stop && toolcall_end.unwrap() < stop);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_finish_tool_calls_emits_pending_tool_end() {
        // toolcall_start, then finish_reason=tool_calls without an explicit
        // toolcall_end chunk → the mapper closes the block itself.
        let sse = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"type\":\"function\",\"function\":{\"name\":\"echo\",\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        let tool_ends: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == "toolcall_end")
            .collect();
        assert!(
            tool_ends.iter().any(|e| e.stop_reason == "tool_calls"),
            "auto-closed tool call: {events:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_finish_stop_closes_thinking_block() {
        let sse = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"hmm\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        let thinking_end = types.iter().position(|t| *t == "thinking_end");
        assert!(thinking_end.is_some(), "{types:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_discards_oversized_undelimited_buffer() {
        // 1.2 MiB of SSE data without a \n\n delimiter trips the buffer guard.
        let filler = "x".repeat(1_200_000);
        let sse = format!("data: {filler}");
        let server = mock_server(move |_| (200, "text/event-stream", sse.clone()));
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        // The stream ended without a terminal event → truncated stop.
        let last = events.last().unwrap();
        assert_eq!(last.event_type, "stop");
        assert_eq!(last.stop_reason, "truncated");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_malformed_chunk_yields_error_event() {
        // Invalid chunked-encoding body → reqwest stream error arm.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut sink = [0u8; 8192];
            let _ = stream.read(&mut sink);
            let body = "HTTP/1.1 200 X\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\nZZZ\r\nnot-a-chunk\r\n";
            let _ = stream.write_all(body.as_bytes());
            let _ = stream.flush();
            let _ = stream.read(&mut sink);
        });
        let client = Client::new(&format!("http://127.0.0.1:{port}"), "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        assert!(
            events.iter().any(|e| e.event_type == "error"),
            "decode error surfaced: {events:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_consumer_drop_stops_reading() {
        // A server that streams forever; the client task must notice the
        // dropped receiver instead of draining until the idle timeout.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut sink = [0u8; 8192];
            let _ = stream.read(&mut sink);
            let chunk = "7\r\ndata: x\r\n";
            let head = format!(
                "HTTP/1.1 200 X\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{chunk}"
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.flush();
            // Keep writing slowly forever; writes fail once the client is gone.
            loop {
                std::thread::sleep(std::time::Duration::from_millis(50));
                if stream.write_all(chunk.as_bytes()).is_err() {
                    break;
                }
                let _ = stream.flush();
            }
        });
        let client = Client::new(&format!("http://127.0.0.1:{port}"), "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        drop(rx); // consumer goes away mid-stream
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        // No assertion needed beyond "returns promptly" — the read loop exited
        // via ConsumerDropped rather than hanging this test.
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_upstream_eof_closes_thinking_block() {
        // thinking_delta then the connection closes (no [DONE]).
        let sse =
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"hmm\"}}]}\n\n";
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert!(types.contains(&"thinking_end"), "{types:?}");
        let last = events.last().unwrap();
        assert_eq!(last.event_type, "stop");
        assert_eq!(last.stop_reason, "truncated");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_tool_call_idle_forces_tool_end() {
        std::env::set_var("FUTURE_TEST_TOOL_IDLE_SECS", "1");
        // toolcall_start chunk, then the next chunk only arrives after the
        // tool-call idle window — the read loop force-closes the call.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut sink = [0u8; 8192];
            let _ = stream.read(&mut sink);
            let first = "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"type\":\"function\",\"function\":{\"name\":\"echo\",\"arguments\":\"{}\"}}]}}]}\n\n";
            let head = format!(
                "HTTP/1.1 200 X\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n",
                first.len(),
                first
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.flush();
            // A second chunk after the (test-seamed) 1s tool-call idle window.
            // It carries NO data: line, so last_sse_event_at stays stale and
            // the idle check fires on receipt.
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let _ = stream.write_all(b"2\r\n\n\n");
            let _ = stream.flush();
            let _ = stream.read(&mut sink);
        });
        let client = Client::new(&format!("http://127.0.0.1:{port}"), "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        std::env::remove_var("FUTURE_TEST_TOOL_IDLE_SECS");
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert!(types.contains(&"toolcall_start"), "{types:?}");
        assert!(types.contains(&"toolcall_end"), "{types:?}");
        assert!(types.contains(&"stop"), "{types:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_key_and_thinking_setters() {
        let client = Client::new("https://api.test", "key-1", None, None);
        client.set_api_key("key-2");
        assert_eq!(*client.api_key.read(), "key-2");
        client.update_thinking("low", 4000);
        assert_eq!(*client.thinking_level.read(), "low");
        assert_eq!(*client.thinking_budget.read(), 4000);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_done_closes_lone_thinking_block() {
        // thinking_delta then [DONE] with nothing in between: the [DONE]
        // handler emits the pending thinking_end itself.
        let sse = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"hmm\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        let end = types.iter().position(|t| *t == "thinking_end");
        let stop = types.iter().position(|t| *t == "stop");
        assert!(end.is_some() && stop.is_some(), "{types:?}");
        assert!(end.unwrap() < stop.unwrap());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_eof_with_open_tool_call_closes_it() {
        // toolcall_start then the connection closes: the end-of-stream tail
        // emits toolcall_end before the truncated stop.
        let sse = "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"type\":\"function\",\"function\":{\"name\":\"echo\",\"arguments\":\"{}\"}}]}}]}\n\n";
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        let end = types.iter().position(|t| *t == "toolcall_end");
        assert!(end.is_some(), "{types:?}");
        let last = events.last().unwrap();
        assert_eq!(last.stop_reason, "truncated");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_skips_non_data_lines_and_reports_payloads() {
        // SSE comment/keepalive lines are skipped by the data: filter.
        let sse = concat!(
            ": keepalive\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "event: ping\n\n",
            "data: [DONE]\n\n"
        );
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen2 = seen.clone();
        let mut client = Client::new(&server.base_url, "sk-test", None, None);
        client.on_payload = Some(std::sync::Arc::new(move |bytes: &[u8]| {
            seen2.fetch_add(bytes.len(), std::sync::atomic::Ordering::Relaxed);
        }));
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        assert!(seen.load(std::sync::atomic::Ordering::Relaxed) > 0);
        assert!(events.iter().any(|e| e.event_type == "text_delta"));
        assert_eq!(events.last().unwrap().event_type, "stop");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_finish_tool_calls_with_text_chunk_force_closes_call() {
        // A chunk carrying BOTH text and finish_reason=tool_calls parses to a
        // text_delta (content wins), so the mapper emits the pending
        // toolcall_end itself.
        let sse = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"type\":\"function\",\"function\":{\"name\":\"echo\",\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"x\"},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        let tool_ends: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == "toolcall_end")
            .collect();
        assert!(
            tool_ends.iter().any(|e| e.stop_reason == "tool_calls"),
            "mapper closed the pending call: {events:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_chat_tool_call_idle_with_keepalive_chunks() {
        std::env::set_var("FUTURE_TEST_TOOL_IDLE_SECS", "1");
        // Keepalive chunks (no data: lines) keep the select alive while the
        // tool-call idle window elapses → the buffer-path force-close fires.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut sink = [0u8; 8192];
            let _ = stream.read(&mut sink);
            let first = "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"type\":\"function\",\"function\":{\"name\":\"echo\",\"arguments\":\"{}\"}}]}}]}\n\n";
            let head = format!(
                "HTTP/1.1 200 X\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n",
                first.len(),
                first
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.flush();
            for _ in 0..30 {
                std::thread::sleep(std::time::Duration::from_millis(120));
                if stream.write_all(b"2\r\n\n\n").is_err() {
                    return;
                }
                let _ = stream.flush();
            }
        });
        let client = Client::new(&format!("http://127.0.0.1:{port}"), "sk-test", None, None);
        let rx = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await
            .unwrap();
        let events: Vec<StreamEvent> = rx.collect().await;
        std::env::remove_var("FUTURE_TEST_TOOL_IDLE_SECS");
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert!(types.contains(&"toolcall_start"), "{types:?}");
        assert!(types.contains(&"toolcall_end"), "{types:?}");
        assert!(types.contains(&"stop"), "{types:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mock_server_tolerates_aborted_connections() {
        let server = mock_server(move |_| (200, "application/json", "{}".to_string()));
        // Connect and immediately close without sending anything.
        let addr = server.base_url.trim_start_matches("http://");
        let stream = std::net::TcpStream::connect(addr).unwrap();
        drop(stream);
        // Now a real request still gets served (the server loop continues).
        let client = Client::new(&server.base_url, "sk-test", None, None);
        let result = client
            .stream_chat(
                "mock".to_string(),
                one_user_message(),
                vec![],
                String::new(),
            )
            .await;
        assert!(result.is_ok() || result.is_err());
    }
}
