use super::{parse_json_arguments, ProtocolAdapter};
use crate::llm::schema::{
    ApiProtocol, ChatReasoningFormat, FinishReason, ModelRequest, ModelStreamEvent, ProtocolConfig,
    ResolvedModelTarget,
};
use crate::llm::sse::SseFrame;
use crate::types::{ContentBlock, ProviderMetadata, Usage};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use std::any::Any;
use std::collections::BTreeMap;

pub struct OpenAiChatAdapter;

#[derive(Debug, Default)]
struct ChatToolState {
    id: String,
    name: String,
    arguments: String,
    started: bool,
}

#[derive(Debug, Default)]
struct ChatStreamState {
    text_open: bool,
    reasoning_open: bool,
    tools: BTreeMap<usize, ChatToolState>,
    finished: bool,
}

impl ProtocolAdapter for OpenAiChatAdapter {
    fn protocol(&self) -> ApiProtocol {
        ApiProtocol::OpenAiChatCompletions
    }

    fn endpoint_path(&self) -> &'static str {
        "/chat/completions"
    }

    fn build_body(&self, target: &ResolvedModelTarget, request: &ModelRequest) -> Result<Value> {
        let ProtocolConfig::OpenAiChat(config) = &target.protocol else {
            bail!("OpenAI Chat adapter received a non-chat target")
        };
        let mut body = json!({
            "model": request.model,
            "messages": lower_messages(request, config.replay_assistant_reasoning),
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": "function",
                            "function": {
                                "name": tool.function.name,
                                "description": tool.function.description,
                                "parameters": tool.function.parameters,
                            }
                        })
                    })
                    .collect(),
            );
            if config.tool_stream {
                body["tool_stream"] = Value::Bool(true);
            }
        }
        if let Some(temperature) = target.generation.temperature {
            body["temperature"] = json!(temperature);
        }
        if let Some(max_tokens) = target.generation.max_output_tokens {
            body[config.max_tokens_field.key()] = json!(max_tokens);
        }
        apply_reasoning(&mut body, target, config);
        Ok(body)
    }

    fn new_stream_state(&self) -> Box<dyn Any + Send> {
        Box::<ChatStreamState>::default()
    }

    fn decode_frame(
        &self,
        frame: &SseFrame,
        state: &mut (dyn Any + Send),
    ) -> Result<Vec<ModelStreamEvent>> {
        let state = state
            .downcast_mut::<ChatStreamState>()
            .ok_or_else(|| anyhow!("invalid OpenAI Chat stream state"))?;
        if frame.data.trim() == "[DONE]" {
            return finish(state, FinishReason::Incomplete, None);
        }
        if frame.data.trim().is_empty() {
            return Ok(Vec::new());
        }
        let chunk: Value = serde_json::from_str(&frame.data)?;
        if let Some(error) = chunk.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("OpenAI-compatible stream error");
            state.finished = true;
            return Ok(vec![ModelStreamEvent::Error {
                message: message.to_string(),
            }]);
        }

        let mut events = Vec::new();
        let usage = chunk
            .get("usage")
            .filter(|value| !value.is_null())
            .map(chat_usage);
        let choice = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first());
        let delta = choice
            .and_then(|choice| choice.get("delta"))
            .cloned()
            .unwrap_or(Value::Null);

        if let Some(text) = delta
            .get("content")
            .or_else(|| delta.get("text"))
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            if !state.text_open {
                state.text_open = true;
                events.push(ModelStreamEvent::TextStart { id: "text".into() });
            }
            events.push(ModelStreamEvent::TextDelta {
                id: "text".into(),
                text: text.to_string(),
            });
        }
        if let Some(text) = delta
            .get("reasoning_content")
            .or_else(|| delta.get("thinking"))
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            if !state.reasoning_open {
                state.reasoning_open = true;
                events.push(ModelStreamEvent::ReasoningStart {
                    id: "reasoning".into(),
                });
            }
            events.push(ModelStreamEvent::ReasoningDelta {
                id: "reasoning".into(),
                text: text.to_string(),
            });
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let id = call.get("id").and_then(Value::as_str).unwrap_or("");
                let function = call.get("function").unwrap_or(&Value::Null);
                let name = function.get("name").and_then(Value::as_str).unwrap_or("");
                let incoming = function
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let tool = state.tools.entry(index).or_default();
                if !id.is_empty() {
                    tool.id = id.to_string();
                }
                if !name.is_empty() {
                    tool.name = name.to_string();
                }
                if !tool.started && !tool.id.is_empty() && !tool.name.is_empty() {
                    tool.started = true;
                    events.push(ModelStreamEvent::ToolInputStart {
                        index,
                        id: tool.id.clone(),
                        name: tool.name.clone(),
                        arguments: None,
                        provider_metadata: ProviderMetadata::new(),
                    });
                }
                if !incoming.is_empty() {
                    let (delta, snapshot) = normalize_tool_delta(&tool.arguments, incoming);
                    if snapshot {
                        tool.arguments = incoming.to_string();
                    } else {
                        tool.arguments.push_str(&delta);
                    }
                    if !delta.is_empty() || snapshot {
                        events.push(ModelStreamEvent::ToolInputDelta {
                            index,
                            id: tool.id.clone(),
                            delta,
                            snapshot,
                        });
                    }
                }
            }
        }

        let finish_reason = choice
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(Value::as_str)
            .filter(|reason| !reason.is_empty());
        if let Some(reason) = finish_reason {
            events.extend(finish(state, map_finish_reason(reason), usage)?);
        } else if let Some(usage) = usage {
            events.push(ModelStreamEvent::Usage(usage));
        }
        Ok(events)
    }

    fn finish_stream(&self, state: &mut (dyn Any + Send)) -> Result<Vec<ModelStreamEvent>> {
        let state = state
            .downcast_mut::<ChatStreamState>()
            .ok_or_else(|| anyhow!("invalid OpenAI Chat stream state"))?;
        if state.finished {
            Ok(Vec::new())
        } else {
            finish(state, FinishReason::Incomplete, None)
        }
    }
}

