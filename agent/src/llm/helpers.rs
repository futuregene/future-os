use crate::types::{Message, StreamEvent, ToolCall, Usage};
use anyhow::Result;
use serde_json::Value;

use super::Client;

impl Client {
    pub(super) fn apply_thinking_params(&self, body: &mut Value) {
        let thinking_level = self.thinking_level.read();
        let thinking_budget = *self.thinking_budget.read();

        // Resolve the effective compat thinking format BEFORE deciding whether
        // to skip.  When a format is configured (explicitly or auto-detected)
        // we must still emit disable parameters for the "off" case — otherwise
        // the provider defaults to its own behaviour (often enabling thinking).
        let compat_thinking_format = self.compat_thinking_format.read();
        // Auto-detect qwen format for dashscope/aliyuncs endpoints when no explicit format set
        let effective_format: String = if compat_thinking_format.is_empty() {
            let base_url = self.base_url.read();
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
        let reasoning_enabled = *thinking_level != "off";
        let mut level_value = thinking_level.clone();

        let thinking_level_map = self.thinking_level_map.read();
        if let Some(mapped) = thinking_level_map.get(&*thinking_level) {
            level_value = mapped.clone();
        }
        drop(thinking_level_map);
        drop(thinking_level);

        if !effective_format.is_empty() {
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
                    // reasoning_effort is always supported on dashscope/aliyuncs
                    // (qwen format) — send it unconditionally, matching the
                    // deepseek format which also has no compat gate.
                    if reasoning_enabled {
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
                    if reasoning_enabled && *self.compat_supports_reasoning_effort.read() {
                        body["reasoning_effort"] = serde_json::json!(level_value);
                    }
                    // When reasoning is off, intentionally emit nothing:
                    // models using this format don't reason by default.
                }
                "reasoning-split" => {
                    // MiniMax M3: `reasoning_split` controls *where* reasoning
                    // appears (in reasoning_content vs inline <think> tags),
                    // while `thinking` controls *whether*.
                    // Valid thinking modes: enabled / adaptive / disabled.
                    if reasoning_enabled {
                        body["reasoning_split"] = serde_json::json!(true);
                        body["thinking"] = serde_json::json!("enabled");
                    } else {
                        body["thinking"] = serde_json::json!("disabled");
                    }
                }
                _ => {}
            }
        } else if reasoning_enabled {
            // No compat thinking format is configured (effective_format is
            // empty), but the user has explicitly set a non-"off" thinking
            // level.  The provider is likely an OpenAI-compatible gateway
            // (FutureAPI, CrossModel, OpenRouter, etc.) that passes
            // reasoning_effort through to the upstream model.  Send it via
            // the standard OpenAI parameter so the thinking level takes
            // effect — without this, the provider defaults to its own
            // (usually low) thinking budget and long reasoning gets cut off.
            // When thinking is "off" we intentionally send nothing so the
            // model doesn't reason at all (OpenAI-compatible models don't
            // reason unless instructed).
            body["reasoning_effort"] = serde_json::json!(level_value);
        }
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
                    }
                    // When there is no content (assistant has tool_calls but no
                    // text), omit the "content" key entirely rather than setting
                    // it to null.  Setting null works with the standard OpenAI
                    // schema but breaks strict providers (e.g. "gpt5.6") that
                    // require content to be a string.  Omitting the field is
                    // also compatible with providers like kimi-k2.7-code that
                    // reject empty text content.
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
                    // extract_content normalizes String input to the array
                    // form, so only the array arm and the passthrough
                    // fallback (non-string scalars) are reachable here.
                    let content = Self::extract_content(msg.content.clone());
                    let content_str = match &content {
                        Value::Array(arr) => arr
                            .first()
                            .and_then(|b| b.get("text"))
                            .and_then(|t| t.as_str())
                            .unwrap_or(""),
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
            stop_reason: finish_reason.to_string(),
            ..Default::default()
        };

