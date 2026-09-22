//! Slack provider tests: event parsing and filtering, mention gating, the
//! mrkdwn reduction, envelope acknowledgements, UTF-16 splitting against the
//! platform's 4000-unit limit, error classification, webhook signature
//! verification, and the outbound method shapes against a mock HTTP server.

// A few tests serialize on a process-global test hook, so the guard is
// deliberately held across awaits.
#![allow(clippy::await_holding_lock)]

use super::*;
use crate::bridge::ChatKind;
use serde_json::json;
use std::time::Duration;

/// The message event shape Socket Mode and the Events API both deliver.
fn message_event(fields: Value) -> Value {
    let mut event = json!({
        "type": "message",
        "user": "U111",
        "text": "hello",
        "channel": "C222",
        "channel_type": "channel",
        "ts": "1727000000.000100",
        "client_msg_id": "cm-1"
    });
    event
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    event
}

// ── Event parsing and subtype filtering ─────────────────────────────────────

#[test]
fn a_plain_channel_message_becomes_a_group_conversation() {
    let inbound = parse_event(&message_event(json!({})), "UBOT").unwrap();
    assert_eq!(inbound.message_id, "cm-1");
    assert_eq!(inbound.sender.id, "U111");
    assert_eq!(inbound.conversation.id, "C222");
    assert_eq!(inbound.conversation.kind, ChatKind::Channel);
    assert_eq!(inbound.conversation.thread_id, None);
    assert_eq!(inbound.text, "hello");
    assert!(!inbound.addressed_to_bot);
    assert_eq!(inbound.created_at_ms, Some(1727000000 * 1000));
}

#[test]
fn a_direct_message_is_always_addressed_to_the_bot() {
    let inbound = parse_event(
        &message_event(json!({ "channel_type": "im", "channel": "D999" })),
        "UBOT",
    )
    .unwrap();
    assert_eq!(inbound.conversation.kind, ChatKind::Direct);
    assert!(inbound.addressed_to_bot);
}

#[test]
fn an_app_mention_event_is_addressed_and_the_mention_is_reduced() {
    let event = json!({
        "type": "app_mention",
        "user": "U111",
        "text": "<@UBOT> what is up",
        "channel": "C222",
        "channel_type": "channel",
        "ts": "1727000000.000200"
    });
    let inbound = parse_event(&event, "UBOT").unwrap();
    assert!(inbound.addressed_to_bot);
    // No client_msg_id: the channel:ts fallback identifies the message.
    assert_eq!(inbound.message_id, "C222:1727000000.000200");
    assert_eq!(inbound.text, "@UBOT what is up");
}

#[test]
fn a_message_that_mentions_the_bot_inline_counts_as_addressed() {
    let inbound = parse_event(
        &message_event(json!({ "text": "hey <@UBOT> look" })),
        "UBOT",
    )
    .unwrap();
    assert!(inbound.addressed_to_bot);
}

#[test]
fn bot_subtypes_and_own_messages_are_dropped() {
    for (label, fields) in [
        (
            "bot relay",
            json!({ "subtype": "bot_message", "bot_id": "B1" }),
        ),
        ("edit", json!({ "subtype": "message_changed" })),
        ("join", json!({ "subtype": "channel_join" })),
        ("bot without subtype", json!({ "bot_id": "B1" })),
        ("our own post", json!({ "user": "UBOT" })),
        ("missing sender", json!({ "user": "" })),
    ] {
        let event = message_event(fields);
        assert!(
            parse_event(&event, "UBOT").is_none(),
            "{label} must be dropped"
        );
    }
}

#[test]
fn non_message_events_are_dropped() {
    let event = json!({ "type": "reaction_added", "user": "U111" });
    assert!(parse_event(&event, "UBOT").is_none());
}

#[test]
fn a_thread_reply_keeps_its_thread_as_the_conversation() {
    let inbound = parse_event(
        &message_event(json!({ "thread_ts": "1726999999.000001" })),
        "UBOT",
    )
    .unwrap();
    assert_eq!(
        inbound.conversation.thread_id.as_deref(),
        Some("1726999999.000001")
    );
    assert_eq!(
        inbound.conversation_key("slack"),
        "slack:C222:1726999999.000001"
    );
}

#[test]
fn a_top_level_message_with_matching_thread_ts_is_not_a_thread() {
    // Slack sets thread_ts == ts on the parent; that is not a reply.
    let inbound = parse_event(
        &message_event(json!({ "thread_ts": "1727000000.000100" })),
        "UBOT",
    )
    .unwrap();
    assert_eq!(inbound.conversation.thread_id, None);
}

#[test]
fn a_file_share_with_files_but_no_text_is_still_a_message() {
    let event = message_event(json!({
        "subtype": "file_share",
        "text": "",
        "files": [{
            "name": "shot.png",
            "mimetype": "image/png",
            "url_private": "https://files.slack.test/shot.png"
        }]
    }));
    let inbound = parse_event(&event, "UBOT").unwrap();
    assert_eq!(inbound.media.len(), 1);
    assert_eq!(inbound.media[0].kind, crate::bridge::MediaKind::Image);
    assert_eq!(
        inbound.media[0].url.as_deref(),
        Some("https://files.slack.test/shot.png")
    );
    assert_eq!(inbound.media[0].filename.as_deref(), Some("shot.png"));
}

#[test]
fn media_kinds_follow_the_mimetype() {
    assert_eq!(
        file_kind(Some("image/jpeg"), None),
        crate::bridge::MediaKind::Image
    );
    assert_eq!(
        file_kind(Some("audio/mpeg"), None),
        crate::bridge::MediaKind::Audio
    );
    assert_eq!(
        file_kind(Some("video/mp4"), None),
        crate::bridge::MediaKind::Video
    );
    assert_eq!(
        file_kind(Some("application/pdf"), None),
        crate::bridge::MediaKind::Document
    );
    assert_eq!(
        file_kind(None, Some("png")),
        crate::bridge::MediaKind::Image
    );
    assert_eq!(
        file_kind(None, Some("zip")),
        crate::bridge::MediaKind::Document
    );
    assert_eq!(file_kind(None, None), crate::bridge::MediaKind::Unknown);
}

#[test]
fn file_kind_covers_every_bucket() {
    use crate::bridge::MediaKind;
    assert_eq!(file_kind(Some("text/plain"), None), MediaKind::Document);
    assert_eq!(
        file_kind(Some("application/zip"), None),
        MediaKind::Document
    );
    // Any application/* mimetype is a document, whatever the filetype says.
    assert_eq!(
        file_kind(Some("application/octet-stream"), Some("mp3")),
        MediaKind::Document
    );
    // A mimetype in no known bucket falls through to the filetype table.
    assert_eq!(file_kind(Some("model/mesh"), Some("mp3")), MediaKind::Audio);
    assert_eq!(file_kind(None, Some("m4a")), MediaKind::Audio);
    assert_eq!(file_kind(None, Some("wav")), MediaKind::Audio);
    assert_eq!(file_kind(None, Some("ogg")), MediaKind::Audio);
    assert_eq!(file_kind(None, Some("mp4")), MediaKind::Video);
    assert_eq!(file_kind(None, Some("mov")), MediaKind::Video);
    assert_eq!(file_kind(None, Some("webm")), MediaKind::Video);
    assert_eq!(file_kind(None, Some("")), MediaKind::Unknown);
}

#[test]
fn an_empty_text_without_files_is_dropped() {
    let inbound = parse_event(&message_event(json!({ "text": "   " })), "UBOT");
    assert!(inbound.is_none(), "whitespace-only text is not a prompt");
}

#[test]
fn a_group_dm_counts_as_a_group_for_the_mention_gate() {
    let inbound = parse_event(
        &message_event(json!({ "channel_type": "mpim", "channel": "G123" })),
        "UBOT",
    )
    .unwrap();
    assert_eq!(inbound.conversation.kind, ChatKind::Group);
    // A group conversation is only handled when the bot is addressed.
    assert!(!inbound.addressed_to_bot);
}

