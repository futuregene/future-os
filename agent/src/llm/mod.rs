//! LLM Client — 1:1 compatible with Go internal/llm/
//!
//! Uses reqwest for HTTP + SSE streaming, matching Go's OpenAI SDK behavior.

use crate::types::{
    Message, StreamEvent, ToolCall, ToolDef, Usage,
};
use anyhow::{anyhow, Result};
use futures::StreamExt;
use reqwest::Client as HttpClient;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

const DEFAULT_MAX_RETRIES: usize = 3;
const DEFAULT_TIMEOUT_SECS: u64 = 120;

// ─── LLM Client ────────────────────────────────────────────────────────────

pub struct Client {
    http: HttpClient,
    base_url: String,
    api_key: String,
    reasoning_effort: String,
    tool_choice: Option<Value>,
    enable_cache_control: bool,
    thinking_budget: i32,
    stream_opts: Option<StreamOptions>,
    on_payload: Option<Arc<dyn Fn(&[u8]) + Send + Sync>>,
    on_response: Option<Arc<dyn Fn(u16, &HashMap<String, String>) + Send + Sync>>,
    is_cloudflare: bool,
    is_copilot: bool,
    thinking_level: String,
    thinking_level_map: HashMap<String, String>,
    compat_thinking_format: String,
    compat_supports_reasoning_effort: bool,
    compat_requires_reasoning_on_assistant: bool,
}

#[derive(Clone, Default)]
pub struct StreamOptions {
    pub thinking_budget: i32,
}