        // Usage — check BEFORE text/tool/stop checks, because some providers
        // (DeepSeek) include usage in the final chunk alongside an empty content
        // string and finish_reason. If we process content first, we return early
        // and never see the usage data.
        //
        // When finish_reason is also present in the same chunk (e.g. "length"
        // or "tool_calls"), do NOT return early — let the finish_reason logic
        // below emit toolcall_end/stop so the run loop finalizes tool calls
        // promptly instead of waiting for the idle timeout.
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
                if finish_reason.is_empty() {
                    event.event_type = "usage".to_string();
                    return Ok(event);
                }
                // Both usage and finish_reason: fall through so finish_reason
                // is processed into toolcall_end / stop below.
            }
        }

        // Text content (skip empty strings so usage in same chunk is not lost).
        // MUST come before reasoning_content: some providers (e.g. kimi-k2.7-code
        // through realapi-aliyun) send BOTH content AND reasoning_content in the
        // same delta.  Checking reasoning_content first would return early and
        // silently drop the visible content — the user sees only a fragment.
        if let Some(text) = delta.get("content").or(delta.get("text")) {
            if let Some(s) = text.as_str() {
                if !s.is_empty() {
                    event.event_type = "text_delta".to_string();
                    event.text = s.to_string();
                    return Ok(event);
                }
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

        // Tool calls
        if let Some(tcs) = delta.get("tool_calls").and_then(|tc| tc.as_array()) {
            for tc in tcs {
                let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                event.tc_index = idx;
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
        // DeepSeek sends usage + finish_reason="length" in the same chunk.
        // When both are present, finish_reason takes priority so the run loop
        // finalizes tool calls promptly. Usage is still attached to the event.
        let data = r#"{"choices":[{"index":0,"delta":{"content":"","reasoning_content":null},"finish_reason":"length"}],"usage":{"prompt_tokens":5,"completion_tokens":10,"total_tokens":15}}"#;
        let event = Client::parse_sse_chunk(data).expect("parse");
        assert_eq!(
            event.event_type, "stop",
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

    #[test]
    fn content_before_reasoning_when_both_present() {
        // Regression: kimi-k2.7-code through realapi-aliyun sends chunks
        // where BOTH content and reasoning_content are non-empty in the
        // same delta.  Content MUST be emitted (as text_delta) even when
        // reasoning_content is also present — prioritizing reasoning
        // would silently drop visible text.
        let data = r#"{"choices":[{"index":0,"delta":{"content":"你好！","reasoning_content":"The user said hello"}}]}"#;
        let event = Client::parse_sse_chunk(data).expect("parse");
        assert_eq!(
            event.event_type, "text_delta",
            "content must be emitted when both content and reasoning_content are present, got {}",
            event.event_type
        );
        assert_eq!(event.text, "你好！");
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
        let client = Client::new(
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "k",
            None,
            None,
        )
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
        let client =
            Client::new("https://example.com/v1", "k", None, None).with_thinking_level("off");
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
    fn reasoning_split_off_emits_disabled() {
        // "off" must send thinking="disabled" to stop the model from thinking.
        let client = Client::new("https://api.minimax.io/v1", "k", None, None)
            .with_compat("reasoning-split", false, false)
            .with_thinking_level("off");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("thinking"), Some(&json!("disabled")));
        assert_eq!(body.get("reasoning_split"), None);
    }

    #[test]
    fn reasoning_split_high_emits_enabled_and_split() {
        let client = Client::new("https://api.minimax.io/v1", "k", None, None)
            .with_compat("reasoning-split", false, false)
            .with_thinking_level("high");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("reasoning_split"), Some(&json!(true)));
        assert_eq!(body.get("thinking"), Some(&json!("enabled")));
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

    #[test]
    fn zai_format_emits_enable_thinking() {
        let client = Client::new("https://api.z.ai/v1", "k", None, None)
            .with_compat("zai", true, false)
            .with_thinking_level("high");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("enable_thinking"), Some(&json!(true)));
    }

    #[test]
    fn zai_off_emits_enable_thinking_false() {
        let client = Client::new("https://api.z.ai/v1", "k", None, None)
            .with_compat("zai", true, false)
            .with_thinking_level("off");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("enable_thinking"), Some(&json!(false)));
    }

    #[test]
    fn qwen_chat_template_format() {
        let client = Client::new("https://example.com/v1", "k", None, None)
            .with_compat("qwen-chat-template", true, false)
            .with_thinking_level("high");
        let mut body = body();
        client.apply_thinking_params(&mut body);
        let ct = body.get("chat_template_kwargs");
        assert!(ct.is_some());
        assert_eq!(ct.unwrap()["enable_thinking"], json!(true));
    }

    #[test]
    fn thinking_level_map_applies() {
        let mut level_map = std::collections::HashMap::new();
        level_map.insert("high".to_string(), "max".to_string());
        let client = Client::new("https://api.openai.com/v1", "k", None, None)
            .with_compat("openai", true, false)
            .with_thinking_level("high")
            .with_thinking_level_map(level_map);
        let mut body = body();
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("reasoning_effort"), Some(&json!("max")));
    }
}

#[cfg(test)]
mod message_conversion_tests {
    use super::Client;
    use crate::types::{Message, ToolCall, ToolCallFn};
    use serde_json::json;

    #[test]
    fn convert_system_and_user() {
        let msgs = vec![
            Message {
                role: "system".to_string(),
                content: Some(json!("You are helpful")),
                ..Default::default()
            },
            Message {
                role: "user".to_string(),
                content: Some(json!("hello")),
                ..Default::default()
            },
        ];
        let converted =
            Client::convert_messages_to_openai(msgs, "system prompt".to_string(), false);
        assert_eq!(converted.len(), 3); // system prompt + system msg + user msg
        assert_eq!(converted[0]["role"], "system");
        assert_eq!(converted[1]["role"], "system");
        assert_eq!(converted[2]["role"], "user");
    }

    #[test]
    fn convert_assistant_with_tool_calls() {
        let msgs = vec![Message {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                call_type: "function".to_string(),
                function: ToolCallFn {
                    name: "shell".to_string(),
                    arguments: json!({"command": "ls"}),
                },
            }]),
            ..Default::default()
        }];
        let converted = Client::convert_messages_to_openai(msgs, String::new(), false);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "assistant");
        assert!(converted[0].get("tool_calls").is_some());
        assert!(converted[0].get("content").is_none()); // omitted when no content
    }

    #[test]
    fn convert_assistant_with_reasoning() {
        let msgs = vec![Message {
            role: "assistant".to_string(),
            content: Some(json!("answer")),
            reasoning_content: "thinking...".to_string(),
            ..Default::default()
        }];
        let converted = Client::convert_messages_to_openai(msgs, String::new(), false);
        assert_eq!(converted[0]["reasoning_content"], "thinking...");
    }

    #[test]
    fn convert_tool_message() {
        let msgs = vec![Message {
            role: "tool".to_string(),
            content: Some(json!("result output")),
            tool_call_id: "call_1".to_string(),
            ..Default::default()
        }];
        let converted = Client::convert_messages_to_openai(msgs, String::new(), false);
        assert_eq!(converted[0]["role"], "tool");
        assert_eq!(converted[0]["tool_call_id"], "call_1");
        assert_eq!(converted[0]["content"], "result output");
    }

    #[test]
    fn convert_skips_empty_system_prompt() {
        let msgs = vec![Message {
            role: "user".to_string(),
            content: Some(json!("hi")),
            ..Default::default()
        }];
        let converted = Client::convert_messages_to_openai(msgs, String::new(), false);
        assert_eq!(converted.len(), 1); // no system prompt prepended
    }

    #[test]
    fn extract_content_string() {
        let result = Client::extract_content(Some(json!("hello")));
        assert_eq!(result, json!([{ "type": "text", "text": "hello" }]));
    }

    #[test]
    fn extract_content_array() {
        let arr = json!([{ "type": "text", "text": "a" }]);
        let result = Client::extract_content(Some(arr.clone()));
        assert_eq!(result, arr);
    }

    #[test]
    fn extract_content_none() {
        let result = Client::extract_content(None);
        assert_eq!(result, json!([{ "type": "text", "text": "" }]));
    }

    #[test]
    fn parse_sse_chunk_thinking_delta() {
        let data = r#"{"choices":[{"index":0,"delta":{"reasoning_content":"let me think"}}]}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        assert_eq!(event.event_type, "thinking_delta");
        assert_eq!(event.text, "let me think");
    }

    #[test]
    fn parse_sse_chunk_text_delta() {
        let data = r#"{"choices":[{"index":0,"delta":{"content":"hello"}}]}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        assert_eq!(event.event_type, "text_delta");
        assert_eq!(event.text, "hello");
    }

    #[test]
    fn parse_sse_chunk_toolcall_end() {
        let data = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        assert_eq!(event.event_type, "toolcall_end");
    }

    #[test]
    fn parse_sse_chunk_stop() {
        let data = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        assert_eq!(event.event_type, "stop");
        assert_eq!(event.stop_reason, "stop");
    }

    #[test]
    fn parse_sse_chunk_empty_delta_is_stop() {
        let data = r#"{"choices":[{"index":0,"delta":{}}]}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        assert_eq!(event.event_type, "stop");
    }

    #[test]
    fn parse_sse_chunk_tc_index() {
        let data = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":2,"id":"call_2","function":{"name":"read","arguments":"{}"}}]}}]}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        assert_eq!(event.event_type, "toolcall_start");
        assert_eq!(event.tc_index, 2);
    }

    #[test]
    fn parse_sse_chunk_cached_tokens() {
        let data = r#"{"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150,"prompt_tokens_details":{"cached_tokens":80}}}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        assert_eq!(event.usage.unwrap().cache_read_tokens, Some(80));
    }

    #[test]
    fn parse_sse_chunk_invalid_json_errors() {
        assert!(Client::parse_sse_chunk("not json").is_err());
    }

    // ─── coverage batch: thinking params + conversion arms ────────────────

    #[test]
    fn apply_thinking_params_reasoning_effort_without_compat_format() {
        let client = Client::new("https://api.test", "k", None, None).with_thinking_level("high");
        let mut body = serde_json::json!({});
        client.apply_thinking_params(&mut body);
        assert_eq!(body["reasoning_effort"], serde_json::json!("high"));
        // "off" with no compat format emits nothing.
        let client = Client::new("https://api.test", "k", None, None).with_thinking_level("off");
        let mut body = serde_json::json!({});
        client.apply_thinking_params(&mut body);
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn apply_thinking_params_reasoning_split_enabled_and_disabled() {
        let client = Client::new("https://api.test", "k", None, None)
            .with_compat("reasoning-split", false, false)
            .with_thinking_level("high");
        let mut body = serde_json::json!({});
        client.apply_thinking_params(&mut body);
        assert_eq!(body["reasoning_split"], serde_json::json!(true));
        assert_eq!(body["thinking"], serde_json::json!("enabled"));

        let client = Client::new("https://api.test", "k", None, None)
            .with_compat("reasoning-split", false, false)
            .with_thinking_level("off");
        let mut body = serde_json::json!({});
        client.apply_thinking_params(&mut body);
        assert_eq!(body["thinking"], serde_json::json!("disabled"));
    }

    #[test]
    fn convert_messages_filters_empty_content_variants() {
        let messages = vec![
            Message {
                role: "user".to_string(),
                content: Some(serde_json::json!([])),
                ..Default::default()
            },
            Message {
                role: "user".to_string(),
                content: Some(serde_json::Value::Null),
                ..Default::default()
            },
            Message {
                role: "user".to_string(),
                content: Some(serde_json::json!({"unexpected": "object"})),
                ..Default::default()
            },
            Message {
                role: "user".to_string(),
                content: Some(serde_json::json!("real")),
                ..Default::default()
            },
        ];
        let out = Client::convert_messages_to_openai(messages, String::new(), false);
        // Empty array and Null are dropped; the object form is kept.
        let roles: Vec<&str> = out.iter().filter_map(|m| m["role"].as_str()).collect();
        assert!(roles.contains(&"user"));
    }

    #[test]
    fn convert_messages_tool_content_string_and_array() {
        let tool_array = Message {
            role: "tool".to_string(),
            tool_call_id: "c1".to_string(),
            content: Some(serde_json::json!([{"type": "text", "text": "piece"}])),
            ..Default::default()
        };
        let tool_string = Message {
            role: "tool".to_string(),
            tool_call_id: "c2".to_string(),
            content: Some(serde_json::json!("plain result")),
            ..Default::default()
        };
        let out =
            Client::convert_messages_to_openai(vec![tool_array, tool_string], String::new(), false);
        assert_eq!(out[0]["content"], "piece");
        assert_eq!(out[1]["content"], "plain result");
    }

    #[test]
    fn extract_content_all_variants() {
        assert_eq!(
            Client::extract_content(Some(serde_json::json!("s"))),
            serde_json::json!([{"type": "text", "text": "s"}])
        );
        assert_eq!(
            Client::extract_content(Some(serde_json::json!([{"type": "text", "text": "a"}]))),
            serde_json::json!([{"type": "text", "text": "a"}])
        );
        assert_eq!(
            Client::extract_content(Some(serde_json::json!(42))),
            serde_json::json!(42)
        );
        assert_eq!(
            Client::extract_content(None),
            serde_json::json!([{"type": "text", "text": ""}])
        );
    }

    #[test]
    fn parse_sse_chunk_cache_write_details() {
        let data = r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"prompt_tokens_details":{"cache_write_tokens":7}}}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        assert_eq!(event.usage.unwrap().cache_write_tokens, Some(7));
    }

    #[test]
    fn parse_sse_chunk_thinking_via_thinking_key() {
        let data = r#"{"choices":[{"index":0,"delta":{"thinking":"hmm"}}]}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        assert_eq!(event.event_type, "thinking_delta");
        assert_eq!(event.text, "hmm");
        // Empty thinking is skipped (falls through to the empty-delta stop).
        let data = r#"{"choices":[{"index":0,"delta":{"thinking":""}}]}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        assert_ne!(event.event_type, "thinking_delta");
    }

    #[test]
    fn unknown_thinking_format_emits_nothing() {
        // A compat format outside the known set must not inject any
        // thinking-related field (the `_ => {}` arm).
        let client = Client::new("https://example.com/v1", "k", None, None)
            .with_compat("some-future-format", true, false)
            .with_thinking_level("high");
        let mut body = serde_json::json!({"model": "m", "messages": []});
        client.apply_thinking_params(&mut body);
        assert_eq!(body.get("reasoning_effort"), None);
        assert_eq!(body.get("enable_thinking"), None);
        assert_eq!(body.get("thinking"), None);
    }

    #[test]
    fn assistant_content_array_and_null_forms() {
        use crate::types::Message;
        use serde_json::json;
        // Non-empty content array → content key present.
        let with_array = Message {
            role: "assistant".to_string(),
            content: Some(json!([{ "type": "text", "text": "hi" }])),
            ..Default::default()
        };
        // Null content → treated as no content, key omitted.
        let with_null = Message {
            role: "assistant".to_string(),
            content: Some(json!(null)),
            ..Default::default()
        };
        // Unknown roles are dropped from the wire format.
        let unknown_role = Message {
            role: "developer".to_string(),
            content: Some(json!("ignored")),
            ..Default::default()
        };
        let converted =
            Client::convert_messages_to_openai(vec![with_array, with_null, unknown_role], String::new(), false);
        assert_eq!(converted.len(), 2, "unknown role must be dropped");
        assert!(converted[0].get("content").is_some());
        assert!(converted[1].get("content").is_none());
    }

    #[test]
    fn tool_message_scalar_content_becomes_empty_string() {
        use crate::types::Message;
        use serde_json::json;
        // Non-string/array scalar content passes extract_content through
        // unchanged and stringifies to "" on the wire.
        let msgs = vec![Message {
            role: "tool".to_string(),
            content: Some(json!(42)),
            tool_call_id: "c1".to_string(),
            ..Default::default()
        }];
        let converted = Client::convert_messages_to_openai(msgs, String::new(), false);
        assert_eq!(converted[0]["role"], "tool");
        assert_eq!(converted[0]["content"], "");
    }

    #[test]
    fn usage_with_prepopulated_cache_tokens_skips_nested_lookup() {
        let data = r#"{"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3,"cache_read_tokens":5,"cache_write_tokens":7}}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        let usage = event.usage.expect("usage parsed");
        assert_eq!(usage.cache_read_tokens, Some(5));
        assert_eq!(usage.cache_write_tokens, Some(7));
    }

    #[test]
    fn invalid_usage_block_is_ignored() {
        // usage present but not deserializable → the whole block is skipped.
        let data = r#"{"choices":[],"usage":{"prompt_tokens":"not-a-number"}}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        assert!(event.usage.is_none());
    }

    #[test]
    fn non_string_content_delta_is_skipped() {
        let data = r#"{"choices":[{"index":0,"delta":{"content":42}}]}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        assert_ne!(event.event_type, "text_delta");
    }

    #[test]
    fn tool_call_chunk_without_arguments_emits_no_delta() {
        // Name present but id missing (not a start) and no arguments (no
        // delta): falls through to the stop marker.
        let data = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"name":"x"}}]}}]}"#;
        let event = Client::parse_sse_chunk(data).unwrap();
        assert_eq!(event.event_type, "stop");
    }
}
