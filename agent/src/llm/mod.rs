//! LLM Client — 1:1 compatible with Go internal/llm/
//!
//! Uses reqwest for HTTP + SSE streaming, matching Go's OpenAI SDK behavior.

mod helpers;
use crate::types::{Message, StreamEvent, ToolDef};
use anyhow::{anyhow, Result};
use tracing::warn;
use futures::StreamExt;
use reqwest::Client as HttpClient;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

const DEFAULT_TIMEOUT_SECS: u64 = 600;
const STREAM_IDLE_TIMEOUT_SECS: u64 = 45;
const STREAM_TOOL_CALL_IDLE_TIMEOUT_SECS: u64 = 2;

// ─── LLM Client ────────────────────────────────────────────────────────────

pub struct Client {
    http: HttpClient,
    base_url: RwLock<String>,
    api_key: RwLock<String>,
    reasoning_effort: String,
    #[allow(dead_code)]
    tool_choice: Option<Value>,
    #[allow(dead_code)]
    enable_cache_control: bool,
    thinking_budget: RwLock<i32>,
    #[allow(dead_code)]
    stream_opts: Option<StreamOptions>,
    #[allow(clippy::type_complexity)]
    on_payload: Option<Arc<dyn Fn(&[u8]) + Send + Sync>>,
    #[allow(clippy::type_complexity)]
    on_response: Option<Arc<dyn Fn(u16, &HashMap<String, String>) + Send + Sync>>,
    #[allow(dead_code)]
    is_cloudflare: bool,
    #[allow(dead_code)]
    is_copilot: bool,
    thinking_level: RwLock<String>,
    thinking_level_map: RwLock<HashMap<String, String>>,
    compat_thinking_format: RwLock<String>,
    compat_supports_reasoning_effort: RwLock<bool>,
    compat_requires_reasoning_on_assistant: RwLock<bool>,
    max_tokens_field: RwLock<String>,
    temperature: Option<f32>,
    max_tokens: Option<i32>,
}

#[derive(Clone, Default)]
pub struct StreamOptions {
    pub thinking_budget: i32,
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

        let is_cloudflare = base_url.contains("cloudflare") || base_url.contains("workers.dev");

        Self {
            http,
            base_url: RwLock::new(base_url.to_string()),
            api_key: RwLock::new(api_key.to_string()),
            reasoning_effort: String::new(),
            tool_choice: None,
            enable_cache_control: false,
            thinking_budget: RwLock::new(0),
            stream_opts: None,
            on_payload: None,
            on_response: None,
            is_cloudflare,
            is_copilot: false,
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
        *self.thinking_level.write().unwrap() = level.to_string();
        self
    }

    pub fn with_thinking_budget(self, budget: i32) -> Self {
        *self.thinking_budget.write().unwrap() = budget;
        self
    }

    pub fn with_compat(
        self,
        format: &str,
        supports_reasoning_effort: bool,
        requires_reasoning_on_assistant: bool,
    ) -> Self {
        *self.compat_thinking_format.write().unwrap() = format.to_string();
        *self.compat_supports_reasoning_effort.write().unwrap() = supports_reasoning_effort;
        *self.compat_requires_reasoning_on_assistant.write().unwrap() =
            requires_reasoning_on_assistant;
        self
    }

    pub fn with_max_tokens_field(self, field: &str) -> Self {
        if !field.is_empty() {
            *self.max_tokens_field.write().unwrap() = field.to_string();
        }
        self
    }

