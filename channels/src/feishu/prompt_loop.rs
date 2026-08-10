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
use std::time::{Duration, Instant};
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
    // Hold the per-chat lock through atomic supersede + start + attach,
    // but not while consuming the stream. This closes the attach race while
    // still letting a newer message interrupt an ongoing response.
    let (expected_run_id, my_gen, mut stream) = {
        let _guard = prompt_lock.lock().await;

        // Agent performs active abort + queued replacement atomically.
        let mut client = agent.read().await.clone();
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
        // Hoisted out of the info! args (macro args only evaluate with a
        // subscriber; this runs regardless).
        let send_preview = if prompt_text.len() > 300 {
            format!("{}...", truncate_at_char(&prompt_text, 300))
        } else {
            prompt_text.clone()
        };
        info!("[SEND] session={} text=\"{}\"", session_id, send_preview);
        let expected_run_id = client
            .prompt_superseding(session_id, &prompt_text, images.to_vec())
            .await?;
        client
            .wait_until_run_active(session_id, &expected_run_id, Duration::from_secs(30))
            .await?;
        // Attach before releasing the per-chat lock, so a newer message cannot
        // roll this run over between its acknowledgement and stream ownership.
        let stream = client
            .stream_run_events(session_id, &expected_run_id)
            .await?;

        // Bump generation — we're now the latest active stream
        let my_gen = gen_counter.fetch_add(1, Ordering::SeqCst) + 1;
        (expected_run_id, my_gen, stream)
    };
    // Lock released here — streaming happens concurrently with other prompts

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

        let parsed = match AgentClient::parse_event(event) {
            // Only consume events for the run we prompted. A different run on the
            // same session (another client, or a stale tail after supersede) is
            // dropped so its agent_end can't finalize this card early.
            Some((rid, ev)) if rid.is_empty() || rid == expected_run_id => Some(ev),
            Some(_) => continue,
            None => None,
        };

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
            Some(AgentEvent::AgentEnd { error, state }) => {
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

                let was_cancelled = state.as_deref() == Some("cancelled");
                if let Some(err) = error {
                    // "interrupted"/cancelled is expected when a newer message
                    // aborts this stream — don't show an error card, just let the
                    // new stream win.
                    if was_cancelled || err.contains("interrupted") || err.contains("Interrupted") {
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
    // MockState scaffolding mutates Default::default() instances per-test by
    // design; field-reassign is the readable form for a 15-field mock.
    #![allow(clippy::field_reassign_with_default)]
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
        // thinking → "---" separator → content. Shared fn so both edges of
        // the condition execute (false edge: already-content case).
        fn maybe_separator(s: &mut String, last_was_content: bool) {
            if !last_was_content && !s.is_empty() {
                s.push_str("\n\n---\n\n");
            }
        }
        let mut stream_text = String::from("💭 **Thinking...**\n\nSome thinking here");
        maybe_separator(&mut stream_text, false);
        stream_text.push_str("Actual answer");

        assert!(stream_text.contains("💭 **Thinking...**"));
        assert!(stream_text.contains("\n\n---\n\n"));
        assert!(stream_text.ends_with("Actual answer"));

        let mut already_content = String::from("some answer");
        maybe_separator(&mut already_content, true);
        assert_eq!(already_content, "some answer");
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
        let new_entry = format!(
            "\n\n✅ **Tool** `{}` **completed**{}",
            tool_name, result_display
        );

        stream_text = stream_text.replace(&old_entry, &new_entry);

        assert!(!stream_text.contains("<!--tid:"));
        assert!(stream_text.contains("✅ **Tool** `shell` **completed**"));
        assert!(stream_text.contains("file1.txt"));
    }

    // ─── run_prompt_loop against mock gRPC + mock Feishu REST ────────────────

    use crate::test_support::{self as ts, HttpRoute, MockState};
    use future_rpc::proto::StreamEvent;

    const TOKEN_ROUTE: &str = "/auth/v3/tenant_access_token/internal";

    fn feishu_routes() -> Vec<HttpRoute> {
        vec![
            HttpRoute::json(
                TOKEN_ROUTE,
                200,
                r#"{"code":0,"tenant_access_token":"tok","expire":7200}"#,
            ),
            HttpRoute::json(
                "/cardkit/v1/cards",
                200,
                r#"{"code":0,"data":{"card_id":"card_1"}}"#,
            ),
            HttpRoute::json(
                "/im/v1/messages/om_user/reply",
                200,
                r#"{"code":0,"data":{"message_id":"om_reply"}}"#,
            ),
            HttpRoute::json(
                "/cardkit/v1/cards/card_1/elements/stream_out/content",
                200,
                r#"{"code":0}"#,
            ),
            HttpRoute::json("/cardkit/v1/cards/card_1", 200, r#"{"code":0}"#),
            HttpRoute::json("/cardkit/v1/cards/card_1/settings", 200, ""),
            HttpRoute::json(
                "/im/v1/messages/om_user/reactions",
                200,
                r#"{"code":0,"data":{"reaction_id":"rid_1"}}"#,
            ),
            HttpRoute::json(
                "/im/v1/messages/om_user/reactions/rid_1",
                200,
                r#"{"code":0}"#,
            ),
        ]
    }

    struct LoopEnv {
        feishu: FeishuRestClient,
        agent: Arc<RwLock<AgentClient>>,
        http: ts::RecordedRequests,
    }

    async fn setup_with(state: MockState, routes: Vec<HttpRoute>) -> (LoopEnv, ts::SharedState) {
        ts::ensure_crypto_provider();
        let (addr, grpc) = ts::spawn_mock_grpc(state).await;
        let agent = AgentClient::connect(&addr).await.expect("connect");
        let (base, http) = ts::spawn_http(routes).await;
        let feishu = FeishuRestClient::new(&base, "app", "secret");
        (
            LoopEnv {
                feishu,
                agent: Arc::new(RwLock::new(agent)),
                http,
            },
            grpc,
        )
    }

    async fn setup(events: Vec<StreamEvent>) -> (LoopEnv, ts::SharedState) {
        let mut state = MockState::default();
        state.events = events;
        setup_with(state, feishu_routes()).await
    }

    /// Run the loop with standard args; returns when the stream closes.
    async fn drive(env: &LoopEnv, streaming: bool, ack: Option<String>) -> Result<()> {
        let lock = tokio::sync::Mutex::new(());
        let gen = AtomicU64::new(0);
        run_prompt_loop(
            &env.feishu,
            &env.agent,
            "sess",
            "om_user",
            "hello",
            &[],
            streaming,
            &lock,
            &gen,
            ack,
        )
        .await
    }

    fn bodies(recorded: &ts::RecordedRequests, path: &str) -> Vec<String> {
        ts::requests_to(recorded, path)
            .iter()
            .map(|r| r.body_string())
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn streaming_text_flow_finalizes_card() {
        let events = vec![
            ts::ev("", 0, "agent_start", "{}"),
            ts::ev("", 1, "ping", ""),
            ts::ev("", 2, "text_chunk", r#"{"text":"hello"}"#),
            ts::ev("", 3, "text_chunk", r#"{"text":" world"}"#),
            ts::ev("", 4, "agent_end", r#"{"state":"completed"}"#),
        ];
        let (env, _) = setup(events).await;
        drive(&env, true, Some("rid_1".into())).await.unwrap();

        // Card created + replied; finalized via settings-false then full update.
        assert_eq!(bodies(&env.http, "/cardkit/v1/cards").len(), 1);
        let replies = bodies(&env.http, "/im/v1/messages/om_user/reply");
        assert!(replies.iter().any(|b| b.contains("card_1")));
        let settings = ts::requests_to(&env.http, "/cardkit/v1/cards/card_1/settings");
        assert_eq!(settings.len(), 1);
        assert!(settings[0].body_string().contains("streaming_mode"));
        let finals = bodies(&env.http, "/cardkit/v1/cards/card_1");
        assert!(finals.iter().any(|b| b.contains("hello world")));
        // ACK reaction swapped Typing → DONE.
        let deletes = ts::requests_to(&env.http, "/im/v1/messages/om_user/reactions/rid_1");
        assert_eq!(deletes.len(), 1);
        let reactions = bodies(&env.http, "/im/v1/messages/om_user/reactions");
        assert!(reactions.iter().any(|b| b.contains("DONE")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn non_streaming_replies_with_complete_card() {
        let events = vec![
            ts::ev("", 0, "text_chunk", r#"{"text":"full answer"}"#),
            ts::ev("", 1, "agent_end", r#"{"state":"completed"}"#),
        ];
        let (env, _) = setup(events).await;
        drive(&env, false, None).await.unwrap();

        // No CardKit card created; the reply is an interactive message.
        assert!(bodies(&env.http, "/cardkit/v1/cards").is_empty());
        let replies = bodies(&env.http, "/im/v1/messages/om_user/reply");
        assert!(replies.iter().any(|b| b.contains("full answer")));
        // DONE reaction even without an ACK reaction to remove.
        let reactions = bodies(&env.http, "/im/v1/messages/om_user/reactions");
        assert!(reactions.iter().any(|b| b.contains("DONE")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn thinking_then_text_and_tool_markers() {
        let events = vec![
            ts::ev("", 0, "thinking_start", "{}"),
            ts::ev("", 1, "thinking_delta", r#"{"text":"pondering"}"#),
            ts::ev("", 2, "thinking_end", "{}"),
            ts::ev(
                "",
                3,
                "tool_start",
                r#"{"tool_id":"t1","tool_name":"shell","tool_args":"ls -la"}"#,
            ),
            ts::ev("", 4, "tool_delta", r#"{"tool_id":"t1","text":"partial"}"#),
            ts::ev("", 5, "tool_end", r#"{"tool_id":"t1","text":"file.txt"}"#),
            // ToolEnd without a matching ToolStart → falls back to tool_id.
            ts::ev("", 6, "tool_end", r#"{"tool_id":"t-orphan","text":"x"}"#),
            ts::ev("", 7, "text_chunk", r#"{"text":"final"}"#),
            // Unmappable event type → parse None arm.
            ts::ev("", 8, "message_ack", "{}"),
            ts::ev("", 9, "agent_end", r#"{"state":"completed"}"#),
        ];
        let (env, _) = setup(events).await;
        drive(&env, true, None).await.unwrap();

        let finals = bodies(&env.http, "/cardkit/v1/cards/card_1");
        let final_card = finals.last().expect("final card update");
        assert!(final_card.contains("Thinking"), "{final_card}");
        assert!(final_card.contains("pondering"));
        // ToolEnd replaced the running marker with the completion entry.
        assert!(final_card.contains("completed"));
        assert!(final_card.contains("file.txt"));
        assert!(final_card.contains("final"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mid_stream_flush_after_throttle_interval() {
        // Slow card creation (300ms) puts the first chunk past the 250ms
        // flush interval → the element update fires mid-stream.
        let mut routes = feishu_routes();
        routes.retain(|r| r.path != "/cardkit/v1/cards");
        routes.push(HttpRoute::slow_json(
            "/cardkit/v1/cards",
            r#"{"code":0,"data":{"card_id":"card_1"}}"#,
            Duration::from_millis(300),
        ));
        let events = vec![
            ts::ev("", 0, "thinking_start", "{}"),
            ts::ev("", 1, "thinking_delta", r#"{"text":"t"}"#),
            ts::ev("", 2, "thinking_end", "{}"),
            ts::ev("", 3, "text_chunk", r#"{"text":"chunk one"}"#),
            ts::ev("", 4, "text_chunk", r#"{"text":"chunk two"}"#),
            ts::ev("", 5, "agent_end", r#"{"state":"completed"}"#),
        ];
        let mut state = MockState::default();
        state.events = events;
        let (env, _) = setup_with(state, routes).await;
        drive(&env, true, None).await.unwrap();
        let updates = bodies(
            &env.http,
            "/cardkit/v1/cards/card_1/elements/stream_out/content",
        );
        assert!(
            updates.len() >= 2,
            "mid-stream throttled updates + final flush expected: {updates:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn approval_flow_finalizes_stream_and_sends_card() {
        let events = vec![
            ts::ev("", 0, "text_chunk", r#"{"text":"working"}"#),
            ts::ev(
                "",
                1,
                "approval_request",
                r#"{"approval_request_id":"req_1","tool_name":"shell","risk_level":"high","title":"Run command","summary":"rm -rf","requested_action":"ls"}"#,
            ),
            ts::ev("", 2, "agent_end", r#"{"state":"completed"}"#),
        ];
        let (env, _) = setup(events).await;
        drive(&env, true, None).await.unwrap();

        // Streaming card finalized before the approval card; approval card
        // created + replied; agent_end (card already finalized → card_id
        // None) replies with the complete card. 2 creates, 3 replies.
        assert_eq!(bodies(&env.http, "/cardkit/v1/cards").len(), 2);
        let settings = ts::requests_to(&env.http, "/cardkit/v1/cards/card_1/settings");
        assert!(!settings.is_empty());
        let replies = bodies(&env.http, "/im/v1/messages/om_user/reply");
        assert_eq!(replies.len(), 3, "stream + approval + complete replies");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn approval_card_failure_falls_back_to_text() {
        // First card create (streaming) succeeds, second (approval) fails.
        let mut routes = feishu_routes();
        routes.retain(|r| r.path != "/cardkit/v1/cards");
        routes.push(HttpRoute::sequence(
            "/cardkit/v1/cards",
            vec![
                (200, r#"{"code":0,"data":{"card_id":"card_1"}}"#),
                (200, r#"{"code":500,"msg":"card quota"}"#),
            ],
        ));
        let events = vec![
            ts::ev("", 0, "text_chunk", r#"{"text":"working"}"#),
            ts::ev(
                "",
                1,
                "approval_request",
                r#"{"approval_request_id":"req_2","tool_name":"shell","risk_level":"low","title":"T","summary":"S","requested_action":{"cmd":"ls"}}"#,
            ),
            ts::ev("", 2, "agent_end", r#"{"state":"completed"}"#),
        ];
        let mut state = MockState::default();
        state.events = events;
        let (env, _) = setup_with(state, routes).await;
        drive(&env, true, None).await.unwrap();
        // Fallback: a text reply mentioning the approval + TUI commands.
        let replies = bodies(&env.http, "/im/v1/messages/om_user/reply");
        assert!(
            replies
                .iter()
                .any(|b| b.contains("Approval") && b.contains("req_2")),
            "fallback text reply expected: {replies:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_end_error_sends_error_card() {
        let events = vec![
            ts::ev("", 0, "text_chunk", r#"{"text":"partial"}"#),
            ts::ev("", 1, "agent_end", r#"{"error":"model exploded"}"#),
        ];
        let (env, _) = setup(events).await;
        drive(&env, true, None).await.unwrap();
        let replies = bodies(&env.http, "/im/v1/messages/om_user/reply");
        assert!(replies.iter().any(|b| b.contains("model exploded")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_end_cancelled_or_interrupted_stays_silent() {
        for data in [
            r#"{"state":"cancelled"}"#,
            r#"{"error":"run interrupted by user"}"#,
            r#"{"error":"Interrupted"}"#,
        ] {
            let events = vec![
                ts::ev("", 0, "text_chunk", r#"{"text":"partial"}"#),
                ts::ev("", 1, "agent_end", data),
            ];
            let (env, _) = setup(events).await;
            drive(&env, true, None).await.unwrap();
            let replies = bodies(&env.http, "/im/v1/messages/om_user/reply");
            assert!(
                !replies
                    .iter()
                    .any(|b| b.contains("error") || b.contains("Error")),
                "no error card for {data}: {replies:?}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn error_event_sends_error_card() {
        let events = vec![ts::ev("", 0, "error", r#"{"error":"transport blew up"}"#)];
        let (env, _) = setup(events).await;
        drive(&env, true, Some("rid_1".into())).await.unwrap();
        let replies = bodies(&env.http, "/im/v1/messages/om_user/reply");
        assert!(replies.iter().any(|b| b.contains("transport blew up")));
        // ACK swapped out even on error.
        let deletes = ts::requests_to(&env.http, "/im/v1/messages/om_user/reactions/rid_1");
        assert_eq!(deletes.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn error_event_without_ack_reaction() {
        // ack_reaction_id None → the removal if-let false path.
        let events = vec![ts::ev("", 0, "error", r#"{"error":"boom"}"#)];
        let (env, _) = setup(events).await;
        drive(&env, true, None).await.unwrap();
        let replies = bodies(&env.http, "/im/v1/messages/om_user/reply");
        assert!(replies.iter().any(|b| b.contains("boom")));
        let deletes = ts::requests_to(&env.http, "/im/v1/messages/om_user/reactions/rid_1");
        assert!(deletes.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn superseded_stream_stops_silently() {
        // Slow card create leaves a window to bump the generation counter.
        let mut routes = feishu_routes();
        routes.retain(|r| r.path != "/cardkit/v1/cards");
        routes.push(HttpRoute::slow_json(
            "/cardkit/v1/cards",
            r#"{"code":0,"data":{"card_id":"card_1"}}"#,
            Duration::from_millis(300),
        ));
        let events = vec![
            ts::ev("", 0, "text_chunk", r#"{"text":"one"}"#),
            ts::ev("", 1, "text_chunk", r#"{"text":"two"}"#),
            ts::ev("", 2, "agent_end", r#"{"state":"completed"}"#),
        ];
        let mut state = MockState::default();
        state.events = events;
        let (env, _) = setup_with(state, routes).await;
        let lock = tokio::sync::Mutex::new(());
        let gen = AtomicU64::new(0);
        // Bump the generation while the first chunk is stuck in card creation.
        let gen_ref = &gen;
        let drive = run_prompt_loop(
            &env.feishu,
            &env.agent,
            "sess",
            "om_user",
            "hello",
            &[],
            true,
            &lock,
            gen_ref,
            None,
        );
        let bump = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            gen_ref.fetch_add(5, Ordering::SeqCst);
        };
        let (r, _) = tokio::join!(drive, bump);
        r.unwrap();
        // Stopped before AgentEnd → no DONE reaction, no finalize.
        // (join instead of .any(): the closure would never run when the
        // request log is empty, leaving an uncovered region.)
        let reactions = bodies(&env.http, "/im/v1/messages/om_user/reactions");
        assert!(!reactions.join("\n").contains("DONE"));
        let settings = ts::requests_to(&env.http, "/cardkit/v1/cards/card_1/settings");
        assert!(settings.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn foreign_run_events_are_dropped() {
        let events = vec![
            ts::ev("other-run", 0, "text_chunk", r#"{"text":"alien"}"#),
            ts::ev("other-run", 1, "agent_end", r#"{"state":"completed"}"#),
        ];
        let (env, _) = setup(events).await;
        drive(&env, true, None).await.unwrap();
        // Nothing streamed: no card, no reply, no DONE.
        assert!(bodies(&env.http, "/cardkit/v1/cards").is_empty());
        let replies = bodies(&env.http, "/im/v1/messages/om_user/reply");
        assert!(!replies.join("\n").contains("alien"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_failure_propagates() {
        let mut state = MockState::default();
        state.fail_commands.insert("prompt".to_string());
        let (env, _) = setup_with(state, feishu_routes()).await;
        let err = drive(&env, true, None).await.unwrap_err();
        assert!(err.to_string().contains("mock failure: prompt"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wait_active_and_attach_failures_propagate() {
        // get_state never reports the run → "cancelled before start".
        let mut state = MockState::default();
        state
            .responses
            .insert("get_state".into(), r#"{"queuedRuns":[]}"#.into());
        let (env, _) = setup_with(state, feishu_routes()).await;
        let err = drive(&env, true, None).await.unwrap_err();
        assert!(err.to_string().contains("cancelled before start"), "{err}");

        // stream_events RPC itself fails.
        let mut state = MockState::default();
        state.stream_status_error = true;
        let (env, _) = setup_with(state, feishu_routes()).await;
        let err = drive(&env, true, None).await.unwrap_err();
        assert!(err.to_string().contains("Failed to attach"), "{err}");

        // Mid-stream transport error.
        let mut state = MockState::default();
        state.events = vec![ts::ev("", 0, "agent_start", "{}")];
        state.stream_mid_error_after = Some(1);
        let (env, _) = setup_with(state, feishu_routes()).await;
        let err = drive(&env, true, None).await.unwrap_err();
        assert!(err.to_string().contains("event stream failed"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn image_inputs_extend_prompt_text() {
        let events = vec![ts::ev("", 0, "agent_end", r#"{"state":"completed"}"#)];
        let (env, grpc) = setup(events).await;
        let images = vec![
            ImageInput {
                content_type: "image_url".into(),
                data: ImageData::Base64("data:image/png;base64,AA==".into()),
                file_path: Some("/tmp/pic.png".into()),
            },
            ImageInput {
                content_type: "image_url".into(),
                data: ImageData::Base64("data:image/png;base64,BB==".into()),
                file_path: None,
            },
            ImageInput {
                content_type: "image_url".into(),
                data: ImageData::Url("https://x/y.png".into()),
                file_path: None,
            },
        ];
        let lock = tokio::sync::Mutex::new(());
        let gen = AtomicU64::new(0);
        run_prompt_loop(
            &env.feishu,
            &env.agent,
            "sess",
            "om_user",
            "look at this",
            &images,
            false,
            &lock,
            &gen,
            None,
        )
        .await
        .unwrap();
        let prompts = ts::recorded_of(&grpc, "prompt");
        let msg = &prompts[0].message;
        assert!(msg.contains("[File saved: /tmp/pic.png]"), "{msg}");
        assert!(msg.contains("[Image attached]"), "{msg}");
        assert!(msg.contains("[Image URL attached]"), "{msg}");
        assert_eq!(prompts[0].images.len(), 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn long_prompt_text_truncated_in_log_only() {
        // current_thread: info! args only evaluate under the thread-local
        // subscriber.
        let _sub = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .finish(),
        );
        let events = vec![ts::ev("", 0, "agent_end", r#"{"state":"completed"}"#)];
        let (env, grpc) = setup(events).await;
        let long_text = "x".repeat(400);
        let lock = tokio::sync::Mutex::new(());
        let gen = AtomicU64::new(0);
        run_prompt_loop(
            &env.feishu,
            &env.agent,
            "sess",
            "om_user",
            &long_text,
            &[],
            false,
            &lock,
            &gen,
            None,
        )
        .await
        .unwrap();
        let prompts = ts::recorded_of(&grpc, "prompt");
        assert_eq!(prompts[0].message.len(), 400, "full text goes to the agent");
    }

    // ─── Residual-arm chase ──────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn thinking_after_text_adds_separator() {
        // ThinkingStart with existing stream_text pushes a blank line first.
        let events = vec![
            ts::ev("", 0, "text_chunk", r#"{"text":"answer part"}"#),
            ts::ev("", 1, "thinking_start", "{}"),
            ts::ev("", 2, "thinking_delta", r#"{"text":"afterthought"}"#),
            ts::ev("", 3, "agent_end", r#"{"state":"completed"}"#),
        ];
        let (env, _) = setup(events).await;
        drive(&env, true, None).await.unwrap();
        let finals = bodies(&env.http, "/cardkit/v1/cards/card_1");
        let last = finals.last().unwrap();
        assert!(last.contains("answer part"), "{last}");
        assert!(last.contains("afterthought"), "{last}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn card_reply_failure_only_warns() {
        // Card create succeeds but replying with the card reference fails.
        let mut routes = feishu_routes();
        routes.retain(|r| r.path != "/im/v1/messages/om_user/reply");
        routes.push(HttpRoute::json(
            "/im/v1/messages/om_user/reply",
            200,
            r#"{"code":61,"msg":"reply quota"}"#,
        ));
        // Thinking flow first (ThinkingDelta create site)…
        let events = vec![
            ts::ev("", 0, "thinking_start", "{}"),
            ts::ev("", 1, "thinking_delta", r#"{"text":"t"}"#),
            ts::ev("", 2, "agent_end", r#"{"state":"completed"}"#),
        ];
        let mut state = MockState::default();
        state.events = events;
        let (env, _) = setup_with(state, routes).await;
        drive(&env, true, None).await.unwrap();
        // The loop completed despite the reply failure (agent_end reached:
        // DONE reaction attempted).
        let reactions = bodies(&env.http, "/im/v1/messages/om_user/reactions");
        assert!(reactions.iter().any(|b| b.contains("DONE")));

        // …and the TextChunk create site.
        let mut routes = feishu_routes();
        routes.retain(|r| r.path != "/im/v1/messages/om_user/reply");
        routes.push(HttpRoute::json(
            "/im/v1/messages/om_user/reply",
            200,
            r#"{"code":61,"msg":"reply quota"}"#,
        ));
        let mut state = MockState::default();
        state.events = vec![
            ts::ev("", 0, "text_chunk", r#"{"text":"x"}"#),
            ts::ev("", 1, "agent_end", r#"{"state":"completed"}"#),
        ];
        let (env, _) = setup_with(state, routes).await;
        drive(&env, true, None).await.unwrap();
        let reactions = bodies(&env.http, "/im/v1/messages/om_user/reactions");
        assert!(reactions.iter().any(|b| b.contains("DONE")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn card_create_failure_only_warns() {
        // Streaming-card create fails at both sites (thinking + text);
        // tool/approval/finalize proceed with no card id.
        let mut routes = feishu_routes();
        routes.retain(|r| r.path != "/cardkit/v1/cards");
        routes.push(HttpRoute::json(
            "/cardkit/v1/cards",
            200,
            r#"{"code":42,"msg":"no cards today"}"#,
        ));
        let events = vec![
            ts::ev("", 0, "thinking_start", "{}"),
            ts::ev("", 1, "thinking_delta", r#"{"text":"t"}"#),
            ts::ev("", 2, "thinking_end", "{}"),
            ts::ev(
                "",
                3,
                "tool_start",
                r#"{"tool_id":"t1","tool_name":"shell"}"#,
            ),
            ts::ev("", 4, "tool_end", r#"{"tool_id":"t1"}"#),
            ts::ev(
                "",
                5,
                "approval_request",
                r#"{"approval_request_id":"req_9","tool_name":"shell","risk_level":"low","title":"T","summary":"S","requested_action":""}"#,
            ),
            ts::ev("", 6, "agent_end", r#"{"state":"completed"}"#),
        ];
        let mut state = MockState::default();
        state.events = events;
        let (env, _) = setup_with(state, routes).await;
        drive(&env, true, None).await.unwrap();
        // Approval fallback replied in text; agent_end replied complete card.
        let replies = bodies(&env.http, "/im/v1/messages/om_user/reply");
        assert!(
            replies.iter().any(|b| b.contains("Approval")),
            "{replies:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn card_create_failure_text_site() {
        let mut routes = feishu_routes();
        routes.retain(|r| r.path != "/cardkit/v1/cards");
        routes.push(HttpRoute::json(
            "/cardkit/v1/cards",
            200,
            r#"{"code":42,"msg":"no cards"}"#,
        ));
        let mut state = MockState::default();
        state.events = vec![
            ts::ev("", 0, "text_chunk", r#"{"text":"x"}"#),
            ts::ev("", 1, "agent_end", r#"{"state":"completed"}"#),
        ];
        let (env, _) = setup_with(state, routes).await;
        drive(&env, true, None).await.unwrap();
        let replies = bodies(&env.http, "/im/v1/messages/om_user/reply");
        assert!(replies.iter().any(|b| b.contains("x")), "{replies:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tool_and_approval_flush_arms_with_paced_stream() {
        // 300ms between events: every flush check sees elapsed ≥ 250ms.
        let long_args = "a".repeat(300);
        let long_result = "r".repeat(600);
        let mut state = MockState::default();
        state.stream_event_delay = Some(Duration::from_millis(300));
        state.events = vec![
            ts::ev("", 0, "text_chunk", r#"{"text":"start"}"#),
            // Long args (>200) → truncation arm; mid-stream flush arm.
            ts::ev(
                "",
                1,
                "tool_start",
                &serde_json::json!({"tool_id":"t1","tool_name":"shell","tool_args": long_args})
                    .to_string(),
            ),
            // Long result (>500) → truncation arm; flush arm.
            ts::ev(
                "",
                2,
                "tool_end",
                &serde_json::json!({"tool_id":"t1","text": long_result}).to_string(),
            ),
            // No args / no result → empty-display arms.
            ts::ev(
                "",
                3,
                "tool_start",
                r#"{"tool_id":"t2","tool_name":"read"}"#,
            ),
            ts::ev("", 4, "tool_end", r#"{"tool_id":"t2"}"#),
            // needs_flush is false right after a flush → approval flush-skip arm.
            ts::ev(
                "",
                5,
                "approval_request",
                r#"{"approval_request_id":"req_1","tool_name":"shell","risk_level":"low","title":"T","summary":"S","requested_action":""}"#,
            ),
            ts::ev("", 6, "agent_end", r#"{"state":"completed"}"#),
        ];
        let (env, _) = setup_with(state, feishu_routes()).await;
        drive(&env, true, None).await.unwrap();
        let updates = bodies(
            &env.http,
            "/cardkit/v1/cards/card_1/elements/stream_out/content",
        );
        assert!(updates.len() >= 3, "paced flushes: {updates:?}");
        let all = updates.join("\n");
        assert!(all.contains("Running tool"), "{all}");
        // Approval flow ran (2 creates total).
        assert_eq!(bodies(&env.http, "/cardkit/v1/cards").len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn finalize_failures_only_warn() {
        // set_card_streaming_mode + update_cardkit_card fail at agent_end.
        let mut routes = feishu_routes();
        routes.retain(|r| r.path != "/cardkit/v1/cards/card_1/settings");
        routes.retain(|r| r.path != "/cardkit/v1/cards/card_1");
        routes.push(HttpRoute::json(
            "/cardkit/v1/cards/card_1/settings",
            500,
            "nope",
        ));
        routes.push(HttpRoute::json(
            "/cardkit/v1/cards/card_1",
            200,
            r#"{"code":88,"msg":"bad card"}"#,
        ));
        let mut state = MockState::default();
        state.events = vec![
            ts::ev("", 0, "text_chunk", r#"{"text":"done text"}"#),
            ts::ev("", 1, "agent_end", r#"{"state":"completed"}"#),
        ];
        let (env, _) = setup_with(state, routes).await;
        drive(&env, true, None).await.unwrap();
        let reactions = bodies(&env.http, "/im/v1/messages/om_user/reactions");
        assert!(reactions.iter().any(|b| b.contains("DONE")));
    }
}
