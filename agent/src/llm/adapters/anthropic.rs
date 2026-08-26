use super::{data_url, namespaced_metadata, parse_json_arguments, ProtocolAdapter};
use crate::llm::schema::{
    AnthropicThinkingMode, ApiProtocol, FinishReason, ModelRequest, ModelStreamEvent,
    ProtocolConfig, ResolvedModelTarget,
};
use crate::llm::sse::SseFrame;
use crate::types::{ContentBlock, ProviderMetadata, Usage};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use std::any::Any;
use std::collections::BTreeMap;

pub struct AnthropicMessagesAdapter;

/// Anthropic requires an enabled thinking budget to be strictly smaller than
/// the response's `max_tokens` allocation. A lower allocation cannot support
/// extended thinking at all (Anthropic's minimum budget is 1024 tokens).
const MIN_THINKING_BUDGET_TOKENS: i32 = 1024;

#[derive(Debug)]
enum AnthropicBlock {
    Text {
        id: String,
    },
    Reasoning {
        id: String,
        signature: String,
        redacted_data: Option<String>,
    },
    Tool {
        id: String,
        name: String,
        arguments: String,
    },
    Other,
}

#[derive(Debug, Default)]
struct AnthropicState {
    blocks: BTreeMap<usize, AnthropicBlock>,
    usage: Usage,
    stop_reason: Option<FinishReason>,
    finished: bool,
}

impl ProtocolAdapter for AnthropicMessagesAdapter {
    fn protocol(&self) -> ApiProtocol {
        ApiProtocol::AnthropicMessages
    }

