use crate::types::{Message, StreamEvent, ToolCall, Usage};
use anyhow::Result;
use serde_json::Value;

use super::Client;

impl Client {
    pub(super) fn apply_thinking_params(&self, body: &mut Value) {
        let thinking_level = self.thinking_level.read().unwrap();
        let thinking_budget = *self.thinking_budget.read().unwrap();

        // Resolve the effective compat thinking format BEFORE deciding whether
        // to skip.  When a format is configured (explicitly or auto-detected)
        // we must still emit disable parameters for the "off" case — otherwise
        // the provider defaults to its own behaviour (often enabling thinking).
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

        // No explicit thinking level set at all — leave the provider default.
        if thinking_level.is_empty() {
            return;
        }

        // When thinking is "off" and there is NO compat format, there's nothing
        // to disable (the provider doesn't understand thinking params at all).
        if *thinking_level == "off"
            && effective_format.is_empty()
            && thinking_budget == 0
            && self.reasoning_effort.is_empty()
        {
            return;
        }

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
                "openrouter" | "openai" => {
                    if reasoning_enabled
                        && *self.compat_supports_reasoning_effort.read().unwrap()
                    {
                        body["reasoning_effort"] = serde_json::json!(level_value);
                    }
                    // When reasoning is off, intentionally emit nothing:
                    // models using this format don't reason by default.
                }
                "reasoning-split" => {
                    // MiniMax M3: reasoning_split controls *where* reasoning
                    // appears, not *whether*.  reasoning_split=false puts
                    // thinking inline in content (wrapped in <think> tags),
                    // which is worse than the default.  When "off", emit
                    // nothing and let the model default to its normal
                    // behaviour (reasoning in reasoning_content, where the
                    // agent can manage visibility).
                    if reasoning_enabled {
                        body["reasoning_split"] = serde_json::json!(true);
                    }
                }
                _ => {}
            }
        }
        // When effective_format is empty (no compat thinking format configured),
        // don't add any thinking parameters — provider doesn't support it.
    }

    pub(super) fn convert_messages_to_openai(
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
                    // Only include content field if there's actual content.
                    // When assistant has tool_calls but no text, content should be
                    // null/omitted to avoid "text content is empty" errors from
                    // strict providers like kimi-k2.7-code.
                    let has_content = msg.content.as_ref().is_some_and(|c| match c {
                        Value::Array(arr) => !arr.is_empty(),
                        Value::String(s) => !s.is_empty(),
                        Value::Null => false,
                        _ => true,
                    });
                    if has_content {
                        obj.insert("content".to_string(), Self::extract_content(msg.content));
                    } else {
                        obj.insert("content".to_string(), Value::Null);
                    }
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

    pub(super) fn extract_content(content: Option<Value>) -> Value {
        match content {
            Some(Value::String(s)) => serde_json::json!([{ "type": "text", "text": s }]),
            Some(Value::Array(arr)) => Value::Array(arr),
            Some(val) => val,
            None => serde_json::json!([{ "type": "text", "text": "" }]),
        }
    }

    pub(super) fn parse_sse_chunk(data: &str) -> Result<StreamEvent> {
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

                // GLM models through third-party gateways (Aliyun MaaS) send
                // empty-string id/name on incremental argument chunks
                // (``"id":"", "name":""``).  Only treat as toolcall_start
                // when BOTH id and name are non-empty; otherwise fall
                // through to the argument-delta path below.
                if let (Some(id), Some(name)) = (has_id, has_name) {
                    if !id.is_empty() && !name.is_empty() {
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
                    }
                }
                // Argument-only delta (subsequent chunks after toolcall_start,
                // and empty-id/name chunks from GLM gateway).
                if let Some(args_text) = has_args {
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

#[cfg(test)]
mod usage_parse_tests {
    use super::Client;

    #[test]
    fn parses_dashscope_empty_choices_usage_chunk() {
        let data = r#"{"choices":[],"object":"chat.completion.chunk","usage":{"prompt_tokens":11,"completion_tokens":215,"total_tokens":226,"completion_tokens_details":{"reasoning_tokens":201,"text_tokens":215},"prompt_tokens_details":{"text_tokens":11}},"created":1782699751,"model":"qwen3.6-plus","id":"x"}"#;
        let event = Client::parse_sse_chunk(data).expect("parse");
        assert_eq!(
            event.event_type, "usage",
            "got event_type={}",
            event.event_type
        );
        assert_eq!(event.usage.expect("usage present").completion_tokens, 215);
    }

    #[test]
    fn parses_deepseek_finish_chunk_usage() {
        let data = r#"{"choices":[{"index":0,"delta":{"content":"","reasoning_content":null},"finish_reason":"length"}],"usage":{"prompt_tokens":5,"completion_tokens":10,"total_tokens":15}}"#;
        let event = Client::parse_sse_chunk(data).expect("parse");
        assert_eq!(
            event.event_type, "usage",
            "got event_type={}",
            event.event_type
        );
        assert_eq!(event.usage.expect("usage present").completion_tokens, 10);
    }

    #[test]
    fn empty_id_name_is_toolcall_delta_not_start() {
        // GLM-5.2 through Aliyun MaaS sends empty-string id/name on
        // incremental argument chunks: {"id":"","name":"","arguments":"\"path\": "}
        // These must emit toolcall_delta, not toolcall_start.
        let data = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"","function":{"name":"","arguments":"\"path\": "}}]}}]}"#;
        let event = Client::parse_sse_chunk(data).expect("parse");
        assert_eq!(
            event.event_type, "toolcall_delta",
            "empty id/name should produce toolcall_delta, got {}",
            event.event_type
        );
        assert_eq!(event.text, "\"path\": ");
    }

    #[test]
    fn non_empty_id_name_is_toolcall_start() {
        let data = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_001","function":{"name":"read","arguments":"{}"}}]}}]}"#;
        let event = Client::parse_sse_chunk(data).expect("parse");
        assert_eq!(
            event.event_type, "toolcall_start",
            "non-empty id/name should produce toolcall_start, got {}",
            event.event_type
        );
        assert_eq!(event.tool_id, "call_001");
        assert_eq!(event.tool_name, "read");
    }
}