#[test]
fn missing_channel_and_ts_fields_default_to_empty_strings() {
    let event = json!({
        "type": "message",
        "user": "U111",
        "text": "bare",
        "channel_type": "im"
    });
    let inbound = parse_event(&event, "UBOT").unwrap();
    assert_eq!(inbound.conversation.id, "");
    // No client_msg_id either: the id falls back to the channel:ts pair.
    assert_eq!(inbound.message_id, ":");
    assert_eq!(inbound.created_at_ms, None);
}

// ── mrkdwn reduction ────────────────────────────────────────────────────────

#[test]
fn mrkdwn_links_keep_the_label_or_the_url() {
    assert_eq!(
        reduce_mrkdwn("see <https://a.test|the docs> now", ""),
        "see the docs now"
    );
    assert_eq!(
        reduce_mrkdwn("see <https://a.test> now", ""),
        "see https://a.test now"
    );
}

#[test]
fn mrkdwn_mentions_and_channels_stay_readable() {
    assert_eq!(reduce_mrkdwn("<@U111> said hi", "UBOT"), "@U111 said hi");
    assert_eq!(reduce_mrkdwn("join <#C123|general>", ""), "join #general");
    // An unterminated angle bracket is left alone.
    assert_eq!(reduce_mrkdwn("a < b", ""), "a < b");
}

#[test]
fn a_message_that_is_only_the_mention_reduces_to_empty() {
    assert_eq!(reduce_mrkdwn("<@UBOT>", "UBOT"), "");
}

#[test]
fn mrkdwn_special_mentions_and_channel_fallbacks_stay_readable() {
    // `<!here>` and friends keep the bang form readable.
    assert_eq!(reduce_mrkdwn("<!here> sync now", ""), "@here sync now");
    // A bare `<#C123>` without a label degrades to the id.
    assert_eq!(reduce_mrkdwn("see <#C123>", ""), "see #C123");
}

#[test]
fn an_empty_bot_id_disables_mention_matching() {
    assert!(!mentions_user("<@U111>", ""));
    assert!(mentions_user("hi <@U111>", "U111"));
    // The reduced text keeps the mention (nothing to strip) and is not
    // mistaken for a bare mention of the bot.
    assert_eq!(reduce_mrkdwn("<@U111>", ""), "@U111");
}

// ── Socket Mode envelopes ───────────────────────────────────────────────────

#[test]
fn every_envelope_with_an_id_is_acked_verbatim() {
    let envelope = json!({
        "envelope_id": "env-123",
        "type": "events_api",
        "payload": { "event": { "type": "message" } }
    });
    assert_eq!(
        envelope_ack(&envelope),
        Some(json!({ "envelope_id": "env-123" }))
    );
}

#[test]
fn an_envelope_without_an_id_cannot_be_acked() {
    assert_eq!(envelope_ack(&json!({ "type": "hello" })), None);
}

#[test]
fn the_disconnect_envelope_type_is_recognized() {
    // The session treats this as a clean end so the reconnect backoff resets.
    let envelope = json!({ "type": "disconnect", "reason": "link_soon_to_expire" });
    assert_eq!(
        envelope.get("type").and_then(Value::as_str),
        Some(ENVELOPE_DISCONNECT)
    );
}

// ── Error classification ────────────────────────────────────────────────────

#[test]
fn permanent_api_errors_cover_credentials_and_deleted_targets() {
    for code in [
        "invalid_auth",
        "not_authed",
        "token_revoked",
        "account_inactive",
        "channel_not_found",
        "is_archived",
        "not_in_channel",
        "message_not_found",
    ] {
        assert_eq!(
            classify_api_error(code, 200),
            crate::transport::http::ErrorClass::Permanent,
            "{code}"
        );
    }
}

#[test]
fn transient_api_errors_cover_rate_limits_and_timeouts() {
    for code in [
        "ratelimited",
        "timeout",
        "internal_error",
        "service_unavailable",
        "fatal_timeout",
        "request_timeout",
    ] {
        assert_eq!(
            classify_api_error(code, 200),
            crate::transport::http::ErrorClass::Transient,
            "{code}"
        );
    }
}

#[test]
fn the_error_tables_cover_their_remaining_members() {
    for code in ["fatal_error", "not_allowed_token_type"] {
        assert_eq!(
            classify_api_error(code, 200),
            crate::transport::http::ErrorClass::Permanent,
            "{code}"
        );
    }
}

