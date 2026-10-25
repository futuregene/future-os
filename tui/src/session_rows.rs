//! Session history rows → chat messages, shared by both TUIs.
//!
//! The agent stores a conversation as journal entries (`get_session_entries`
//! rows), and the display projection is one shape regardless of which entry
//! kind produced it. Turning that into the [`ChatMessage`] list the transcript
//! renders is not a detail either client may own on its own: the inline TUI and
//! the workbench must show the same conversation, the same way, including the
//! structural rules below (a `tool_call` step with no text renders nothing and
//! must not break a folded run; a compaction entry becomes a divider).
//!
//! So the mapping lives here and both call it.

use std::collections::HashMap;

use serde_json::Value;

use crate::components::chat_area::{ChatMessage, ChatRole, ToolStatus};

/// `crypto.randomUUID()` — the id a row without one gets.
pub fn random_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The divider text a compaction entry renders as.
pub fn compaction_text(before: Option<&Value>, after: Option<&Value>) -> String {
    match (
        before.and_then(Value::as_u64),
        after.and_then(Value::as_u64),
    ) {
        (Some(before), Some(after)) => {
            format!("Context compacted: {before} → {after} tokens (estimated)")
        }
        _ => "Context compacted".into(),
    }
}

pub fn rows_to_messages(rows: &[Value]) -> Vec<ChatMessage> {
    // A `tool_result` block carries only the call id — the display name and
    // arguments live on the matching `tool_call` block. Index those first so
    // replayed tool messages render like live ones instead of falling back
    // to the raw call id (`call_00_...`).
    let mut tool_calls: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    for msg in rows {
        let Some(blocks) = msg.get("blocks").and_then(Value::as_array) else {
            continue;
        };
        for b in blocks {
            if b["kind"].as_str() != Some("tool_call") {
                continue;
            }
            let Some(call_id) = b["toolCallId"].as_str().filter(|s| !s.is_empty()) else {
                continue;
            };
            tool_calls.insert(
                call_id.to_string(),
                (
                    b["name"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                    b.get("arguments").map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    }),
                ),
            );
        }
    }

    let mut messages = Vec::new();
    for msg in rows {
        let Some(obj) = msg.as_object() else { continue };
        if obj.get("kind").and_then(Value::as_str) == Some("compaction") {
            let checkpoint = &obj["checkpoint"];
            messages.push(ChatMessage::new(
                obj.get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("compaction")
                    .to_string(),
                ChatRole::System,
                &compaction_text(
                    checkpoint.get("tokensBefore"),
                    checkpoint.get("tokensAfter"),
                ),
            ));
            continue;
        }
        let role = obj.get("role").and_then(Value::as_str).unwrap_or("");
        // Only render user, assistant, and tool messages.
        if !["user", "assistant", "tool"].contains(&role) {
            continue;
        }

        let blocks = obj
            .get("blocks")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let content = blocks
            .iter()
            .filter(|b| matches!(b["kind"].as_str(), Some("text" | "tool_result")))
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("");
        let thinking = blocks
            .iter()
            .filter(|b| b["kind"] == "reasoning")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("");
        // A row the transcript would render as nothing is dropped.
        //
        // The agent stores a tool-using turn as one assistant entry per call
        // (the `tool_call` step, which carries no text) followed by the
        // call's own `tool` entry, so those steps are structural, not
        // content: kept as messages they render no row of their own while
        // still taking a separator (a second blank line between calls the
        // live view does not have) and breaking every run of consecutive
        // calls, so a loaded session never folded. A call row is never
        // dropped on that ground — it has a row of its own to render, and
        // the live view shows it too even when the call returned nothing.
        let renders_something = role == "tool" || !content.is_empty() || !thinking.is_empty();
        if !renders_something {
            continue;
        }
        // (Pre-filtered above to user/assistant/tool.)
        let role_enum = match role {
            "user" => ChatRole::User,
            "assistant" => ChatRole::Assistant,
            _ => ChatRole::Tool,
        };
        let id = obj.get("id").and_then(Value::as_str).unwrap_or("");
        let id = if id.is_empty() {
            random_id()
        } else {
            id.to_string()
        };
        let mut cm = ChatMessage::new(id, role_enum, &content);
        let tool = blocks
            .iter()
            .find(|b| matches!(b["kind"].as_str(), Some("tool_call" | "tool_result")));
        let call_id = tool
            .and_then(|b| b["toolCallId"].as_str())
            .filter(|s| !s.is_empty());
        let known_call = call_id.and_then(|id| tool_calls.get(id));
        cm.name = tool
            .and_then(|b| b["name"].as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| known_call.and_then(|(name, _)| name.clone()));
        cm.tool = call_id.map(str::to_owned);
        cm.tool_args = match tool.and_then(|b| b.get("arguments")) {
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => Some(other.to_string()),
            None => known_call.and_then(|(_, args)| args.clone()),
        };
        cm.thinking = (!thinking.is_empty()).then_some(thinking);
        if role == "tool" {
            cm.tool_status = Some(
                if tool.and_then(|b| b["isError"].as_bool()).unwrap_or(false) {
                    ToolStatus::Error
                } else {
                    ToolStatus::Complete
                },
            );
        }
        messages.push(cm);
    }
    messages
}
