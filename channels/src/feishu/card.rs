//! Interactive Card builder for Feishu channel bridge.
//!
//! Two card formats are supported:
//! - Message API format (legacy): no "schema" field, "elements" at root
//! - CardKit format (schema 2.0): "schema":"2.0", "body":{"elements":[...]}
//!
//! Card builders produce Message API format by default. Use `to_cardkit_format()`
//! to convert to CardKit format for the CardKit API (/cardkit/v1/cards).

use serde_json::{json, Value};

/// Build a "thinking" indicator card.
pub fn thinking_card() -> Value {
    json!({
        "config": {
            "update_multi": true
        },
        "header": {
            "title": {"tag": "plain_text", "content": "Thinking..."},
            "template": "indigo"
        },
        "elements": [
            {
                "tag": "div",
                "text": {"tag": "lark_md", "content": "The agent is processing your request..."}
            }
        ]
    })
}

/// Build a streaming card that supports real-time text updates.
/// Returns the card JSON and the element_id used for text updates.
/// After sending this card, use the message_id from the response as card_id
/// with the Card Kit API to update the element content.
pub fn streaming_card(header_text: &str) -> (Value, String) {
    let element_id = "stream_out";
    let card = json!({
        "config": {
            "update_multi": true,
            "streaming_mode": true,
            "enable_forward": true
        },
        "header": {
            "title": {"tag": "plain_text", "content": header_text},
            "template": "blue"
        },
        "elements": [
            {
                "tag": "markdown",
                "element_id": element_id,
                "content": ""
            }
        ]
    });
    (card, element_id.to_string())
}

/// Build a complete (non-streaming) response card with markdown content.
/// Sets config.summary to the first ~120 chars of plain-text content
/// so the Feishu message list shows a preview instead of "thinking...".
pub fn complete_card(header_text: &str, content: &str) -> Value {
    let summary_text = strip_markdown(content);
    let summary = if summary_text.is_empty() {
        None
    } else {
        Some(serde_json::json!({
            "content": truncate_at_char(&summary_text, 120)
        }))
    };
    let mut config = serde_json::json!({
        "update_multi": true,
        "streaming_mode": false,
        "enable_forward": true
    });
    if let Some(ref s) = summary {
        config["summary"] = s.clone();
    }
    json!({
        "config": config,
        "header": {
            "title": {"tag": "plain_text", "content": header_text},
            "template": "blue"
        },
        "elements": [
            {
                "tag": "markdown",
                "content": truncate_markdown(content, 30000)
            }
        ]
    })
}

/// Build an error notification card.
pub fn error_card(error: &str) -> Value {
    json!({
        "config": {
            "update_multi": false
        },
        "header": {
            "title": {"tag": "plain_text", "content": "Error"},
            "template": "red"
        },
        "elements": [
            {
                "tag": "div",
                "text": {"tag": "lark_md", "content": error}
            }
        ]
    })
}

/// Build a tool execution status card.
pub fn tool_card(tool_name: &str, args: &str) -> Value {
    let short_args = if args.len() > 200 {
        format!("{}...", args.chars().take(200).collect::<String>())
    } else {
        args.to_string()
    };
    json!({
        "config": {
            "update_multi": true
        },
        "header": {
            "title": {"tag": "plain_text", "content": format!("Running: {}", tool_name)},
            "template": "wathet"
        },
        "elements": [
            {
                "tag": "div",
                "text": {"tag": "lark_md", "content": short_args}
            }
        ]
    })
}

/// Build a status card (for /status command).
pub fn status_card(
    model: &str, image_support: bool, thinking: &str,
    context_tokens: i64, context_window: i64,
    tokens_in: i64, tokens_out: i64, msg_count: usize,
) -> Value {
    let image_icon = if image_support { "🖼️" } else { "" };
    let context_pct = if context_window > 0 {
        format!(" ({:.0}%)", (context_tokens as f64 / context_window as f64) * 100.0)
    } else {
        String::new()
    };
    json!({
        "config": {
            "update_multi": false
        },
        "header": {
            "title": {"tag": "plain_text", "content": "Session Status"},
            "template": "green"
        },
        "elements": [
            {"tag": "div", "text": {"tag": "lark_md", "content": format!("**Model:** {} {}", model, image_icon)}},
            {"tag": "div", "text": {"tag": "lark_md", "content": format!("**Thinking:** {}", thinking)}},
            {"tag": "div", "text": {"tag": "lark_md", "content": format!("**Context:** {} / {}{}", context_tokens, context_window, context_pct)}},
            {"tag": "div", "text": {"tag": "lark_md", "content": format!("**Tokens:** {} in / {} out", tokens_in, tokens_out)}},
            {"tag": "div", "text": {"tag": "lark_md", "content": format!("**Messages:** {}", msg_count)}},
        ]
    })
}

/// Build help card for slash commands.
pub fn help_card() -> Value {
    json!({
        "config": { "update_multi": false },
        "header": {
            "title": {"tag": "plain_text", "content": "Slash Commands"},
            "template": "blue"
        },
        "elements": [
            {"tag": "div", "text": {"tag": "lark_md", "content": "/new — Start a new session"}},
            {"tag": "div", "text": {"tag": "lark_md", "content": "/status — Show current session info"}},
            {"tag": "div", "text": {"tag": "lark_md", "content": "/model <provider/model> — Switch model"}},
            {"tag": "div", "text": {"tag": "lark_md", "content": "/models — List available models"}},
            {"tag": "div", "text": {"tag": "lark_md", "content": "/effort <level> — Set thinking level (off/minimal/low/medium/high/xhigh)"}},
            {"tag": "div", "text": {"tag": "lark_md", "content": "/stop — Stop current generation"}},
            {"tag": "div", "text": {"tag": "lark_md", "content": "/help — Show this help"}},
        ]
    })
}