#[test]
fn an_unknown_code_falls_back_to_the_http_status() {
    assert_eq!(
        classify_api_error("some_new_error", 500),
        crate::transport::http::ErrorClass::Transient
    );
    assert_eq!(
        classify_api_error("some_new_error", 403),
        crate::transport::http::ErrorClass::Permanent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn check_ok_rejects_the_200_with_ok_false_envelope() {
    let (base, _) = crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
        "/auth.test",
        200,
        r#"{"ok":false,"error":"invalid_auth"}"#,
    )])
    .await;
    let api = api_for(&base);
    let error = match auth_identity(&api).await {
        Ok(_) => panic!("an ok:false envelope must fail"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(message.contains("invalid_auth"), "{message}");
    assert!(message.contains("permanent"), "{message}");
}

// ── Webhook signature verification ──────────────────────────────────────────

#[test]
fn a_valid_signed_request_is_accepted() {
    let secret = "top-secret";
    let body = br#"{"type":"event_callback"}"#;
    let timestamp = "1727000000";
    let mut base = format!("v0:{timestamp}:").into_bytes();
    base.extend_from_slice(body);
    let digest = crate::transport::signature::hmac_sha256_hex(secret.as_bytes(), &base);
    let header = format!("v0={digest}");
    assert!(verify_request_signature(
        secret, timestamp, &header, body, 1727000000
    ));
}

#[test]
fn wrong_secrets_bodies_and_stale_timestamps_are_rejected() {
    let secret = "top-secret";
    let body = b"{}";
    let timestamp = "1727000000";
    let mut base = format!("v0:{timestamp}:").into_bytes();
    base.extend_from_slice(body);
    let digest = crate::transport::signature::hmac_sha256_hex(secret.as_bytes(), &base);
    let header = format!("v0={digest}");

    // A different body invalidates the digest.
    assert!(!verify_request_signature(
        secret, timestamp, &header, b"{ }", 1727000000
    ));
    // A different secret invalidates the digest.
    assert!(!verify_request_signature(
        "other", timestamp, &header, body, 1727000000
    ));
    // A replay from an hour ago is outside the freshness window.
    assert!(!verify_request_signature(
        secret,
        timestamp,
        &header,
        body,
        1727000000 + 3600
    ));
    // Missing pieces never verify.
    assert!(!verify_request_signature(
        "", timestamp, &header, body, 1727000000
    ));
    assert!(!verify_request_signature(
        secret, "", &header, body, 1727000000
    ));
    assert!(!verify_request_signature(
        secret, timestamp, "", body, 1727000000
    ));
    assert!(!verify_request_signature(
        secret,
        "not-a-number",
        &header,
        body,
        1727000000
    ));
}

// ── UTF-16 splitting against the 4000-unit limit ────────────────────────────
//
// The shared splitter owns the mechanics; what is Slack-specific is the
// combination of `Utf16` counting with the 4000 cap, and that emoji-heavy
// replies must still split cleanly.

#[test]
fn slack_chunks_never_exceed_4000_utf16_units() {
    let unit = DEFINITION.length_unit;
    assert_eq!(unit, LengthUnit::Utf16);
    let limit = DEFINITION.max_text_len;
    assert_eq!(limit, 4000);

    // BMP-heavy text and emoji-heavy text split differently under UTF-16.
    let bmp = "汉字 abc ".repeat(1200);
    let emoji = "🙂🚀✨ ".repeat(1500);
    for text in [bmp, emoji] {
        assert!(unit.len(&text) > limit);
        let chunks = crate::transport::chunk(&text, limit, unit);
        assert!(chunks.len() > 1);
        assert!(
            chunks.iter().all(|chunk| unit.len(chunk) <= limit),
            "chunk over the limit: {:?}",
            chunks
                .iter()
                .map(|chunk| unit.len(chunk))
                .collect::<Vec<_>>()
        );
        assert_eq!(chunks.concat(), text);
    }
}

#[test]
fn a_split_inside_a_code_block_is_closed_and_reopened() {
    let unit = DEFINITION.length_unit;
    let limit = DEFINITION.max_text_len;
    let body = (0..600)
        .map(|i| format!("let value_{i} = compute({i});\n"))
        .collect::<String>();
    let text = format!("```rust\n{body}```\n");
    assert!(unit.len(&text) > limit);
    let chunks = crate::transport::chunk(&text, limit, unit);
    assert!(chunks.len() > 1, "{chunks:?}");
    for chunk in &chunks {
        assert!(unit.len(chunk) <= limit);
        let fences = chunk.matches("```").count();
        assert_eq!(fences % 2, 0, "unbalanced fence in chunk: {chunk:?}");
    }
}

#[test]
fn astral_emoji_are_never_cut_in_half() {
    // One emoji is two UTF-16 units; a budget of one must not produce a lone
    // surrogate.
    let chunks = crate::transport::chunk("🙂🙂🙂", 1, LengthUnit::Utf16);
    assert!(chunks.iter().all(|chunk| chunk.chars().count() == 1));
    assert_eq!(chunks.concat(), "🙂🙂🙂");
}

// ── Outbound method shapes (mock HTTP) ──────────────────────────────────────

fn test_config(base: &str) -> SlackConfig {
    SlackConfig {
        enabled: true,
        bot_token: "xoxb-test".to_string(),
        api_base: base.to_string(),
        ..SlackConfig::default()
    }
}

fn offline_ctx() -> ProviderCtx {
    ProviderCtx::offline(&DEFINITION)
}

fn api_for(base: &str) -> SlackApi {
    SlackApi::from_config(&test_config(base), &offline_ctx())
}

#[tokio::test(flavor = "multi_thread")]
async fn post_message_carries_thread_ts_and_returns_the_message_ts() {
    let (base, recorded) =
        crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
            "/chat.postMessage",
            200,
            r#"{"ok":true,"ts":"1727000001.000500"}"#,
        )])
        .await;
    let sender = SlackSender {
        api: api_for(&base),
    };
    let conversation = ConversationRef {
        id: "C222".into(),
        thread_id: Some("1727000000.000100".into()),
        kind: ChatKind::Channel,
    };
    let ts = sender.send_text(&conversation, "hi there").await.unwrap();
    assert_eq!(ts.as_deref(), Some("1727000001.000500"));

    let requests = crate::test_support::requests_to(&recorded, "/chat.postMessage");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.header("Authorization"), Some("Bearer xoxb-test"));
    let body: Value = serde_json::from_str(&request.body_string()).unwrap();
    assert_eq!(body["channel"], "C222");
    assert_eq!(body["text"], "hi there");
    assert_eq!(body["thread_ts"], "1727000000.000100");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_top_level_send_omits_thread_ts() {
    let (base, recorded) =
        crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
            "/chat.postMessage",
            200,
            r#"{"ok":true,"ts":"1.2"}"#,
        )])
        .await;
    let sender = SlackSender {
        api: api_for(&base),
    };
    let conversation = ConversationRef {
        id: "C222".into(),
        thread_id: None,
        kind: ChatKind::Channel,
    };
    sender.send_text(&conversation, "top").await.unwrap();
    let requests = crate::test_support::requests_to(&recorded, "/chat.postMessage");
    let body: Value = serde_json::from_str(&requests[0].body_string()).unwrap();
    assert!(body.get("thread_ts").is_none(), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_and_reaction_use_the_expected_methods() {
    let (base, recorded) = crate::test_support::spawn_http(vec![
        crate::test_support::HttpRoute::json("/chat.update", 200, r#"{"ok":true}"#),
        crate::test_support::HttpRoute::json("/reactions.add", 200, r#"{"ok":true}"#),
    ])
    .await;
    let sender = SlackSender {
        api: api_for(&base),
    };
    let conversation = ConversationRef {
        id: "C222".into(),
        thread_id: None,
        kind: ChatKind::Channel,
    };
    sender
        .edit_text(&conversation, "1.2", "new text")
        .await
        .unwrap();
    sender.react(&conversation, "1.2", "eyes").await.unwrap();

    let edits = crate::test_support::requests_to(&recorded, "/chat.update");
    let body: Value = serde_json::from_str(&edits[0].body_string()).unwrap();
    assert_eq!(body["channel"], "C222");
    assert_eq!(body["ts"], "1.2");
    assert_eq!(body["text"], "new text");

    let reactions = crate::test_support::requests_to(&recorded, "/reactions.add");
    let body: Value = serde_json::from_str(&reactions[0].body_string()).unwrap();
    assert_eq!(body["timestamp"], "1.2");
    assert_eq!(body["name"], "eyes");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ok_false_response_fails_the_call_with_the_error_code() {
    let (base, _) = crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
        "/chat.postMessage",
        200,
        r#"{"ok":false,"error":"channel_not_found"}"#,
    )])
    .await;
    let sender = SlackSender {
        api: api_for(&base),
    };
    let error = match sender.send_text(&ConversationRef::default(), "hi").await {
        Ok(_) => panic!("an ok:false response must fail the call"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("channel_not_found"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_429_is_retried_honoring_retry_after() {
    let (base, recorded) =
        crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::sequence(
            "/chat.postMessage",
            vec![
                (429, r#"{"ok":false,"error":"ratelimited"}"#),
                (200, r#"{"ok":true,"ts":"9.9"}"#),
            ],
        )])
        .await;
    let sender = SlackSender {
        api: api_for(&base),
    };
    let ts = sender
        .send_text(&ConversationRef::default(), "hi")
        .await
        .unwrap();
    assert_eq!(ts.as_deref(), Some("9.9"));
    assert_eq!(
        crate::test_support::requests_to(&recorded, "/chat.postMessage").len(),
        2
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_connection_handshake_uses_the_app_token_and_returns_the_url() {
    let (base, recorded) =
        crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
            "/apps.connections.open",
            200,
            r#"{"ok":true,"url":"wss://wss-primary.slack.test/link"}"#,
        )])
        .await;
    let mut config = test_config(&base);
    config.app_token = "xapp-test".to_string();
    let api = SlackApi::from_config(&config, &offline_ctx());
    let url = api.open_connection().await.unwrap();
    assert_eq!(url, "wss://wss-primary.slack.test/link");

    let requests = crate::test_support::requests_to(&recorded, "/apps.connections.open");
    assert_eq!(
        request_auth(&requests[0]),
        Some("Bearer xapp-test".to_string())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_handshake_without_a_url_is_an_error() {
    let (base, _) = crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
        "/apps.connections.open",
        200,
        r#"{"ok":true}"#,
    )])
    .await;
    let mut config = test_config(&base);
    config.app_token = "xapp-test".to_string();
    let api = SlackApi::from_config(&config, &offline_ctx());
    let error = api.open_connection().await.unwrap_err();
    assert!(error.to_string().contains("websocket URL"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn file_downloads_send_the_bot_bearer_token() {
    let (base, recorded) =
        crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::binary(
            "/files/shot.png",
            200,
            b"PNG".to_vec(),
        )])
        .await;
    let api = api_for(&base);
    let (bytes, _content_type) = api
        .download(&format!("{base}/files/shot.png"))
        .await
        .unwrap();
    assert_eq!(bytes, b"PNG");
    let requests = crate::test_support::requests_to(&recorded, "/files/shot.png");
    assert_eq!(
        request_auth(&requests[0]),
        Some("Bearer xoxb-test".to_string())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_probe_reports_the_bot_identity() {
    let (base, _) = crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
        "/auth.test",
        200,
        r#"{"ok":true,"user_id":"UBOT","team_id":"T1"}"#,
    )])
    .await;
    let api = api_for(&base);
    assert_eq!(auth_identity(&api).await.unwrap(), "UBOT");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transient_ok_false_error_is_reported_as_transient() {
    let (base, _) = crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
        "/chat.postMessage",
        200,
        r#"{"ok":false,"error":"ratelimited"}"#,
    )])
    .await;
    let sender = SlackSender {
        api: api_for(&base),
    };
    let error = sender
        .send_text(&ConversationRef::default(), "hi")
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("ratelimited"), "{message}");
    assert!(message.contains("transient"), "{message}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_http_error_status_fails_before_the_envelope_check() {
    let (base, _) = crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
        "/chat.postMessage",
        403,
        r#"{"ok":false,"error":"forbidden"}"#,
    )])
    .await;
    let sender = SlackSender {
        api: api_for(&base),
    };
    let error = sender
        .send_text(&ConversationRef::default(), "hi")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("403"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_handshake_fails_on_an_http_error_or_bad_envelope() {
    // HTTP failure: the platform status, not the envelope, is the message.
    let (base, _) = crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
        "/apps.connections.open",
        500,
        r#"{"ok":false,"error":"internal_error"}"#,
    )])
    .await;
    let mut config = test_config(&base);
    config.app_token = "xapp-test".to_string();
    let api = SlackApi::from_config(&config, &offline_ctx());
    let error = api.open_connection().await.unwrap_err();
    assert!(error.to_string().contains("500"), "{error}");

    // ok:false: the platform error code is the message.
    let (base, _) = crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
        "/apps.connections.open",
        200,
        r#"{"ok":false,"error":"invalid_auth"}"#,
    )])
    .await;
    let mut config = test_config(&base);
    config.app_token = "xapp-test".to_string();
    let api = SlackApi::from_config(&config, &offline_ctx());
    let error = api.open_connection().await.unwrap_err();
    assert!(error.to_string().contains("invalid_auth"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_handshake_url_must_be_a_websocket_url() {
    let (base, _) = crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
        "/apps.connections.open",
        200,
        r#"{"ok":true,"url":"https://not-a-socket.slack.test/"}"#,
    )])
    .await;
    let mut config = test_config(&base);
    config.app_token = "xapp-test".to_string();
    let api = SlackApi::from_config(&config, &offline_ctx());
    let error = api.open_connection().await.unwrap_err();
    assert!(error.to_string().contains("websocket URL"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_download_is_an_http_status_error() {
    let (base, _) = crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
        "/files/gone.png",
        404,
        r#"{"ok":false,"error":"not_found"}"#,
    )])
    .await;
    let api = api_for(&base);
    let error = api
        .download(&format!("{base}/files/gone.png"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("404"), "{error}");
}