    fn endpoint_path(&self) -> &'static str {
        "/messages"
    }

    fn build_body(&self, target: &ResolvedModelTarget, request: &ModelRequest) -> Result<Value> {
        let ProtocolConfig::AnthropicMessages(config) = &target.protocol else {
            bail!("Anthropic Messages adapter received a non-anthropic target")
        };
        let messages = lower_messages(request)?;
        let max_tokens = target.generation.max_output_tokens.unwrap_or({
            if target.capabilities.max_output_tokens > 0 {
                target.capabilities.max_output_tokens
            } else {
                4096
            }
        });
        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "stream": true,
            "max_tokens": max_tokens,
        });
        if !request.system_prompt.is_empty() {
            body["system"] = json!([{"type": "text", "text": request.system_prompt}]);
        }
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "name": tool.function.name,
                            "description": tool.function.description,
                            "input_schema": tool.function.parameters,
                        })
                    })
                    .collect(),
            );
        }
        let thinking_enabled = target.generation.thinking_budget >= MIN_THINKING_BUDGET_TOKENS;
        if thinking_enabled && config.thinking_mode == AnthropicThinkingMode::Adaptive {
            body["thinking"] = json!({"type": "adaptive"});
            body["output_config"] = json!({
                "effort": anthropic_effort(&target.generation.thinking_level),
            });
        } else if thinking_enabled {
            let thinking_budget = target
                .generation
                .thinking_budget
                .min(max_tokens.saturating_sub(1));
            if thinking_budget >= MIN_THINKING_BUDGET_TOKENS {
                body["thinking"] = json!({
                    "type": "enabled",
                    "budget_tokens": thinking_budget,
                });
            }
        } else if let Some(temperature) = target.generation.temperature {
            body["temperature"] = json!(temperature);
        }
        Ok(body)
    }

    fn new_stream_state(&self) -> Box<dyn Any + Send> {
        Box::<AnthropicState>::default()
    }

    fn decode_frame(
        &self,
        frame: &SseFrame,
        state: &mut (dyn Any + Send),
    ) -> Result<Vec<ModelStreamEvent>> {
        let state = state
            .downcast_mut::<AnthropicState>()
            .ok_or_else(|| anyhow!("invalid Anthropic stream state"))?;
        if frame.data.trim().is_empty() {
            return Ok(Vec::new());
        }
        let event: Value = serde_json::from_str(&frame.data)?;
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .or(frame.event.as_deref())
            .unwrap_or("");
        let mut events = Vec::new();
        match event_type {
            "message_start" => {
                if let Some(usage) = event
                    .get("message")
                    .and_then(|message| message.get("usage"))
                {
                    update_usage(&mut state.usage, usage);
                }
            }
            "content_block_start" => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let block = event.get("content_block").unwrap_or(&Value::Null);
                match block.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text" => {
                        let id = format!("text-{index}");
                        state
                            .blocks
                            .insert(index, AnthropicBlock::Text { id: id.clone() });
                        events.push(ModelStreamEvent::TextStart { id });
                    }
                    "thinking" => {
                        let id = format!("thinking-{index}");
                        state.blocks.insert(
                            index,
                            AnthropicBlock::Reasoning {
                                id: id.clone(),
                                signature: block
                                    .get("signature")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string(),
                                redacted_data: None,
                            },
                        );
                        events.push(ModelStreamEvent::ReasoningStart { id });
                    }
                    "redacted_thinking" => {
                        let id = format!("thinking-{index}");
                        state.blocks.insert(
                            index,
                            AnthropicBlock::Reasoning {
                                id: id.clone(),
                                signature: String::new(),
                                redacted_data: block
                                    .get("data")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                            },
                        );
                        events.push(ModelStreamEvent::ReasoningStart { id });
                    }
                    "tool_use" => {
                        let id = block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        state.blocks.insert(
                            index,
                            AnthropicBlock::Tool {
                                id: id.clone(),
                                name: name.clone(),
                                arguments: String::new(),
                            },
                        );
                        events.push(ModelStreamEvent::ToolInputStart {
                            index,
                            id,
                            name,
                            arguments: None,
                            provider_metadata: ProviderMetadata::new(),
                        });
                    }
                    _ => {
                        state.blocks.insert(index, AnthropicBlock::Other);
                    }
                }
            }
            "content_block_delta" => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = event.get("delta").unwrap_or(&Value::Null);
                match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text_delta" => {
                        if let Some(AnthropicBlock::Text { id }) = state.blocks.get(&index) {
                            events.push(ModelStreamEvent::TextDelta {
                                id: id.clone(),
                                text: delta
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string(),
                            });
                        }
                    }
                    "thinking_delta" => {
                        if let Some(AnthropicBlock::Reasoning { id, .. }) = state.blocks.get(&index)
                        {
                            events.push(ModelStreamEvent::ReasoningDelta {
                                id: id.clone(),
                                text: delta
                                    .get("thinking")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string(),
                            });
                        }
                    }
                    "signature_delta" => {
                        if let Some(AnthropicBlock::Reasoning { signature, .. }) =
                            state.blocks.get_mut(&index)
                        {
                            signature.push_str(
                                delta.get("signature").and_then(Value::as_str).unwrap_or(""),
                            );
                        }
                    }
                    "input_json_delta" => {
                        if let Some(AnthropicBlock::Tool { id, arguments, .. }) =
                            state.blocks.get_mut(&index)
                        {
                            let partial = delta
                                .get("partial_json")
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            arguments.push_str(partial);
                            events.push(ModelStreamEvent::ToolInputDelta {
                                index,
                                id: id.clone(),
                                delta: partial.to_string(),
                                snapshot: false,
                            });
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                if let Some(block) = state.blocks.remove(&index) {
                    match block {
                        AnthropicBlock::Text { id } => {
                            events.push(ModelStreamEvent::TextEnd { id });
                        }
                        AnthropicBlock::Reasoning {
                            id,
                            signature,
                            redacted_data,
                        } => {
                            events.push(ModelStreamEvent::ReasoningEnd {
                                id,
                                provider_metadata: anthropic_reasoning_metadata(
                                    &signature,
                                    redacted_data.as_deref(),
                                ),
                            });
                        }
                        AnthropicBlock::Tool {
                            id,
                            name,
                            arguments,
                        } => events.push(ModelStreamEvent::ToolInputEnd {
                            index,
                            id,
                            name,
                            arguments: parse_json_arguments(&Value::String(arguments)),
                            provider_metadata: ProviderMetadata::new(),
                        }),
                        AnthropicBlock::Other => {}
                    }
                }
            }
            "message_delta" => {
                if let Some(usage) = event.get("usage") {
                    update_usage(&mut state.usage, usage);
                }
                state.stop_reason = event
                    .get("delta")
                    .and_then(|delta| delta.get("stop_reason"))
                    .and_then(Value::as_str)
                    .map(map_finish_reason);
            }
            "message_stop" => {
                state.finished = true;
                events.push(ModelStreamEvent::Finish {
                    reason: state.stop_reason.take().unwrap_or(FinishReason::Stop),
                    usage: Some(state.usage.clone()),
                });
            }
            "error" => {
                state.finished = true;
                let message = event
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("Anthropic stream error");
                events.push(ModelStreamEvent::Error {
                    message: message.to_string(),
                });
            }
            "ping" => {}
            _ => {}
        }
        Ok(events)
    }

    fn finish_stream(&self, state: &mut (dyn Any + Send)) -> Result<Vec<ModelStreamEvent>> {
        let state = state
            .downcast_mut::<AnthropicState>()
            .ok_or_else(|| anyhow!("invalid Anthropic stream state"))?;
        if state.finished {
            Ok(Vec::new())
        } else {
            state.finished = true;
            Ok(vec![ModelStreamEvent::Finish {
                reason: FinishReason::Incomplete,
                usage: Some(state.usage.clone()),
            }])
        }
    }
}