/// Build an approval card with Approve / Reject buttons.
/// Uses CardKit actions so button clicks are delivered as card.action.trigger events.
pub fn approval_card(
    approval_request_id: &str,
    tool_name: &str,
    risk_level: &str,
    title: &str,
    summary: &str,
    requested_action: &str,
) -> Value {
    let risk_emoji = match risk_level {
        "high" => "🔴",
        "medium" => "🟡",
        _ => "⚪",
    };
    let body_text = format!(
        "**{}** {}\n\n**Tool:** `{}`\n**Risk:** {}\n\n{}",
        risk_emoji, title, tool_name, risk_level, summary
    );
    let mut elements: Vec<Value> = vec![
        json!({"tag": "markdown", "content": body_text}),
    ];
    if !requested_action.is_empty() {
        let preview = if requested_action.len() > 500 {
            format!("{}\n..._(truncated)_", &requested_action[..500])
        } else {
            requested_action.to_string()
        };
        elements.push(json!({
            "tag": "markdown",
            "content": format!("```\n{}\n```", preview)
        }));
    }

    json!({
        "config": { "update_multi": false },
        "header": {
            "title": {"tag": "plain_text", "content": format!("{} Approval Required", risk_emoji)},
            "template": "yellow"
        },
        "elements": elements,
        "actions": [
            {
                "tag": "button",
                "text": {"tag": "plain_text", "content": "✅ Approve"},
                "type": "primary",
                "value": {
                    "action": "approve",
                    "approval_request_id": approval_request_id
                }
            },
            {
                "tag": "button",
                "text": {"tag": "plain_text", "content": "❌ Reject"},
                "type": "danger",
                "value": {
                    "action": "reject",
                    "approval_request_id": approval_request_id
                }
            }
        ]
    })
}

/// Build the card "content" field for sending as interactive message.
pub fn card_content(card: &Value) -> String {
    serde_json::to_string(card).unwrap_or_else(|_| "{}".into())
}

/// Convert a Message API format card to CardKit schema 2.0 format.
///
/// Message API: {"config":{...}, "header":{...}, "elements":[...]}
/// CardKit:     {"schema":"2.0", "config":{...}, "header":{...}, "body":{"elements":[...]}}
pub fn to_cardkit_format(card: &Value) -> Value {
    let mut ck = serde_json::Map::new();
    ck.insert("schema".to_string(), json!("2.0"));
    if let Some(config) = card.get("config") {
        ck.insert("config".to_string(), config.clone());
    }
    if let Some(header) = card.get("header") {
        ck.insert("header".to_string(), header.clone());
    }
    let elements = card.get("elements").cloned().unwrap_or_else(|| json!([]));
    let mut body = serde_json::Map::new();
    body.insert("elements".to_string(), elements);
    // Carry over actions (buttons) if present in card
    if let Some(actions) = card.get("actions") {
        body.insert("actions".to_string(), actions.clone());
    }
    ck.insert("body".to_string(), json!(body));
    // Carry over any card_link if present
    if let Some(link) = card.get("card_link") {
        ck.insert("card_link".to_string(), link.clone());
    }
    json!(ck)
}

/// Feishu markdown has a practical limit. Truncate at max_len.
fn truncate_markdown(content: &str, max_len: usize) -> String {
    if content.len() <= max_len {
        return content.to_string();
    }
    let truncated = &content[..max_len];
    format!("{}\n\n..._(truncated)_", truncated)
}

/// Truncate at character boundary (safe for multi-byte UTF-8).
fn truncate_at_char(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// Strip common markdown syntax to produce plain text for the card summary.
fn strip_markdown(text: &str) -> String {
    // Remove fenced code blocks, our custom status lines, and separators
    let mut result = String::new();
    let mut in_code_block = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }
        // Skip our custom status/separator lines
        if trimmed.starts_with("💭") || trimmed.starts_with("🔧") || trimmed.starts_with("✅") {
            continue;
        }
        if trimmed == "---" {
            continue;
        }
        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(line);
    }

    // Remove common inline markdown syntax
    result = result.replace("**", "");       // bold
    result = result.replace("*", "");        // italic
    result = result.replace("__", "");       // bold alt
    result = result.replace("_", "");        // italic alt
    result = result.replace("`", "");        // inline code
    result = result.replace("~~", "");       // strikethrough
    result = result.replace("###", "");      // h3
    result = result.replace("##", "");       // h2
    result = result.replace("#", "");        // h1
    result = result.replace("> ", "");       // blockquote

    // Remove links: [text](url) → text
    while let Some(start) = result.find('[') {
        if let Some(mid) = result[start..].find("](") {
            let mid = start + mid;
            if let Some(end) = result[mid..].find(')') {
                let end = mid + end;
                let link_text = result[start + 1..mid].to_string();
                result = format!("{}{}{}", &result[..start], link_text, &result[end + 1..]);
                continue;
            }
        }
        break;
    }

    // Collapse whitespace
    let words: Vec<&str> = result.split_whitespace().collect();
    words.join(" ")
}
