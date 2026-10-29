//! Tool-call assembly and repair for the model stream.
//!
//! Providers disagree on how a call arrives — repeated starts for the same id,
//! argument fragments out of prefix order, half-written JSON when the stream is
//! cut — so the stream is normalized here before anything reaches the
//! transcript.

use crate::types::{AgentToolCall, ToolCall};

use super::blocks::{record_assistant_block, AssistantBlockOrder};
pub(super) fn clear_unfinished_provider_identity(tool: &mut AgentToolCall) {
    if let Some(metadata) = tool
        .provider_metadata
        .get_mut("openai")
        .and_then(serde_json::Value::as_object_mut)
    {
        metadata.remove("id");
        metadata.remove("item_id");
    }
}

pub(super) fn push_finalized_tool_call(
    tool_calls: &mut Vec<AgentToolCall>,
    order: &mut Vec<AssistantBlockOrder>,
    tool_call: AgentToolCall,
) {
    let index = tool_calls.len();
    tool_calls.push(finalize_agent_tool_call(tool_call));
    record_assistant_block(order, AssistantBlockOrder::Tool(index));
}

/// Merge a repeated tool-input start (same tool id at the same stream index)
/// into the pending call's args, returning true when the event was consumed
/// as a repeat. Always prefers the longer args string — it's more complete:
/// some gateways (e.g. Aliyun MaaS) send chunks out of prefix order, or a
/// trailing fragment shorter than the accumulated args, and overwriting
/// longer data with shorter data is the primary cause of argument loss.
pub(super) fn merge_repeated_tool_input_start(
    current_tool_calls: &mut [Option<AgentToolCall>],
    index: usize,
    id: &str,
    name: &str,
    arguments: Option<&serde_json::Value>,
    provider_metadata: &crate::types::ProviderMetadata,
) -> bool {
    let Some(Some(existing)) = current_tool_calls.get_mut(index) else {
        return false;
    };
    if existing.id != id {
        return false;
    }
    if !name.is_empty() {
        existing.name = name.to_string();
    }
    if !provider_metadata.is_empty() {
        existing.provider_metadata = provider_metadata.clone();
    }
    if let Some(arguments) = arguments {
        if let serde_json::Value::String(new_args) = arguments {
            let mut updated = false;
            if let serde_json::Value::String(ref mut s) = existing.args {
                if new_args.len() > s.len() {
                    if new_args.starts_with(s.as_str()) {
                        s.push_str(&new_args[s.len()..]);
                    } else {
                        *s = new_args.clone();
                    }
                }
                updated = true;
            }
            if !updated {
                existing.args = serde_json::Value::String(new_args.clone());
            }
        } else if !arguments.is_null() {
            existing.args = arguments.clone();
        }
    }
    true
}

pub(super) fn finalize_agent_tool_call(mut tool_call: AgentToolCall) -> AgentToolCall {
    repair_partial_tool_args(&mut tool_call.args);
    tool_call
}

pub(super) fn agent_tool_call_to_tool_call(tool_call: AgentToolCall) -> ToolCall {
    ToolCall {
        id: tool_call.id,
        call_type: "function".to_string(),
        function: crate::types::ToolCallFn {
            name: tool_call.name,
            arguments: tool_call.args,
        },
    }
}

pub(super) fn repair_partial_tool_args(args: &mut serde_json::Value) {
    future_rpc::tool_arguments::repair_string_v1(args);
}

#[cfg(test)]
pub(super) fn repair_partial_json_object(raw: &str) -> Option<String> {
    future_rpc::tool_arguments::repair_partial_json_object(raw)
}

#[cfg(test)]
pub(super) fn has_unclosed_string(value: &str) -> bool {
    future_rpc::tool_arguments::has_unclosed_string(value)
}