fn lower_messages(request: &ModelRequest) -> Result<Vec<Value>> {
    let mut messages: Vec<Value> = Vec::new();
    for message in &request.messages {
        let mut user = Vec::new();
        let mut assistant = Vec::new();
        for block in message.model_content() {
            match block {
                ContentBlock::Text { text } => {
                    let wire = json!({"type": "text", "text": text});
                    if message.role == "assistant" {
                        assistant.push(wire);
                    } else {
                        user.push(wire);
                    }
                }
                ContentBlock::Image { image_url } => {
                    if let Some(url) = image_url.url {
                        let (media_type, data) = data_url(&url).ok_or_else(|| {
                            anyhow!("Anthropic Messages only supports base64 data URL images")
                        })?;
                        user.push(json!({
                            "type": "image",
                            "source": {"type": "base64", "media_type": media_type, "data": data},
                        }));
                    }
                }
                ContentBlock::Reasoning {
                    text,
                    provider_metadata,
                } => {
                    let anthropic = provider_metadata
                        .get("anthropic")
                        .and_then(Value::as_object);
                    if let Some(data) = anthropic
                        .and_then(|value| value.get("redacted_data"))
                        .and_then(Value::as_str)
                    {
                        assistant.push(json!({"type": "redacted_thinking", "data": data}));
                    } else if let Some(signature) = anthropic
                        .and_then(|value| value.get("signature"))
                        .and_then(Value::as_str)
                    {
                        assistant.push(json!({
                            "type": "thinking",
                            "thinking": text,
                            "signature": signature,
                        }));
                    }
                }
                ContentBlock::ToolCall { id, name, args, .. } => {
                    assistant.push(json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": parse_json_arguments(&args),
                    }));
                }
                ContentBlock::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                } => user.push(json!({
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": content,
                    "is_error": is_error,
                })),
            }
        }
        if !assistant.is_empty() {
            push_message(&mut messages, "assistant", assistant);
        }
        if !user.is_empty() {
            push_message(&mut messages, "user", user);
        }
    }
    Ok(messages)
}

fn push_message(messages: &mut Vec<Value>, role: &str, content: Vec<Value>) {
    let target = messages
        .last_mut()
        .filter(|previous| previous.get("role").and_then(Value::as_str) == Some(role))
        .and_then(|previous| previous.get_mut("content").and_then(Value::as_array_mut));
    match target {
        Some(previous_content) => previous_content.extend(content),
        None => messages.push(json!({"role": role, "content": content})),
    }
}

fn anthropic_reasoning_metadata(signature: &str, redacted_data: Option<&str>) -> ProviderMetadata {
    let mut value = Map::new();
    if !signature.is_empty() {
        value.insert("signature".into(), json!(signature));
    }
    if let Some(data) = redacted_data {
        value.insert("redacted_data".into(), json!(data));
    }
    namespaced_metadata("anthropic", Value::Object(value))
}

