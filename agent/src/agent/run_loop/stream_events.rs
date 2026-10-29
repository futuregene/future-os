//! Consumption of one model stream into a single assembled assistant turn.
//!
//! A model step arrives as one ordered stream of reasoning, text, usage and
//! tool-call events. `StreamState` accumulates them in arrival order and records
//! how the step ended - a genuine terminal event, a truncation, an interrupt, or
//! a provider error - because the caller treats each of those differently.

use crate::agent::{Loop, RunEvent, StreamContext, C_GREEN, C_MAGENTA, C_RESET};
use crate::llm::schema::{FinishReason, ModelStreamEvent};
use crate::types::AgentToolCall;
use anyhow::anyhow;
use tokio::sync::mpsc::Receiver;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use super::blocks::{
    reasoning_block_mut, record_assistant_block, text_block_mut, AccumulatedReasoningBlock,
    AccumulatedTextBlock, AssistantBlockOrder,
};
use super::tool_stream::{
    clear_unfinished_provider_identity, merge_repeated_tool_input_start, push_finalized_tool_call,
    repair_partial_tool_args,
};

/// Everything one model step accumulates, in arrival order.
///
/// Tool calls are held apart from the ordered block list: a provider may revise
/// the same call several times before it is final, and only finalized calls
/// enter that order.
#[derive(Default)]
pub(super) struct StreamState {
    /// Assistant text for this turn, seeded with text recovered from a reconnect.
    pub(super) assistant_text: String,
    pub(super) reasoning_blocks: Vec<AccumulatedReasoningBlock>,
    pub(super) text_blocks: Vec<AccumulatedTextBlock>,
    pub(super) block_order: Vec<AssistantBlockOrder>,
    pub(super) agent_tool_calls: Vec<AgentToolCall>,
    pub(super) total_usage: Option<crate::types::Usage>,
    pub(super) current_tool_calls: Vec<Option<AgentToolCall>>,
    pub(super) output_started: bool,
    pub(super) was_outputting: bool,
    pub(super) stream_error: Option<anyhow::Error>,
    pub(super) interrupted_during_stream: bool,
    pub(super) model_stream_failed: bool,
    /// Set when the stream ended without a terminal event, or was cut short: the
    /// accumulated text is a prefix, not a finished answer.
    pub(super) stream_truncated: bool,
    /// How the truncation was first detected, for the run-terminal context.
    pub(super) truncation_detected_by: Option<&'static str>,
}

impl StreamState {
    /// A fresh step, carrying `retry_text` recovered from a stream being resumed.
    pub(super) fn new(retry_text: &str) -> Self {
        Self {
            assistant_text: retry_text.to_string(),
            ..Default::default()
        }
    }
}

