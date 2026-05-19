//! LLM Client — 1:1 compatible with Go internal/llm/
//!
//! Uses reqwest for HTTP + SSE streaming, matching Go's OpenAI SDK behavior.

use crate::types::{Message, StreamEvent, ToolCall, ToolDef, Usage};
use anyhow::{anyhow, Result};
use futures::StreamExt;
use reqwest::Client as HttpClient;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

const DEFAULT_TIMEOUT_SECS: u64 = 120;

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

        eprintln!(
            "[LLM] Request to {}:\n{}",
            url,
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );

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
        eprintln!("[LLM] Response status: {}", resp.status());

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
            let resp_headers: Vec<String> = resp
                .headers()
                .iter()
                .map(|(k, v)| format!("{}: {:?}", k, v.to_str().unwrap_or("")))
                .collect();
            eprintln!(
                "[LLM] Error response headers ({}): {:?}",
                status_code, resp_headers
            );
            let text = resp.text().await.unwrap_or_default();
            eprintln!(
                "[LLM] Error response body ({}): {}",
                status_code,
                &text[..text.len().min(500)]
            );

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
            let mut buffer = String::new();

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
                }
                true // continue
            }

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(bytes) => {
                        if let Some(ref cb) = on_payload {
                            cb(&bytes);
                        }
                        // Use from_utf8_lossy — SSE structure is ASCII so \n\n
                        // delimiters are never affected by replacement chars.
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        // Process complete SSE events (delimited by \n\n).
                        // Keep trailing partial event in buffer for next chunk.
                        while let Some(pos) = buffer.find("\n\n") {
                            let event_block = buffer[..pos].to_string();
                            buffer = buffer[pos + 2..].to_string();
                            let mut done = false;
                            for line in event_block.lines() {
                                let line = line.trim();
                                if !line.starts_with("data: ") {
                                    continue;
                                }
                                let data = &line[6..];
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
                                break;
                            }
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

impl Client {
    fn apply_thinking_params(&self, body: &mut Value) {
        let thinking_level = self.thinking_level.read().unwrap();
        let thinking_budget = *self.thinking_budget.read().unwrap();

        if thinking_level.is_empty() && thinking_budget == 0 && self.reasoning_effort.is_empty() {
            return;
        }

        let compat_thinking_format = self.compat_thinking_format.read().unwrap();
        // Auto-detect qwen format for dashscope/aliyuncs endpoints when no explicit format set
        let effective_format: String = if compat_thinking_format.is_empty() {
            let base_url = self.base_url.read().unwrap();
            if base_url.contains("dashscope") || base_url.contains("aliyuncs") {
                "qwen".to_string()
            } else {
                String::new()
            }
        } else {
            compat_thinking_format.clone()
        };
        let effective_format_str = effective_format.as_str();
        if !effective_format.is_empty() {
            let reasoning_enabled = *thinking_level != "off";
            let mut level_value = thinking_level.clone();

            let thinking_level_map = self.thinking_level_map.read().unwrap();
            if let Some(mapped) = thinking_level_map.get(&*thinking_level) {
                level_value = mapped.clone();
            }
            drop(thinking_level_map);
            drop(thinking_level);

            match effective_format_str {
                "zai" => {
                    body["enable_thinking"] = serde_json::json!(reasoning_enabled);
                }
                "qwen" | "qwen-chat-template" => {
                    if effective_format_str == "qwen-chat-template" {
                        body["chat_template_kwargs"] = serde_json::json!({
                            "enable_thinking": reasoning_enabled,
                            "preserve_thinking": true,
                        });
                    } else {
                        body["enable_thinking"] = serde_json::json!(reasoning_enabled);
                    }
                    if reasoning_enabled && *self.compat_supports_reasoning_effort.read().unwrap() {
                        body["reasoning_effort"] = serde_json::json!(level_value);
                    }
                }
                "deepseek" => {
                    let thinking_type = if reasoning_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    };
                    let mut extra = serde_json::json!({
                        "thinking": { "type": thinking_type }
                    });
                    if reasoning_enabled {
                        extra["reasoning_effort"] = serde_json::json!(level_value);
                    }
                    for (k, v) in extra.as_object().unwrap() {
                        body[k] = v.clone();
                    }
                }
                "openrouter" | "openai"
                    if reasoning_enabled
                        && *self.compat_supports_reasoning_effort.read().unwrap() =>
                {
                    body["reasoning_effort"] = serde_json::json!(level_value);
                }
                _ => {}
            }
        }
        // When effective_format is empty (no compat thinking format configured),
        // don't add any thinking parameters — provider doesn't support it.
    }

    fn convert_messages_to_openai(
        messages: Vec<Message>,
        system_prompt: String,
        _needs_empty_reasoning: bool,
    ) -> Vec<Value> {
        let mut result = Vec::new();

        // Prepend system prompt
        if !system_prompt.is_empty() {
            result.push(serde_json::json!({
                "role": "system",
                "content": [{ "type": "text", "text": system_prompt }],
            }));
        }

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    result.push(serde_json::json!({
                        "role": "system",
                        "content": Self::extract_content(msg.content),
                    }));
                }
                "user" => {
                    result.push(serde_json::json!({
                        "role": "user",
                        "content": Self::extract_content(msg.content),
                    }));
                }
                "assistant" => {
                    let mut obj = serde_json::Map::new();
                    obj.insert("role".to_string(), serde_json::json!("assistant"));
                    if !msg.reasoning_content.is_empty() {
                        obj.insert(
                            "reasoning_content".to_string(),
                            serde_json::json!(msg.reasoning_content),
                        );
                    }
                    obj.insert("content".to_string(), Self::extract_content(msg.content));
                    if let Some(tcs) = msg.tool_calls {
                        let tools: Vec<Value> = tcs
                            .into_iter()
                            .map(|tc| {
                                serde_json::json!({
                                    "id": tc.id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.function.name,
                                        "arguments": tc.function.arguments,
                                    }
                                })
                            })
                            .collect();
                        obj.insert("tool_calls".to_string(), serde_json::json!(tools));
                    }
                    result.push(serde_json::json!(obj));
                }
                "tool" => {
                    let content = Self::extract_content(msg.content.clone());
                    let content_str = match &content {
                        Value::Array(arr) => arr
                            .first()
                            .and_then(|b| b.get("text"))
                            .and_then(|t| t.as_str())
                            .unwrap_or(""),
                        Value::String(s) => s.as_str(),
                        _ => "",
                    };
                    result.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": msg.tool_call_id,
                        "content": content_str,
                    }));
                }
                _ => {}
            }
        }

        result
    }

    fn extract_content(content: Option<Value>) -> Value {
        match content {
            Some(Value::String(s)) => serde_json::json!([{ "type": "text", "text": s }]),
            Some(Value::Array(arr)) => Value::Array(arr),
            Some(val) => val,
            None => serde_json::json!([{ "type": "text", "text": "" }]),
        }
    }

    fn parse_sse_chunk(data: &str) -> Result<StreamEvent> {
        let chunk: serde_json::Value = serde_json::from_str(data)?;

        let choices = chunk
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first());
        let delta = choices
            .and_then(|c| c.get("delta"))
            .cloned()
            .unwrap_or_default();

        let finish_reason = choices
            .and_then(|c| c.get("finish_reason"))
            .and_then(|fr| fr.as_str())
            .unwrap_or("");

        let mut event = StreamEvent {
            event_type: String::new(),
            text: String::new(),
            tool_call: None,
            tool_name: String::new(),
            tool_id: String::new(),
            usage: None,
            stop_reason: finish_reason.to_string(),
            error_text: String::new(),
        };

        // Usage — check BEFORE text/tool/stop checks, because some providers
        // (DeepSeek) include usage in the final chunk alongside an empty content
        // string and finish_reason. If we process content first, we return early
        // and never see the usage data.
        if let Some(usage_val) = chunk.get("usage").filter(|v| !v.is_null()) {
            if let Ok(mut usage) = serde_json::from_value::<Usage>(usage_val.clone()) {
                // DeepSeek nests cache tokens under prompt_tokens_details.
                if usage.cache_read_tokens.is_none() {
                    if let Some(cached) = usage_val
                        .get("prompt_tokens_details")
                        .and_then(|d| d.get("cached_tokens"))
                        .and_then(|v| v.as_i64())
                    {
                        usage.cache_read_tokens = Some(cached);
                    }
                }
                if usage.cache_write_tokens.is_none() {
                    if let Some(cached) = usage_val
                        .get("prompt_tokens_details")
                        .and_then(|d| d.get("cache_write_tokens"))
                        .and_then(|v| v.as_i64())
                    {
                        usage.cache_write_tokens = Some(cached);
                    }
                }
                event.usage = Some(usage);
                event.event_type = "usage".to_string();
                return Ok(event);
            }
        }

        // Reasoning content (from extra fields for DeepSeek-style)
        if let Some(rc) = delta.get("reasoning_content").or(delta.get("thinking")) {
            if let Some(s) = rc.as_str() {
                if !s.is_empty() {
                    event.event_type = "thinking_delta".to_string();
                    event.text = s.to_string();
                    return Ok(event);
                }
            }
        }

        // Text content (skip empty strings so usage in same chunk is not lost)
        if let Some(text) = delta.get("content").or(delta.get("text")) {
            if let Some(s) = text.as_str() {
                if !s.is_empty() {
                    event.event_type = "text_delta".to_string();
                    event.text = s.to_string();
                    return Ok(event);
                }
            }
        }

        // Tool calls
        if let Some(tcs) = delta.get("tool_calls").and_then(|tc| tc.as_array()) {
            for tc in tcs {
                let _idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                let has_id = tc.get("id").and_then(|v| v.as_str());
                let has_name = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str());
                let has_args = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|v| v.as_str());

                if let (Some(id), Some(name)) = (has_id, has_name) {
                    event.event_type = "toolcall_start".to_string();
                    event.tool_id = id.to_string();
                    event.tool_name = name.to_string();
                    event.tool_call = Some(ToolCall {
                        id: id.to_string(),
                        call_type: "function".to_string(),
                        function: crate::types::ToolCallFn {
                            name: name.to_string(),
                            arguments: tc
                                .get("function")
                                .and_then(|f| f.get("arguments"))
                                .cloned()
                                .unwrap_or(serde_json::Value::String(String::new())),
                        },
                    });
                    return Ok(event);
                } else if let Some(args_text) = has_args {
                    // Argument-only delta (subsequent chunks after toolcall_start)
                    event.event_type = "toolcall_delta".to_string();
                    event.text = args_text.to_string();
                    return Ok(event);
                }
            }
        }

        // Finish reason: tool_calls means the model finished emitting tool calls
        if finish_reason == "tool_calls" {
            event.event_type = "toolcall_end".to_string();
            return Ok(event);
        }

        // Empty text event (stop marker)
        if event.event_type.is_empty() {
            event.event_type = "stop".to_string();
        }

        Ok(event)
    }
}