#[test]
fn the_sender_reports_the_slack_definition() {
    let sender = SlackSender {
        api: api_for("http://127.0.0.1:1"),
    };
    assert!(std::ptr::eq(sender.definition(), &DEFINITION));
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_test_fails_on_an_http_error_status() {
    let (base, _) = crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
        "/auth.test",
        503,
        r#"{"ok":false,"error":"service_unavailable"}"#,
    )])
    .await;
    let api = api_for(&base);
    let error = auth_identity(&api).await.unwrap_err();
    assert!(error.to_string().contains("503"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_provider_builds_a_sender_with_a_bot_token() {
    let dir = crate::test_support::temp_dir("slack-sender-build");
    let ctx = dispatch_ctx(config_value(&test_config("http://127.0.0.1:1")), &dir);
    let sender = super::Slack
        .sender(&ctx)
        .expect("a bot token builds a sender");
    assert!(std::ptr::eq(sender.definition(), &DEFINITION));
}

fn request_auth(request: &crate::test_support::RecordedRequest) -> Option<String> {
    request.header("Authorization").map(str::to_string)
}

// ── Config and provider wiring ──────────────────────────────────────────────

// ── Event dispatch and attachment fetching ──────────────────────────────────

/// A sender that records reactions and fails any real delivery — dispatch
/// tests assert on the ack reaction, not on platform traffic.
struct RecordingSender {
    definition: &'static ChannelDefinition,
    reactions: std::sync::Mutex<Vec<(String, String, String)>>,
}

impl RecordingSender {
    fn new() -> Self {
        Self {
            definition: &DEFINITION,
            reactions: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn taken(&self) -> Vec<(String, String, String)> {
        self.reactions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

#[async_trait]
impl ChannelSender for RecordingSender {
    fn definition(&self) -> &'static ChannelDefinition {
        self.definition
    }

    async fn send_text(
        &self,
        _conversation: &ConversationRef,
        _text: &str,
    ) -> Result<Option<String>> {
        Ok(Some("recorded".into()))
    }

    async fn react(
        &self,
        conversation: &ConversationRef,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        self.reactions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push((
                conversation.id.clone(),
                message_id.to_string(),
                emoji.to_string(),
            ));
        Ok(())
    }
}

/// The config as JSON, with the test-only overrides applied on top.
fn config_value(config: &SlackConfig) -> Value {
    json!({
        "enabled": config.enabled,
        "bot_token": config.bot_token,
        "app_token": config.app_token,
        "signing_secret": config.signing_secret,
        "webhook_port": config.webhook_port,
        "webhook_path": config.webhook_path,
        "api_base": config.api_base,
    })
}

/// A bridge context with an open DM policy, wired to an unroutable agent so
/// handled events reach the policy/queue pipeline without network traffic.
fn dispatch_ctx(config: Value, data_dir: &std::path::Path) -> ProviderCtx {
    dispatch_ctx_to_agent(config, "http://127.0.0.1:1", data_dir)
}

/// [`dispatch_ctx`] against a chosen agent address (a mock gRPC server).
fn dispatch_ctx_to_agent(
    config: Value,
    agent_addr: &str,
    data_dir: &std::path::Path,
) -> ProviderCtx {
    let sessions = Arc::new(crate::session_store::SessionStore::new(
        data_dir.join("sessions.json"),
    ));
    let agent_cfg = crate::config::AgentConfig {
        grpc_addr: agent_addr.into(),
        cwd: data_dir.to_string_lossy().into_owned(),
        ..crate::config::AgentConfig::default()
    };
    let bridge = crate::bridge::Bridge::new(
        Arc::new(agent_cfg),
        crate::policy::AccessPolicyConfig {
            dm_policy: "open".into(),
            dm_allowlist: Vec::new(),
            group_policy: "open".into(),
            group_allowlist: Vec::new(),
            require_mention: true,
        },
        data_dir.to_path_buf(),
        Arc::new(crate::status::StatusBoard::new(
            data_dir.join("status.json"),
        )),
    );
    ProviderCtx::new(
        &DEFINITION,
        config,
        bridge,
        data_dir.to_path_buf(),
        sessions,
        crate::bridge::Shutdown::new(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn an_accepted_event_gets_the_eyes_reaction() {
    // A mock agent accepts the approval answer the bridge delivers, which
    // makes the outcome "accepted" without any model round trip.
    let (addr, _state) =
        crate::test_support::spawn_mock_grpc(crate::test_support::MockState::default()).await;
    let dir = crate::test_support::temp_dir("slack-dispatch");
    let ctx = dispatch_ctx_to_agent(
        config_value(&test_config("http://127.0.0.1:1")),
        &format!("http://{addr}"),
        &dir,
    );
    let sender: Arc<RecordingSender> = Arc::new(RecordingSender::new());
    let event = message_event(json!({
        "channel_type": "im",
        "channel": "D999",
        "text": "yes",
        // Fresh and unique: the bridge drops stale replays and dedups on the
        // message id.
        "ts": format!("{}.000100", crate::bridge::dedup::now_ms() / 1000),
        "client_msg_id": format!("cm-{}", crate::bridge::dedup::now_ms()),
    }));
    // Route an approval into the conversation so the answer is accepted as
    // an approval answer, not queued as a prompt.
    ctx.bridge().approvals().insert(
        "slack:D999",
        crate::bridge::approval::ApprovalRoute {
            session_id: "s1".into(),
            request_id: "r1".into(),
            tool_name: "shell".into(),
        },
    );
    let dyn_sender: Arc<dyn ChannelSender> = sender.clone();
    dispatch_event(&ctx, &dyn_sender, &event, "UBOT").await;
    let reactions = sender.taken();
    assert_eq!(reactions.len(), 1, "an accepted event gets exactly one ack");
    assert_eq!(reactions[0].0, "D999");
    assert_eq!(reactions[0].2, "eyes");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_denied_event_gets_no_reaction_and_bot_events_are_ignored() {
    // The default offline bridge denies group messages without a mention.
    let ctx = ProviderCtx::offline(&DEFINITION);
    let sender: Arc<RecordingSender> = Arc::new(RecordingSender::new());
    let dyn_sender: Arc<dyn ChannelSender> = sender.clone();
    dispatch_event(&ctx, &dyn_sender, &message_event(json!({})), "UBOT").await;
    assert!(
        sender.taken().is_empty(),
        "a denied message must not be acked with a reaction"
    );

    // A bot-authored event is dropped before the bridge is even consulted.
    let bot_event = message_event(json!({ "bot_id": "B1", "text": "beep" }));
    dispatch_event(&ctx, &dyn_sender, &bot_event, "UBOT").await;
    assert!(sender.taken().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_are_downloaded_with_the_bot_credentials() {
    let (base, recorded) =
        crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::binary(
            "/files/shot.png",
            200,
            b"PNGDATA".to_vec(),
        )])
        .await;
    let dir = crate::test_support::temp_dir("slack-attach");
    let ctx = dispatch_ctx(config_value(&test_config(&base)), &dir);
    let mut inbound = Inbound::new_direct("m1", "U111", "D999", "see this");
    inbound.media.push(MediaRef {
        kind: MediaKind::Image,
        url: Some(format!("{base}/files/shot.png")),
        ..MediaRef::default()
    });
    // Already-downloaded and non-image media stay untouched.
    inbound.media.push(MediaRef {
        kind: MediaKind::Image,
        url: Some(format!("{base}/files/already.png")),
        data: Some(b"old".to_vec()),
        ..MediaRef::default()
    });
    inbound.media.push(MediaRef {
        kind: MediaKind::Document,
        url: Some(format!("{base}/files/spec.pdf")),
        ..MediaRef::default()
    });
    inbound.media.push(MediaRef {
        kind: MediaKind::Image,
        url: None,
        ..MediaRef::default()
    });
    fetch_attachments(&ctx, &mut inbound).await.unwrap();
    assert_eq!(inbound.media[0].data.as_deref(), Some(&b"PNGDATA"[..]));
    assert_eq!(inbound.media[1].data.as_deref(), Some(&b"old"[..]));
    assert!(
        inbound.media[2].data.is_none(),
        "documents stay URL references"
    );
    assert!(inbound.media[3].data.is_none(), "no URL, nothing to fetch");
    let requests = crate::test_support::requests_to(&recorded, "/files/shot.png");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        request_auth(&requests[0]),
        Some("Bearer xoxb-test".to_string())
    );
    assert!(crate::test_support::requests_to(&recorded, "/files/spec.pdf").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_bot_token_or_a_failed_download_leaves_the_reference() {
    let dir = crate::test_support::temp_dir("slack-attach-skip");
    let mut config = test_config("http://127.0.0.1:1");
    config.bot_token = String::new();
    let ctx = dispatch_ctx(config_value(&config), &dir);
    let mut inbound = Inbound::new_direct("m1", "U111", "D999", "see this");
    inbound.media.push(MediaRef {
        kind: MediaKind::Image,
        url: Some("https://files.slack.test/shot.png".into()),
        ..MediaRef::default()
    });
    fetch_attachments(&ctx, &mut inbound).await.unwrap();
    assert!(
        inbound.media[0].data.is_none(),
        "without a bot token no download is attempted"
    );

    // A 404 on the file leaves the URL reference in place.
    let (base, _) = crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
        "/files/gone.png",
        404,
        "{}",
    )])
    .await;
    let dir = crate::test_support::temp_dir("slack-attach-404");
    let ctx = dispatch_ctx(config_value(&test_config(&base)), &dir);
    let mut inbound = Inbound::new_direct("m1", "U111", "D999", "see this");
    inbound.media.push(MediaRef {
        kind: MediaKind::Image,
        url: Some(format!("{base}/files/gone.png")),
        ..MediaRef::default()
    });
    fetch_attachments(&ctx, &mut inbound).await.unwrap();
    assert!(inbound.media[0].data.is_none());
}

#[test]
fn config_defaults_keep_unknown_policy_keys_harmless() {
    // The framework mixes access-policy keys into the same block; the
    // provider's own struct must not reject them.
    let value = json!({
        "enabled": true,
        "bot_token": "xoxb-1",
        "dm_policy": "allowlist",
        "dm_allowlist": ["U1"],
        "require_mention": true
    });
    let config: SlackConfig = serde_json::from_value(value).unwrap();
    assert!(config.enabled);
    assert_eq!(config.bot_token, "xoxb-1");
    assert_eq!(config.webhook_port, DEFAULT_WEBHOOK_PORT);
    assert_eq!(config.api_base(), API_BASE);
}

#[test]
fn the_definition_matches_the_platform_limits() {
    assert!(DEFINITION.is_implemented());
    assert_eq!(DEFINITION.maturity, Maturity::Preview);
    assert_eq!(DEFINITION.max_text_len, 4000);
    assert_eq!(DEFINITION.length_unit, LengthUnit::Utf16);
    assert!(DEFINITION.capabilities.edit);
    assert!(DEFINITION.capabilities.threads);
    assert!(DEFINITION.capabilities.reactions);
    let example: Value = serde_json::from_str(DEFINITION.config_example).unwrap();
    assert!(example.get("bot_token").is_some());
    assert!(example.get("app_token").is_some());
}

#[test]
fn a_sender_cannot_be_built_without_a_bot_token() {
    let ctx = offline_ctx();
    // The offline ctx has an empty config block, so the token is missing.
    let error = match super::Slack.sender(&ctx) {
        Ok(_) => panic!("a sender without a bot token must not build"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("bot_token"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_probe_never_succeeds_without_a_real_request() {
    // No mock server: the unreachable API base must fail the probe.
    let ctx = ProviderCtx::offline(&DEFINITION);
    let mut config_value = serde_json::Map::new();
    config_value.insert("enabled".into(), json!(true));
    config_value.insert("bot_token".into(), json!("xoxb-1"));
    config_value.insert("api_base".into(), json!("http://127.0.0.1:1"));
    let ctx = ProviderCtx::new(
        &DEFINITION,
        Value::Object(config_value),
        ctx.bridge().clone(),
        ctx.data_dir().to_path_buf(),
        std::sync::Arc::new(crate::session_store::SessionStore::new(
            ctx.data_dir().join("sessions.json"),
        )),
        ctx.shutdown().clone(),
    );
    assert!(super::Slack.probe(&ctx).await.is_err());
}

// ── Socket Mode session against a mock gateway ─────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn socket_mode_acks_envelopes_and_stops_on_a_server_disconnect() {
    let event = message_event(json!({ "bot_id": "B1" }));
    let envelope = json!({
        "envelope_id": "env-1",
        "type": "events_api",
        "payload": { "event": event }
    });
    let (url, received) = crate::test_support::spawn_ws(vec![
        crate::test_support::WsAction::SendText("not json at all".into()),
        crate::test_support::WsAction::SendText(json!({ "type": "hello" }).to_string()),
        crate::test_support::WsAction::SendText(envelope.to_string()),
        // Wait long enough for the ack and the pong to reach the server-side
        // record before the session ends.
        crate::test_support::WsAction::Delay(Duration::from_millis(300)),
        crate::test_support::WsAction::SendBinary(vec![0x00, 0x01]),
        crate::test_support::WsAction::SendPing(b"hb".to_vec()),
        crate::test_support::WsAction::Delay(Duration::from_millis(300)),
        crate::test_support::WsAction::SendText(
            json!({ "type": "disconnect", "reason": "link_soon_to_expire" }).to_string(),
        ),
    ])
    .await;
    // A bridge context with an open policy so the delivered event reaches
    // the pipeline (the offline default would deny it, which is fine too —
    // the assertion is on the ack, not the outcome).
    let dir = crate::test_support::temp_dir("slack-session-ack");
    let ctx = dispatch_ctx(config_value(&test_config("http://127.0.0.1:1")), &dir);
    let socket = crate::transport::ws::connect(&url, &[]).await.unwrap();
    let sender: Arc<dyn ChannelSender> = Arc::new(RecordingSender::new());
    tokio::time::timeout(
        Duration::from_secs(5),
        socket_mode_session(&ctx, sender, "UBOT", socket),
    )
    .await
    .expect("a server-requested disconnect must end the session promptly")
    .expect("a server-requested disconnect is a clean end, not an error");

    let received = received
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let acks = received
        .iter()
        .filter_map(|message| message.to_text().ok())
        .filter(|text| text.contains("envelope_id"))
        .count();
    assert_eq!(
        acks, 1,
        "only the envelope with an id is acked: {received:?}"
    );
    assert!(
        received
            .iter()
            .any(|message| matches!(message, WsMessage::Pong(_))),
        "a protocol ping must be answered with a pong: {received:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn socket_mode_fails_when_the_platform_drops_the_socket() {
    // A clean close frame.
    let (url, _) =
        crate::test_support::spawn_ws(vec![crate::test_support::WsAction::SendClose]).await;
    let ctx = ProviderCtx::offline(&DEFINITION);
    let socket = crate::transport::ws::connect(&url, &[]).await.unwrap();
    let sender: Arc<dyn ChannelSender> = Arc::new(RecordingSender::new());
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        socket_mode_session(&ctx, sender, "UBOT", socket),
    )
    .await
    .expect("the session must notice the close promptly")
    .expect_err("a platform close is a reconnect reason, not a clean exit");
    assert!(error.to_string().contains("closed"), "{error}");

    // A protocol error on the wire.
    let (url, _) = crate::test_support::spawn_ws(vec![
        crate::test_support::WsAction::SendRawBytes(vec![0x83, 0x00]),
        crate::test_support::WsAction::Delay(Duration::from_millis(300)),
    ])
    .await;
    let ctx = ProviderCtx::offline(&DEFINITION);
    let socket = crate::transport::ws::connect(&url, &[]).await.unwrap();
    let sender: Arc<dyn ChannelSender> = Arc::new(RecordingSender::new());
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        socket_mode_session(&ctx, sender, "UBOT", socket),
    )
    .await
    .expect("the session must notice the failure promptly")
    .expect_err("a read failure must fail the session");
    assert!(error.to_string().contains("read failed"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn socket_mode_fails_when_the_stream_ends_without_a_close_frame() {
    // The server sends a close frame, then drops the connection without
    // reading the client's reply. The session bails on the close frame (the
    // close arm), but the queued close reply still has to be flushed, which
    // the drain below does: once the client has sent its close it enters
    // CloseAcknowledged, and the server's FIN then ends the message stream
    // with `None` rather than a protocol error — the end-of-stream arm.
    let (url, _) = crate::test_support::spawn_ws(vec![
        crate::test_support::WsAction::SendClose,
        crate::test_support::WsAction::Delay(Duration::from_millis(10)),
    ])
    .await;
    let ctx = ProviderCtx::offline(&DEFINITION);
    let socket = crate::transport::ws::connect(&url, &[]).await.unwrap();
    let sender: Arc<dyn ChannelSender> = Arc::new(RecordingSender::new());
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        socket_mode_session(&ctx, sender, "UBOT", socket),
    )
    .await
    .expect("the session must notice the close promptly")
    .expect_err("a close frame is a reconnect reason, not a clean exit");
    assert!(error.to_string().contains("closed"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drained_close_handshake_ends_the_message_stream_with_none() {
    // Companion to the session test above: after the close handshake the
    // FIN ends the stream with `None`, which the session maps to the
    // end-of-stream arm. Driving the handshake directly keeps that mapping
    // deterministic.
    let (url, _) = crate::test_support::spawn_ws(vec![
        crate::test_support::WsAction::SendClose,
        crate::test_support::WsAction::Delay(Duration::from_millis(200)),
    ])
    .await;
    let mut socket = crate::transport::ws::connect(&url, &[]).await.unwrap();
    // The close frame arrives; tungstenite queues the reply.
    let mut saw_close = false;
    use futures_util::StreamExt;
    for _ in 0..100 {
        match tokio::time::timeout(Duration::from_millis(50), socket.next()).await {
            Ok(Some(Ok(WsMessage::Close(_)))) => {
                saw_close = true;
                break;
            }
            Ok(Some(Ok(_))) => {}
            _ => break,
        }
    }
    assert!(saw_close, "the close frame must arrive");
    // Sending any frame flushes the queued close reply, completing the
    // handshake; the server's FIN then ends the stream.
    use futures_util::SinkExt;
    let _ = socket.send(WsMessage::Ping(vec![])).await;
    let mut ended = false;
    for _ in 0..100 {
        match tokio::time::timeout(Duration::from_millis(50), socket.next()).await {
            Ok(None) => {
                ended = true;
                break;
            }
            Ok(Some(Ok(_))) => {}
            _ => break,
        }
    }
    assert!(ended, "after the handshake the stream ends with None");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn socket_mode_fails_when_the_ack_cannot_be_written() {
    let envelope = json!({ "envelope_id": "env-9", "type": "events_api" });
    let (url, _) = crate::test_support::spawn_ws(vec![
        crate::test_support::WsAction::SendText(envelope.to_string()),
        // Hold the connection open; the test kills the client's write half.
        crate::test_support::WsAction::Delay(Duration::from_secs(30)),
    ])
    .await;
    let ctx = ProviderCtx::offline(&DEFINITION);
    let socket = crate::transport::ws::connect(&url, &[]).await.unwrap();
    // Kill the write half up front: the ack send fails when it flushes.
    super::kill_write_half(&socket);
    let sender: Arc<dyn ChannelSender> = Arc::new(RecordingSender::new());
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        socket_mode_session(&ctx, sender, "UBOT", socket),
    )
    .await
    .expect("the session must notice the dead socket promptly")
    .expect_err("a dead socket must fail the session");
    assert!(
        error.to_string().contains("not writable"),
        "the ack write must be the reported failure: {error}"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn socket_mode_fails_when_the_pong_cannot_be_written() {
    let (url, _) = crate::test_support::spawn_ws(vec![
        crate::test_support::WsAction::SendPing(b"hb".to_vec()),
        crate::test_support::WsAction::Delay(Duration::from_secs(30)),
    ])
    .await;
    let ctx = ProviderCtx::offline(&DEFINITION);
    let socket = crate::transport::ws::connect(&url, &[]).await.unwrap();
    // Kill the write half up front: the pong send fails when it flushes.
    super::kill_write_half(&socket);
    let sender: Arc<dyn ChannelSender> = Arc::new(RecordingSender::new());
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        socket_mode_session(&ctx, sender, "UBOT", socket),
    )
    .await
    .expect("the session must notice the dead socket promptly")
    .expect_err("a dead socket must fail the session");
    assert!(
        error.to_string().contains("not writable"),
        "the pong write must be the reported failure: {error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn socket_mode_stops_cleanly_on_shutdown() {
    // The server holds the socket open; only the shutdown signal ends the
    // session.
    let (url, _) = crate::test_support::spawn_ws(vec![crate::test_support::WsAction::Delay(
        Duration::from_secs(30),
    )])
    .await;
    let ctx = ProviderCtx::offline(&DEFINITION);
    let socket = crate::transport::ws::connect(&url, &[]).await.unwrap();
    let shutdown = ctx.shutdown().clone();
    let sender: Arc<dyn ChannelSender> = Arc::new(RecordingSender::new());
    let session =
        tokio::spawn(async move { socket_mode_session(&ctx, sender, "UBOT", socket).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    shutdown.trigger();
    tokio::time::timeout(Duration::from_secs(5), session)
        .await
        .expect("shutdown must end the session promptly")
        .expect("the session task must not panic")
        .expect("shutdown is a clean end");
}

// ── run() transport dispatch ────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn run_requires_a_receive_path() {
    let dir = crate::test_support::temp_dir("slack-run-no-receive");
    // auth.test fails (unroutable): the warning arm runs, then the missing
    // receive path is the fatal error.
    let ctx = dispatch_ctx(config_value(&test_config("http://127.0.0.1:1")), &dir);
    let error = super::Slack.run(ctx).await.unwrap_err();
    assert!(
        error.to_string().contains("app_token"),
        "without app_token or signing_secret there is no receive path: {error}"
    );

    // auth.test succeeds, but neither transport is configured.
    let (base, _) = crate::test_support::spawn_http(vec![crate::test_support::HttpRoute::json(
        "/auth.test",
        200,
        r#"{"ok":true,"user_id":"UBOT"}"#,
    )])
    .await;
    let ctx = dispatch_ctx(config_value(&test_config(&base)), &dir);
    let error = super::Slack.run(ctx).await.unwrap_err();
    assert!(error.to_string().contains("app_token"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn run_socket_mode_connects_acks_and_reconnects_until_shutdown() {
    // A non-message event: the envelope is acked, the payload is ignored.
    let envelope = json!({
        "envelope_id": "env-1",
        "type": "events_api",
        "payload": { "event": { "type": "typing", "user": "U111" } }
    });
    // Connection 1 delivers one envelope then the platform drops the socket;
    // connection 2 holds open so shutdown lands in the session select.
    let (ws_url, received) = crate::test_support::spawn_ws_per_connection(vec![
        vec![
            crate::test_support::WsAction::SendText(envelope.to_string()),
            // Give the client time to write the ack before the script ends
            // and the connection drops.
            crate::test_support::WsAction::Delay(Duration::from_millis(500)),
        ],
        vec![crate::test_support::WsAction::Delay(Duration::from_secs(
            30,
        ))],
    ])
    .await;
    let (base, _) = crate::test_support::spawn_http(vec![
        crate::test_support::HttpRoute::json("/auth.test", 200, r#"{"ok":true,"user_id":"UBOT"}"#),
        crate::test_support::HttpRoute::json(
            "/apps.connections.open",
            200,
            &json!({ "ok": true, "url": ws_url }).to_string(),
        ),
    ])
    .await;
    let dir = crate::test_support::temp_dir("slack-run-socket");
    let mut config = test_config(&base);
    config.app_token = "xapp-test".to_string();
    let ctx = dispatch_ctx(config_value(&config), &dir);
    let shutdown = ctx.shutdown().clone();
    let running = tokio::spawn(async move { super::Slack.run(ctx).await });
    let acked = crate::test_support::wait_until(
        || {
            received
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .iter()
                .any(|message| {
                    message
                        .to_text()
                        .map(|text| text.contains("env-1"))
                        .unwrap_or(false)
                })
        },
        Duration::from_secs(5),
    )
    .await;
    assert!(acked, "the envelope must be acked on the first connection");
    // Shutdown can land in two sequential waits — the session's select and
    // the supervise loop's backoff sleep — so follow the `Started::stop`
    // pattern: wake whoever is parked now (`notify_waiters`) and leave one
    // stored permit for the wait that registers next (`notify_one`).
    shutdown.trigger();
    shutdown.trigger();
    tokio::time::timeout(Duration::from_secs(10), running)
        .await
        .expect("shutdown during the reconnect backoff must end run() promptly")
        .expect("the run task must not panic")
        .expect("shutdown during the reconnect backoff is a clean end");
}

// ── Events API webhook end to end ───────────────────────────────────────────

fn slack_signature(secret: &str, timestamp: &str, body: &[u8]) -> String {
    let mut base = format!("v0:{timestamp}:").into_bytes();
    base.extend_from_slice(body);
    let digest = crate::transport::signature::hmac_sha256_hex(secret.as_bytes(), &base);
    format!("v0={digest}")
}

fn slack_now() -> String {
    (crate::bridge::dedup::now_ms() / 1000).to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_events_webhook_verifies_signatures_and_answers_the_challenge() {
    let _hook = super::webhook_test_hook::lock();
    let dir = crate::test_support::temp_dir("slack-webhook");
    let mut config = test_config("http://127.0.0.1:1");
    config.signing_secret = "signing-secret".to_string();
    // No leading slash: the server must normalize the path.
    config.webhook_path = "slack-events".to_string();
    let slot: Arc<std::sync::Mutex<(u16, String, u16)>> =
        Arc::new(std::sync::Mutex::new((0, String::new(), 0)));
    super::webhook_test_hook::arm(slot.clone());
    // A mock agent that opens sessions, so a dispatched prompt is accepted and
    // acknowledged instead of failing to reach the agent.
    let (addr, _state) =
        crate::test_support::spawn_mock_grpc(crate::test_support::MockState::default()).await;
    let ctx = dispatch_ctx_to_agent(config_value(&config), &format!("http://{addr}"), &dir);
    let webhook_config = ctx.config::<SlackConfig>().unwrap();
    let recording: Arc<RecordingSender> = Arc::new(RecordingSender::new());
    let sender: Arc<dyn ChannelSender> = recording.clone();
    let shutdown = ctx.shutdown().clone();
    let serving = tokio::spawn(async move {
        run_events_webhook(&ctx, sender, &webhook_config, "UBOT".into()).await
    });
    let bound = crate::test_support::wait_until(
        || slot.lock().unwrap_or_else(|error| error.into_inner()).2 != 0,
        Duration::from_secs(5),
    )
    .await;
    assert!(bound, "the webhook must bind its port");
    let (path, port) = {
        let slot = slot.lock().unwrap_or_else(|error| error.into_inner());
        (slot.1.clone(), slot.2)
    };
    assert_eq!(
        path, "/slack-events",
        "a path without a leading slash is normalized"
    );
    let url = format!("http://127.0.0.1:{port}{path}");
    let client = reqwest::Client::new();

    // The one-time URL verification challenge is answered verbatim.
    let body = json!({ "type": "url_verification", "challenge": "challenge-abc" }).to_string();
    let timestamp = slack_now();
    let signature = slack_signature("signing-secret", &timestamp, body.as_bytes());
    let response = client
        .post(&url)
        .header("x-slack-request-timestamp", &timestamp)
        .header("x-slack-signature", &signature)
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "challenge-abc");

    // A bad signature is a 401, no matter the payload.
    let response = client
        .post(&url)
        .header("x-slack-request-timestamp", slack_now())
        .header("x-slack-signature", "v0=forged")
        .body(json!({ "type": "url_verification", "challenge": "x" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);

    // A signed event callback is acked fast (200); the event is processed
    // off the request path.
    let event = message_event(json!({
        "channel_type": "im",
        "channel": "D999",
        "ts": format!("{}.000100", crate::bridge::dedup::now_ms() / 1000),
        "client_msg_id": format!("hook-{}", crate::bridge::dedup::now_ms()),
    }));
    let body = json!({ "type": "event_callback", "event": event }).to_string();
    let timestamp = slack_now();
    let signature = slack_signature("signing-secret", &timestamp, body.as_bytes());
    let response = client
        .post(&url)
        .header("x-slack-request-timestamp", &timestamp)
        .header("x-slack-signature", &signature)
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    // The event is processed off the request path: waiting for the accepted
    // prompt's acknowledgement proves the spawned dispatch task ran to
    // completion (a sleep would prove nothing).
    let acked =
        crate::test_support::wait_until(|| !recording.taken().is_empty(), Duration::from_secs(5))
            .await;
    assert!(acked, "the dispatched event must reach the bridge pipeline");

    // A signed payload of any other type is acked and ignored.
    let body = json!({ "type": "app_rate_limited" }).to_string();
    let timestamp = slack_now();
    let signature = slack_signature("signing-secret", &timestamp, body.as_bytes());
    let response = client
        .post(&url)
        .header("x-slack-request-timestamp", &timestamp)
        .header("x-slack-signature", &signature)
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        recording.taken().len(),
        1,
        "an ignored payload type adds no second acknowledgement"
    );

    shutdown.trigger();
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .expect("the webhook must stop promptly after shutdown")
        .expect("the webhook task must not panic")
        .expect("shutdown is a clean end");
}

// ── Remaining session/dispatch/webhook arms ────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_skips_events_that_are_not_prompts_and_survives_attachment_failures() {
    let (base, _) = crate::test_support::spawn_http(vec![
        // The private file URL answers with a platform error: the download
        // fails, the reference survives, and the event is still handled.
        crate::test_support::HttpRoute::json("/files/private.png", 500, "{}"),
    ])
    .await;
    let dir = crate::test_support::temp_dir("slack-dispatch-edges");
    let ctx = dispatch_ctx(config_value(&test_config(&base)), &dir);
    let recording: Arc<RecordingSender> = Arc::new(RecordingSender::new());
    let sender: Arc<dyn ChannelSender> = recording.clone();

    // An event whose shape cannot become an inbound message is dropped
    // without ever reaching the bridge pipeline.
    let malformed = json!({ "type": "message", "text": null });
    dispatch_event(&ctx, &sender, &malformed, "UBOT").await;

    // A file_share with no text is still a prompt; its image attachment
    // cannot be fetched (HTTP 500 above), which is logged and tolerated.
    let event = message_event(json!({
        "subtype": "file_share",
        "text": "",
        "channel_type": "im",
        "channel": "D999",
        "ts": format!("{}.000100", crate::bridge::dedup::now_ms() / 1000),
        "client_msg_id": format!("dispatch-{}", crate::bridge::dedup::now_ms()),
        "files": [{
            "mimetype": "image/png",
            "url_private": format!("{base}/files/private.png")
        }],
    }));
    dispatch_event(&ctx, &sender, &event, "UBOT").await;

    assert!(
        recording.taken().is_empty(),
        "neither dispatch sends anything visible"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_channel_config_does_not_drop_the_prompt() {
    // Fetching an attachment needs the channel's own configuration; when that
    // cannot be read the download is skipped, but the prompt itself must still
    // reach the bridge and be acknowledged — an enrichment failure is not a
    // reason to lose the message.
    let (addr, _state) =
        crate::test_support::spawn_mock_grpc(crate::test_support::MockState::default()).await;
    let dir = crate::test_support::temp_dir("slack-attach-config-error");
    let ctx = dispatch_ctx_to_agent(
        json!({ "enabled": "not-a-bool" }),
        &format!("http://{addr}"),
        &dir,
    );
    assert!(
        ctx.config::<SlackConfig>().is_err(),
        "this test needs a configuration the provider cannot read"
    );
    let recording: Arc<RecordingSender> = Arc::new(RecordingSender::new());
    let sender: Arc<dyn ChannelSender> = recording.clone();
    let event = message_event(json!({
        "channel_type": "im",
        "channel": "D999",
        "ts": format!("{}.000100", crate::bridge::dedup::now_ms() / 1000),
        "client_msg_id": format!("cfg-{}", crate::bridge::dedup::now_ms()),
        "files": [{ "mimetype": "image/png", "url_private": "http://127.0.0.1:1/shot.png" }],
    }));
    dispatch_event(&ctx, &sender, &event, "UBOT").await;
    assert_eq!(
        recording.taken().len(),
        1,
        "a prompt whose attachments cannot be fetched is still accepted"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_events_webhook_binds_the_configured_port_without_the_test_hook() {
    let _hook = super::webhook_test_hook::lock();
    // No hook armed: `run_events_webhook` must bind the configured port.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        drop(listener);
        port
    };
    let dir = crate::test_support::temp_dir("slack-webhook-configured-port");
    let mut config = test_config("http://127.0.0.1:1");
    config.signing_secret = "signing-secret".to_string();
    config.webhook_port = port;
    config.webhook_path = "/events".to_string();
    let ctx = dispatch_ctx(config_value(&config), &dir);
    let webhook_config = ctx.config::<SlackConfig>().unwrap();
    let shutdown = ctx.shutdown().clone();
    let sender: Arc<dyn ChannelSender> = Arc::new(RecordingSender::new());
    let serving = tokio::spawn(async move {
        run_events_webhook(&ctx, sender, &webhook_config, "UBOT".into()).await
    });

    // The challenge answered over the configured port proves the bind.
    let url = format!("http://127.0.0.1:{port}/events");
    let client = reqwest::Client::new();
    let mut answered = false;
    for _ in 0..50 {
        let body =
            json!({ "type": "url_verification", "challenge": "configured-port" }).to_string();
        let timestamp = slack_now();
        let signature = slack_signature("signing-secret", &timestamp, body.as_bytes());
        match client
            .post(&url)
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", &signature)
            .body(body)
            .send()
            .await
        {
            Ok(response) if response.status() == 200 => {
                assert_eq!(response.text().await.unwrap(), "configured-port");
                answered = true;
                break;
            }
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    assert!(answered, "the webhook must answer on the configured port");

    shutdown.trigger();
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .expect("the webhook must stop promptly after shutdown")
        .expect("the webhook task must not panic")
        .expect("shutdown is a clean end");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_events_webhook_normalizes_an_already_slash_prefixed_path() {
    let _hook = super::webhook_test_hook::lock();
    let dir = crate::test_support::temp_dir("slack-webhook-slash");
    let mut config = test_config("http://127.0.0.1:1");
    config.signing_secret = "signing-secret".to_string();
    // Leading slash already present: the path is registered as-is.
    config.webhook_path = "/already/slashed".to_string();
    let slot: Arc<std::sync::Mutex<(u16, String, u16)>> =
        Arc::new(std::sync::Mutex::new((0, String::new(), 0)));
    super::webhook_test_hook::arm(slot.clone());
    let ctx = dispatch_ctx(config_value(&config), &dir);
    let webhook_config = ctx.config::<SlackConfig>().unwrap();
    let shutdown = ctx.shutdown().clone();
    let sender: Arc<dyn ChannelSender> = Arc::new(RecordingSender::new());
    let serving = tokio::spawn(async move {
        run_events_webhook(&ctx, sender, &webhook_config, "UBOT".into()).await
    });
    let bound = crate::test_support::wait_until(
        || slot.lock().unwrap_or_else(|error| error.into_inner()).2 != 0,
        Duration::from_secs(5),
    )
    .await;
    assert!(bound, "the webhook must bind its port");
    let (path, port) = {
        let slot = slot.lock().unwrap_or_else(|error| error.into_inner());
        (slot.1.clone(), slot.2)
    };
    assert_eq!(path, "/already/slashed");

    let body = json!({ "type": "url_verification", "challenge": "c2" }).to_string();
    let timestamp = slack_now();
    let signature = slack_signature("signing-secret", &timestamp, body.as_bytes());
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}{path}"))
        .header("x-slack-request-timestamp", &timestamp)
        .header("x-slack-signature", &signature)
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "c2");

    shutdown.trigger();
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .expect("the webhook must stop promptly after shutdown")
        .expect("the webhook task must not panic")
        .expect("shutdown is a clean end");
}