impl Loop {
    /// Drain one model stream into `state` until it ends, fails, or is interrupted.
    ///
    /// Returns once the stream is finished; how it finished is read from `state`,
    /// which is what keeps this method free of the turn-level control flow.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn consume_stream<OnText, OnEvent>(
        &self,
        state: &mut StreamState,
        rx: &mut ReceiverStream<ModelStreamEvent>,
        ctx: &StreamContext,
        on_text: &mut OnText,
        on_event: &OnEvent,
        interrupt_rx: &mut Option<Receiver<()>>,
        reconnecting: &mut bool,
        stream_retry_attempt: usize,
        max_stream_retries: usize,
        turn: usize,
    ) where
        OnText: Fn(String),
        OnEvent: Fn(RunEvent) + Sync,
    {
        let StreamState {
            assistant_text,
            reasoning_blocks,
            text_blocks,
            block_order: assistant_block_order,
            agent_tool_calls,
            total_usage,
            current_tool_calls,
            output_started,
            was_outputting,
            stream_error,
            interrupted_during_stream,
            model_stream_failed,
            stream_truncated,
            truncation_detected_by,
        } = state;
        // A terminal event is the only proof a step finished; silence is not.
        let mut saw_terminal_event = false;
        loop {
            // No event deadline: providers can reason silently for minutes.
            // Reuse the interrupt helper so flag-only cancellation also wakes
            // this wait, even without an interrupt channel.
            let model_event = match self
                .await_or_interrupt(rx.next(), interrupt_rx.as_mut())
                .await
            {
                Some(Some(event)) => event,
                None => {
                    *interrupted_during_stream = true;
                    *stream_error = Some(anyhow!("interrupted"));
                    break;
                }
                Some(None) => {
                    if !saw_terminal_event {
                        *stream_truncated = true;
                        truncation_detected_by.get_or_insert("eof_no_terminal");
                    }
                    break;
                }
            };
            // Provider indices must never directly control unbounded vector
            // allocation (including usize::MAX + 1 overflow).
            if matches!(&model_event,
                    ModelStreamEvent::ToolInputStart { index, .. }
                    | ModelStreamEvent::ToolInputDelta { index, .. }
                    | ModelStreamEvent::ToolInputEnd { index, .. } if *index >= 256)
            {
                let message = "provider tool index exceeds the 256-call response limit".to_string();
                on_event(RunEvent::Model(ModelStreamEvent::Error {
                    message: message.clone(),
                }));
                *stream_error = Some(anyhow::anyhow!(message));
                *model_stream_failed = true;
                *stream_truncated = true;
                *truncation_detected_by = Some("invalid_tool_index");
                break;
            }
            // HTTP headers alone do not mean recovery. Wait for real model
            // progress; EOF cleanup/usage events must not clear the hint.
            if *reconnecting
                && matches!(
                    &model_event,
                    ModelStreamEvent::TextStart { .. }
                        | ModelStreamEvent::TextDelta { .. }
                        | ModelStreamEvent::ReasoningStart { .. }
                        | ModelStreamEvent::ReasoningDelta { .. }
                        | ModelStreamEvent::ToolInputStart { .. }
                        | ModelStreamEvent::ToolInputDelta { .. }
                        | ModelStreamEvent::Finish { .. }
                )
            {
                *reconnecting = false;
                on_event(RunEvent::StreamResumed);
            }
            let mut model_event = model_event;
            if ctx.segmented_storage {
                match &mut model_event {
                    ModelStreamEvent::ToolInputStart { id, .. } if id.is_empty() => {
                        *id = crate::utils::generate_entry_id();
                    }
                    ModelStreamEvent::ToolInputDelta { index, id, .. }
                    | ModelStreamEvent::ToolInputEnd { index, id, .. }
                        if id.is_empty() =>
                    {
                        *id = current_tool_calls
                            .get(*index)
                            .and_then(Option::as_ref)
                            .map(|tool| tool.id.clone())
                            .filter(|id| !id.is_empty())
                            .unwrap_or_else(crate::utils::generate_entry_id);
                    }
                    _ => {}
                }
                if let ModelStreamEvent::ToolInputEnd { arguments, .. } = &mut model_event {
                    repair_partial_tool_args(arguments);
                }
            }
            // Clients treat `error` as terminal. Do not stop their stream
            // for a disconnect that will be recovered below.
            let retryable_disconnect = matches!(
                &model_event,
                ModelStreamEvent::Error { message }
                    if message.starts_with(crate::llm::UPSTREAM_DISCONNECTED)
            );
            if !retryable_disconnect || stream_retry_attempt >= max_stream_retries {
                on_event(RunEvent::Model(model_event.clone()));
            }

            // Close the text-output block before switching to a different
            // event type — text_end may never arrive from the LLM.
            let is_text = matches!(model_event, ModelStreamEvent::TextDelta { .. });
            if self.verbose && *was_outputting && !is_text {
                crate::eprintln_log!();
                *was_outputting = false;
            }
            if is_text && self.verbose {
                *was_outputting = true;
            }

            match model_event {
                ModelStreamEvent::ReasoningStart { id } => {
                    reasoning_block_mut(reasoning_blocks, &id);
                    record_assistant_block(
                        assistant_block_order,
                        AssistantBlockOrder::Reasoning(id),
                    );
                    if self.verbose {
                        crate::eprint_log!("\n{}[thinking]{} ", C_MAGENTA, C_RESET);
                    }
                }
                ModelStreamEvent::ReasoningDelta { id, text } => {
                    reasoning_block_mut(reasoning_blocks, &id)
                        .text
                        .push_str(&text);
                    record_assistant_block(
                        assistant_block_order,
                        AssistantBlockOrder::Reasoning(id),
                    );
                    if self.verbose {
                        crate::eprint_log!("{}", text);
                    }
                }
                ModelStreamEvent::ReasoningEnd {
                    id,
                    provider_metadata,
                } => {
                    reasoning_block_mut(reasoning_blocks, &id).provider_metadata =
                        provider_metadata;
                    record_assistant_block(
                        assistant_block_order,
                        AssistantBlockOrder::Reasoning(id),
                    );
                    if self.verbose {
                        crate::eprintln_log!(); // blank line after thinking
                    }
                }
                ModelStreamEvent::TextDelta { id, text } => {
                    assistant_text.push_str(&text);
                    text_block_mut(text_blocks, &id).text.push_str(&text);
                    record_assistant_block(assistant_block_order, AssistantBlockOrder::Text(id));
                    if self.verbose && !*output_started {
                        *output_started = true;
                        crate::eprint_log!("\n{}[output]{} ", C_GREEN, C_RESET);
                    }
                    if self.verbose {
                        crate::eprint_log!("{}", text);
                    }
                    on_text(text);
                }
                ModelStreamEvent::TextStart { id } => {
                    text_block_mut(text_blocks, &id);
                    record_assistant_block(assistant_block_order, AssistantBlockOrder::Text(id));
                }
                ModelStreamEvent::TextEnd { id } => {
                    text_block_mut(text_blocks, &id);
                    record_assistant_block(assistant_block_order, AssistantBlockOrder::Text(id));
                    if self.verbose {
                        crate::eprintln_log!(); // blank line after output
                    }
                }
                ModelStreamEvent::ToolInputStart {
                    index,
                    id,
                    name,
                    arguments,
                    provider_metadata,
                } => {
                    // Some providers (e.g. GLM/Z.AI without tool_stream)
                    // send id+name in every argument chunk instead of just
                    // the first — a repeat is merged into the pending call
                    // rather than starting a new one.
                    if merge_repeated_tool_input_start(
                        current_tool_calls,
                        index,
                        &id,
                        &name,
                        arguments.as_ref(),
                        &provider_metadata,
                    ) {
                        continue;
                    }
                    if index >= current_tool_calls.len() {
                        current_tool_calls.resize(index + 1, None);
                    }

                    // Finalize any existing tool call at this index (different id)
                    if let Some(tc) = current_tool_calls[index].take() {
                        push_finalized_tool_call(agent_tool_calls, assistant_block_order, tc);
                    }

                    current_tool_calls[index] = Some(AgentToolCall {
                        id,
                        name,
                        args: arguments.unwrap_or(serde_json::Value::Null),
                        provider_metadata,
                    });
                }
                ModelStreamEvent::ToolInputDelta {
                    index,
                    id,
                    delta,
                    snapshot,
                } => {
                    if index >= current_tool_calls.len() {
                        current_tool_calls.resize(index + 1, None);
                    }
                    if let Some(tc_ref) = &mut current_tool_calls[index] {
                        if tc_ref.id.is_empty() && !id.is_empty() {
                            tc_ref.id = id;
                        }
                        if let serde_json::Value::String(ref mut s) = tc_ref.args {
                            // Some proxies (e.g. Anthropic → OpenAI) send the
                            // full current state of the tool input JSON in every
                            // delta, not incremental fragments. Detect this by
                            // checking if the delta starts with '{' — incremental
                            // OpenAI fragments always start with ',' or another
                            // continuation character. When the accumulated args
                            // are empty, just "{}", or a prefix of the delta,
                            // replace instead of concatenating to avoid corrupt
                            // JSON like {}{"path":...}.
                            if snapshot {
                                *s = delta;
                            } else {
                                s.push_str(&delta);
                            }
                        } else {
                            tc_ref.args = serde_json::Value::String(delta);
                        }
                    } else {
                        // Delta arrived before start — create placeholder
                        current_tool_calls[index] = Some(AgentToolCall {
                            id,
                            name: String::new(),
                            args: serde_json::Value::String(delta),
                            provider_metadata: Default::default(),
                        });
                    }
                }
                ModelStreamEvent::ToolInputEnd {
                    index,
                    id,
                    name,
                    arguments,
                    provider_metadata,
                } => {
                    if index >= current_tool_calls.len() {
                        current_tool_calls.resize(index + 1, None);
                    }
                    let slot = current_tool_calls[index].get_or_insert_with(|| AgentToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        args: arguments.clone(),
                        provider_metadata: provider_metadata.clone(),
                    });
                    if !id.is_empty() {
                        slot.id = id;
                    }
                    if !name.is_empty() {
                        slot.name = name;
                    }
                    if !arguments.is_null() {
                        slot.args = arguments;
                    }
                    if !provider_metadata.is_empty() {
                        slot.provider_metadata = provider_metadata;
                    }
                    if let Some(tc) = current_tool_calls[index].take() {
                        push_finalized_tool_call(agent_tool_calls, assistant_block_order, tc);
                    }
                }
                ModelStreamEvent::Usage(usage) => {
                    self.process_usage_event(&usage, total_usage);
                }
                ModelStreamEvent::Finish { reason, usage } => {
                    tracing::debug!(model = %self.model, turn, finish_reason = reason.as_str(),
                            text_bytes = assistant_text.len(), reasoning_blocks = reasoning_blocks.len(),
                            tool_calls = agent_tool_calls.len(), "LLM generation ended");
                    saw_terminal_event = true;
                    if matches!(
                        &reason,
                        FinishReason::Incomplete
                            | FinishReason::Length
                            | FinishReason::ContentFilter
                            | FinishReason::Error
                            | FinishReason::Cancelled
                            | FinishReason::Paused
                            | FinishReason::Unknown(_)
                    ) {
                        *stream_truncated = true;
                        truncation_detected_by.get_or_insert(match &reason {
                            FinishReason::Length => "finish_length",
                            FinishReason::ContentFilter => "finish_content_filter",
                            FinishReason::Error => "finish_error",
                            FinishReason::Cancelled => "provider_cancelled",
                            FinishReason::Paused => "model_paused",
                            FinishReason::Unknown(_) => "finish_unknown",
                            _ => "finish_incomplete",
                        });
                    }
                    if let Some(ref usage) = usage {
                        self.process_usage_event(usage, total_usage);
                    }
                    for tc_opt in current_tool_calls.iter_mut() {
                        if let Some(mut tc) = tc_opt.take() {
                            clear_unfinished_provider_identity(&mut tc);
                            push_finalized_tool_call(agent_tool_calls, assistant_block_order, tc);
                        }
                    }
                }
                ModelStreamEvent::Error { message } => {
                    tracing::error!(
                        turn,
                        model = %self.model,
                        output_len = assistant_text.len(),
                        tool_calls_so_far = agent_tool_calls.len(),
                        error = %message,
                        "LLM model stream failed"
                    );
                    *model_stream_failed = true;
                    // A model-stream error always leaves the accumulated
                    // response as a prefix. Preserve both that fact and a
                    // stable category so clients can show actionable copy
                    // without matching reqwest/provider wording.
                    *stream_truncated = true;
                    truncation_detected_by.get_or_insert(
                        if message.starts_with("[RESPONSE_TIMEOUT]") {
                            "request_timeout"
                        } else if message.starts_with(crate::llm::UPSTREAM_DISCONNECTED) {
                            "upstream_disconnected"
                        } else {
                            "model_response_error"
                        },
                    );
                    *stream_error = Some(anyhow!(message));
                    break;
                }
            }
        }
    }
}