fn update_usage(usage: &mut Usage, value: &Value) {
    if let Some(input) = value.get("input_tokens").and_then(Value::as_i64) {
        usage.prompt_tokens = input;
    }
    if let Some(output) = value.get("output_tokens").and_then(Value::as_i64) {
        usage.completion_tokens = output;
    }
    if let Some(cache_read) = value.get("cache_read_input_tokens").and_then(Value::as_i64) {
        usage.cache_read_tokens = Some(cache_read);
    }
    if let Some(cache_write) = value
        .get("cache_creation_input_tokens")
        .and_then(Value::as_i64)
    {
        usage.cache_write_tokens = Some(cache_write);
    }
    usage.total_tokens = usage.prompt_tokens + usage.completion_tokens;
}

fn map_finish_reason(reason: &str) -> FinishReason {
    match reason {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "tool_use" => FinishReason::ToolCalls,
        "max_tokens" | "model_context_window_exceeded" => FinishReason::Length,
        // FutureOS does not currently send Anthropic server tools, so it
        // cannot replay their opaque pause payload unchanged. Never surface a
        // paused server-tool turn as a completed assistant response.
        "pause_turn" => FinishReason::Incomplete,
        "refusal" => FinishReason::Refusal,
        other => FinishReason::Unknown(other.to_string()),
    }
}

