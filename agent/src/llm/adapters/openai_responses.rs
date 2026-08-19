use super::{namespaced_metadata, parse_json_arguments, ProtocolAdapter};
use crate::llm::schema::{
    ApiProtocol, FinishReason, ModelRequest, ModelStreamEvent, ProtocolConfig, ResolvedModelTarget,
};
use crate::llm::sse::SseFrame;
use crate::types::{ContentBlock, ProviderMetadata, Usage};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use std::any::Any;
use std::collections::{btree_map::Entry, BTreeMap};

pub struct OpenAiResponsesAdapter;

#[derive(Debug, Default)]
struct ResponseToolState {
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Default)]
struct ResponsesState {
    text_open: BTreeMap<usize, String>,
    reasoning_open: BTreeMap<usize, String>,
    tools: BTreeMap<usize, ResponseToolState>,
    finished: bool,
}

impl ProtocolAdapter for OpenAiResponsesAdapter {
    fn protocol(&self) -> ApiProtocol {
        ApiProtocol::OpenAiResponses
    }

    fn endpoint_path(&self) -> &'static str {
        "/responses"
    }

    fn build_body(&self, target: &ResolvedModelTarget, request: &ModelRequest) -> Result<Value> {
        let ProtocolConfig::OpenAiResponses(config) = &target.protocol else {
            bail!("OpenAI Responses adapter received a non-responses target")
        };
        let mut input = Vec::new();
        if !request.system_prompt.is_empty() {
            input.push(json!({
                "type": "message",
                "role": "developer",
                "content": [{"type": "input_text", "text": request.system_prompt}],
            }));
        }
        for message in &request.messages {
            lower_message(message, &mut input)?;
        }

        let mut body = json!({
            "model": request.model,
            "input": input,
            "stream": true,
            "store": config.store,
        });
        if config.include_encrypted_reasoning && target.capabilities.reasoning.supported {
            body["include"] = json!(["reasoning.encrypted_content"]);
        }
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": "function",
                            "name": tool.function.name,
                            "description": tool.function.description,
                            "parameters": tool.function.parameters,
                            "strict": false,
                        })
                    })
                    .collect(),
            );
        }
        if let Some(temperature) = target.generation.temperature {
            body["temperature"] = json!(temperature);
        }
        if let Some(max_tokens) = target.generation.max_output_tokens {
            body["max_output_tokens"] = json!(max_tokens);
        }
        let level = target.generation.thinking_level.as_str();
        if target.capabilities.reasoning.supported && !level.is_empty() && level != "off" {
            let mapped = target
                .capabilities
                .reasoning
                .levels
                .get(level)
                .and_then(Value::as_str)
                .unwrap_or(level);
            body["reasoning"] = json!({"effort": mapped, "summary": "auto"});
        }
        Ok(body)
    }

    fn new_stream_state(&self) -> Box<dyn Any + Send> {
        Box::<ResponsesState>::default()
    }

    fn decode_frame(
        &self,
        frame: &SseFrame,
        state: &mut (dyn Any + Send),
    ) -> Result<Vec<ModelStreamEvent>> {
        let state = state
            .downcast_mut::<ResponsesState>()
            .ok_or_else(|| anyhow!("invalid OpenAI Responses stream state"))?;
        if frame.data.trim().is_empty() || frame.data.trim() == "[DONE]" {
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
            "response.output_item.added" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let item = event.get("item").unwrap_or(&Value::Null);
                match item.get("type").and_then(Value::as_str).unwrap_or("") {
                    "function_call" => {
                        let item_id = item.get("id").and_then(Value::as_str).unwrap_or("");
                        let call_id = item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .unwrap_or(item_id);
                        let name = item.get("name").and_then(Value::as_str).unwrap_or("");
                        state.tools.insert(
                            index,
                            ResponseToolState {
                                item_id: item_id.to_string(),
                                call_id: call_id.to_string(),
                                name: name.to_string(),
                                arguments: String::new(),
                            },
                        );
                        events.push(ModelStreamEvent::ToolInputStart {
                            index,
                            id: call_id.to_string(),
                            name: name.to_string(),
                            arguments: None,
                            provider_metadata: openai_item_metadata(item_id, None),
                        });
                    }
                    "reasoning" => {
                        let id = item
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("reasoning");
                        state.reasoning_open.insert(index, id.to_string());
                        events.push(ModelStreamEvent::ReasoningStart { id: id.to_string() });
                    }
                    _ => {}
                }
            }
            "response.output_text.delta" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let id = event
                    .get("item_id")
                    .and_then(Value::as_str)
                    .unwrap_or("text");
                if let Entry::Vacant(entry) = state.text_open.entry(index) {
                    entry.insert(id.to_string());
                    events.push(ModelStreamEvent::TextStart { id: id.to_string() });
                }
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    events.push(ModelStreamEvent::TextDelta {
                        id: id.to_string(),
                        text: delta.to_string(),
                    });
                }
            }
            "response.output_text.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(id) = state.text_open.remove(&index) {
                    events.push(ModelStreamEvent::TextEnd { id });
                }
            }
            "response.refusal.delta" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let id = event
                    .get("item_id")
                    .and_then(Value::as_str)
                    .unwrap_or("refusal");
                if let Entry::Vacant(entry) = state.text_open.entry(index) {
                    entry.insert(id.to_string());
                    events.push(ModelStreamEvent::TextStart { id: id.to_string() });
                }
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    events.push(ModelStreamEvent::TextDelta {
                        id: id.to_string(),
                        text: delta.to_string(),
                    });
                }
            }
            "response.refusal.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(id) = state.text_open.remove(&index) {
                    events.push(ModelStreamEvent::TextEnd { id });
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let id = event
                    .get("item_id")
                    .and_then(Value::as_str)
                    .unwrap_or("reasoning");
                if let Entry::Vacant(entry) = state.reasoning_open.entry(index) {
                    entry.insert(id.to_string());
                    events.push(ModelStreamEvent::ReasoningStart { id: id.to_string() });
                }
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    events.push(ModelStreamEvent::ReasoningDelta {
                        id: id.to_string(),
                        text: delta.to_string(),
                    });
                }
            }
            "response.function_call_arguments.delta" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(tool) = state.tools.get_mut(&index) {
                    let delta = event.get("delta").and_then(Value::as_str).unwrap_or("");
                    tool.arguments.push_str(delta);
                    events.push(ModelStreamEvent::ToolInputDelta {
                        index,
                        id: tool.call_id.clone(),
                        delta: delta.to_string(),
                        snapshot: false,
                    });
                }
            }
            "response.output_item.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let item = event.get("item").unwrap_or(&Value::Null);
                match item.get("type").and_then(Value::as_str).unwrap_or("") {
                    "function_call" => {
                        let tool =
                            state
                                .tools
                                .remove(&index)
                                .unwrap_or_else(|| ResponseToolState {
                                    item_id: item
                                        .get("id")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .into(),
                                    call_id: item
                                        .get("call_id")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .into(),
                                    name: item
                                        .get("name")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .into(),
                                    arguments: String::new(),
                                });
                        let arguments = item
                            .get("arguments")
                            .cloned()
                            .unwrap_or_else(|| Value::String(tool.arguments.clone()));
                        events.push(ModelStreamEvent::ToolInputEnd {
                            index,
                            id: tool.call_id,
                            name: tool.name,
                            arguments: parse_json_arguments(&arguments),
                            provider_metadata: openai_item_metadata(&tool.item_id, None),
                        });
                    }
                    "reasoning" => {
                        let id = item
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("reasoning");
                        state.reasoning_open.remove(&index);
                        let encrypted = item.get("encrypted_content").and_then(Value::as_str);
                        events.push(ModelStreamEvent::ReasoningEnd {
                            id: id.to_string(),
                            provider_metadata: openai_item_metadata(id, encrypted),
                        });
                    }
                    _ => {}
                }
            }
            "response.completed" => {
                let response = event.get("response").unwrap_or(&Value::Null);
                let usage = response
                    .get("usage")
                    .filter(|value| !value.is_null())
                    .map(responses_usage);
                events.extend(close_open(state));
                let status = response
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("completed");
                if status == "failed" {
                    events.push(ModelStreamEvent::Error {
                        message: response_error_message(response),
                    });
                } else {
                    events.push(ModelStreamEvent::Finish {
                        reason: match status {
                            "completed" => completed_output_reason(response),
                            "incomplete" => incomplete_reason(response),
                            "cancelled" => FinishReason::Cancelled,
                            _ => FinishReason::Incomplete,
                        },
                        usage,
                    });
                }
                state.finished = true;
            }
            "response.incomplete" => {
                let response = event.get("response").unwrap_or(&Value::Null);
                let reason = incomplete_reason(response);
                events.extend(close_open(state));
                events.push(ModelStreamEvent::Finish {
                    reason,
                    usage: response
                        .get("usage")
                        .filter(|value| !value.is_null())
                        .map(responses_usage),
                });
                state.finished = true;
            }
            "response.failed" | "error" => {
                let error = event
                    .get("response")
                    .and_then(|response| response.get("error"))
                    .or_else(|| event.get("error"));
                let message = error
                    .and_then(|error| error.get("message"))
                    .or_else(|| event.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("OpenAI Responses stream failed");
                state.finished = true;
                events.push(ModelStreamEvent::Error {
                    message: message.to_string(),
                });
            }
            _ => {}
        }
        Ok(events)
    }

    fn finish_stream(&self, state: &mut (dyn Any + Send)) -> Result<Vec<ModelStreamEvent>> {
        let state = state
            .downcast_mut::<ResponsesState>()
            .ok_or_else(|| anyhow!("invalid OpenAI Responses stream state"))?;
        if state.finished {
            return Ok(Vec::new());
        }
        let mut events = close_open(state);
        events.push(ModelStreamEvent::Finish {
            reason: FinishReason::Incomplete,
            usage: None,
        });
        state.finished = true;
        Ok(events)
    }
}