fn lower_messages(request: &ModelRequest, replay_reasoning: bool) -> Vec<Value> {
    let mut messages = Vec::new();
    if !request.system_prompt.is_empty() {
        messages.push(json!({"role": "system", "content": request.system_prompt}));
    }
    for message in &request.messages {
        let blocks = message.model_content();
        let mut tool_results = Vec::new();
        let mut content = Vec::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();
        for block in blocks {
            match block {
                ContentBlock::ToolResult {
                    tool_call_id,
                    content: result,
                    ..
                } => tool_results.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": result,
                })),
                ContentBlock::Text { text } => content.push(json!({"type": "text", "text": text})),
                ContentBlock::Image { image_url } => {
                    if let Some(url) = image_url.url {
                        content.push(json!({"type": "image_url", "image_url": {"url": url}}));
                    }
                }
                ContentBlock::Reasoning { text, .. } => reasoning.push_str(&text),
                ContentBlock::ToolCall { id, name, args, .. } => tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments_string(&args)},
                })),
            }
        }
        if !tool_results.is_empty() {
            messages.extend(tool_results);
            continue;
        }
        let mut wire = Map::new();
        wire.insert("role".into(), json!(message.role));
        if !content.is_empty() {
            wire.insert("content".into(), Value::Array(content));
        }
        if replay_reasoning && !reasoning.is_empty() {
            wire.insert("reasoning_content".into(), Value::String(reasoning));
        }
        if !tool_calls.is_empty() {
            wire.insert("tool_calls".into(), Value::Array(tool_calls));
        }
        if wire.len() > 1 {
            messages.push(Value::Object(wire));
        }
    }
    messages
}