impl Client {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        let http = HttpClient::builder()
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|_| HttpClient::new());

        let is_cloudflare = base_url.contains("cloudflare") || base_url.contains("workers.dev");

        Self {
            http,
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            reasoning_effort: String::new(),
            tool_choice: None,
            enable_cache_control: false,
            thinking_budget: 0,
            stream_opts: None,
            on_payload: None,
            on_response: None,
            is_cloudflare,
            is_copilot: false,
            thinking_level: String::new(),
            thinking_level_map: HashMap::new(),
            compat_thinking_format: String::new(),
            compat_supports_reasoning_effort: false,
            compat_requires_reasoning_on_assistant: false,
        }
    }

    pub fn with_thinking_level(mut self, level: &str) -> Self {
        self.thinking_level = level.to_string();
        self
    }

    pub fn with_thinking_budget(mut self, budget: i32) -> Self {
        self.thinking_budget = budget;
        self
    }

    pub fn with_compat(
        mut self,
        format: &str,
        supports_reasoning_effort: bool,
        requires_reasoning_on_assistant: bool,
    ) -> Self {
        self.compat_thinking_format = format.to_string();
        self.compat_supports_reasoning_effort = supports_reasoning_effort;
        self.compat_requires_reasoning_on_assistant = requires_reasoning_on_assistant;
        self
    }

    pub fn with_thinking_level_map(mut self, map: HashMap<String, String>) -> Self {
        self.thinking_level_map = map;
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

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let mut body = serde_json::json!({
            "model": model,
            "messages": Self::convert_messages_to_openai(messages, system_prompt, self.compat_requires_reasoning_on_assistant),
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

        // Add thinking/reasoning parameters (compat format)
        self.apply_thinking_params(&mut body);

        eprintln!("[LLM] Request to {}: {}", url, serde_json::to_string(&body).unwrap_or_default());

        let req = self.http
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .build()?;

        let resp = self.http.execute(req).await?;
        eprintln!("[LLM] Response status: {}", resp.status());

        let status = resp.status();
        let headers: HashMap<String, String> = resp.headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        if let Some(ref cb) = self.on_response {
            cb(status.as_u16(), &headers);
        }

        if !status.is_success() {
            let text = resp.text().await?;
            return Err(anyhow!("LLM API error {}: {}", status, text));
        }

        let stream = resp.bytes_stream();
        let on_payload = self.on_payload.clone();

        tokio::spawn(async move {
            let mut stream = stream;
            let tx = tx;

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(bytes) => {
                        if let Some(ref cb) = on_payload {
                            cb(&bytes);
                        }
                        if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                            // Parse SSE lines
                            for line in text.lines() {
                                let line = line.trim();
                                if !line.starts_with("data: ") {
                                    continue;
                                }
                                let data = &line[6..];
                                if data == "[DONE]" {
                                    let _ = tx.send(StreamEvent {
                                        event_type: "stop".to_string(),
                                        text: String::new(),
                                        tool_call: None,
                                        tool_name: String::new(),
                                        tool_id: String::new(),
                                        usage: None,
                                        stop_reason: String::new(),
                                        error_text: String::new(),
                                    }).await;
                                    break;
                                }
                                if let Ok(event) = Self::parse_sse_chunk(data) {
                                    let _ = tx.send(event).await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(StreamEvent {
                            event_type: "error".to_string(),
                            text: String::new(),
                            tool_call: None,
                            tool_name: String::new(),
                            tool_id: String::new(),
                            usage: None,
                            stop_reason: String::new(),
                            error_text: e.to_string(),
                        }).await;
                    }
                }
            }

            let _ = tx.send(StreamEvent {
                event_type: "stop".to_string(),
                text: String::new(),
                tool_call: None,
                tool_name: String::new(),
                tool_id: String::new(),
                usage: None,
                stop_reason: String::new(),
                error_text: String::new(),
            }).await;

            Ok::<_, ()>(())
        });

        Ok(ReceiverStream::new(rx))
    }
}

impl Client {
    fn apply_thinking_params(&self, body: &mut Value) {
        if self.thinking_level.is_empty() && self.thinking_budget == 0 && self.reasoning_effort.is_empty() {
            return;
        }

        if !self.compat_thinking_format.is_empty() {
            let reasoning_enabled = self.thinking_level != "off";
            let mut level_value = self.thinking_level.clone();

            if let Some(mapped) = self.thinking_level_map.get(&self.thinking_level) {
                level_value = mapped.clone();
            }

            match self.compat_thinking_format.as_str() {
                "zai" => {
                    body["enable_thinking"] = serde_json::json!(reasoning_enabled);
                }
                "qwen" | "qwen-chat-template" => {
                    if self.compat_thinking_format == "qwen-chat-template" {
                        body["chat_template_kwargs"] = serde_json::json!({
                            "enable_thinking": reasoning_enabled,
                            "preserve_thinking": true,
                        });
                    } else {
                        body["enable_thinking"] = serde_json::json!(reasoning_enabled);
                    }
                }
                "deepseek" => {
                    let thinking_type = if reasoning_enabled { "enabled" } else { "disabled" };
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
                "openrouter" | "openai" => {
                    if reasoning_enabled && self.compat_supports_reasoning_effort {
                        body["reasoning_effort"] = serde_json::json!(level_value);
                    }
                }
                _ => {}
            }
        } else if self.thinking_budget > 0 {
            // Legacy fallback
            body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": self.thinking_budget,
            });
        } else if self.thinking_budget == 0 && self.reasoning_effort.is_empty() && self.thinking_level.is_empty() {
            // Explicitly disable
            body["thinking"] = serde_json::json!({ "type": "disabled" });
        }
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
                        obj.insert("reasoning_content".to_string(), serde_json::json!(msg.reasoning_content));
                    }
                    obj.insert("content".to_string(), Self::extract_content(msg.content));
                    if let Some(tcs) = msg.tool_calls {
                        let tools: Vec<Value> = tcs.into_iter().map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.function.name,
                                    "arguments": tc.function.arguments,
                                }
                            })
                        }).collect();
                        obj.insert("tool_calls".to_string(), serde_json::json!(tools));
                    }
                    result.push(serde_json::json!(obj));
                }
                "tool" => {
                    result.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": msg.tool_call_id,
                        "content": Self::extract_content(msg.content.clone()).as_str().unwrap_or(""),
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

        let choices = chunk.get("choices").and_then(|c| c.as_array()).and_then(|a| a.first());
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

        // Reasoning content (from extra fields for DeepSeek-style)
        if let Some(rc) = delta.get("reasoning_content").or(delta.get("thinking")) {
            if let Some(s) = rc.as_str() {
                event.event_type = "thinking_delta".to_string();
                event.text = s.to_string();
                return Ok(event);
            }
        }

        // Text content
        if let Some(text) = delta.get("content").or(delta.get("text")) {
            if let Some(s) = text.as_str() {
                event.event_type = "text_delta".to_string();
                event.text = s.to_string();
                return Ok(event);
            }
        }

        // Tool calls
        if let Some(tcs) = delta.get("tool_calls").and_then(|tc| tc.as_array()) {
            for tc in tcs {
                let _idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                if let (Some(id), Some(name)) = (
                    tc.get("id").and_then(|v| v.as_str()),
                    tc.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()),
                ) {
                    event.event_type = "toolcall_start".to_string();
                    event.tool_id = id.to_string();
                    event.tool_name = name.to_string();
                    event.tool_call = Some(ToolCall {
                        id: id.to_string(),
                        call_type: "function".to_string(),
                        function: crate::types::ToolCallFn {
                            name: name.to_string(),
                            arguments: tc.get("function")
                                .and_then(|f| f.get("arguments"))
                                .cloned()
                                .unwrap_or(serde_json::Value::String(String::new())),
                        },
                    });
                    return Ok(event);
                }
            }
        }

        // Usage
        if let Some(usage_val) = chunk.get("usage") {
            if let Ok(usage) = serde_json::from_value::<Usage>(usage_val.clone()) {
                event.usage = Some(usage);
                event.event_type = "usage".to_string();
                return Ok(event);
            }
        }

        // Empty text event (stop marker)
        if event.event_type.is_empty() {
            event.event_type = "stop".to_string();
        }

        Ok(event)
    }
}
