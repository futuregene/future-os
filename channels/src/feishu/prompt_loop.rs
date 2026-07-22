//! The prompt → stream → respond loop for a single user message.
//!
//! Split out of `bridge.rs`: this is the hot path that streams agent events
//! into a CardKit card (250ms-throttled element updates), handles tool-call
//! markers, approval cards, and supersede detection via generation counters.

use super::card;
use super::feishu_rest::FeishuRestClient;
use crate::grpc_client::{AgentClient, AgentEvent, ImageData, ImageInput};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Run the prompt → stream → respond loop.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_prompt_loop(
    feishu: &FeishuRestClient,
    agent: &Arc<RwLock<AgentClient>>,
    session_id: &str,
    feishu_msg_id: &str,
    text: &str,
    images: &[ImageInput],
    streaming: bool,
    prompt_lock: &tokio::sync::Mutex<()>,
    gen_counter: &AtomicU64,
    ack_reaction_id: Option<String>,
) -> Result<()> {
    // Hold the per-chat lock only for abort+prompt, not during streaming.
    // This lets a new message interrupt an ongoing stream.
    let my_gen = {
        let _guard = prompt_lock.lock().await;

        // Abort current generation, then send the new prompt
        {
            let mut client = agent.write().await;
            let _ = client.abort(session_id).await;
            let mut prompt_text = text.to_string();
            for img in images {
                match &img.data {
                    ImageData::Base64(_) => {
                        if let Some(ref fp) = img.file_path {
                            prompt_text.push_str(&format!("\n[File saved: {}]", fp));
                        } else {
                            prompt_text.push_str("\n[Image attached]");
                        }
                    }
                    ImageData::Url(_) => prompt_text.push_str("\n[Image URL attached]"),
                }
            }
            info!(
                "[SEND] session={} text=\"{}\"",
                session_id,
                if prompt_text.len() > 300 {
                    format!("{}...", truncate_at_char(&prompt_text, 300))
                } else {
                    prompt_text.clone()
                }
            );
            client
                .prompt(session_id, &prompt_text, images.to_vec())
                .await?;
        }

        // Bump generation — we're now the latest active stream
        gen_counter.fetch_add(1, Ordering::SeqCst) + 1
    };
    // Lock released here — streaming happens concurrently with other prompts

    let mut stream = {
        let mut client = agent.write().await;
        client.stream_events(session_id).await?
    };

    let mut stream_text = String::new();
    let mut last_was_content = false;
    // Track per-tool running text for precise ToolEnd replacement (keyed by tool_id).
    // Format in stream_text: "<!--tid:{tool_id}-->🔧 **Running tool:** `{name}`..."
    let mut tool_running: HashMap<String, String> = HashMap::new();
    let streaming_element_id = "stream_out";

    // CardKit state: create card entity, then reply to user with card reference
    let mut cardkit_card_id: Option<String> = None;
    let mut card_seq: u64 = 0;

    // Eagerly create CardKit card + reply as soon as streaming starts
    let mut card_ready = false;

    // Throttle: batch TextChunks to reduce HTTP round-trips (default 150ms)
    let flush_interval = std::time::Duration::from_millis(250);
    let mut last_flush = Instant::now();
    let mut needs_flush = false;

    /// Check if we've been superseded by a newer prompt. If so, stop silently
    /// — the newer stream owns the response.
    macro_rules! check_superseded {
        () => {
            if gen_counter.load(Ordering::SeqCst) != my_gen {
                info!("[STREAM] gen={} superseded, stopping", my_gen);
                return Ok(());
            }
        };
    }

    while let Some(event) = stream.message().await? {
        check_superseded!();

        let parsed = AgentClient::parse_event(event);

        match parsed {
            Some(AgentEvent::AgentStart) | Some(AgentEvent::Ping) => {}
            Some(AgentEvent::ThinkingStart) => {
                last_was_content = false;
                if !stream_text.is_empty() {
                    stream_text.push_str("\n\n");
                }
                stream_text.push_str("💭 **Thinking...**\n\n");
                needs_flush = true;
            }
            Some(AgentEvent::ThinkingDelta(text)) => {
                last_was_content = false;
                stream_text.push_str(&text);
                // Create card on first visible content
                if !card_ready && !stream_text.trim().is_empty() {
                    card_ready = true;
                    let (stream_card, _) = card::streaming_card("");
                    let ck_card = card::to_cardkit_format(&stream_card);
                    match feishu.create_cardkit_card(&ck_card).await {
                        Ok(cid) => {
                            cardkit_card_id = Some(cid.clone());
                            match feishu.reply_with_card_id(feishu_msg_id, &cid).await {
                                Ok(resp) => info!(
                                    "[CARD] reply_with_card_id card_id={} msg_id={}",
                                    cid, resp.message_id
                                ),
                                Err(e) => warn!("[CARD] reply_with_card_id failed: {}", e),
                            }
                        }
                        Err(e) => warn!("[CARD] create_cardkit_card failed: {}", e),
                    }
                }
                needs_flush = true;
                if let Some(ref cid) = cardkit_card_id {
                    let thinking_flush = std::time::Duration::from_millis(100);
                    if last_flush.elapsed() >= thinking_flush {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::ThinkingEnd) => {
                if needs_flush {
                    if let Some(ref cid) = cardkit_card_id {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::TextChunk(chunk)) => {
                if !last_was_content && !stream_text.is_empty() {
                    stream_text.push_str("\n\n---\n\n");
                }
                last_was_content = true;
                stream_text.push_str(&chunk);

                if streaming && !card_ready && !stream_text.trim().is_empty() {
                    card_ready = true;
                    let (stream_card, _) = card::streaming_card("");
                    let ck_card = card::to_cardkit_format(&stream_card);
                    match feishu.create_cardkit_card(&ck_card).await {
                        Ok(cid) => {
                            cardkit_card_id = Some(cid.clone());
                            match feishu.reply_with_card_id(feishu_msg_id, &cid).await {
                                Ok(resp) => info!(
                                    "[CARD] reply_with_card_id card_id={} msg_id={}",
                                    cid, resp.message_id
                                ),
                                Err(e) => warn!("[CARD] reply_with_card_id failed: {}", e),
                            }
                        }
                        Err(e) => warn!("[CARD] create_cardkit_card failed: {}", e),
                    }
                }

                needs_flush = true;
                if let Some(ref cid) = cardkit_card_id {
                    if last_flush.elapsed() >= flush_interval {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::ToolStart {
                tool_id,
                tool_name,
                tool_args,
                ..
            }) => {
                last_was_content = false;
                let args_preview = tool_args.as_deref().unwrap_or("");
                let args_display = if !args_preview.is_empty() {
                    let truncated = if args_preview.len() > 200 {
                        format!("{}...", truncate_at_char(args_preview, 200))
                    } else {
                        args_preview.to_string()
                    };
                    format!("\n```\n{}\n```", truncated)
                } else {
                    String::new()
                };
                let marker = format!("<!--tid:{}-->", tool_id);
                let running_text = format!(
                    "\n\n{}🔧 **Running tool:** `{}`{}",
                    marker, tool_name, args_display
                );
                tool_running.insert(tool_id.clone(), running_text.clone());
                stream_text.push_str(&running_text);
                needs_flush = true;
                if let Some(ref cid) = cardkit_card_id {
                    if last_flush.elapsed() >= flush_interval {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::ToolEnd {
                tool_id,
                text: result,
            }) => {
                last_was_content = false;
                if let Some(ref cid) = cardkit_card_id {
                    let (tool_name, old_entry) = {
                        let entry = tool_running.remove(&tool_id);
                        let name = entry
                            .as_ref()
                            .and_then(|s| s.split("**Running tool:** `").nth(1))
                            .and_then(|s| s.split('`').next())
                            .unwrap_or(&tool_id)
                            .to_string();
                        (name, entry)
                    };
                    let result_preview = result.as_deref().unwrap_or("");
                    let result_display = if !result_preview.is_empty() {
                        let truncated = if result_preview.len() > 500 {
                            format!("{}...", truncate_at_char(result_preview, 500))
                        } else {
                            result_preview.to_string()
                        };
                        format!("\n```\n{}\n```", truncated)
                    } else {
                        String::new()
                    };
                    let new_entry = format!(
                        "\n\n✅ **Tool** `{}` **completed**{}",
                        tool_name, result_display
                    );
                    if let Some(ref old) = old_entry {
                        stream_text = stream_text.replace(old, &new_entry);
                    }
                    needs_flush = true;
                    if last_flush.elapsed() >= flush_interval {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::ToolDelta { .. }) => {}
            Some(AgentEvent::ApprovalRequest {
                approval_request_id,
                tool_name,
                risk_level,
                title,
                summary,
                requested_action,
                ..
            }) => {
                // Flush any pending text before sending approval card
                if needs_flush {
                    if let Some(ref cid) = cardkit_card_id {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                        needs_flush = false;
                        last_flush = Instant::now();
                    }
                }
                // Finalize current streaming card
                if let Some(ref cid) = cardkit_card_id {
                    let _ = feishu.set_card_streaming_mode(cid, false, card_seq).await;
                    let complete_card = card::complete_card("", &stream_text);
                    let ck_complete = card::to_cardkit_format(&complete_card);
                    card_seq += 1;
                    let _ = feishu
                        .update_cardkit_card(cid, &ck_complete, card_seq)
                        .await;
                    cardkit_card_id = None;
                }
                // Send approval card with Approve/Reject buttons
                let action_preview = if let serde_json::Value::String(s) = &requested_action {
                    s.clone()
                } else {
                    serde_json::to_string_pretty(&requested_action).unwrap_or_default()
                };
                let approval = card::approval_card(
                    &approval_request_id,
                    &tool_name,
                    &risk_level,
                    &title,
                    &summary,
                    &action_preview,
                );
                let ck_card = card::to_cardkit_format(&approval);
                match feishu.create_cardkit_card(&ck_card).await {
                    Ok(cid) => {
                        info!(
                            "[APPROVAL] card created cid={} request_id={}",
                            cid, approval_request_id
                        );
                        card_seq += 1;
                        let _ = feishu.reply_with_card_id(feishu_msg_id, &cid).await;
                    }
                    Err(e) => {
                        warn!("[APPROVAL] create_cardkit_card failed: {}", e);
                        // Fallback: text message
                        let fallback = format!(
                            "⚠️ Approval: {}\nTool: {}\nUse `/approve {}` or `/reject {}` in TUI",
                            title, tool_name, approval_request_id, approval_request_id
                        );
                        feishu
                            .reply_message(
                                feishu_msg_id,
                                "text",
                                &serde_json::json!({"text": fallback}).to_string(),
                            )
                            .await?;
                    }
                }
            }
            Some(AgentEvent::AgentEnd { error }) => {
                check_superseded!();
                // Flush any pending text before finalizing
                if needs_flush {
                    if let Some(ref cid) = cardkit_card_id {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                    }
                }
                // Swap reactions: remove "Typing", add "DONE"
                if let Some(ref rid) = ack_reaction_id {
                    let _ = feishu.remove_reaction(feishu_msg_id, rid).await;
                }
                let _ = feishu.react_to_message(feishu_msg_id, "DONE").await;

                if let Some(err) = error {
                    // "interrupted" is expected when a newer message aborts this
                    // stream — don't show an error card, just let the new stream win.
                    if err.contains("interrupted") || err.contains("Interrupted") {
                        info!("[REPLY] interrupted by newer message, stopping silently");
                    } else {
                        info!("[REPLY] error=\"{}\"", err);
                        let err_card = card::error_card(&err);
                        feishu
                            .reply_message(
                                feishu_msg_id,
                                "interactive",
                                &card::card_content(&err_card),
                            )
                            .await?;
                    }
                } else if !stream_text.trim().is_empty() {
                    info!("[REPLY] text_len={}", stream_text.len());
                    if let Some(ref cid) = cardkit_card_id {
                        card_seq += 1;
                        if let Err(e) = feishu.set_card_streaming_mode(cid, false, card_seq).await {
                            warn!("[CARD] set_card_streaming_mode failed: {}", e);
                        }
                        let complete_card = card::complete_card("", &stream_text);
                        let ck_complete = card::to_cardkit_format(&complete_card);
                        card_seq += 1;
                        if let Err(e) = feishu
                            .update_cardkit_card(cid, &ck_complete, card_seq)
                            .await
                        {
                            warn!("[CARD] update_cardkit_card failed: {}", e);
                        }
                    } else {
                        let card = card::complete_card("", &stream_text);
                        feishu
                            .reply_message(feishu_msg_id, "interactive", &card::card_content(&card))
                            .await?;
                    }
                }
                break;
            }
            Some(AgentEvent::Error(err)) => {
                check_superseded!();
                if let Some(ref rid) = ack_reaction_id {
                    let _ = feishu.remove_reaction(feishu_msg_id, rid).await;
                }
                let _ = feishu.react_to_message(feishu_msg_id, "DONE").await;
                info!("[REPLY] error=\"{}\"", err);
                let err_card = card::error_card(&err);
                feishu
                    .reply_message(feishu_msg_id, "interactive", &card::card_content(&err_card))
                    .await?;
                break;
            }
            None => {}
        }
    }

    Ok(())
}

pub(super) fn truncate_at_char(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── truncate_at_char ────────────────────────────────────────────────────

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate_at_char("hello", 10), "hello");
    }

    #[test]
    fn truncate_exact_length_unchanged() {
        assert_eq!(truncate_at_char("hello", 5), "hello");
    }

    #[test]
    fn truncate_long_string() {
        assert_eq!(truncate_at_char("hello world", 5), "hello");
    }

    #[test]
    fn truncate_empty_string() {
        assert_eq!(truncate_at_char("", 10), "");
    }

    #[test]
    fn truncate_zero_limit() {
        assert_eq!(truncate_at_char("hello", 0), "");
    }

    #[test]
    fn truncate_utf8_emoji_safe() {
        let s = "🦀🦀🦀🦀🦀";
        assert_eq!(truncate_at_char(s, 3), "🦀🦀🦀");
    }

    #[test]
    fn truncate_utf8_cjk_safe() {
        let s = "你好世界你好世界";
        assert_eq!(truncate_at_char(s, 4), "你好世界");
    }

    #[test]
    fn truncate_mixed_ascii_unicode() {
        let s = "ab你好cd";
        assert_eq!(truncate_at_char(s, 4), "ab你好");
    }

    // ─── Stream text accumulation patterns ───────────────────────────────────

    #[test]
    fn stream_text_separator_between_thinking_and_content() {
        // Simulates the pattern used in run_prompt_loop:
        // thinking → "---" separator → content
        let mut stream_text = String::new();
        stream_text.push_str("💭 **Thinking...**\n\nSome thinking here");
        let last_was_content = false;

        // Simulate TextChunk after thinking
        if !last_was_content && !stream_text.is_empty() {
            stream_text.push_str("\n\n---\n\n");
        }
        stream_text.push_str("Actual answer");

        assert!(stream_text.contains("💭 **Thinking...**"));
        assert!(stream_text.contains("\n\n---\n\n"));
        assert!(stream_text.ends_with("Actual answer"));
    }

    #[test]
    fn stream_text_tool_marker_format() {
        // Simulates the tool_running marker format
        let tool_id = "call_abc123";
        let tool_name = "shell";
        let args_preview = "ls -la";

        let marker = format!("<!--tid:{}-->", tool_id);
        let running_text = format!(
            "\n\n{}🔧 **Running tool:** `{}`\n```\n{}\n```",
            marker, tool_name, args_preview
        );

        assert!(running_text.contains("<!--tid:call_abc123-->"));
        assert!(running_text.contains("🔧 **Running tool:** `shell`"));
        assert!(running_text.contains("```\nls -la\n```"));
    }

    #[test]
    fn stream_text_tool_completion_replaces_running() {
        // Simulates the ToolEnd replacement logic
        let tool_id = "call_abc123";
        let tool_name = "shell";
        let marker = format!("<!--tid:{}-->", tool_id);
        let old_entry = format!("\n\n{}🔧 **Running tool:** `{}`", marker, tool_name);

        let mut stream_text = String::from("Some text");
        stream_text.push_str(&old_entry);

        let result_preview = "file1.txt\nfile2.txt";
        let result_display = format!("\n```\n{}\n```", result_preview);
        let new_entry = format!("\n\n✅ **Tool** `{}` **completed**{}", tool_name, result_display);

        stream_text = stream_text.replace(&old_entry, &new_entry);

        assert!(!stream_text.contains("<!--tid:"));
        assert!(stream_text.contains("✅ **Tool** `shell` **completed**"));
        assert!(stream_text.contains("file1.txt"));
    }
}
