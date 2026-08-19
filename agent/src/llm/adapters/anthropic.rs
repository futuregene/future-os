use super::{data_url, namespaced_metadata, parse_json_arguments, ProtocolAdapter};
use crate::llm::schema::{
    ApiProtocol, FinishReason, ModelRequest, ModelStreamEvent, ProtocolConfig, ResolvedModelTarget,
};
use crate::llm::sse::SseFrame;
use crate::types::{ContentBlock, ProviderMetadata, Usage};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use std::any::Any;
use std::collections::BTreeMap;

pub struct AnthropicMessagesAdapter;

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
        let ProtocolConfig::AnthropicMessages(_) = &target.protocol else {
            bail!("Anthropic Messages adapter received a non-anthropic target")
        };
        let messages = lower_messages(request)?;
        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "stream": true,
            "max_tokens": target.generation.max_output_tokens.unwrap_or({
                if target.capabilities.max_output_tokens > 0 {
                    target.capabilities.max_output_tokens
                } else {
                    4096
                }
            }),
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
        if target.generation.thinking_budget > 0 {
            body["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": target.generation.thinking_budget,
            });
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
                    events.push(ModelStreamEvent::Usage(state.usage.clone()));
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
    if let Some(previous) = messages.last_mut() {
        if previous.get("role").and_then(Value::as_str) == Some(role) {
            if let Some(previous_content) =
                previous.get_mut("content").and_then(Value::as_array_mut)
            {
                previous_content.extend(content);
                return;
            }
        }
    }
    messages.push(json!({"role": role, "content": content}));
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
        "end_turn" | "stop_sequence" | "pause_turn" => FinishReason::Stop,
        "tool_use" => FinishReason::ToolCalls,
        "max_tokens" => FinishReason::Length,
        "refusal" => FinishReason::Refusal,
        other => FinishReason::Unknown(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