fn apply_reasoning(
    body: &mut Value,
    target: &ResolvedModelTarget,
    config: &crate::llm::schema::OpenAiChatConfig,
) {
    let level = target.generation.thinking_level.as_str();
    if level.is_empty() {
        return;
    }
    let enabled = level != "off";
    let mapped = target
        .capabilities
        .reasoning
        .levels
        .get(level)
        .and_then(Value::as_str)
        .unwrap_or(level);
    match config.reasoning {
        ChatReasoningFormat::None => {
            if enabled && target.capabilities.reasoning.supported {
                body["reasoning_effort"] = json!(mapped);
            }
        }
        ChatReasoningFormat::ReasoningEffort => {
            if enabled && config.supports_reasoning_effort {
                body["reasoning_effort"] = json!(mapped);
            }
        }
        ChatReasoningFormat::Qwen { chat_template } => {
            if chat_template {
                body["chat_template_kwargs"] = json!({
                    "enable_thinking": enabled,
                    "preserve_thinking": true,
                });
            } else {
                body["enable_thinking"] = json!(enabled);
            }
            if enabled {
                body["reasoning_effort"] = json!(mapped);
            }
        }
        ChatReasoningFormat::DeepSeek => {
            body["thinking"] = json!({"type": if enabled { "enabled" } else { "disabled" }});
            if enabled {
                body["reasoning_effort"] = json!(mapped);
            }
        }
        ChatReasoningFormat::Zai => body["enable_thinking"] = json!(enabled),
        ChatReasoningFormat::ReasoningSplit => {
            body["thinking"] = json!(if enabled { "enabled" } else { "disabled" });
            if enabled {
                body["reasoning_split"] = Value::Bool(true);
            }
        }
    }
}

fn arguments_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn normalize_tool_delta(current: &str, incoming: &str) -> (String, bool) {
    if incoming.starts_with('{') {
        if current.is_empty() || current == "{}" {
            return (incoming.to_string(), true);
        }
        if let Some(suffix) = incoming.strip_prefix(current) {
            return (suffix.to_string(), false);
        }
        // A standard incremental fragment can itself start with `{` when the
        // tool argument contains JSON text (for example write.content).  It is
        // not a cumulative snapshot unless it extends the already accumulated
        // prefix; replacing here would discard earlier fields such as `path`.
        return (incoming.to_string(), false);
    }
    (incoming.to_string(), false)
}

