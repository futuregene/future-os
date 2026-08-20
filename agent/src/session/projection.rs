//! Mappings between persisted `SessionEntry`s and the in-memory LLM message
//! types (`AgentMessage` / `Message`), plus `truncate_visible`.

use super::entry::{
    SessionEntry, ENTRY_TYPE_ASSISTANT, ENTRY_TYPE_SYSTEM, ENTRY_TYPE_TOOL, ENTRY_TYPE_USER,
};
use crate::types::{Message, ToolCall};
use crate::utils::generate_entry_id;
use chrono::Local;

/// Rebuild in-memory messages from persisted entries when a session is loaded
/// (new_session restore / fork). `model_supports_images` gates image
/// re-hydration: GUI image attachments have their base64 stripped from the JSONL
/// (to keep it small — see `agent_message_to_entry`) and are re-read from their
/// on-disk paths here so the model still sees them after a reload. Legacy
/// `images`-field base64 (TUI / channels) is kept on disk and preserved as-is.
pub fn entries_to_agent_messages(
    entries: &[SessionEntry],
    model_supports_images: bool,
) -> Vec<crate::types::AgentMessage> {
    use crate::types::{AgentToolCall, ContentBlock};
    let mut msgs = vec![];
    for entry in entries {
        let role = match entry.entry_type.as_str() {
            "user" | "system" | "assistant" | "tool" => entry.entry_type.clone(),
            _ => continue,
        };

        let mut content: Vec<ContentBlock> = match &entry.content {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|value| {
                    let block = serde_json::from_value::<ContentBlock>(value.clone()).ok()?;
                    match &block {
                        ContentBlock::Image { image_url } => {
                            // Preserve an on-disk base64 image_url (channels/TUI); a
                            // stripped/empty one (GUI) is skipped — rebuilt from meta.
                            image_url
                                .url
                                .as_ref()
                                .is_some_and(|url| !url.is_empty())
                                .then_some(block)
                        }
                        _ => Some(block),
                    }
                })
                .collect(),
            Some(serde_json::Value::String(s)) => {
                vec![ContentBlock::Text { text: s.clone() }]
            }
            _ => vec![],
        };

        // Re-hydrate GUI image attachments from their paths (base64 was stripped
        // from the JSONL). Skipped for text-only models — they never got the
        // image; the file-path text block (if any) is already in `content`.
        if model_supports_images {
            if let Some(atts) = entry
                .meta
                .as_ref()
                .and_then(|m| m.get("attachments"))
                .and_then(|a| a.as_array())
            {
                for att in atts {
                    if att.get("kind").and_then(|k| k.as_str()) != Some("image") {
                        continue;
                    }
                    if let Some(path) = att.get("path").and_then(|p| p.as_str()) {
                        if let Some(url) = crate::utils::image_data_url_for_model(path) {
                            content.push(ContentBlock::image(url));
                        }
                    }
                }
            }
        }

        let legacy_tool_calls: Vec<AgentToolCall> = entry
            .tool_calls
            .iter()
            .map(|tc| AgentToolCall {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                args: tc.function.arguments.clone(),
                provider_metadata: Default::default(),
            })
            .collect();

        if !entry.thinking.is_empty()
            && !content
                .iter()
                .any(|block| matches!(block, ContentBlock::Reasoning { .. }))
        {
            content.insert(
                0,
                ContentBlock::reasoning(entry.thinking.clone(), Default::default()),
            );
        }
        if !content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolCall { .. }))
        {
            content.extend(legacy_tool_calls.iter().map(|call| {
                ContentBlock::tool_call(
                    call.id.clone(),
                    call.name.clone(),
                    call.args.clone(),
                    call.provider_metadata.clone(),
                )
            }));
        }
        if role == "tool"
            && !content
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolResult { .. }))
        {
            let text = content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            content.retain(|block| !matches!(block, ContentBlock::Text { .. }));
            content.push(ContentBlock::tool_result(
                entry.tool_call_id.clone(),
                text,
                false,
            ));
        }
        msgs.push(crate::types::AgentMessage {
            role,
            content,
            name: entry.name.clone(),
            tool_args: entry.tool_args.clone(),
            metadata: entry.meta.as_ref().and_then(|m| m.as_object().cloned()),
        });
    }
    msgs
}