fn lower_message(message: &crate::types::AgentMessage, input: &mut Vec<Value>) -> Result<()> {
    let mut pending_content = Vec::new();
    for block in message.model_content() {
        match block {
            ContentBlock::Text { text } => pending_content.push(json!({
                "type": if message.role == "assistant" { "output_text" } else { "input_text" },
                "text": text,
            })),
            ContentBlock::Image { image_url } => {
                if let Some(url) = image_url.url {
                    pending_content
                        .push(json!({"type": "input_image", "image_url": url, "detail": "auto"}));
                }
            }
            ContentBlock::Reasoning {
                text,
                provider_metadata,
            } => {
                let openai = provider_metadata.get("openai").and_then(Value::as_object);
                let id = openai
                    .and_then(|value| value.get("id"))
                    .and_then(Value::as_str);
                let encrypted = openai
                    .and_then(|value| value.get("encrypted_content"))
                    .and_then(Value::as_str);
                if id.is_some() || encrypted.is_some() {
                    flush_message_content(&message.role, &mut pending_content, input);
                    let mut item = Map::new();
                    item.insert("type".into(), json!("reasoning"));
                    if let Some(id) = id {
                        item.insert("id".into(), json!(id));
                    }
                    if let Some(encrypted) = encrypted {
                        item.insert("encrypted_content".into(), json!(encrypted));
                    }
                    if !text.is_empty() {
                        item.insert(
                            "summary".into(),
                            json!([{"type": "summary_text", "text": text}]),
                        );
                    }
                    input.push(Value::Object(item));
                }
            }
            ContentBlock::ToolCall {
                id,
                name,
                args,
                provider_metadata,
            } => {
                flush_message_content(&message.role, &mut pending_content, input);
                let item_id = provider_metadata
                    .get("openai")
                    .and_then(Value::as_object)
                    .and_then(|value| value.get("item_id"))
                    .and_then(Value::as_str);
                let mut item = Map::new();
                item.insert("type".into(), json!("function_call"));
                item.insert("call_id".into(), json!(id));
                item.insert("name".into(), json!(name));
                item.insert("arguments".into(), json!(arguments_string(&args)));
                if let Some(item_id) = item_id {
                    item.insert("id".into(), json!(item_id));
                }
                input.push(Value::Object(item));
            }
            ContentBlock::ToolResult {
                tool_call_id,
                content,
                ..
            } => {
                flush_message_content(&message.role, &mut pending_content, input);
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_call_id,
                    "output": content,
                }));
            }
        }
    }
    flush_message_content(&message.role, &mut pending_content, input);
    Ok(())
}

