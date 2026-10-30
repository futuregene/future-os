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

use crate::components::chat_area::{ChatMessage, ChatPicture, ChatRole, ToolStatus};

/// The pictures attached to a row: the file's path, and the name its caption
/// shows.
fn image_attachments(row: &serde_json::Map<String, Value>) -> Vec<ChatPicture> {
    let Some(list) = row
        .get("metadata")
        .and_then(|meta| meta.get("attachments"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    list.iter()
        .filter(|attachment| attachment["kind"].as_str() == Some("image"))
        .filter_map(|attachment| {
            let path = attachment["path"]
                .as_str()
                .filter(|path| !path.is_empty())?;
            let name = attachment["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| {
                    std::path::Path::new(path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(path)
                });
            // The caption is markdown text the renderer is handed, so a bracket
            // or a newline in a file name would close the row's own syntax. The
            // path travels as a field: `terminal_image::path_href` escapes it
            // where the hyperlink is written.
            let name = name.replace(['[', ']', '\n', '\r'], " ");
            Some(ChatPicture {
                path: path.to_string(),
                name,
            })
        })
        .collect()
}

/// `crypto.randomUUID()` — the id a row without one gets.
pub fn random_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The divider text a compaction entry renders as.
///
/// The counts come from the committed checkpoint: `tokensBefore` is what the
/// turn was about to send, `tokensAfter` the agent's estimate of the next
/// turn's prompt. They are only meaningful as a pair — a lone count (a
/// released journal, a legacy row) keeps the bare label.
pub fn compaction_text_of(before: Option<u64>, after: Option<u64>) -> String {
    match (before, after) {
        (Some(before), Some(after)) if before > 0 && after > 0 => {
            format!("Context compacted: {before} → {after} tokens (estimated)")
        }
        _ => "Context compacted".into(),
    }
}

/// [`compaction_text_of`] over the display projection's JSON fields, the shape
/// a stored `compaction` entry carries (`checkpoint.tokensBefore`).
pub fn compaction_text(before: Option<&Value>, after: Option<&Value>) -> String {
    compaction_text_of(
        before.and_then(Value::as_u64),
        after.and_then(Value::as_u64),
    )
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
        // A picture the reader attached travels as a *path* in the entry's
        // metadata (the JSONL keeps no bytes). It is carried as a field rather
        // than glued into the markdown: the transcript draws a thumbnail with a
        // caption row underneath, which is a layout decision the renderer owns.
        // A sentence here would have to guess at it, and this mapping is shared
        // by both TUIs and has no language.
        let pictures = image_attachments(obj);
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
        let renders_something =
            role == "tool" || !content.is_empty() || !thinking.is_empty() || !pictures.is_empty();
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
        // A row that came from the journal *is* its entry, so its id is the
        // journal's. A row without one is a shape this build never writes, and
        // gets a throwaway id.
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
        cm.pictures = pictures;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_user_messages_pictures_become_pictures_not_markdown() {
        let rows = vec![json!({
            "id": "m1",
            "role": "user",
            "blocks": [{"kind": "text", "text": "look at this"}],
            "metadata": {"attachments": [
                {"kind": "image", "path": "/tmp/a b.png", "name": "a b.png"},
                {"kind": "file", "path": "/tmp/notes.txt", "name": "notes.txt"},
            ]},
        })];
        let messages = rows_to_messages(&rows);
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.content, "look at this");
        // The path travels as a field: the transcript draws the thumbnail over
        // it and writes the hyperlink itself.
        assert_eq!(message.pictures.len(), 1, "{:?}", message.pictures);
        assert_eq!(message.pictures[0].path, "/tmp/a b.png");
        assert_eq!(message.pictures[0].name, "a b.png");
        // A file attachment is not a picture and is not dressed as one.
        assert!(message.pictures.iter().all(|p| p.name != "notes.txt"));
    }

    #[test]
    fn a_picture_with_no_name_is_captioned_by_its_file_name() {
        let rows = vec![json!({
            "id": "m2",
            "role": "user",
            "blocks": [{"kind": "text", "text": "x"}],
            "metadata": {"attachments": [
                {"kind": "image", "path": "/tmp/shot(1).png"},
            ]},
        })];
        let pictures = rows_to_messages(&rows)[0].pictures.clone();
        assert_eq!(pictures.len(), 1);
        assert_eq!(pictures[0].name, "shot(1).png");
        // The raw path is kept; escaping is the hyperlink's business.
        assert_eq!(pictures[0].path, "/tmp/shot(1).png");
    }

    #[test]
    fn a_picture_alone_still_makes_a_rendered_row() {
        // A message whose whole content is a picture renders nothing as text,
        // and a row the transcript drops is a picture nobody can reach.
        let rows = vec![json!({
            "id": "m3",
            "role": "user",
            "blocks": [],
            "metadata": {"attachments": [
                {"kind": "image", "path": "/tmp/a.png", "name": "a.png"},
            ]},
        })];
        assert_eq!(rows_to_messages(&rows).len(), 1);
    }
}