    pub fn with_thinking_level_map(self, map: HashMap<String, String>) -> Self {
        *self.thinking_level_map.write().unwrap() = map;
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

impl Client {
    pub fn update_compat_dyn(
        &self,
        thinking_format: &str,
        supports_reasoning_effort: bool,
        requires_reasoning_on_assistant: bool,
        thinking_level_map: HashMap<String, String>,
    ) {
        *self.compat_thinking_format.write().unwrap() = thinking_format.to_string();
        *self.compat_supports_reasoning_effort.write().unwrap() = supports_reasoning_effort;
        *self.compat_requires_reasoning_on_assistant.write().unwrap() =
            requires_reasoning_on_assistant;
        *self.thinking_level_map.write().unwrap() = thinking_level_map;
    }

    pub fn update_thinking_dyn(&self, level: &str, budget: i32) {
        *self.thinking_level.write().unwrap() = level.to_string();
        *self.thinking_budget.write().unwrap() = budget;
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

        let base_url = self.base_url.read().unwrap().clone();
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

        let mut body = serde_json::json!({
            "model": model,
            "messages": Self::convert_messages_to_openai(messages, system_prompt, *self.compat_requires_reasoning_on_assistant.read().unwrap()),
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
        }

        // Add temperature
        if let Some(temp) = self.temperature {
            body["temperature"] = serde_json::json!(temp);
        }

        // Use model-specific max_tokens field name (from compat.maxTokensField)
        // pi sets maxTokensField to "max_completion_tokens" for o1/o3/gpt-5 reasoning models
        if let Some(mt) = self.max_tokens {
            let field = self.max_tokens_field.read().unwrap();
            body[field.as_str()] = serde_json::json!(mt);
        }

        // Add stream_options for usage stats in streaming
        body["stream_options"] = serde_json::json!({"include_usage": true});

        // Add thinking/reasoning parameters (compat format)
        self.apply_thinking_params(&mut body);

        let req = self
            .http
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.read().unwrap()),
            )
            .header("Content-Type", "application/json")
            .header(
                "User-Agent",
                concat!("future-agent/", env!("CARGO_PKG_VERSION")),
            )
            .json(&body)
            .build()?;

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

            // Parse Azure/OpenAI error body for a user-friendly message
            if let Ok(err_body) = serde_json::from_str::<serde_json::Value>(&text) {
                let code = err_body
                    .get("error")
                    .and_then(|e| e.get("code"))
                    .and_then(|c| c.as_str())
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
                    _ => {}
                }
            }

            return Err(anyhow!(
                "API request failed (HTTP {}). {}",
                status_code,
                if text.is_empty() {
                    "No response body.".to_string()
                } else if text.len() > 200 {
                    format!("{}…", &text[..200])
                } else {
                    text
                }
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

            // Helper to emit events from a parsed SSE data line, handling
            // thinking/tool-call bookending (matches original per-line logic).
            async fn process_data_line(
                data: &str,
                tx: &mpsc::Sender<StreamEvent>,
                in_thinking: &mut bool,
                in_tool_call: &mut bool,
            ) -> bool {
                if data == "[DONE]" {
                    if *in_tool_call {
                        let _ = tx
                            .send(StreamEvent {
                                event_type: "toolcall_end".to_string(),
                                text: String::new(),
                                tool_call: None,
                                tool_name: String::new(),
                                tool_id: String::new(),
                                usage: None,
                                stop_reason: String::new(),
                                error_text: String::new(),
                            })
                            .await;
                        *in_tool_call = false;
                    }
                    if *in_thinking {
                        let _ = tx
                            .send(StreamEvent {
                                event_type: "thinking_end".to_string(),
                                text: String::new(),
                                tool_call: None,
                                tool_name: String::new(),
                                tool_id: String::new(),
                                usage: None,
                                stop_reason: String::new(),
                                error_text: String::new(),
                            })
                            .await;
                        *in_thinking = false;
                    }
                    let _ = tx
                        .send(StreamEvent {
                            event_type: "stop".to_string(),
                            text: String::new(),
                            tool_call: None,
                            tool_name: String::new(),
                            tool_id: String::new(),
                            usage: None,
                            stop_reason: String::new(),
                            error_text: String::new(),
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
                                    text: String::new(),
                                    tool_call: None,
                                    tool_name: String::new(),
                                    tool_id: String::new(),
                                    usage: None,
                                    stop_reason: String::new(),
                                    error_text: String::new(),
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
                                text: String::new(),
                                tool_call: None,
                                tool_name: String::new(),
                                tool_id: String::new(),
                                usage: None,
                                stop_reason: String::new(),
                                error_text: String::new(),
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
                                text: String::new(),
                                tool_call: None,
                                tool_name: String::new(),
                                tool_id: String::new(),
                                usage: None,
                                stop_reason: "tool_calls".to_string(),
                                error_text: String::new(),
                            })
                            .await;
                        *in_tool_call = false;
                    }

                    if should_emit_thinking_end {
                        let _ = tx
                            .send(StreamEvent {
                                event_type: "thinking_end".to_string(),
                                text: String::new(),
                                tool_call: None,
                                tool_name: String::new(),
                                tool_id: String::new(),
                                usage: None,
                                stop_reason: String::new(),
                                error_text: String::new(),
                            })
                            .await;
                        *in_thinking = false;
                    }

                    if should_finish_response {
                        let _ = tx
                            .send(StreamEvent {
                                event_type: "stop".to_string(),
                                text: String::new(),
                                tool_call: None,
                                tool_name: String::new(),
                                tool_id: String::new(),
                                usage: None,
                                stop_reason,
                                error_text: String::new(),
                            })
                            .await;
                        return false;
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
                let chunk_result = match tokio::time::timeout(
                    std::time::Duration::from_secs(idle_timeout_secs),
                    stream.next(),
                )
                .await
                {
                    Ok(Some(chunk_result)) => chunk_result,
                    Ok(None) => break,
                    Err(_) => break,
                };

                match chunk_result {
                    Ok(bytes) => {
                        if let Some(ref cb) = on_payload {
                            cb(&bytes);
                        }
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
                        while let Some(pos) = buffer
                            .windows(2)
                            .position(|w| w == b"\n\n")
                        {
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
                                    text: String::new(),
                                    tool_call: None,
                                    tool_name: String::new(),
                                    tool_id: String::new(),
                                    usage: None,
                                    stop_reason: "tool_calls".to_string(),
                                    error_text: String::new(),
                                })
                                .await;
                            let _ = tx
                                .send(StreamEvent {
                                    event_type: "stop".to_string(),
                                    text: String::new(),
                                    tool_call: None,
                                    tool_name: String::new(),
                                    tool_id: String::new(),
                                    usage: None,
                                    stop_reason: "tool_calls".to_string(),
                                    error_text: String::new(),
                                })
                                .await;
                            return Ok::<_, ()>(());
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(StreamEvent {
                                event_type: "error".to_string(),
                                text: String::new(),
                                tool_call: None,
                                tool_name: String::new(),
                                tool_id: String::new(),
                                usage: None,
                                stop_reason: String::new(),
                                error_text: e.to_string(),
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
                        text: String::new(),
                        tool_call: None,
                        tool_name: String::new(),
                        tool_id: String::new(),
                        usage: None,
                        stop_reason: String::new(),
                        error_text: String::new(),
                    })
                    .await;
            }
            if in_thinking {
                let _ = tx
                    .send(StreamEvent {
                        event_type: "thinking_end".to_string(),
                        text: String::new(),
                        tool_call: None,
                        tool_name: String::new(),
                        tool_id: String::new(),
                        usage: None,
                        stop_reason: String::new(),
                        error_text: String::new(),
                    })
                    .await;
            }

            let _ = tx
                .send(StreamEvent {
                    event_type: "stop".to_string(),
                    text: String::new(),
                    tool_call: None,
                    tool_name: String::new(),
                    tool_id: String::new(),
                    usage: None,
                    stop_reason: String::new(),
                    error_text: String::new(),
                })
                .await;

            Ok::<_, ()>(())
        });

        Ok(ReceiverStream::new(rx))
    }

    fn update_compat(
        &self,
        thinking_format: &str,
        supports_reasoning_effort: bool,
        requires_reasoning_on_assistant: bool,
        thinking_level_map: HashMap<String, String>,
    ) {
        *self.compat_thinking_format.write().unwrap() = thinking_format.to_string();
        *self.compat_supports_reasoning_effort.write().unwrap() = supports_reasoning_effort;
        *self.compat_requires_reasoning_on_assistant.write().unwrap() =
            requires_reasoning_on_assistant;
        *self.thinking_level_map.write().unwrap() = thinking_level_map;
    }

    fn update_endpoint(&self, base_url: &str, api_key: &str) {
        *self.base_url.write().unwrap() = base_url.to_string();
        *self.api_key.write().unwrap() = api_key.to_string();
    }

    fn update_thinking(&self, level: &str, budget: i32) {
        *self.thinking_level.write().unwrap() = level.to_string();
        *self.thinking_budget.write().unwrap() = budget;
    }

    fn update_max_tokens_field(&self, field: &str) {
        *self.max_tokens_field.write().unwrap() = field.to_string();
    }
}
