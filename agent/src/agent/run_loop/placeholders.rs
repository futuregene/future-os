//! Tool-result placeholders for calls that never executed.
//!
//! An assistant message carrying tool calls must be followed by one tool
//! message per call, or the next request is rejected with HTTP 400.

use crate::types::{AgentMessage, AgentToolCall, ContentBlock};
/// Append placeholder tool-result messages for tool calls that were never
/// executed (truncated stream or stop condition). Without a matching tool
/// message per tool_call_id, the LLM API rejects the conversation on the
/// next request with HTTP 400: "An assistant message with 'tool_calls'
/// must be followed by tool messages responding to each 'tool_call_id'".
pub(super) fn append_skipped_tool_placeholders(
    messages: &mut Vec<AgentMessage>,
    tool_calls: &[AgentToolCall],
    reason: &str,
    save: Option<&crate::agent::PersistCallback>,
) {
    for tc in tool_calls {
        let note = format!(
            "[Tool execution skipped — {} was not executed because {reason}]",
            tc.name
        );
        let mut placeholder = AgentMessage {
            role: "tool".to_string(),
            content: vec![ContentBlock::tool_result(tc.id.clone(), &note, false)],
            name: tc.name.clone(),
            ..Default::default()
        };
        if let Some(save) = save {
            save(&mut placeholder);
        }
        messages.push(placeholder);
    }
}