fn flush_message_content(role: &str, content: &mut Vec<Value>, input: &mut Vec<Value>) {
    if content.is_empty() {
        return;
    }
    input.push(json!({
        "type": "message",
        "role": if role == "assistant" { "assistant" } else { "user" },
        "content": std::mem::take(content),
    }));
}

fn arguments_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn openai_item_metadata(id: &str, encrypted_content: Option<&str>) -> ProviderMetadata {
    let mut value = Map::new();
    if !id.is_empty() {
        value.insert("id".into(), json!(id));
        value.insert("item_id".into(), json!(id));
    }
    if let Some(encrypted_content) = encrypted_content {
        value.insert("encrypted_content".into(), json!(encrypted_content));
    }
    namespaced_metadata("openai", Value::Object(value))
}

fn responses_usage(value: &Value) -> Usage {
    Usage {
        prompt_tokens: value
            .get("input_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        completion_tokens: value
            .get("output_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        total_tokens: value
            .get("total_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        cache_read_tokens: value
            .get("input_tokens_details")
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_i64),
        cache_write_tokens: value
            .get("input_tokens_details")
            .and_then(|details| details.get("cache_write_tokens"))
            .and_then(Value::as_i64),
        reasoning_tokens: value
            .get("output_tokens_details")
            .and_then(|details| details.get("reasoning_tokens"))
            .and_then(Value::as_i64),
        credit_cost: None,
        provider_metadata: None,
    }
}

fn completed_output_reason(response: &Value) -> FinishReason {
    let output = response
        .get("output")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if output
        .iter()
        .any(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
    {
        return FinishReason::ToolCalls;
    }
    if output.iter().any(|item| {
        item.get("type").and_then(Value::as_str) == Some("refusal")
            || item
                .get("content")
                .and_then(Value::as_array)
                .is_some_and(|content| {
                    content
                        .iter()
                        .any(|part| part.get("type").and_then(Value::as_str) == Some("refusal"))
                })
    }) {
        FinishReason::Refusal
    } else {
        FinishReason::Stop
    }
}

fn incomplete_reason(response: &Value) -> FinishReason {
    response
        .get("incomplete_details")
        .and_then(|details| details.get("reason"))
        .and_then(Value::as_str)
        .map(|reason| match reason {
            "max_output_tokens" => FinishReason::Length,
            "content_filter" => FinishReason::ContentFilter,
            other => FinishReason::Unknown(other.to_string()),
        })
        .unwrap_or(FinishReason::Incomplete)
}

fn response_error_message(response: &Value) -> String {
    response
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("OpenAI Responses stream failed")
        .to_string()
}

fn close_open(state: &mut ResponsesState) -> Vec<ModelStreamEvent> {
    let mut events = Vec::new();
    for (_, id) in std::mem::take(&mut state.text_open) {
        events.push(ModelStreamEvent::TextEnd { id });
    }
    for (_, id) in std::mem::take(&mut state.reasoning_open) {
        events.push(ModelStreamEvent::ReasoningEnd {
            id,
            provider_metadata: ProviderMetadata::new(),
        });
    }
    for (index, tool) in std::mem::take(&mut state.tools) {
        events.push(ModelStreamEvent::ToolInputEnd {
            index,
            id: tool.call_id,
            name: tool.name,
            arguments: parse_json_arguments(&Value::String(tool.arguments)),
            provider_metadata: openai_item_metadata(&tool.item_id, None),
        });
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(event_type: &str, body: Value) -> SseFrame {
        SseFrame {
            event: Some(event_type.into()),
            data: body.to_string(),
        }
    }

    #[test]
    fn stateless_reasoning_is_replayed() {
        let mut metadata = ProviderMetadata::new();
        metadata.insert(
            "openai".into(),
            json!({"id": "rs_1", "encrypted_content": "cipher"}),
        );
        let message = crate::types::AgentMessage {
            role: "assistant".into(),
            content: vec![ContentBlock::reasoning("summary", metadata)],
            ..Default::default()
        };
        let mut input = Vec::new();
        lower_message(&message, &mut input).unwrap();
        assert_eq!(input[0]["type"], "reasoning");
        assert_eq!(input[0]["encrypted_content"], "cipher");
    }

    #[test]
    fn preserves_interleaved_output_item_order() {
        let message = crate::types::AgentMessage {
            role: "assistant".into(),
            content: vec![
                ContentBlock::text("before"),
                ContentBlock::tool_call(
                    "call_1",
                    "lookup",
                    json!({"q": "rust"}),
                    ProviderMetadata::new(),
                ),
                ContentBlock::text("after"),
            ],
            ..Default::default()
        };
        let mut input = Vec::new();
        lower_message(&message, &mut input).unwrap();
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["content"][0]["type"], "output_text");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[2]["type"], "message");
    }

    #[test]
    fn replays_reasoning_tool_call_and_result_for_a_second_turn() {
        let mut reasoning_metadata = ProviderMetadata::new();
        reasoning_metadata.insert(
            "openai".into(),
            json!({"id": "rs_1", "encrypted_content": "cipher"}),
        );
        let mut tool_metadata = ProviderMetadata::new();
        tool_metadata.insert("openai".into(), json!({"item_id": "fc_1"}));
        let messages = vec![
            crate::types::AgentMessage {
                role: "assistant".into(),
                content: vec![
                    ContentBlock::reasoning("summary", reasoning_metadata),
                    ContentBlock::tool_call(
                        "call_1",
                        "lookup",
                        json!({"q": "rust"}),
                        tool_metadata,
                    ),
                ],
                ..Default::default()
            },
            crate::types::AgentMessage {
                role: "tool".into(),
                content: vec![ContentBlock::tool_result("call_1", "result", false)],
                ..Default::default()
            },
        ];
        let mut input = Vec::new();
        for message in &messages {
            lower_message(message, &mut input).unwrap();
        }
        assert_eq!(input[0]["type"], "reasoning");
        assert_eq!(input[0]["encrypted_content"], "cipher");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["id"], "fc_1");
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_1");
    }

    #[test]
    fn decodes_streaming_function_call_with_metadata() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let mut events = Vec::new();
        for frame in [
            frame(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": 1,
                    "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup"}
                }),
            ),
            frame(
                "response.function_call_arguments.delta",
                json!({
                    "type": "response.function_call_arguments.delta",
                    "output_index": 1,
                    "item_id": "fc_1",
                    "delta": "{\"q\":"
                }),
            ),
            frame(
                "response.function_call_arguments.delta",
                json!({
                    "type": "response.function_call_arguments.delta",
                    "output_index": 1,
                    "item_id": "fc_1",
                    "delta": "\"rust\"}"
                }),
            ),
            frame(
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": 1,
                    "item": {
                        "type": "function_call",
                        "id": "fc_1",
                        "call_id": "call_1",
                        "name": "lookup",
                        "arguments": "{\"q\":\"rust\"}"
                    }
                }),
            ),
            frame(
                "response.completed",
                json!({
                    "type": "response.completed",
                    "response": {
                        "status": "completed",
                        "output": [{"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup"}],
                        "usage": {"input_tokens": 10, "output_tokens": 4, "total_tokens": 14}
                    }
                }),
            ),
        ] {
            events.extend(adapter.decode_frame(&frame, state.as_mut()).unwrap());
        }

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
                index: 1,
                id,
                name,
                arguments,
                provider_metadata,
            } if id == "call_1"
                && name == "lookup"
                && arguments == &json!({"q": "rust"})
                && provider_metadata["openai"]["item_id"] == "fc_1"
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
    fn abnormal_close_preserves_provider_text_id() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let start = adapter
            .decode_frame(
                &frame(
                    "response.output_text.delta",
                    json!({
                        "type": "response.output_text.delta",
                        "output_index": 3,
                        "item_id": "msg_3",
                        "delta": "partial"
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        let end = adapter.finish_stream(state.as_mut()).unwrap();
        assert!(matches!(
            start.first(),
            Some(ModelStreamEvent::TextStart { id }) if id == "msg_3"
        ));
        assert!(matches!(
            end.first(),
            Some(ModelStreamEvent::TextEnd { id }) if id == "msg_3"
        ));
        assert!(matches!(
            end.last(),
            Some(ModelStreamEvent::Finish {
                reason: FinishReason::Incomplete,
                ..
            })
        ));
    }

    #[test]
    fn completed_status_and_output_determine_finish_reason() {
        let cases = [
            (
                json!({"status": "completed", "output": [{"type": "message", "content": [{"type": "output_text"}]}]}),
                FinishReason::Stop,
            ),
            (
                json!({"status": "completed", "output": [{"type": "message", "content": [{"type": "refusal"}]}]}),
                FinishReason::Refusal,
            ),
            (
                json!({"status": "cancelled", "output": []}),
                FinishReason::Cancelled,
            ),
            (
                json!({"status": "in_progress", "output": []}),
                FinishReason::Incomplete,
            ),
        ];
        for (response, expected) in cases {
            let adapter = OpenAiResponsesAdapter;
            let mut state = adapter.new_stream_state();
            let events = adapter
                .decode_frame(
                    &frame(
                        "response.completed",
                        json!({"type": "response.completed", "response": response}),
                    ),
                    state.as_mut(),
                )
                .unwrap();
            assert!(matches!(
                events.last(),
                Some(ModelStreamEvent::Finish { reason, .. }) if reason == &expected
            ));
        }

        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let failed = adapter
            .decode_frame(
                &frame(
                    "response.completed",
                    json!({
                        "type": "response.completed",
                        "response": {"status": "failed", "error": {"message": "boom"}}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            failed.as_slice(),
            [ModelStreamEvent::Error { message }] if message == "boom"
        ));

        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let incomplete = adapter
            .decode_frame(
                &frame(
                    "response.incomplete",
                    json!({
                        "type": "response.incomplete",
                        "response": {
                            "status": "incomplete",
                            "incomplete_details": {"reason": "max_output_tokens"}
                        }
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            incomplete.as_slice(),
            [ModelStreamEvent::Finish {
                reason: FinishReason::Length,
                ..
            }]
        ));
    }
}