fn chat_usage(value: &Value) -> Usage {
    Usage {
        prompt_tokens: value
            .get("prompt_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        completion_tokens: value
            .get("completion_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        total_tokens: value
            .get("total_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        cache_read_tokens: value
            .get("prompt_tokens_details")
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_i64),
        cache_write_tokens: value
            .get("prompt_tokens_details")
            .and_then(|details| details.get("cache_write_tokens"))
            .and_then(Value::as_i64),
        reasoning_tokens: value
            .get("completion_tokens_details")
            .and_then(|details| details.get("reasoning_tokens"))
            .and_then(Value::as_i64),
        credit_cost: value
            .get("credit_cost")
            .and_then(|cost| cost.as_f64().or_else(|| cost.as_str()?.parse().ok())),
        provider_metadata: None,
    }
}

fn map_finish_reason(reason: &str) -> FinishReason {
    match reason {
        "stop" => FinishReason::Stop,
        "tool_calls" | "function_call" => FinishReason::ToolCalls,
        "length" | "max_tokens" => FinishReason::Length,
        "content_filter" => FinishReason::ContentFilter,
        other => FinishReason::Unknown(other.to_string()),
    }
}

fn finish(
    state: &mut ChatStreamState,
    reason: FinishReason,
    usage: Option<Usage>,
) -> Result<Vec<ModelStreamEvent>> {
    if state.finished {
        return Ok(Vec::new());
    }
    let mut events = Vec::new();
    if state.reasoning_open {
        events.push(ModelStreamEvent::ReasoningEnd {
            id: "reasoning".into(),
            provider_metadata: ProviderMetadata::new(),
        });
        state.reasoning_open = false;
    }
    if state.text_open {
        events.push(ModelStreamEvent::TextEnd { id: "text".into() });
        state.text_open = false;
    }
    for (index, tool) in &state.tools {
        if tool.started {
            events.push(ModelStreamEvent::ToolInputEnd {
                index: *index,
                id: tool.id.clone(),
                name: tool.name.clone(),
                arguments: parse_json_arguments(&Value::String(tool.arguments.clone())),
                provider_metadata: ProviderMetadata::new(),
            });
        }
    }
    events.push(ModelStreamEvent::Finish { reason, usage });
    state.finished = true;
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::schema::{
        ChatReasoningFormat, GenerationConfig, OpenAiChatConfig, ProviderRoute,
    };

    fn frame(data: Value) -> SseFrame {
        SseFrame {
            event: None,
            data: data.to_string(),
        }
    }

    #[test]
    fn tool_snapshots_are_normalized() {
        assert_eq!(
            normalize_tool_delta("", "{\"a\":1}"),
            ("{\"a\":1}".into(), true)
        );
        assert_eq!(
            normalize_tool_delta("{\"a\":", "{\"a\":1}"),
            ("1}".into(), false)
        );
    }

    #[test]
    fn opening_brace_inside_json_string_remains_incremental() {
        let current = r#"{"path":"/tmp/data.json","content":""#;
        let incoming = r#"{\"metadata\":{\"version\":1}"#;

        assert_eq!(
            normalize_tool_delta(current, incoming),
            (incoming.into(), false)
        );
    }

    #[test]
    fn decodes_json_content_fragment_without_losing_path() {
        let adapter = OpenAiChatAdapter;
        let mut state = adapter.new_stream_state();
        let mut events = adapter
            .decode_frame(
                &frame(json!({
                    "choices": [{"delta": {"tool_calls": [{
                        "index": 0,
                        "id": "call_write",
                        "function": {
                            "name": "write",
                            "arguments": r#"{"path":"/tmp/data.json","content":""#,
                        },
                    }]}}],
                })),
                state.as_mut(),
            )
            .unwrap();
        events.extend(
            adapter
                .decode_frame(
                    &frame(json!({
                        "choices": [{"delta": {"tool_calls": [{
                            "index": 0,
                            "function": {
                                "arguments": r#"{\"metadata\":{\"version\":1}}"}"#,
                            },
                        }]}}],
                    })),
                    state.as_mut(),
                )
                .unwrap(),
        );
        events.extend(
            adapter
                .decode_frame(
                    &frame(json!({
                        "choices": [{"delta": {}, "finish_reason": "tool_calls"}],
                    })),
                    state.as_mut(),
                )
                .unwrap(),
        );

        let arguments = events
            .iter()
            .find_map(|event| match event {
                ModelStreamEvent::ToolInputEnd { arguments, .. } => Some(arguments),
                _ => None,
            })
            .expect("tool input should finish");
        assert_eq!(arguments["path"], "/tmp/data.json");
        assert_eq!(arguments["content"], r#"{"metadata":{"version":1}}"#);
    }

    #[test]
    fn builds_chat_body_from_canonical_blocks() {
        let target = ResolvedModelTarget {
            model_id: "m".into(),
            route: ProviderRoute {
                provider_id: "p".into(),
                base_url: "https://example.test/v1".into(),
                api_key: "k".into(),
                auth: crate::llm::schema::AuthScheme::Bearer,
                headers: Default::default(),
            },
            protocol: ProtocolConfig::OpenAiChat(OpenAiChatConfig::default()),
            capabilities: Default::default(),
            generation: GenerationConfig::default(),
        };
        let request = ModelRequest {
            model: "m".into(),
            system_prompt: "system".into(),
            messages: vec![crate::types::AgentMessage::new_user("user", json!("hello"))],
            tools: Vec::new(),
        };
        let body = OpenAiChatAdapter.build_body(&target, &request).unwrap();
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
    }

    #[test]
    fn decodes_incremental_parallel_tool_calls() {
        let adapter = OpenAiChatAdapter;
        let mut state = adapter.new_stream_state();
        let mut events = adapter
            .decode_frame(
                &frame(json!({
                    "choices": [{"delta": {"tool_calls": [
                        {"index": 0, "id": "call_0", "function": {"name": "first", "arguments": "{\"a\":"}},
                        {"index": 1, "id": "call_1", "function": {"name": "second", "arguments": "{\"b\":"}}
                    ]}}]
                })),
                state.as_mut(),
            )
            .unwrap();
        events.extend(
            adapter
                .decode_frame(
                    &frame(json!({
                        "choices": [{"delta": {"tool_calls": [
                            {"index": 0, "function": {"arguments": "1}"}},
                            {"index": 1, "function": {"arguments": "2}"}}
                        ]}}]
                    })),
                    state.as_mut(),
                )
                .unwrap(),
        );
        events.extend(
            adapter
                .decode_frame(
                    &frame(json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]})),
                    state.as_mut(),
                )
                .unwrap(),
        );

        let starts = events
            .iter()
            .filter(|event| matches!(event, ModelStreamEvent::ToolInputStart { .. }))
            .count();
        assert_eq!(starts, 2);
        assert!(events.iter().any(|event| matches!(
            event,
            ModelStreamEvent::ToolInputDelta { snapshot: true, .. }
        )));
        let ends: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                ModelStreamEvent::ToolInputEnd {
                    index,
                    id,
                    name,
                    arguments,
                    ..
                } => Some((*index, id.as_str(), name.as_str(), arguments.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(ends[0], (0, "call_0", "first", json!({"a": 1})));
        assert_eq!(ends[1], (1, "call_1", "second", json!({"b": 2})));
        assert!(matches!(
            events.last(),
            Some(ModelStreamEvent::Finish {
                reason: FinishReason::ToolCalls,
                ..
            })
        ));
    }

    #[test]
    fn done_without_finish_reason_is_incomplete() {
        let adapter = OpenAiChatAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &SseFrame {
                    event: None,
                    data: "[DONE]".into(),
                },
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::Finish {
                reason: FinishReason::Incomplete,
                ..
            }]
        ));
    }

    #[test]
    fn decodes_reasoning_and_stream_error_frames() {
        let adapter = OpenAiChatAdapter;
        let mut state = adapter.new_stream_state();
        let mut events = adapter
            .decode_frame(
                &frame(json!({"choices": [{"delta": {"reasoning_content": "think"}}]})),
                state.as_mut(),
            )
            .unwrap();
        events.extend(adapter.finish_stream(state.as_mut()).unwrap());
        assert!(matches!(
            events.first(),
            Some(ModelStreamEvent::ReasoningStart { .. })
        ));
        assert!(
            matches!(events.get(1), Some(ModelStreamEvent::ReasoningDelta { text, .. }) if text == "think")
        );
        assert!(matches!(
            events.get(2),
            Some(ModelStreamEvent::ReasoningEnd { .. })
        ));

        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(json!({"error": {"message": "provider boom"}})),
                state.as_mut(),
            )
            .unwrap();
        assert!(
            matches!(events.as_slice(), [ModelStreamEvent::Error { message }] if message == "provider boom")
        );
    }

    #[test]
    fn apply_reasoning_uses_each_protocol_shape() {
        let mut target = ResolvedModelTarget {
            model_id: "m".into(),
            route: ProviderRoute {
                provider_id: "p".into(),
                base_url: "https://example.test".into(),
                api_key: "k".into(),
                auth: crate::llm::schema::AuthScheme::Bearer,
                headers: Default::default(),
            },
            protocol: ProtocolConfig::OpenAiChat(OpenAiChatConfig::default()),
            capabilities: Default::default(),
            generation: GenerationConfig {
                thinking_level: "high".into(),
                ..Default::default()
            },
        };
        target.capabilities.reasoning.supported = true;
        for (format, expected) in [
            (
                ChatReasoningFormat::Qwen {
                    chat_template: false,
                },
                json!({"enable_thinking": true, "reasoning_effort": "high"}),
            ),
            (
                ChatReasoningFormat::Qwen {
                    chat_template: true,
                },
                json!({"chat_template_kwargs": {"enable_thinking": true, "preserve_thinking": true}, "reasoning_effort": "high"}),
            ),
            (
                ChatReasoningFormat::DeepSeek,
                json!({"thinking": {"type": "enabled"}, "reasoning_effort": "high"}),
            ),
            (ChatReasoningFormat::Zai, json!({"enable_thinking": true})),
            (
                ChatReasoningFormat::ReasoningSplit,
                json!({"thinking": "enabled", "reasoning_split": true}),
            ),
        ] {
            let config = OpenAiChatConfig {
                reasoning: format,
                ..Default::default()
            };
            let mut body = json!({});
            apply_reasoning(&mut body, &target, &config);
            assert_eq!(body, expected);
        }
    }

    fn test_target(protocol: ProtocolConfig, generation: GenerationConfig) -> ResolvedModelTarget {
        ResolvedModelTarget {
            model_id: "m".into(),
            route: ProviderRoute {
                provider_id: "p".into(),
                base_url: "https://example.test/v1".into(),
                api_key: "k".into(),
                auth: crate::llm::schema::AuthScheme::Bearer,
                headers: Default::default(),
            },
            protocol,
            capabilities: Default::default(),
            generation,
        }
    }

    #[test]
    fn build_body_rejects_non_chat_target() {
        let target = test_target(
            ProtocolConfig::AnthropicMessages(
                crate::llm::schema::AnthropicMessagesConfig::default(),
            ),
            GenerationConfig::default(),
        );
        let request = ModelRequest {
            model: "m".into(),
            system_prompt: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
        };
        let error = OpenAiChatAdapter.build_body(&target, &request).unwrap_err();
        assert!(error.to_string().contains("non-chat target"));
    }

    #[test]
    fn build_body_serializes_tools_temperature_and_max_tokens() {
        let config = OpenAiChatConfig {
            tool_stream: true,
            max_tokens_field: crate::llm::schema::ChatMaxTokensField::MaxCompletionTokens,
            ..Default::default()
        };
        let target = test_target(
            ProtocolConfig::OpenAiChat(config),
            GenerationConfig {
                temperature: Some(0.5),
                max_output_tokens: Some(123),
                ..Default::default()
            },
        );
        let request = ModelRequest {
            model: "m".into(),
            system_prompt: String::new(),
            messages: Vec::new(),
            tools: vec![crate::types::ToolDef {
                tool_type: "function".into(),
                function: crate::types::FunctionDef {
                    name: "f".into(),
                    description: "d".into(),
                    parameters: json!({"type": "object"}),
                },
            }],
        };
        let body = OpenAiChatAdapter.build_body(&target, &request).unwrap();
        assert!(body["tools"].is_array());
        assert_eq!(body["tool_stream"], true);
        assert_eq!(body["temperature"], 0.5);
        assert_eq!(body["max_completion_tokens"], 123);
    }

    #[test]
    fn decode_frame_ignores_empty_data() {
        let adapter = OpenAiChatAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &SseFrame {
                    event: None,
                    data: "   ".into(),
                },
                state.as_mut(),
            )
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn decode_frame_tool_call_without_arguments_starts_only() {
        let adapter = OpenAiChatAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(json!({
                    "choices": [{"delta": {"tool_calls": [
                        {"index": 0, "id": "call_0", "function": {"name": "f"}}
                    ]}}]
                })),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::ToolInputStart { .. }]
        ));
    }

    #[test]
    fn decode_frame_emits_usage_and_maps_every_token_bucket() {
        let adapter = OpenAiChatAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(json!({
                    "choices": [{"delta": {"content": "hi"}}],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 20,
                        "total_tokens": 30,
                        "prompt_tokens_details": {"cached_tokens": 5, "cache_write_tokens": 6},
                        "completion_tokens_details": {"reasoning_tokens": 7},
                        "credit_cost": 0.5
                    }
                })),
                state.as_mut(),
            )
            .unwrap();
        let usage = events
            .iter()
            .find_map(|event| match event {
                ModelStreamEvent::Usage(usage) => Some(usage),
                _ => None,
            })
            .expect("usage event expected");
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 20);
        assert_eq!(usage.total_tokens, 30);
        assert_eq!(usage.cache_read_tokens, Some(5));
        assert_eq!(usage.cache_write_tokens, Some(6));
        assert_eq!(usage.reasoning_tokens, Some(7));
        assert_eq!(usage.credit_cost, Some(0.5));
    }

    #[test]
    fn finish_stream_after_finish_returns_empty() {
        let adapter = OpenAiChatAdapter;
        let mut state = adapter.new_stream_state();
        adapter
            .decode_frame(
                &frame(json!({"choices": [{"delta": {}, "finish_reason": "stop"}]})),
                state.as_mut(),
            )
            .unwrap();
        assert!(adapter.finish_stream(state.as_mut()).unwrap().is_empty());
    }

    #[test]
    fn lower_messages_handles_tool_results_and_multimodal_blocks() {
        let request = ModelRequest {
            model: "m".into(),
            system_prompt: String::new(),
            messages: vec![
                crate::types::AgentMessage {
                    role: "tool".into(),
                    content: vec![ContentBlock::tool_result("call_1", "result text", false)],
                    ..Default::default()
                },
                crate::types::AgentMessage {
                    role: "assistant".into(),
                    content: vec![
                        ContentBlock::text("hi"),
                        ContentBlock::image("http://img.example/1.png"),
                        ContentBlock::reasoning("thinking", ProviderMetadata::new()),
                        ContentBlock::tool_call(
                            "call_2",
                            "fn",
                            json!({"a": 1}),
                            ProviderMetadata::new(),
                        ),
                    ],
                    ..Default::default()
                },
            ],
            tools: Vec::new(),
        };
        let messages = lower_messages(&request, true);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "tool");
        assert_eq!(messages[0]["tool_call_id"], "call_1");
        assert_eq!(messages[0]["content"], "result text");

        let assistant = &messages[1];
        assert_eq!(assistant["role"], "assistant");
        assert_eq!(assistant["reasoning_content"], "thinking");
        assert_eq!(assistant["tool_calls"][0]["function"]["name"], "fn");
        let content = assistant["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0], json!({"type": "text", "text": "hi"}));
        assert_eq!(
            content[1],
            json!({"type": "image_url", "image_url": {"url": "http://img.example/1.png"}})
        );
    }

    #[test]
    fn apply_reasoning_none_and_effort_arms() {
        let mut target = test_target(
            ProtocolConfig::OpenAiChat(OpenAiChatConfig::default()),
            GenerationConfig {
                thinking_level: "high".into(),
                ..Default::default()
            },
        );
        target.capabilities.reasoning.supported = true;

        let config = OpenAiChatConfig {
            reasoning: ChatReasoningFormat::None,
            ..Default::default()
        };
        let mut body = json!({});
        apply_reasoning(&mut body, &target, &config);
        assert_eq!(body, json!({"reasoning_effort": "high"}));

        let config = OpenAiChatConfig {
            reasoning: ChatReasoningFormat::ReasoningEffort,
            supports_reasoning_effort: true,
            ..Default::default()
        };
        let mut body = json!({});
        apply_reasoning(&mut body, &target, &config);
        assert_eq!(body, json!({"reasoning_effort": "high"}));
    }

    #[test]
    fn arguments_string_serializes_non_string_values() {
        assert_eq!(arguments_string(&json!({"a": 1})), "{\"a\":1}");
        assert_eq!(arguments_string(&json!("literal")), "literal");
    }

    #[test]
    fn map_finish_reason_covers_length_filter_and_unknown() {
        assert_eq!(map_finish_reason("length"), FinishReason::Length);
        assert_eq!(map_finish_reason("max_tokens"), FinishReason::Length);
        assert_eq!(
            map_finish_reason("content_filter"),
            FinishReason::ContentFilter
        );
        assert_eq!(
            map_finish_reason("weird"),
            FinishReason::Unknown("weird".into())
        );
    }

    #[test]
    fn finish_is_idempotent() {
        let mut state = ChatStreamState::default();
        let events = finish(&mut state, FinishReason::Stop, None).unwrap();
        assert!(!events.is_empty());
        assert!(finish(&mut state, FinishReason::Stop, None)
            .unwrap()
            .is_empty());
    }
}