#[cfg(test)]
mod apply_thinking_params_tests {
    use super::Client;
    use serde_json::json;

    fn body() -> serde_json::Value {
        json!({"model": "qwen3.7-plus", "messages": []})
    }

    #[test]
    fn qwen_off_emits_enable_thinking_false() {
        // Regression: Qwen3 on DashScope defaults to thinking-on. Setting the
        // GUI thinking level to "off" must send `enable_thinking: false`;
        // otherwise the upstream ignores the omission and produces reasoning.
        let client = Client::new("https://future-os.cn/api", "k", None, None)
            .with_compat("qwen", true, false)
            .with_thinking_level("off");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(
            body.get("enable_thinking"),
            Some(&json!(false)),
            "off must produce enable_thinking=false, got {body}"
        );
        assert_eq!(body.get("reasoning_effort"), None);
    }

    #[test]
    fn qwen_high_emits_enable_thinking_true_and_effort() {
        let client = Client::new("https://future-os.cn/api", "k", None, None)
            .with_compat("qwen", true, false)
            .with_thinking_level("high");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("enable_thinking"), Some(&json!(true)));
        assert_eq!(body.get("reasoning_effort"), Some(&json!("high")));
    }

    #[test]
    fn qwen_off_autodetected_from_aliyuncs_base_url() {
        // No explicit compat format — the aliyuncs base URL auto-detects qwen,
        // and "off" must still emit enable_thinking=false.
        let client =
            Client::new("https://dashscope.aliyuncs.com/compatible-mode/v1", "k", None, None)
                .with_thinking_level("off");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("enable_thinking"), Some(&json!(false)));
    }

    #[test]
    fn deepseek_off_emits_thinking_disabled() {
        let client = Client::new("https://api.deepseek.com/v1", "k", None, None)
            .with_compat("deepseek", true, false)
            .with_thinking_level("off");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(
            body.get("thinking"),
            Some(&json!({ "type": "disabled" })),
            "off must produce thinking.type=disabled, got {body}"
        );
        assert_eq!(body.get("reasoning_effort"), None);
    }

    #[test]
    fn empty_format_off_emits_nothing() {
        // A provider that doesn't support thinking params: "off" must not
        // inject any thinking-related field (preserve provider default).
        let client = Client::new("https://example.com/v1", "k", None, None)
            .with_thinking_level("off");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("enable_thinking"), None);
        assert_eq!(body.get("thinking"), None);
        assert_eq!(body.get("reasoning_effort"), None);
    }

    #[test]
    fn empty_level_emits_nothing() {
        // No thinking level configured at all → nothing injected.
        let client = Client::new("https://future-os.cn/api", "k", None, None)
            .with_compat("qwen", true, false);
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("enable_thinking"), None);
    }

    #[test]
    fn openai_off_emits_nothing() {
        // openai-format models don't reason by default; "off" is the default
        // state so nothing needs to be injected.
        let client = Client::new("https://api.openai.com/v1", "k", None, None)
            .with_compat("openai", true, false)
            .with_thinking_level("off");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("reasoning_effort"), None);
        assert_eq!(body.get("enable_thinking"), None);
        assert_eq!(body.get("thinking"), None);
    }

    #[test]
    fn openai_high_emits_reasoning_effort() {
        let client = Client::new("https://api.openai.com/v1", "k", None, None)
            .with_compat("openai", true, false)
            .with_thinking_level("high");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("reasoning_effort"), Some(&json!("high")));
    }

    #[test]
    fn reasoning_split_off_emits_nothing() {
        // reasoning_split controls *where* reasoning appears, not *whether*.
        // Sending reasoning_split=false puts thinking inline in <think> tags,
        // which is worse.  "off" emits nothing — model uses its default.
        let client = Client::new("https://api.minimax.io/v1", "k", None, None)
            .with_compat("reasoning-split", false, false)
            .with_thinking_level("off");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("reasoning_split"), None);
    }

    #[test]
    fn reasoning_split_high_emits_true() {
        let client = Client::new("https://api.minimax.io/v1", "k", None, None)
            .with_compat("reasoning-split", false, false)
            .with_thinking_level("high");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("reasoning_split"), Some(&json!(true)));
    }

    #[test]
    fn openrouter_off_emits_nothing() {
        let client = Client::new("https://openrouter.ai/api/v1", "k", None, None)
            .with_compat("openrouter", true, false)
            .with_thinking_level("off");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("reasoning_effort"), None);
    }
}