/// Build context messages from session entries (matching Go BuildContext)
pub fn build_context(entries: &[SessionEntry]) -> Vec<Message> {
    let mut msgs = vec![];
    for entry in entries {
        let role = match entry.entry_type.as_str() {
            "user" | "system" => entry.entry_type.clone(),
            "assistant" => "assistant".to_string(),
            "tool" => "tool".to_string(),
            _ => continue,
        };

        let content = entry.content.clone().unwrap_or(serde_json::Value::Null);
        let tool_calls: Vec<ToolCall> = entry.tool_calls.clone();
        msgs.push(Message {
            role,
            content: Some(content),
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: entry.tool_call_id.clone(),
            name: String::new(),
            tool_args: String::new(),
            reasoning_content: entry.thinking.clone(),
        });
    }
    msgs
}

/// Convert AgentMessage back to SessionEntry for persistence
pub fn agent_message_to_entry(msg: &crate::types::AgentMessage) -> SessionEntry {
    let entry_type = match msg.role.as_str() {
        "user" => ENTRY_TYPE_USER,
        "assistant" => ENTRY_TYPE_ASSISTANT,
        "tool" => ENTRY_TYPE_TOOL,
        "system" => ENTRY_TYPE_SYSTEM,
        _ => ENTRY_TYPE_USER,
    };

    // A GUI message records its images in `meta`, so their (multi-MB) base64
    // image_url blocks are redundant on disk — drop them to keep the JSONL small;
    // entries_to_agent_messages re-reads them from the attachment paths on load.
    // Legacy `images`-field images (TUI / channels) have no meta and are kept.
    let strip_image_blocks = msg
        .metadata
        .as_ref()
        .and_then(|m| m.get("attachments"))
        .and_then(|a| a.as_array())
        .is_some_and(|arr| {
            arr.iter()
                .any(|a| a.get("kind").and_then(|k| k.as_str()) == Some("image"))
        });
    let content_blocks: Vec<serde_json::Value> = msg
        .model_content()
        .iter()
        .map(|b| serde_json::to_value(b).unwrap_or(serde_json::Value::Null))
        .filter(|v| {
            !(strip_image_blocks && v.get("type").and_then(|t| t.as_str()) == Some("image_url"))
        })
        .collect();
    let content = if content_blocks.is_empty() {
        None
    } else {
        Some(serde_json::Value::Array(content_blocks))
    };

    let tool_calls: Vec<crate::types::ToolCall> = msg
        .tool_calls()
        .into_iter()
        .map(|tc| crate::types::ToolCall {
            id: tc.id,
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: tc.name,
                arguments: tc.args,
            },
        })
        .collect();

    SessionEntry {
        id: generate_entry_id(),
        entry_type: entry_type.to_string(),
        role: msg.role.clone(),
        content,
        tool_calls,
        timestamp: Local::now(),
        tool_call_id: msg.tool_call_id(),
        name: msg.name.clone(),
        tool_args: msg.tool_args.clone(),
        thinking: msg.reasoning_text(),
        // Populated at the save site (session_prompt.rs): only the final
        // assistant entry of a run gets a non-zero value, and prior entries'
        // values are preserved from the previously-saved session.
        // Carry structured metadata (e.g. user attachments) into the JSONL so it
        // survives reload; the reverse mapping restores it in
        // entries_to_agent_messages.
        meta: msg.metadata.clone().map(serde_json::Value::Object),
    }
}

