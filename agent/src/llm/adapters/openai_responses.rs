use super::{namespaced_metadata, parse_json_arguments, ProtocolAdapter};
use crate::llm::schema::{
    ApiProtocol, FinishReason, ModelRequest, ModelStreamEvent, ProtocolConfig, ResolvedModelTarget,
};
use crate::llm::sse::SseFrame;
use crate::types::{ContentBlock, ProviderMetadata, Usage};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use std::any::Any;
use std::collections::{btree_map::Entry, BTreeMap, BTreeSet};

pub struct OpenAiResponsesAdapter;

#[derive(Debug, Default)]
struct ResponseToolState {
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Default)]
struct ResponseTextState {
    id: String,
    text: String,
    open: bool,
}

#[derive(Debug, Default)]
struct ResponseReasoningState {
    /// Stable stream-local identity used only to assemble one output slot.
    id: String,
    /// Provider identity from the wire. Synthetic assembly ids must never be replayed.
    provider_id: Option<String>,
    summary_parts: BTreeMap<usize, String>,
    content_parts: BTreeMap<usize, String>,
    emitted_summary_parts: BTreeSet<usize>,
}

#[derive(Debug, Default)]
struct ResponsesState {
    texts: BTreeMap<usize, ResponseTextState>,
    reasoning_open: BTreeMap<usize, ResponseReasoningState>,
    tools: BTreeMap<usize, ResponseToolState>,
    finished_output_items: BTreeSet<usize>,
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
        if let Some(prompt_cache_options) = &config.prompt_cache_options {
            body["prompt_cache_options"] = prompt_cache_options.clone();
        }
        let level = target.generation.thinking_level.as_str();
        let has_responses_reasoning_config =
            config.reasoning_context.is_some() || config.reasoning_mode.is_some();
        if target.capabilities.reasoning.supported
            && level != "off"
            && (!level.is_empty() || has_responses_reasoning_config)
        {
            let mut reasoning = Map::new();
            if !level.is_empty() {
                let mapped = target
                    .capabilities
                    .reasoning
                    .levels
                    .get(level)
                    .and_then(Value::as_str)
                    .unwrap_or(level);
                reasoning.insert("effort".into(), json!(mapped));
            }
            if config.supports_reasoning_summary {
                reasoning.insert("summary".into(), json!("auto"));
            }
            if let Some(context) = &config.reasoning_context {
                reasoning.insert("context".into(), json!(context));
            }
            if let Some(mode) = &config.reasoning_mode {
                reasoning.insert("mode".into(), json!(mode));
            }
            if !reasoning.is_empty() {
                body["reasoning"] = Value::Object(reasoning);
            }
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
                        let id = item.get("id").and_then(Value::as_str).unwrap_or("");
                        open_reasoning(state, index, id, &mut events);
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
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    append_text_delta(state, index, id, delta, &mut events);
                }
            }
            "response.output_text.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let id = event
                    .get("item_id")
                    .and_then(Value::as_str)
                    .unwrap_or("text");
                let text = event.get("text").and_then(Value::as_str);
                reconcile_text(state, index, id, text, true, &mut events);
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
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    append_text_delta(state, index, id, delta, &mut events);
                }
            }
            "response.refusal.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let id = event
                    .get("item_id")
                    .and_then(Value::as_str)
                    .unwrap_or("refusal");
                let text = event.get("refusal").and_then(Value::as_str);
                reconcile_text(state, index, id, text, true, &mut events);
            }
            "response.reasoning_summary_part.added" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let id = event.get("item_id").and_then(Value::as_str).unwrap_or("");
                open_reasoning(state, index, id, &mut events);
                let summary_index = event
                    .get("summary_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(reasoning) = state.reasoning_open.get_mut(&index) {
                    reasoning.summary_parts.entry(summary_index).or_default();
                }
            }
            "response.reasoning_summary_text.delta" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let id = event.get("item_id").and_then(Value::as_str).unwrap_or("");
                let summary_index = event
                    .get("summary_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    append_reasoning_summary_delta(
                        state,
                        index,
                        id,
                        summary_index,
                        delta,
                        &mut events,
                    );
                }
            }
            "response.reasoning_summary_part.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let id = event.get("item_id").and_then(Value::as_str).unwrap_or("");
                let summary_index = event
                    .get("summary_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(text) = event
                    .get("part")
                    .and_then(|part| part.get("text"))
                    .and_then(Value::as_str)
                {
                    reconcile_reasoning_summary(state, index, id, summary_index, text, &mut events);
                }
            }
            "response.reasoning_summary_text.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let id = event.get("item_id").and_then(Value::as_str).unwrap_or("");
                let summary_index = event
                    .get("summary_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(text) = event.get("text").and_then(Value::as_str) {
                    reconcile_reasoning_summary(state, index, id, summary_index, text, &mut events);
                }
            }
            "response.reasoning_text.delta" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let id = event.get("item_id").and_then(Value::as_str).unwrap_or("");
                let content_index = event
                    .get("content_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    append_reasoning_content(state, index, id, content_index, delta, &mut events);
                }
            }
            "response.reasoning_text.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let id = event.get("item_id").and_then(Value::as_str).unwrap_or("");
                let content_index = event
                    .get("content_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(text) = event.get("text").and_then(Value::as_str) {
                    reconcile_reasoning_content(state, index, id, content_index, text, &mut events);
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
                reconcile_output_item(state, index, item, &mut events);
                state.finished_output_items.insert(index);
            }
            "response.completed" => {
                let response = event.get("response").unwrap_or(&Value::Null);
                reconcile_response_output(state, response, &mut events);
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
                reconcile_response_output(state, response, &mut events);
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
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty());
                let encrypted = openai
                    .and_then(|value| value.get("encrypted_content"))
                    .and_then(Value::as_str);
                let exact_summary = openai
                    .and_then(|value| value.get("summary"))
                    .filter(|value| value.is_array())
                    .cloned();
                let exact_content = openai
                    .and_then(|value| value.get("content"))
                    .filter(|value| value.is_array())
                    .cloned();
                // A Responses reasoning input item is provider-owned state.
                // Keep id-less reasoning for the UI/history, but do not send it
                // back as though the provider had issued a durable item id.
                if id.is_some() {
                    flush_message_content(&message.role, &mut pending_content, input);
                    let mut item = Map::new();
                    item.insert("type".into(), json!("reasoning"));
                    if let Some(id) = id {
                        item.insert("id".into(), json!(id));
                    }
                    if let Some(encrypted) = encrypted {
                        item.insert("encrypted_content".into(), json!(encrypted));
                    }
                    item.insert(
                        "summary".into(),
                        exact_summary.unwrap_or_else(|| {
                            if text.is_empty() {
                                Value::Array(Vec::new())
                            } else {
                                json!([{"type": "summary_text", "text": text}])
                            }
                        }),
                    );
                    if let Some(content) = exact_content {
                        item.insert("content".into(), content);
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

fn open_text(
    state: &mut ResponsesState,
    index: usize,
    id: &str,
    events: &mut Vec<ModelStreamEvent>,
) {
    let text = state.texts.entry(index).or_default();
    if text.id.is_empty() {
        text.id = id.to_string();
    }
    if !text.open {
        text.open = true;
        events.push(ModelStreamEvent::TextStart {
            id: text.id.clone(),
        });
    }
}

fn append_text_delta(
    state: &mut ResponsesState,
    index: usize,
    id: &str,
    delta: &str,
    events: &mut Vec<ModelStreamEvent>,
) {
    if delta.is_empty() {
        return;
    }
    open_text(state, index, id, events);
    let text = state.texts.get_mut(&index).expect("text state is open");
    text.text.push_str(delta);
    events.push(ModelStreamEvent::TextDelta {
        id: text.id.clone(),
        text: delta.to_string(),
    });
}

fn reconcile_text(
    state: &mut ResponsesState,
    index: usize,
    id: &str,
    complete: Option<&str>,
    close: bool,
    events: &mut Vec<ModelStreamEvent>,
) {
    if let Some(complete) = complete {
        let suffix = state
            .texts
            .get(&index)
            .and_then(|text| complete.strip_prefix(&text.text))
            .unwrap_or_else(|| {
                if state
                    .texts
                    .get(&index)
                    .is_none_or(|text| text.text.is_empty())
                {
                    complete
                } else {
                    ""
                }
            });
        append_text_delta(state, index, id, suffix, events);
        if let Some(text) = state.texts.get_mut(&index) {
            text.text = complete.to_string();
        }
    }
    if close {
        if let Some(text) = state.texts.get_mut(&index) {
            if text.open {
                text.open = false;
                events.push(ModelStreamEvent::TextEnd {
                    id: text.id.clone(),
                });
            }
        }
    }
}

fn open_reasoning(
    state: &mut ResponsesState,
    index: usize,
    id: &str,
    events: &mut Vec<ModelStreamEvent>,
) {
    match state.reasoning_open.entry(index) {
        Entry::Vacant(entry) => {
            let assembly_id = if id.is_empty() {
                format!("reasoning-{index}")
            } else {
                id.to_string()
            };
            entry.insert(ResponseReasoningState {
                id: assembly_id.clone(),
                provider_id: (!id.is_empty()).then(|| id.to_string()),
                ..Default::default()
            });
            events.push(ModelStreamEvent::ReasoningStart { id: assembly_id });
        }
        Entry::Occupied(mut entry) => {
            if entry.get().provider_id.is_none() && !id.is_empty() {
                entry.get_mut().provider_id = Some(id.to_string());
            }
        }
    }
}

fn append_reasoning_summary_delta(
    state: &mut ResponsesState,
    index: usize,
    id: &str,
    summary_index: usize,
    delta: &str,
    events: &mut Vec<ModelStreamEvent>,
) {
    if delta.is_empty() {
        return;
    }
    open_reasoning(state, index, id, events);
    let reasoning = state
        .reasoning_open
        .get_mut(&index)
        .expect("reasoning state is open");
    let first_delta_for_part = reasoning.emitted_summary_parts.insert(summary_index);
    if first_delta_for_part
        && summary_index > 0
        && reasoning
            .summary_parts
            .iter()
            .any(|(part_index, text)| *part_index < summary_index && !text.is_empty())
    {
        events.push(ModelStreamEvent::ReasoningDelta {
            id: reasoning.id.clone(),
            text: "\n\n".to_string(),
        });
    }
    reasoning
        .summary_parts
        .entry(summary_index)
        .or_default()
        .push_str(delta);
    events.push(ModelStreamEvent::ReasoningDelta {
        id: reasoning.id.clone(),
        text: delta.to_string(),
    });
}

fn reconcile_reasoning_summary(
    state: &mut ResponsesState,
    index: usize,
    id: &str,
    summary_index: usize,
    complete: &str,
    events: &mut Vec<ModelStreamEvent>,
) {
    open_reasoning(state, index, id, events);
    let suffix = state
        .reasoning_open
        .get(&index)
        .and_then(|reasoning| reasoning.summary_parts.get(&summary_index))
        .and_then(|current| complete.strip_prefix(current))
        .unwrap_or_else(|| {
            if state
                .reasoning_open
                .get(&index)
                .and_then(|reasoning| reasoning.summary_parts.get(&summary_index))
                .is_none_or(String::is_empty)
            {
                complete
            } else {
                ""
            }
        });
    append_reasoning_summary_delta(state, index, id, summary_index, suffix, events);
    if let Some(reasoning) = state.reasoning_open.get_mut(&index) {
        reasoning
            .summary_parts
            .insert(summary_index, complete.to_string());
    }
}

fn append_reasoning_content(
    state: &mut ResponsesState,
    index: usize,
    id: &str,
    content_index: usize,
    delta: &str,
    events: &mut Vec<ModelStreamEvent>,
) {
    open_reasoning(state, index, id, events);
    state
        .reasoning_open
        .get_mut(&index)
        .expect("reasoning state is open")
        .content_parts
        .entry(content_index)
        .or_default()
        .push_str(delta);
}

fn reconcile_reasoning_content(
    state: &mut ResponsesState,
    index: usize,
    id: &str,
    content_index: usize,
    complete: &str,
    events: &mut Vec<ModelStreamEvent>,
) {
    open_reasoning(state, index, id, events);
    if let Some(reasoning) = state.reasoning_open.get_mut(&index) {
        reasoning
            .content_parts
            .insert(content_index, complete.to_string());
    }
}

fn reconcile_response_output(
    state: &mut ResponsesState,
    response: &Value,
    events: &mut Vec<ModelStreamEvent>,
) {
    let Some(output) = response.get("output").and_then(Value::as_array) else {
        return;
    };
    for (index, item) in output.iter().enumerate() {
        if !state.finished_output_items.contains(&index) {
            reconcile_output_item(state, index, item, events);
            state.finished_output_items.insert(index);
        }
    }
}

fn reconcile_output_item(
    state: &mut ResponsesState,
    index: usize,
    item: &Value,
    events: &mut Vec<ModelStreamEvent>,
) {
    match item.get("type").and_then(Value::as_str).unwrap_or("") {
        "message" => {
            let id = item.get("id").and_then(Value::as_str).unwrap_or("text");
            let complete = item
                .get("content")
                .and_then(Value::as_array)
                .map(|content| {
                    content
                        .iter()
                        .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                            Some("output_text") => part.get("text").and_then(Value::as_str),
                            Some("refusal") => part.get("refusal").and_then(Value::as_str),
                            _ => None,
                        })
                        .collect::<String>()
                });
            reconcile_text(state, index, id, complete.as_deref(), true, events);
        }
        "refusal" => {
            let id = item.get("id").and_then(Value::as_str).unwrap_or("refusal");
            reconcile_text(
                state,
                index,
                id,
                item.get("refusal").and_then(Value::as_str),
                true,
                events,
            );
        }
        "function_call" => {
            let tool = state
                .tools
                .remove(&index)
                .unwrap_or_else(|| ResponseToolState {
                    item_id: item.get("id").and_then(Value::as_str).unwrap_or("").into(),
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
        "reasoning" => reconcile_reasoning_item(state, index, item, events),
        _ => {}
    }
}

fn reconcile_reasoning_item(
    state: &mut ResponsesState,
    index: usize,
    item: &Value,
    events: &mut Vec<ModelStreamEvent>,
) {
    let id = item.get("id").and_then(Value::as_str).unwrap_or("");
    open_reasoning(state, index, id, events);
    if let Some(summary) = item.get("summary").and_then(Value::as_array) {
        for (summary_index, part) in summary.iter().enumerate() {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                reconcile_reasoning_summary(state, index, id, summary_index, text, events);
            }
        }
    }
    if let Some(content) = item.get("content").and_then(Value::as_array) {
        for (content_index, part) in content.iter().enumerate() {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                reconcile_reasoning_content(state, index, id, content_index, text, events);
            }
        }
    }
    let reasoning = state
        .reasoning_open
        .remove(&index)
        .unwrap_or_else(|| ResponseReasoningState {
            id: id.to_string(),
            ..Default::default()
        });
    events.push(ModelStreamEvent::ReasoningEnd {
        id: reasoning.id.clone(),
        provider_metadata: openai_reasoning_metadata(
            &reasoning,
            item.get("encrypted_content").and_then(Value::as_str),
        ),
    });
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

fn openai_reasoning_metadata(
    reasoning: &ResponseReasoningState,
    encrypted_content: Option<&str>,
) -> ProviderMetadata {
    let mut value = Map::new();
    if let Some(provider_id) = &reasoning.provider_id {
        value.insert("id".into(), json!(provider_id));
        value.insert("item_id".into(), json!(provider_id));
    }
    if let Some(encrypted_content) = encrypted_content {
        value.insert("encrypted_content".into(), json!(encrypted_content));
    }
    value.insert(
        "summary".into(),
        Value::Array(
            reasoning
                .summary_parts
                .values()
                .map(|text| json!({"type": "summary_text", "text": text}))
                .collect(),
        ),
    );
    if !reasoning.content_parts.is_empty() {
        value.insert(
            "content".into(),
            Value::Array(
                reasoning
                    .content_parts
                    .values()
                    .map(|text| json!({"type": "reasoning_text", "text": text}))
                    .collect(),
            ),
        );
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
    for (_, text) in std::mem::take(&mut state.texts) {
        if text.open {
            events.push(ModelStreamEvent::TextEnd { id: text.id });
        }
    }
    for (_, reasoning) in std::mem::take(&mut state.reasoning_open) {
        events.push(ModelStreamEvent::ReasoningEnd {
            id: reasoning.id.clone(),
            provider_metadata: openai_reasoning_metadata(&reasoning, None),
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
    use crate::llm::schema::{GenerationConfig, OpenAiResponsesConfig, ProviderRoute};

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
        assert_eq!(
            input[0]["summary"],
            json!([{"type": "summary_text", "text": "summary"}])
        );
    }

    #[test]
    fn stateless_reasoning_without_summary_replays_an_empty_summary() {
        let mut metadata = ProviderMetadata::new();
        metadata.insert(
            "openai".into(),
            json!({"id": "rs_1", "encrypted_content": "cipher"}),
        );
        let message = crate::types::AgentMessage {
            role: "assistant".into(),
            content: vec![ContentBlock::reasoning("", metadata)],
            ..Default::default()
        };
        let mut input = Vec::new();
        lower_message(&message, &mut input).unwrap();
        assert_eq!(input[0]["summary"], json!([]));
    }

    #[test]
    fn stateless_reasoning_replays_exact_summary_and_content_parts() {
        let mut metadata = ProviderMetadata::new();
        metadata.insert(
            "openai".into(),
            json!({
                "id": "rs_1",
                "encrypted_content": "cipher",
                "summary": [
                    {"type": "summary_text", "text": "step one"},
                    {"type": "summary_text", "text": "step two"}
                ],
                "content": [{"type": "reasoning_text", "text": "raw"}]
            }),
        );
        let message = crate::types::AgentMessage {
            role: "assistant".into(),
            content: vec![ContentBlock::reasoning("flattened", metadata)],
            ..Default::default()
        };
        let mut input = Vec::new();
        lower_message(&message, &mut input).unwrap();
        assert_eq!(
            input[0]["summary"],
            json!([
                {"type": "summary_text", "text": "step one"},
                {"type": "summary_text", "text": "step two"}
            ])
        );
        assert_eq!(
            input[0]["content"],
            json!([{"type": "reasoning_text", "text": "raw"}])
        );
    }

    #[test]
    fn idless_reasoning_uses_a_synthetic_stream_id_but_is_not_replayed() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = ResponsesState::default();
        let added = adapter
            .decode_frame(
                &frame(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": 2,
                        "item": {"type": "reasoning"}
                    }),
                ),
                &mut state,
            )
            .unwrap();
        assert!(matches!(
            &added[0],
            ModelStreamEvent::ReasoningStart { id } if id == "reasoning-2"
        ));

        let done = adapter
            .decode_frame(
                &frame(
                    "response.output_item.done",
                    json!({
                        "type": "response.output_item.done",
                        "output_index": 2,
                        "item": {
                            "type": "reasoning",
                            "summary": [],
                            "encrypted_content": "cipher"
                        }
                    }),
                ),
                &mut state,
            )
            .unwrap();
        let ModelStreamEvent::ReasoningEnd {
            id,
            provider_metadata,
        } = &done[0]
        else {
            panic!("expected reasoning end")
        };
        assert_eq!(id, "reasoning-2");
        assert!(provider_metadata["openai"].get("id").is_none());
        assert!(provider_metadata["openai"].get("item_id").is_none());

        let message = crate::types::AgentMessage {
            role: "assistant".into(),
            content: vec![ContentBlock::reasoning("", provider_metadata.clone())],
            ..Default::default()
        };
        let mut input = Vec::new();
        lower_message(&message, &mut input).unwrap();
        assert!(input.is_empty());
    }

    #[test]
    fn late_wire_reasoning_id_is_persisted_without_changing_the_assembly_id() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = ResponsesState::default();
        adapter
            .decode_frame(
                &frame(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": 1,
                        "item": {"type": "reasoning"}
                    }),
                ),
                &mut state,
            )
            .unwrap();
        let done = adapter
            .decode_frame(
                &frame(
                    "response.output_item.done",
                    json!({
                        "type": "response.output_item.done",
                        "output_index": 1,
                        "item": {"type": "reasoning", "id": "rs_wire", "summary": []}
                    }),
                ),
                &mut state,
            )
            .unwrap();
        let ModelStreamEvent::ReasoningEnd {
            id,
            provider_metadata,
        } = &done[0]
        else {
            panic!("expected reasoning end")
        };
        assert_eq!(id, "reasoning-1");
        assert_eq!(provider_metadata["openai"]["id"], "rs_wire");
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

        let mut state = adapter.new_stream_state();
        let filtered = adapter
            .decode_frame(
                &frame(
                    "response.incomplete",
                    json!({
                        "type": "response.incomplete",
                        "response": {
                            "status": "incomplete",
                            "incomplete_details": {"reason": "content_filter"}
                        }
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            filtered.as_slice(),
            [ModelStreamEvent::Finish {
                reason: FinishReason::ContentFilter,
                ..
            }]
        ));
    }

    fn target(protocol: ProtocolConfig) -> ResolvedModelTarget {
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
            generation: GenerationConfig::default(),
        }
    }

    #[test]
    fn build_body_rejects_non_responses_target() {
        let target = target(ProtocolConfig::OpenAiChat(
            crate::llm::schema::OpenAiChatConfig::default(),
        ));
        let request = ModelRequest {
            model: "m".into(),
            system_prompt: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
        };
        let error = OpenAiResponsesAdapter
            .build_body(&target, &request)
            .unwrap_err();
        assert!(error.to_string().contains("non-responses target"));
    }

    #[test]
    fn build_body_serializes_tools_temperature_and_reasoning() {
        let responses_config = OpenAiResponsesConfig {
            reasoning_context: Some("all_turns".into()),
            reasoning_mode: Some("pro".into()),
            prompt_cache_options: Some(json!({"mode": "explicit", "ttl": "30m"})),
            ..Default::default()
        };
        let mut target = target(ProtocolConfig::OpenAiResponses(responses_config));
        target.capabilities.reasoning.supported = true;
        target.generation.temperature = Some(0.5);
        target.generation.max_output_tokens = Some(123);
        target.generation.thinking_level = "high".into();
        let request = ModelRequest {
            model: "m".into(),
            system_prompt: "system".into(),
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
        let body = OpenAiResponsesAdapter
            .build_body(&target, &request)
            .unwrap();
        assert!(body["tools"].is_array());
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(body["temperature"], 0.5);
        assert_eq!(body["max_output_tokens"], 123);
        assert_eq!(
            body["reasoning"],
            json!({
                "effort": "high",
                "summary": "auto",
                "context": "all_turns",
                "mode": "pro"
            })
        );
        assert_eq!(
            body["prompt_cache_options"],
            json!({"mode": "explicit", "ttl": "30m"})
        );
        assert_eq!(body["input"][0]["role"], "developer");
    }

    #[test]
    fn decode_frame_ignores_empty_and_done_data() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        for data in ["   ", "[DONE]"] {
            let events = adapter
                .decode_frame(
                    &SseFrame {
                        event: None,
                        data: data.into(),
                    },
                    state.as_mut(),
                )
                .unwrap();
            assert!(events.is_empty());
        }
    }

    #[test]
    fn output_item_added_reasoning_starts_reasoning() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": 0,
                        "item": {"type": "reasoning", "id": "rs_1"}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::ReasoningStart { id }] if id == "rs_1"
        ));
    }

    #[test]
    fn output_item_added_ignores_unknown_item_types() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": 0,
                        "item": {"type": "message"}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn output_text_done_emits_text_end() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        adapter
            .decode_frame(
                &frame(
                    "response.output_text.delta",
                    json!({
                        "type": "response.output_text.delta",
                        "output_index": 0,
                        "item_id": "msg_0",
                        "delta": "hi"
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.output_text.done",
                    json!({
                        "type": "response.output_text.done",
                        "output_index": 0,
                        "item_id": "msg_0"
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::TextEnd { id }] if id == "msg_0"
        ));
    }

    #[test]
    fn refusal_delta_emits_text_events() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.refusal.delta",
                    json!({
                        "type": "response.refusal.delta",
                        "output_index": 0,
                        "item_id": "ref_0",
                        "delta": "no"
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::TextStart { id }, ModelStreamEvent::TextDelta { text, .. }]
                if id == "ref_0" && text == "no"
        ));
    }

    #[test]
    fn refusal_done_emits_text_end() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        adapter
            .decode_frame(
                &frame(
                    "response.refusal.delta",
                    json!({
                        "type": "response.refusal.delta",
                        "output_index": 0,
                        "item_id": "ref_0",
                        "delta": "no"
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.refusal.done",
                    json!({
                        "type": "response.refusal.done",
                        "output_index": 0,
                        "item_id": "ref_0"
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::TextEnd { id }] if id == "ref_0"
        ));
    }

    #[test]
    fn reasoning_summary_delta_emits_visible_reasoning() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.reasoning_summary_text.delta",
                    json!({
                        "type": "response.reasoning_summary_text.delta",
                        "output_index": 0,
                        "item_id": "rs_0",
                        "summary_index": 0,
                        "delta": "think"
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::ReasoningStart { id }, ModelStreamEvent::ReasoningDelta { text, .. }]
                if id == "rs_0" && text == "think"
        ));
    }

    #[test]
    fn raw_reasoning_is_preserved_but_not_rendered_as_summary() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.reasoning_text.delta",
                    json!({
                        "type": "response.reasoning_text.delta",
                        "output_index": 0,
                        "item_id": "rs_0",
                        "content_index": 0,
                        "delta": "raw"
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::ReasoningStart { id }] if id == "rs_0"
        ));
        let events = adapter.finish_stream(state.as_mut()).unwrap();
        assert!(matches!(
            events.first(),
            Some(ModelStreamEvent::ReasoningEnd { provider_metadata, .. })
                if provider_metadata["openai"]["content"]
                    == json!([{"type": "reasoning_text", "text": "raw"}])
        ));
    }

    #[test]
    fn reasoning_summary_part_done_preserves_a_summary_without_deltas() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.reasoning_summary_part.done",
                    json!({
                        "type": "response.reasoning_summary_part.done",
                        "output_index": 0,
                        "item_id": "rs_0",
                        "part": {"type": "summary_text", "text": "finished summary"}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::ReasoningStart { id }, ModelStreamEvent::ReasoningDelta { text, .. }]
                if id == "rs_0" && text == "finished summary"
        ));
    }

    #[test]
    fn reasoning_summary_done_preserves_multiple_indexed_parts() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let mut events = Vec::new();
        for (summary_index, text) in [(0, "step one"), (1, "step two")] {
            events.extend(
                adapter
                    .decode_frame(
                        &frame(
                            "response.reasoning_summary_text.done",
                            json!({
                                "type": "response.reasoning_summary_text.done",
                                "output_index": 0,
                                "item_id": "rs_0",
                                "summary_index": summary_index,
                                "text": text
                            }),
                        ),
                        state.as_mut(),
                    )
                    .unwrap(),
            );
        }
        assert!(events.iter().any(|event| {
            matches!(event, ModelStreamEvent::ReasoningDelta { text, .. } if text == "step two")
        }));
        let events = adapter
            .decode_frame(
                &frame(
                    "response.output_item.done",
                    json!({
                        "type": "response.output_item.done",
                        "output_index": 0,
                        "item": {
                            "type": "reasoning",
                            "id": "rs_0",
                            "summary": [
                                {"type": "summary_text", "text": "step one"},
                                {"type": "summary_text", "text": "step two"}
                            ],
                            "encrypted_content": "cipher"
                        }
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::ReasoningEnd { provider_metadata, .. }]
                if provider_metadata["openai"]["summary"]
                    == json!([
                        {"type": "summary_text", "text": "step one"},
                        {"type": "summary_text", "text": "step two"}
                    ])
        ));
    }

    #[test]
    fn output_item_done_without_prior_add_builds_tool_from_item() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.output_item.done",
                    json!({
                        "type": "response.output_item.done",
                        "output_index": 2,
                        "item": {
                            "type": "function_call",
                            "id": "fc_9",
                            "call_id": "call_9",
                            "name": "lookup",
                            "arguments": "{\"q\":1}"
                        }
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::ToolInputEnd {
                index: 2,
                id,
                name,
                arguments,
                ..
            }] if id == "call_9" && name == "lookup" && arguments == &json!({"q": 1})
        ));
    }

    #[test]
    fn output_item_done_reasoning_emits_reasoning_end_with_encrypted() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.output_item.done",
                    json!({
                        "type": "response.output_item.done",
                        "output_index": 0,
                        "item": {
                            "type": "reasoning",
                            "id": "rs_1",
                            "encrypted_content": "cipher"
                        }
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::ReasoningStart { id: start_id }, ModelStreamEvent::ReasoningEnd { id, provider_metadata }]
                if id == "rs_1"
                    && start_id == "rs_1"
                    && provider_metadata["openai"]["encrypted_content"] == "cipher"
        ));
    }

    #[test]
    fn output_item_done_recovers_message_without_deltas() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.output_item.done",
                    json!({
                        "type": "response.output_item.done",
                        "output_index": 0,
                        "item": {
                            "type": "message",
                            "id": "msg_1",
                            "role": "assistant",
                            "content": [{"type": "output_text", "text": "complete"}]
                        }
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::TextStart { id }, ModelStreamEvent::TextDelta { text, .. }, ModelStreamEvent::TextEnd { .. }]
                if id == "msg_1" && text == "complete"
        ));
    }

    #[test]
    fn response_completed_recovers_output_without_item_done() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.completed",
                    json!({
                        "type": "response.completed",
                        "response": {
                            "status": "completed",
                            "output": [{
                                "type": "message",
                                "id": "msg_1",
                                "role": "assistant",
                                "content": [{"type": "output_text", "text": "complete"}]
                            }]
                        }
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [
                ModelStreamEvent::TextStart { id },
                ModelStreamEvent::TextDelta { text, .. },
                ModelStreamEvent::TextEnd { .. },
                ModelStreamEvent::Finish { reason: FinishReason::Stop, .. }
            ] if id == "msg_1" && text == "complete"
        ));
    }

    #[test]
    fn output_item_done_ignores_unknown_item_types() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.output_item.done",
                    json!({
                        "type": "response.output_item.done",
                        "output_index": 0,
                        "item": {"type": "future_unknown"}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn failed_and_error_events_emit_error() {
        let adapter = OpenAiResponsesAdapter;

        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.failed",
                    json!({
                        "type": "response.failed",
                        "response": {"error": {"message": "boom"}}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::Error { message }] if message == "boom"
        ));

        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "error",
                    json!({"type": "error", "error": {"message": "boom2"}}),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::Error { message }] if message == "boom2"
        ));

        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame("error", json!({"type": "error", "message": "top boom"})),
                state.as_mut(),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::Error { message }] if message == "top boom"
        ));

        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(&frame("error", json!({"type": "error"})), state.as_mut())
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ModelStreamEvent::Error { message }]
                if message == "OpenAI Responses stream failed"
        ));
    }

    #[test]
    fn unknown_event_type_is_ignored() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        let events = adapter
            .decode_frame(
                &frame(
                    "response.something.weird",
                    json!({"type": "response.something.weird"}),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn finish_stream_after_finish_returns_empty() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        adapter
            .decode_frame(
                &frame(
                    "response.completed",
                    json!({"type": "response.completed", "response": {"status": "completed"}}),
                ),
                state.as_mut(),
            )
            .unwrap();
        assert!(adapter.finish_stream(state.as_mut()).unwrap().is_empty());
    }

    #[test]
    fn lower_message_serializes_image_blocks() {
        let message = crate::types::AgentMessage {
            role: "user".into(),
            content: vec![ContentBlock::image("http://img/1.png")],
            ..Default::default()
        };
        let mut input = Vec::new();
        lower_message(&message, &mut input).unwrap();
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["content"][0]["type"], "input_image");
    }

    #[test]
    fn lower_message_ignores_reasoning_without_openai_metadata() {
        let message = crate::types::AgentMessage {
            role: "assistant".into(),
            content: vec![ContentBlock::reasoning("plain", ProviderMetadata::new())],
            ..Default::default()
        };
        let mut input = Vec::new();
        lower_message(&message, &mut input).unwrap();
        assert!(input.is_empty());
    }

    #[test]
    fn arguments_string_serializes_both_shapes() {
        assert_eq!(arguments_string(&json!("literal")), "literal");
        assert_eq!(arguments_string(&json!({"a": 1})), "{\"a\":1}");
    }

    #[test]
    fn incomplete_reason_unknown_maps_to_unknown() {
        assert_eq!(
            incomplete_reason(&json!({"incomplete_details": {"reason": "weird"}})),
            FinishReason::Unknown("weird".into())
        );
        assert_eq!(incomplete_reason(&json!({})), FinishReason::Incomplete);
    }

    #[test]
    fn finish_stream_closes_open_reasoning_and_tools() {
        let adapter = OpenAiResponsesAdapter;
        let mut state = adapter.new_stream_state();
        adapter
            .decode_frame(
                &frame(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": 0,
                        "item": {"type": "reasoning", "id": "rs_1"}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        adapter
            .decode_frame(
                &frame(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": 1,
                        "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup"}
                    }),
                ),
                state.as_mut(),
            )
            .unwrap();
        let events = adapter.finish_stream(state.as_mut()).unwrap();
        assert!(events.iter().any(|e| matches!(
            e,
            ModelStreamEvent::ReasoningEnd { id, .. } if id == "rs_1"
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            ModelStreamEvent::ToolInputEnd { index: 1, id, .. } if id == "call_1"
        )));
        assert!(matches!(
            events.last(),
            Some(ModelStreamEvent::Finish {
                reason: FinishReason::Incomplete,
                ..
            })
        ));
    }
}