fn anthropic_effort(level: &str) -> &'static str {
    match level {
        "minimal" | "low" => "low",
        "medium" => "medium",
        "xhigh" => "max",
        _ => "high",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(max_tokens: Option<i32>, thinking_budget: i32) -> ResolvedModelTarget {
        ResolvedModelTarget {
            model_id: "claude".into(),
            route: crate::llm::schema::ProviderRoute {
                provider_id: "fixture".into(),
                base_url: "https://api.example.test".into(),
                api_key: "secret".into(),
                auth: crate::llm::schema::AuthScheme::AnthropicApiKey,
                headers: Default::default(),
            },
            protocol: ProtocolConfig::AnthropicMessages(Default::default()),
            capabilities: Default::default(),
            generation: crate::llm::schema::GenerationConfig {
                max_output_tokens: max_tokens,
                thinking_budget,
                ..Default::default()
            },
        }
    }

    fn adaptive_target(level: &str) -> ResolvedModelTarget {
        let mut target = target(None, 16_000);
        target.generation.thinking_level = level.to_string();
        target.protocol =
            ProtocolConfig::AnthropicMessages(crate::llm::schema::AnthropicMessagesConfig {
                thinking_mode: AnthropicThinkingMode::Adaptive,
                ..Default::default()
            });
        target
    }

    fn empty_request() -> ModelRequest {
        ModelRequest {
            model: "claude".into(),
            system_prompt: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
        }
    }

    #[test]
    fn thinking_budget_is_clamped_below_max_tokens() {
        let body = AnthropicMessagesAdapter
            .build_body(&target(None, 16_000), &empty_request())
            .unwrap();
        assert_eq!(body["max_tokens"], 4_096);
        assert_eq!(body["thinking"]["budget_tokens"], 4_095);
    }

    #[test]
    fn thinking_is_omitted_when_max_tokens_cannot_fit_minimum_budget() {
        let body = AnthropicMessagesAdapter
            .build_body(
                &target(Some(MIN_THINKING_BUDGET_TOKENS), 16_000),
                &empty_request(),
            )
            .unwrap();
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn adaptive_thinking_uses_effort_instead_of_manual_budget() {
        let body = AnthropicMessagesAdapter
            .build_body(&adaptive_target("xhigh"), &empty_request())
            .unwrap();
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "max");
        assert!(body["thinking"].get("budget_tokens").is_none());
    }

    #[test]
    fn anthropic_stop_reasons_preserve_truncation() {
        assert_eq!(map_finish_reason("pause_turn"), FinishReason::Incomplete);
        assert_eq!(
            map_finish_reason("model_context_window_exceeded"),
            FinishReason::Length
        );
    }

    fn frame(event: &str, data: Value) -> SseFrame {
        SseFrame {
            event: Some(event.into()),
            data: data.to_string(),
        }
    }

    #[test]
    fn thinking_signature_round_trips_into_anthropic_block() {
        let metadata = anthropic_reasoning_metadata("sig", None);
        let request = ModelRequest {
            model: "claude".into(),
            system_prompt: String::new(),
            messages: vec![crate::types::AgentMessage {
                role: "assistant".into(),
                content: vec![ContentBlock::reasoning("summary", metadata)],
                ..Default::default()
            }],
            tools: Vec::new(),
        };
        let messages = lower_messages(&request).unwrap();
        assert_eq!(messages[0]["content"][0]["type"], "thinking");
        assert_eq!(messages[0]["content"][0]["signature"], "sig");
    }

    #[test]
    fn replays_signed_thinking_tool_use_and_tool_result_for_a_second_turn() {
        let request = ModelRequest {
            model: "claude".into(),
            system_prompt: String::new(),
            messages: vec![
                crate::types::AgentMessage {
                    role: "assistant".into(),
                    content: vec![
                        ContentBlock::reasoning(
                            "thinking",
                            anthropic_reasoning_metadata("sig", None),
                        ),
                        ContentBlock::tool_call(
                            "tool_1",
                            "lookup",
                            json!({"q": "rust"}),
                            ProviderMetadata::new(),
                        ),
                    ],
                    ..Default::default()
                },
                crate::types::AgentMessage {
                    role: "tool".into(),
                    content: vec![ContentBlock::tool_result("tool_1", "result", false)],
                    ..Default::default()
                },
            ],
            tools: Vec::new(),
        };
        let messages = lower_messages(&request).unwrap();
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["content"][0]["signature"], "sig");
        assert_eq!(messages[0]["content"][1]["type"], "tool_use");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"][0]["type"], "tool_result");
    }

    #[test]
    fn cumulative_usage_is_emitted_once_at_message_stop() {
        let adapter = AnthropicMessagesAdapter;
        let mut state = adapter.new_stream_state();

        let start = SseFrame {
            event: Some("message_start".into()),
            data: json!({
                "type": "message_start",
                "message": {
                    "usage": {
                        "input_tokens": 1102,
                        "output_tokens": 0,
                        "cache_read_input_tokens": 2816
                    }
                }
            })
            .to_string(),
        };
        assert!(adapter
            .decode_frame(&start, state.as_mut())
            .unwrap()
            .is_empty());

        let delta = SseFrame {
            event: Some("message_delta".into()),
            data: json!({
                "type": "message_delta",
                "delta": {"stop_reason": "end_turn"},
                "usage": {"output_tokens": 205}
            })
            .to_string(),
        };
        assert!(adapter
            .decode_frame(&delta, state.as_mut())
            .unwrap()
            .is_empty());

        let stop = SseFrame {
            event: Some("message_stop".into()),
            data: json!({"type": "message_stop"}).to_string(),
        };
        let events = adapter.decode_frame(&stop, state.as_mut()).unwrap();
        assert_eq!(events.len(), 1);
        let (reason, usage) = expect_finish_usage(&events[0]);
        assert_eq!(reason, &FinishReason::Stop);
        assert_eq!(usage.prompt_tokens, 1102);
        assert_eq!(usage.completion_tokens, 205);
        assert_eq!(usage.total_tokens, 1307);
        assert_eq!(usage.cache_read_tokens, Some(2816));
    }

    #[test]
    fn decodes_streaming_tool_input_and_tool_finish() {
        let adapter = AnthropicMessagesAdapter;
        let mut state = adapter.new_stream_state();
        let mut events = Vec::new();
        for frame in [
            frame(
                "content_block_start",
                json!({
                    "type": "content_block_start",
                    "index": 2,
                    "content_block": {"type": "tool_use", "id": "tool_2", "name": "lookup"}
                }),
            ),
            frame(
                "content_block_delta",
                json!({
                    "type": "content_block_delta",
                    "index": 2,
                    "delta": {"type": "input_json_delta", "partial_json": "{\"q\":"}
                }),
            ),
            frame(
                "content_block_delta",
                json!({
                    "type": "content_block_delta",
                    "index": 2,
                    "delta": {"type": "input_json_delta", "partial_json": "\"rust\"}"}
                }),
            ),
            frame(
                "content_block_stop",
                json!({"type": "content_block_stop", "index": 2}),
            ),
            frame(
                "message_delta",
                json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": "tool_use"},
                    "usage": {"output_tokens": 12}
                }),
            ),
            frame("message_stop", json!({"type": "message_stop"})),
        ] {
            events.extend(adapter.decode_frame(&frame, state.as_mut()).unwrap());
        }

        assert!(matches!(
            events.first(),
            Some(ModelStreamEvent::ToolInputStart {
                index: 2,
                id,
                name,
                ..
            }) if id == "tool_2" && name == "lookup"
        ));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ModelStreamEvent::ToolInputDelta { .. }))
                .count(),
            2
        );
        assert!(events.iter().any(|event| matches!(
            event,
            ModelStreamEvent::ToolInputEnd {
                index: 2,
                id,
                name,
                arguments,
                ..
            } if id == "tool_2" && name == "lookup" && arguments == &json!({"q": "rust"})
        )));
        assert!(matches!(
            events.last(),
            Some(ModelStreamEvent::Finish {
                reason: FinishReason::ToolCalls,
                ..
            })
        ));
    }

    #[test]
    fn decodes_redacted_thinking_and_error_frames() {
        let adapter = AnthropicMessagesAdapter;
        let mut state = adapter.new_stream_state();
        let mut events = adapter
            .decode_frame(
                &frame(
                    "content_block_start",
                    json!({
                        "type": "content_block_start", "index": 0,
                        "content_block": {"type": "redacted_thinking", "data": "opaque"}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        events.extend(
            adapter
                .decode_frame(
                    &frame(
                        "content_block_stop",
                        json!({"type": "content_block_stop", "index": 0}),
                    ),
                    state.as_mut(),
                )
                .unwrap(),
        );
        assert!(matches!(
            events.first(),
            Some(ModelStreamEvent::ReasoningStart { .. })
        ));
        assert!(matches!(
            events.get(1),
            Some(ModelStreamEvent::ReasoningEnd { provider_metadata, .. })
                if provider_metadata["anthropic"]["redacted_data"] == "opaque"
        ));

        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "error",
                    json!({"type": "error", "error": {"message": "boom"}}),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(
            matches!(events.as_slice(), [ModelStreamEvent::Error { message }] if message == "boom")
        );
    }

    fn expect_finish_usage(event: &ModelStreamEvent) -> (&FinishReason, &Usage) {
        match event {
            ModelStreamEvent::Finish {
                reason,
                usage: Some(usage),
            } => (reason, usage),
            other => panic!("expected Finish with usage, got {other:?}"),
        }
    }

    #[test]
    #[should_panic(expected = "expected Finish with usage")]
    fn expect_finish_usage_rejects_non_finish_events() {
        expect_finish_usage(&ModelStreamEvent::Error {
            message: "x".into(),
        });
    }

    #[test]
    fn build_body_rejects_non_anthropic_target() {
        let target = ResolvedModelTarget {
            model_id: "m".into(),
            route: crate::llm::schema::ProviderRoute {
                provider_id: "p".into(),
                base_url: "https://example.test".into(),
                api_key: "k".into(),
                auth: crate::llm::schema::AuthScheme::Bearer,
                headers: Default::default(),
            },
            protocol: ProtocolConfig::OpenAiChat(crate::llm::schema::OpenAiChatConfig::default()),
            capabilities: Default::default(),
            generation: crate::llm::schema::GenerationConfig::default(),
        };
        let error = AnthropicMessagesAdapter
            .build_body(&target, &empty_request())
            .unwrap_err();
        assert!(error.to_string().contains("non-anthropic target"));
    }

    #[test]
    fn build_body_uses_capabilities_max_tokens_when_generation_absent() {
        let mut t = target(None, 0);
        t.capabilities.max_output_tokens = 2048;
        let body = AnthropicMessagesAdapter
            .build_body(&t, &empty_request())
            .unwrap();
        assert_eq!(body["max_tokens"], 2048);
    }

    #[test]
    fn build_body_serializes_tools_system_and_temperature() {
        let mut t = target(None, 0);
        t.generation.temperature = Some(0.5);
        let request = ModelRequest {
            model: "claude".into(),
            system_prompt: "sys".into(),
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
        let body = AnthropicMessagesAdapter.build_body(&t, &request).unwrap();
        assert!(body["tools"].is_array());
        assert_eq!(body["system"][0]["text"], "sys");
        assert_eq!(body["temperature"], 0.5);
    }

    #[test]
    fn decode_frame_ignores_empty_data() {
        let adapter = AnthropicMessagesAdapter;
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
    fn thinking_block_deltas_emit_reasoning_and_signature() {
        let adapter = AnthropicMessagesAdapter;
        let mut state = adapter.new_stream_state();
        let start = adapter
            .decode_frame(
                &frame(
                    "content_block_start",
                    json!({
                        "type": "content_block_start",
                        "index": 0,
                        "content_block": {"type": "thinking", "signature": "sig-"}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            start.as_slice(),
            [ModelStreamEvent::ReasoningStart { .. }]
        ));

        let delta = adapter
            .decode_frame(
                &frame(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": 0,
                        "delta": {"type": "thinking_delta", "thinking": "think"}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            delta.as_slice(),
            [ModelStreamEvent::ReasoningDelta { text, .. }] if text == "think"
        ));

        let signature = adapter
            .decode_frame(
                &frame(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": 0,
                        "delta": {"type": "signature_delta", "signature": "rest"}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(signature.is_empty());
    }

    #[test]
    fn unknown_content_block_types_are_tracked_and_ignored() {
        let adapter = AnthropicMessagesAdapter;
        let mut state = adapter.new_stream_state();
        let start = adapter
            .decode_frame(
                &frame(
                    "content_block_start",
                    json!({
                        "type": "content_block_start",
                        "index": 5,
                        "content_block": {"type": "server_tool_use", "id": "x"}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(start.is_empty());
        let delta = adapter
            .decode_frame(
                &frame(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": 5,
                        "delta": {"type": "server_tool_input_delta", "input": "x"}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(delta.is_empty());
    }

    #[test]
    fn content_block_stop_on_other_and_missing_blocks() {
        let adapter = AnthropicMessagesAdapter;
        let mut state = adapter.new_stream_state();
        adapter
            .decode_frame(
                &frame(
                    "content_block_start",
                    json!({
                        "type": "content_block_start",
                        "index": 5,
                        "content_block": {"type": "server_tool_use"}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        let other = adapter
            .decode_frame(
                &frame(
                    "content_block_stop",
                    json!({"type": "content_block_stop", "index": 5}),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(other.is_empty());

        let missing = adapter
            .decode_frame(
                &frame(
                    "content_block_stop",
                    json!({"type": "content_block_stop", "index": 99}),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(missing.is_empty());
    }

    #[test]
    fn ping_and_unknown_events_are_ignored() {
        let adapter = AnthropicMessagesAdapter;
        let mut state = adapter.new_stream_state();
        for data in [json!({"type": "ping"}), json!({"type": "totally_unknown"})] {
            let events = adapter
                .decode_frame(
                    &SseFrame {
                        event: None,
                        data: data.to_string(),
                    },
                    state.as_mut(),
                )
                .unwrap();
            assert!(events.is_empty());
        }
    }

    #[test]
    fn finish_stream_emits_incomplete_when_not_finished() {
        let adapter = AnthropicMessagesAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter.finish_stream(state.as_mut()).unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::Finish {
                reason: FinishReason::Incomplete,
                ..
            }]
        ));
    }

    #[test]
    fn finish_stream_after_finish_returns_empty() {
        let adapter = AnthropicMessagesAdapter;
        let mut state = adapter.new_stream_state();
        adapter
            .decode_frame(
                &frame("message_stop", json!({"type": "message_stop"})),
                state.as_mut(),
            )
            .unwrap();
        assert!(adapter.finish_stream(state.as_mut()).unwrap().is_empty());
    }

    #[test]
    fn lower_messages_routes_text_by_role() {
        let request = ModelRequest {
            model: "claude".into(),
            system_prompt: String::new(),
            messages: vec![
                crate::types::AgentMessage {
                    role: "user".into(),
                    content: vec![ContentBlock::text("u")],
                    ..Default::default()
                },
                crate::types::AgentMessage {
                    role: "assistant".into(),
                    content: vec![ContentBlock::text("a")],
                    ..Default::default()
                },
            ],
            tools: Vec::new(),
        };
        let messages = lower_messages(&request).unwrap();
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"][0]["text"], "u");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["text"], "a");
    }

    #[test]
    fn lower_messages_serializes_images_and_rejects_non_data_urls() {
        let request = ModelRequest {
            model: "claude".into(),
            system_prompt: String::new(),
            messages: vec![crate::types::AgentMessage {
                role: "user".into(),
                content: vec![ContentBlock::image("data:image/png;base64,AAAA")],
                ..Default::default()
            }],
            tools: Vec::new(),
        };
        let messages = lower_messages(&request).unwrap();
        assert_eq!(messages[0]["content"][0]["type"], "image");
        assert_eq!(
            messages[0]["content"][0]["source"]["media_type"],
            "image/png"
        );
        assert_eq!(messages[0]["content"][0]["source"]["data"], "AAAA");

        let request = ModelRequest {
            model: "claude".into(),
            system_prompt: String::new(),
            messages: vec![crate::types::AgentMessage {
                role: "user".into(),
                content: vec![ContentBlock::image("http://example/1.png")],
                ..Default::default()
            }],
            tools: Vec::new(),
        };
        let error = lower_messages(&request).unwrap_err();
        assert!(error.to_string().contains("base64 data URL"));

        // An image block with no URL is silently skipped.
        let request = ModelRequest {
            model: "claude".into(),
            system_prompt: String::new(),
            messages: vec![crate::types::AgentMessage {
                role: "user".into(),
                content: vec![ContentBlock::Image {
                    image_url: crate::types::ImageUrlData { url: None },
                }],
                ..Default::default()
            }],
            tools: Vec::new(),
        };
        let messages = lower_messages(&request).unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn lower_messages_replays_redacted_thinking() {
        let mut metadata = ProviderMetadata::new();
        metadata.insert("anthropic".into(), json!({"redacted_data": "opaque"}));
        let request = ModelRequest {
            model: "claude".into(),
            system_prompt: String::new(),
            messages: vec![crate::types::AgentMessage {
                role: "assistant".into(),
                content: vec![ContentBlock::reasoning("", metadata)],
                ..Default::default()
            }],
            tools: Vec::new(),
        };
        let messages = lower_messages(&request).unwrap();
        assert_eq!(messages[0]["content"][0]["type"], "redacted_thinking");
        assert_eq!(messages[0]["content"][0]["data"], "opaque");
    }

    #[test]
    fn lower_messages_merges_consecutive_same_role_messages() {
        let request = ModelRequest {
            model: "claude".into(),
            system_prompt: String::new(),
            messages: vec![
                crate::types::AgentMessage {
                    role: "user".into(),
                    content: vec![ContentBlock::text("one")],
                    ..Default::default()
                },
                crate::types::AgentMessage {
                    role: "user".into(),
                    content: vec![ContentBlock::text("two")],
                    ..Default::default()
                },
            ],
            tools: Vec::new(),
        };
        let messages = lower_messages(&request).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn update_usage_records_cache_creation_tokens() {
        let mut usage = Usage::default();
        update_usage(
            &mut usage,
            &json!({
                "input_tokens": 10,
                "output_tokens": 5,
                "cache_read_input_tokens": 3,
                "cache_creation_input_tokens": 4
            }),
        );
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.cache_read_tokens, Some(3));
        assert_eq!(usage.cache_write_tokens, Some(4));
        assert_eq!(usage.total_tokens, 15);
    }

    #[test]
    fn map_finish_reason_covers_all_variants() {
        assert_eq!(map_finish_reason("end_turn"), FinishReason::Stop);
        assert_eq!(map_finish_reason("stop_sequence"), FinishReason::Stop);
        assert_eq!(map_finish_reason("tool_use"), FinishReason::ToolCalls);
        assert_eq!(map_finish_reason("max_tokens"), FinishReason::Length);
        assert_eq!(map_finish_reason("pause_turn"), FinishReason::Incomplete);
        assert_eq!(map_finish_reason("refusal"), FinishReason::Refusal);
        assert_eq!(
            map_finish_reason("weird"),
            FinishReason::Unknown("weird".into())
        );
    }

    #[test]
    fn anthropic_effort_maps_levels() {
        assert_eq!(anthropic_effort("minimal"), "low");
        assert_eq!(anthropic_effort("low"), "low");
        assert_eq!(anthropic_effort("medium"), "medium");
        assert_eq!(anthropic_effort("xhigh"), "max");
        assert_eq!(anthropic_effort("whatever"), "high");
    }
}