pub(crate) fn hydrate_entry_projections(entry: &mut SessionEntry) {
    let Some(serde_json::Value::Array(values)) = entry.content.as_ref() else {
        return;
    };
    let blocks: Vec<crate::types::ContentBlock> = values
        .iter()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .collect();
    if entry.thinking.is_empty() {
        entry.thinking = blocks
            .iter()
            .filter_map(|block| match block {
                crate::types::ContentBlock::Reasoning { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
    }
    if entry.tool_calls.is_empty() {
        entry.tool_calls = blocks
            .iter()
            .filter_map(|block| match block {
                crate::types::ContentBlock::ToolCall { id, name, args, .. } => {
                    Some(crate::types::ToolCall {
                        id: id.clone(),
                        call_type: "function".into(),
                        function: crate::types::ToolCallFn {
                            name: name.clone(),
                            arguments: args.clone(),
                        },
                    })
                }
                _ => None,
            })
            .collect();
    }
    if entry.tool_call_id.is_empty() {
        if let Some(id) = blocks.iter().find_map(|block| match block {
            crate::types::ContentBlock::ToolResult { tool_call_id, .. } => {
                Some(tool_call_id.clone())
            }
            _ => None,
        }) {
            entry.tool_call_id = id;
        }
    }
}

/// Truncate a string to max_vis visible columns. CJK characters count as 2,
/// everything else as 1. Matches approximate terminal rendering width.
pub fn truncate_visible(s: &str, max_vis: usize) -> String {
    let mut vis: usize = 0;
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        let w = if ('\u{1100}'..='\u{115f}').contains(&ch)   // Hangul Jamo
            || ('\u{2e80}'..='\u{a4cf}').contains(&ch)       // CJK radicals + Yi
            || ('\u{ac00}'..='\u{d7a3}').contains(&ch)       // Hangul Syllables
            || ('\u{f900}'..='\u{faff}').contains(&ch)       // CJK Compatibility
            || ('\u{fe30}'..='\u{fe4f}').contains(&ch)       // CJK Compatibility Forms
            || ('\u{ff00}'..='\u{ffef}').contains(&ch)       // Fullwidth Forms
            || ('\u{1f300}'..='\u{1f5ff}').contains(&ch)     // Misc Symbols
            || ('\u{1f900}'..='\u{1f9ff}').contains(&ch)     // Supplemental Symbols
            || ('\u{1f600}'..='\u{1f64f}').contains(&ch)     // Emoticons
            || ('\u{20000}'..='\u{2fffd}').contains(&ch)     // SIP
            || ('\u{30000}'..='\u{3fffd}').contains(&ch)
        // TIP
        {
            2
        } else {
            1
        };
        if vis + w > max_vis {
            break;
        }
        vis += w;
        result.push(ch);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::entry::ENTRY_TYPE_COMPACTION;
    use crate::session::run_journal::RUN_STATE_COMPLETED;

    // ─── truncate_visible ───────────────────────────────────────────────────

    #[test]
    fn truncate_visible_ascii() {
        assert_eq!(truncate_visible("hello world", 5), "hello");
    }

    #[test]
    fn truncate_visible_cjk() {
        assert_eq!(truncate_visible("你好世界", 4), "你好");
    }

    #[test]
    fn truncate_visible_mixed() {
        assert_eq!(truncate_visible("ab你好cd", 4), "ab你");
    }

    #[test]
    fn truncate_visible_emoji() {
        assert_eq!(truncate_visible("a🦀b", 3), "a🦀");
    }

    #[test]
    fn truncate_visible_exact_fit() {
        assert_eq!(truncate_visible("hello", 5), "hello");
    }

    #[test]
    fn truncate_visible_zero() {
        assert_eq!(truncate_visible("hello", 0), "");
    }

    #[test]
    fn truncate_visible_empty_string() {
        assert_eq!(truncate_visible("", 10), "");
    }

    #[test]
    fn truncate_visible_cjk_never_splits() {
        // "中" is 2 cols. 3 cols budget → only "中" fits
        assert_eq!(truncate_visible("中文中文", 3), "中");
    }

    // ─── build_context ──────────────────────────────────────────────────────

    #[test]
    fn build_context_from_entries() {
        let entries = vec![
            SessionEntry::new_user("user", serde_json::json!("hello")),
            SessionEntry::new_assistant(serde_json::json!("hi"), vec![]),
            SessionEntry::new_tool("c1", "output"),
        ];
        let msgs = build_context(&entries);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[2].role, "tool");
        assert_eq!(msgs[2].tool_call_id, "c1");
    }

    #[test]
    fn build_context_skips_non_message_types() {
        let mut compaction = SessionEntry::new_user("user", serde_json::json!("x"));
        compaction.entry_type = ENTRY_TYPE_COMPACTION.to_string();
        let entries = vec![
            SessionEntry::new_user("user", serde_json::json!("hello")),
            compaction,
        ];
        let msgs = build_context(&entries);
        assert_eq!(msgs.len(), 1); // compaction skipped
    }

    #[test]
    fn build_context_preserves_thinking() {
        let mut e = SessionEntry::new_assistant(serde_json::json!("answer"), vec![]);
        e.thinking = "let me think...".to_string();
        let msgs = build_context(&[e]);
        assert_eq!(msgs[0].reasoning_content, "let me think...");
    }

    #[test]
    fn build_context_empty_entries() {
        let msgs = build_context(&[]);
        assert!(msgs.is_empty());
    }

    // ─── agent_message_to_entry ─────────────────────────────────────────────

    #[test]
    fn agent_message_to_entry_user() {
        let msg = crate::types::AgentMessage {
            role: "user".to_string(),
            content: vec![crate::types::ContentBlock::text("hello")],
            ..Default::default()
        };
        let entry = agent_message_to_entry(&msg);
        assert_eq!(entry.entry_type, ENTRY_TYPE_USER);
        assert_eq!(entry.role, "user");
    }

    #[test]
    fn agent_message_to_entry_assistant_with_tool_calls() {
        let msg = crate::types::AgentMessage {
            role: "assistant".to_string(),
            content: vec![
                crate::types::ContentBlock::text("answer"),
                crate::types::ContentBlock::tool_call(
                    "c1",
                    "shell",
                    serde_json::json!({"cmd": "ls"}),
                    Default::default(),
                ),
            ],
            ..Default::default()
        };
        let entry = agent_message_to_entry(&msg);
        assert_eq!(entry.entry_type, ENTRY_TYPE_ASSISTANT);
        assert_eq!(entry.tool_calls.len(), 1);
        assert_eq!(entry.tool_calls[0].function.name, "shell");
    }

    #[test]
    fn agent_message_to_entry_tool() {
        let msg = crate::types::AgentMessage {
            role: "tool".to_string(),
            content: vec![crate::types::ContentBlock::tool_result(
                "c1", "result", false,
            )],
            ..Default::default()
        };
        let entry = agent_message_to_entry(&msg);
        assert_eq!(entry.entry_type, ENTRY_TYPE_TOOL);
        assert_eq!(entry.tool_call_id, "c1");
    }

    #[test]
    fn agent_message_to_entry_preserves_thinking() {
        let msg = crate::types::AgentMessage {
            role: "assistant".to_string(),
            content: vec![
                crate::types::ContentBlock::text("answer"),
                crate::types::ContentBlock::reasoning("reasoning here", Default::default()),
            ],
            ..Default::default()
        };
        let entry = agent_message_to_entry(&msg);
        assert_eq!(entry.thinking, "reasoning here");
    }

    #[test]
    fn agent_message_to_entry_preserves_meta() {
        let mut meta = serde_json::Map::new();
        meta.insert("key".to_string(), serde_json::json!("value"));
        let msg = crate::types::AgentMessage {
            role: "user".to_string(),
            content: vec![crate::types::ContentBlock::text("hi")],
            metadata: Some(meta),
            ..Default::default()
        };
        let entry = agent_message_to_entry(&msg);
        assert!(entry.meta.is_some());
        assert_eq!(entry.meta.unwrap()["key"], "value");
    }

    #[test]
    fn agent_message_to_entry_unknown_role_defaults_user() {
        let msg = crate::types::AgentMessage {
            role: "custom_role".to_string(),
            content: vec![crate::types::ContentBlock::text("x")],
            ..Default::default()
        };
        let entry = agent_message_to_entry(&msg);
        assert_eq!(entry.entry_type, ENTRY_TYPE_USER);
    }

    // ─── entries_to_agent_messages ──────────────────────────────────────────

    #[test]
    fn entries_to_messages_basic() {
        let entries = vec![
            SessionEntry::new_user("user", serde_json::json!("hello")),
            SessionEntry::new_assistant(serde_json::json!("hi"), vec![]),
        ];
        let msgs = entries_to_agent_messages(&entries, false);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn entries_to_messages_skips_non_standard_types() {
        let mut compaction = SessionEntry::new_user("user", serde_json::json!("x"));
        compaction.entry_type = ENTRY_TYPE_COMPACTION.to_string();
        let entries = vec![
            SessionEntry::new_user("user", serde_json::json!("hello")),
            compaction,
        ];
        let msgs = entries_to_agent_messages(&entries, false);
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn entries_to_messages_string_content() {
        let entries = vec![SessionEntry::new_user(
            "user",
            serde_json::json!("plain string"),
        )];
        let msgs = entries_to_agent_messages(&entries, false);
        assert_eq!(msgs[0].text(), "plain string");
    }

    #[test]
    fn entries_to_messages_array_content() {
        let entries = vec![SessionEntry::new_user(
            "user",
            serde_json::json!([
                {"type": "text", "text": "first"},
                {"type": "text", "text": " second"},
            ]),
        )];
        let msgs = entries_to_agent_messages(&entries, false);
        assert_eq!(msgs[0].text(), "first second");
    }

    #[test]
    fn entries_to_messages_tool_calls() {
        let tool_calls = vec![crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "read".to_string(),
                arguments: serde_json::json!({"path": "/tmp"}),
            },
        }];
        let entries = vec![SessionEntry::new_assistant(
            serde_json::json!("reading..."),
            tool_calls,
        )];
        let msgs = entries_to_agent_messages(&entries, false);
        assert_eq!(msgs[0].tool_calls().len(), 1);
        assert_eq!(msgs[0].tool_calls()[0].name, "read");
    }

    #[test]
    fn entries_to_messages_empty_entries() {
        let msgs = entries_to_agent_messages(&[], false);
        assert!(msgs.is_empty());
    }

    #[test]
    fn entries_to_agent_messages_skips_run_markers_and_extra_info() {
        let entries = vec![
            SessionEntry::session_info(
                serde_json::json!({"model": "m"}),
                "m".to_string(),
                "low".to_string(),
            ),
            SessionEntry::new_user("user", serde_json::json!("q")),
            SessionEntry::run_started("run-1", 1),
            SessionEntry::new_assistant(serde_json::json!("a"), vec![]),
            SessionEntry::run_terminal("run-1", RUN_STATE_COMPLETED, 1, 1, None),
            SessionEntry::session_info(
                serde_json::json!({"model": "m"}),
                "m".to_string(),
                "low".to_string(),
            ),
        ];
        let msgs = entries_to_agent_messages(&entries, false);
        // Only the user + assistant content enters the model context; run
        // markers and session_info snapshots are filtered out.
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn entries_to_agent_messages_rehydrates_image_attachments() {
        // A user entry whose meta carries an image attachment gets an image
        // block when the model supports images.
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("pic.png");
        // A real PNG (the loader decodes it for re-encoding).
        image::RgbImage::from_fn(8, 8, |_, _| image::Rgb([1u8, 2, 3]))
            .save(&image)
            .unwrap();
        let mut user = SessionEntry::new_user("user", serde_json::json!("look"));
        user.meta = Some(serde_json::json!({
            "attachments": [{"kind": "image", "path": image.to_string_lossy(), "name": "pic.png"}]
        }));
        let messages = entries_to_agent_messages(&[user], true);
        assert_eq!(messages.len(), 1);
        let content_blocks = messages[0].content.clone();
        assert!(
            content_blocks
                .iter()
                .any(|b| matches!(b, crate::types::ContentBlock::Image { .. })),
            "image block rehydrated: {content_blocks:?}"
        );
    }

    #[test]
    fn entries_to_agent_messages_skips_non_image_and_missing_attachments() {
        let mut user = SessionEntry::new_user("user", serde_json::json!("look"));
        user.meta = Some(serde_json::json!({
            "attachments": [
                {"kind": "file", "path": "/tmp/x.pdf", "name": "x.pdf"},
                {"kind": "image", "path": "/definitely/missing.png", "name": "m.png"},
                {"kind": "image"}
            ]
        }));
        let messages = entries_to_agent_messages(&[user], true);
        assert_eq!(messages.len(), 1);
        assert!(
            !messages[0]
                .content
                .iter()
                .any(|b| matches!(b, crate::types::ContentBlock::Image { .. })),
            "no image blocks from file/missing/unreadable attachments"
        );
        // Text-only models never rehydrate even valid images.
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("pic.png");
        image::RgbImage::from_fn(8, 8, |_, _| image::Rgb([1u8, 2, 3]))
            .save(&image)
            .unwrap();
        let mut user = SessionEntry::new_user("user", serde_json::json!("look"));
        user.meta = Some(serde_json::json!({
            "attachments": [{"kind": "image", "path": image.to_string_lossy(), "name": "pic.png"}]
        }));
        let messages = entries_to_agent_messages(&[user], false);
        assert!(
            !messages[0]
                .content
                .iter()
                .any(|b| matches!(b, crate::types::ContentBlock::Image { .. })),
            "text-only model keeps no image blocks"
        );
    }

    #[test]
    fn entries_to_agent_messages_non_textual_content_and_tool_calls() {
        // Non-string/non-array content maps to no content blocks.
        let numeric = SessionEntry::new_user("user", serde_json::json!(42));
        let messages = entries_to_agent_messages(&[numeric], true);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].content.is_empty());

        // An assistant entry carrying tool_calls keeps them on the message.
        let mut assistant = SessionEntry::new_user("assistant", serde_json::json!("calling"));
        assistant.tool_calls = vec![crate::types::ToolCall {
            id: "tc1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "shell".to_string(),
                arguments: serde_json::json!({}),
            },
        }];
        let messages = build_context(&[assistant]);
        assert_eq!(messages[0].tool_calls.as_ref().unwrap().len(), 1);
    }
}

#[cfg(test)]
mod image_persistence_tests {
    use super::*;
    use crate::types::{AgentMessage, ContentBlock};

    fn write_png(tag: &str) -> std::path::PathBuf {
        let img = image::RgbImage::from_fn(8, 8, |_, _| image::Rgb([1u8, 2, 3]));
        let p = std::env::temp_dir().join(format!(
            "futureos-sess-img-{}-{}.png",
            std::process::id(),
            tag
        ));
        img.save(&p).unwrap();
        p
    }

    fn user_msg_with_image_meta() -> AgentMessage {
        let mut meta = serde_json::Map::new();
        meta.insert(
            "attachments".to_string(),
            serde_json::json!([{"path": "/x.png", "kind": "image", "name": "x.png"}]),
        );
        AgentMessage {
            role: "user".to_string(),
            content: vec![
                ContentBlock::text("hi"),
                ContentBlock::image("data:image/png;base64,AAAA"),
            ],
            name: String::new(),
            tool_args: String::new(),
            metadata: Some(meta),
        }
    }

    #[test]
    fn base64_image_is_stripped_from_jsonl_when_backed_by_meta() {
        let entry = agent_message_to_entry(&user_msg_with_image_meta());
        let arr = entry.content.unwrap();
        let arr = arr.as_array().unwrap();
        // The base64 image_url block is gone; only the text block persists.
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "text");
    }

    #[test]
    fn image_is_rehydrated_from_meta_path_on_reload() {
        let png = write_png("rehydrate");
        // A reloaded user entry: text-only content (image stripped), meta points
        // at the on-disk image.
        let mut entry =
            SessionEntry::new_user("user", serde_json::json!([{"type": "text", "text": "hi"}]));
        entry.meta = Some(serde_json::json!({
            "attachments": [{"path": png.to_string_lossy(), "kind": "image", "name": "x.png"}]
        }));

        let has_image = |msgs: &[AgentMessage]| {
            msgs[0]
                .content
                .iter()
                .any(|b| matches!(b, ContentBlock::Image { .. }))
        };

        // Image-capable model → rebuilt from the path.
        assert!(has_image(&entries_to_agent_messages(
            std::slice::from_ref(&entry),
            true
        )));
        // Text-only model → not rebuilt.
        assert!(!has_image(&entries_to_agent_messages(&[entry], false)));

        std::fs::remove_file(&png).ok();
    }

    #[test]
    fn legacy_image_url_without_meta_is_preserved() {
        // A channels/TUI message (base64 image_url in content, no meta) keeps its
        // image on both save and reload.
        let msg = AgentMessage {
            role: "user".to_string(),
            content: vec![
                ContentBlock::text("hi"),
                ContentBlock::image("data:image/png;base64,ZZZZ"),
            ],
            name: String::new(),
            tool_args: String::new(),
            metadata: None,
        };
        let entry = agent_message_to_entry(&msg);
        // Not stripped on save.
        let arr = entry.content.clone().unwrap();
        assert_eq!(arr.as_array().unwrap().len(), 2);
        // Preserved on reload (no re-read needed; base64 is on disk).
        let msgs = entries_to_agent_messages(&[entry], true);
        assert!(msgs[0]
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::Image { .. })));
    }
}
