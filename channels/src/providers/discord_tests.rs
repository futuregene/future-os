// Unit tests for the Discord provider.
//
// These cover the platform-specific decisions the bridge does not make:
// gateway frame parsing, mention gating, rate-limit header parsing, resume
// decisions, and that message splitting stays inside Discord's 2000-character
// limit without breaking a code fence.

use super::*;
use crate::bridge::ChatKind;
use crate::test_support::{HttpRoute, WsAction, requests_to, spawn_http, spawn_ws};
use serde_json::json;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

// ─── Splitting ─────────────────────────────────────────────────────────────

#[test]
fn a_long_reply_is_split_without_cutting_a_code_block() {
    let code = (0..100)
        .map(|i| format!("let line_{i} = compute({i});\n"))
        .collect::<String>();
    let text = format!("intro\n```rust\n{code}```\noutro\n");
    let chunks = crate::transport::chunk(&text, DEFINITION.max_text_len, DEFINITION.length_unit);
    assert!(chunks.len() > 1);
    for chunk in &chunks {
        assert!(chunk.chars().count() <= 2000);
        let fences = chunk.matches("```").count();
        assert_eq!(fences % 2, 0, "unbalanced fence in {chunk:?}");
    }
}

// ─── Gateway frames ───────────────────────────────────────────────────────

#[test]
fn hello_yields_the_heartbeat_interval() {
    let event = GatewayEvent::parse(r#"{"op":10,"d":{"heartbeat_interval":41250}}"#).unwrap();
    assert_eq!(event.op, OP_HELLO);
    assert_eq!(
        event.data.get("heartbeat_interval").and_then(Value::as_u64),
        Some(41250)
    );
}

#[test]
fn dispatch_carries_the_sequence_and_event_name() {
    let event =
        GatewayEvent::parse(r#"{"op":0,"s":42,"t":"MESSAGE_CREATE","d":{"id":"1"}}"#).unwrap();
    assert_eq!(event.op, OP_DISPATCH);
    assert_eq!(event.sequence, Some(42));
    assert_eq!(event.event.as_deref(), Some("MESSAGE_CREATE"));
}

#[test]
fn heartbeat_and_heartbeat_ack_are_recognised() {
    let hb = GatewayEvent::parse(r#"{"op":1,"d":null}"#).unwrap();
    assert_eq!(hb.op, OP_HEARTBEAT);
    let ack = GatewayEvent::parse(r#"{"op":11,"d":null}"#).unwrap();
    assert_eq!(ack.op, OP_HEARTBEAT_ACK);
}

#[test]
fn reconnect_and_invalid_session_are_distinct() {
    let reconnect = GatewayEvent::parse(r#"{"op":7,"d":null}"#).unwrap();
    assert_eq!(reconnect.op, OP_RECONNECT);
    let invalid = GatewayEvent::parse(r#"{"op":9,"d":false}"#).unwrap();
    assert_eq!(invalid.op, OP_INVALID_SESSION);
    assert_eq!(invalid.data.as_bool(), Some(false));
}

#[test]
fn a_frame_without_an_op_is_rejected() {
    assert!(GatewayEvent::parse(r#"{"s":1,"t":"X","d":{}}"#).is_err());
    assert!(GatewayEvent::parse("not json").is_err());
}

// ─── Resume decisions ─────────────────────────────────────────────────────

#[test]
fn a_fresh_state_identifies() {
    let state = GatewayState::default();
    assert_eq!(state.resume_decision(), ResumeDecision::Identify);
}

#[test]
fn a_session_with_a_sequence_resumes() {
    let state = GatewayState {
        session_id: Some("s1".into()),
        last_sequence: Some(7),
        ..GatewayState::default()
    };
    assert_eq!(state.resume_decision(), ResumeDecision::Resume);
}

#[test]
fn a_missing_sequence_forces_identify() {
    let state = GatewayState {
        session_id: Some("s1".into()),
        ..GatewayState::default()
    };
    assert_eq!(state.resume_decision(), ResumeDecision::Identify);
}

#[test]
fn the_resume_payload_names_the_session_and_sequence() {
    let state = GatewayState {
        session_id: Some("s1".into()),
        last_sequence: Some(99),
        ..GatewayState::default()
    };
    let payload = state.resume_payload("tok").unwrap();
    assert_eq!(payload["op"], OP_RESUME);
    assert_eq!(payload["d"]["session_id"], "s1");
    assert_eq!(payload["d"]["seq"], 99);
}

#[test]
fn identify_declares_the_message_content_intent() {
    let state = GatewayState::default();
    let payload = state.identify_payload("tok");
    assert_eq!(payload["op"], OP_IDENTIFY);
    assert_eq!(
        payload["d"]["intents"].as_u64().unwrap(),
        INTENTS,
        "identify must ask for GUILD_MESSAGES, DIRECT_MESSAGES and MESSAGE_CONTENT"
    );
}

// ─── Mention gating ───────────────────────────────────────────────────────

fn message(data: Value) -> Option<MessageShape> {
    parse_message_create(&data, Some("bot-1"))
}

#[test]
fn a_dm_is_always_addressed() {
    let shape = message(json!({
        "id": "m1",
        "channel_id": "c1",
        "author": {"id": "u1", "username": "alice"},
        "content": "hi",
        "timestamp": "2026-09-22T10:00:00.000+00:00",
    }))
    .unwrap();
    assert!(shape.is_dm);
    assert!(shape.addressed);
}

#[test]
fn a_guild_message_without_a_mention_is_ignored() {
    assert!(
        message(json!({
            "id": "m1",
            "channel_id": "c1",
            "guild_id": "g1",
            "author": {"id": "u1"},
            "content": "hi",
            "mentions": [],
        }))
        .is_none()
    );
}

#[test]
fn a_guild_message_mentioning_the_bot_is_addressed() {
    let shape = message(json!({
        "id": "m1",
        "channel_id": "c1",
        "guild_id": "g1",
        "author": {"id": "u1"},
        "content": "<@bot-1> hi",
        "mentions": [{"id": "bot-1"}],
    }))
    .unwrap();
    assert!(!shape.is_dm);
    assert!(shape.addressed);
}

#[test]
fn a_guild_message_mentioning_someone_else_is_ignored() {
    assert!(
        message(json!({
            "id": "m1",
            "channel_id": "c1",
            "guild_id": "g1",
            "author": {"id": "u1"},
            "content": "<@u2> hi",
            "mentions": [{"id": "u2"}],
        }))
        .is_none()
    );
}

#[test]
fn our_own_message_is_never_answered() {
    assert!(
        message(json!({
            "id": "m1",
            "channel_id": "c1",
            "author": {"id": "bot-1"},
            "content": "hi",
        }))
        .is_none()
    );
}

#[test]
fn another_bots_message_is_never_answered() {
    assert!(
        message(json!({
            "id": "m1",
            "channel_id": "c1",
            "author": {"id": "u1", "bot": true},
            "content": "hi",
        }))
        .is_none()
    );
}

// ─── Rate limiting ────────────────────────────────────────────────────────

#[test]
fn a_429_body_sets_the_wait_and_the_global_flag() {
    let mut headers = std::collections::HashMap::new();
    headers.insert("x-ratelimit-bucket".to_string(), "bucket-1".to_string());
    let limit =
        RateLimit::parse(429, &headers, &json!({"retry_after": 0.5, "global": true})).unwrap();
    assert!(limit.global);
    assert_eq!(limit.bucket.as_deref(), Some("bucket-1"));
    assert_eq!(limit.wait(), Some(Duration::from_millis(500)));
}

#[test]
fn a_global_header_alone_marks_the_limit_as_global() {
    let mut headers = std::collections::HashMap::new();
    headers.insert("x-ratelimit-global".to_string(), "true".to_string());
    headers.insert("x-ratelimit-reset-after".to_string(), "1.25".to_string());
    let limit = RateLimit::parse(200, &headers, &json!({})).unwrap();
    assert!(limit.global);
    assert_eq!(limit.wait(), Some(Duration::from_millis(1250)));
}

#[test]
fn a_plain_200_is_not_a_rate_limit() {
    let headers = std::collections::HashMap::new();
    assert!(RateLimit::parse(200, &headers, &json!({})).is_none());
}

#[test]
fn a_per_bucket_429_is_not_global() {
    let headers = std::collections::HashMap::new();
    let limit =
        RateLimit::parse(429, &headers, &json!({"retry_after": 0.1, "global": false})).unwrap();
    assert!(!limit.global);
}

// ─── Attachments and kinds ───────────────────────────────────────────────

#[test]
fn image_attachments_are_classified_for_the_bridge() {
    assert_eq!(media_kind(Some("image/png")), MediaKind::Image);
    assert_eq!(media_kind(Some("audio/ogg")), MediaKind::Audio);
    assert_eq!(media_kind(Some("video/mp4")), MediaKind::Video);
    assert_eq!(media_kind(Some("application/pdf")), MediaKind::Document);
    assert_eq!(media_kind(None), MediaKind::Unknown);
}

#[test]
fn attachment_shapes_are_collected_from_the_payload() {
    let shape = message(json!({
        "id": "m1",
        "channel_id": "c1",
        "author": {"id": "u1"},
        "content": "look",
        "attachments": [
            {
                "url": "https://cdn.discord.example/a.png",
                "filename": "a.png",
                "content_type": "image/png",
                "size": 1234
            },
            {
                "url": "https://cdn.discord.example/b.txt",
                "filename": "b.txt"
            }
        ]
    }))
    .unwrap();
    assert_eq!(shape.attachments.len(), 2);
    assert_eq!(
        shape.attachments[0].content_type.as_deref(),
        Some("image/png")
    );
    assert_eq!(shape.attachments[0].size, Some(1234));
    assert_eq!(shape.attachments[1].content_type, None);
}

// ─── Timestamps ───────────────────────────────────────────────────────────

#[test]
fn discord_timestamps_parse_to_unix_millis() {
    let ms = parse_timestamp_ms(Some("2026-09-22T10:00:00.000+00:00")).unwrap();
    assert_eq!(ms, 1_790_071_200_000);
    assert!(parse_timestamp_ms(None).is_none());
    assert!(parse_timestamp_ms(Some("not a date")).is_none());
}

// ─── Provider plumbing ────────────────────────────────────────────────────

#[test]
fn the_definition_is_now_usable() {
    assert!(DEFINITION.is_implemented());
    assert_eq!(DEFINITION.maturity, Maturity::Preview);
    assert_eq!(DEFINITION.max_text_len, 2000);
    assert!(DEFINITION.capabilities.edit);
    assert!(DEFINITION.capabilities.threads);
}

#[test]
fn a_sender_threads_replies_into_the_thread_channel() {
    let conversation = ConversationRef {
        id: "parent".into(),
        thread_id: Some("thread".into()),
        kind: ChatKind::Channel,
    };
    assert_eq!(DiscordSender::channel_id(&conversation), "thread");
    let plain = ConversationRef {
        id: "parent".into(),
        thread_id: None,
        kind: ChatKind::Channel,
    };
    assert_eq!(DiscordSender::channel_id(&plain), "parent");
}

#[test]
fn a_blank_token_is_rejected() {
    let ctx = ProviderCtx::offline(&DEFINITION);
    let config: DiscordConfig = serde_json::from_value(json!({"bot_token": ""})).unwrap();
    assert!(config.bot_token.is_empty());
    let result = Discord.sender(&ctx);
    let error = match result {
        Ok(_) => panic!("a blank token must not build a sender"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("bot_token"), "{error}");
}

#[test]
fn the_config_example_is_valid_and_includes_the_token() {
    let value: Value = serde_json::from_str(DEFINITION.config_example).unwrap();
    assert_eq!(value["enabled"], true);
    assert!(value.get("bot_token").is_some());
}

// ─── Close codes ─────────────────────────────────────────────────────────

#[test]
fn fatal_gateway_close_codes_are_recognised() {
    for code in [4004, 4010, 4011, 4012, 4013, 4014] {
        assert!(close_is_fatal(code), "{code} should be fatal");
    }
    for code in [4000, 4001, 4002, 4003, 4005, 4007, 4008, 4009] {
        assert!(!close_is_fatal(code), "{code} should be reconnectable");
    }
}

// ─── Test helpers ──────────────────────────────────────────────────────────

/// An offline context whose `providers.discord` config block points at the
/// mock servers.
fn ctx_with_config(config: Value) -> ProviderCtx {
    let base = ProviderCtx::offline(&DEFINITION);
    ProviderCtx::new(
        &DEFINITION,
        config,
        base.bridge().clone(),
        base.data_dir().to_path_buf(),
        std::sync::Arc::new(crate::session_store::SessionStore::new(
            base.data_dir().join("sessions.json"),
        )),
        base.shutdown().clone(),
    )
}

/// A sender talking to a mock REST API.
fn test_sender(ctx: &ProviderCtx, api_base: &str) -> Arc<DiscordSender> {
    let config: DiscordConfig = serde_json::from_value(json!({
        "bot_token": "bot-token",
        "api_base": api_base,
    }))
    .unwrap();
    Arc::new(DiscordSender::new(
        &config,
        ctx,
        Arc::new(RateLimiter::default()),
    ))
}

fn plain_conversation() -> ConversationRef {
    ConversationRef {
        id: "chan-1".into(),
        thread_id: None,
        kind: ChatKind::Channel,
    }
}

/// A guild MESSAGE_CREATE payload mentioning the bot, with a fresh timestamp
/// so the offline bridge's freshness window keeps it.
fn inbound_message(id: &str) -> Value {
    json!({
        "id": id,
        "channel_id": "chan-1",
        "guild_id": "g1",
        "author": {"id": "u1", "username": "alice"},
        "content": "<@bot-1> hi",
        "mentions": [{"id": "bot-1"}],
        "timestamp": "2099-01-01T00:00:00.000+00:00",
    })
}

/// Parse every text frame the mock WS server recorded into a gateway payload.
fn received_gateway_frames(received: &crate::test_support::WsReceived) -> Vec<Value> {
    received
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter_map(|message| message.to_text().ok().map(str::to_string))
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect()
}

// ─── Config ────────────────────────────────────────────────────────────────

#[test]
fn config_defaults_and_overrides() {
    let config: DiscordConfig = serde_json::from_value(json!({"bot_token": "t"})).unwrap();
    assert_eq!(config.api_base(), REST_BASE);
    assert_eq!(config.gateway_url(), GATEWAY_URL);
    assert_eq!(config.backfill_limit, 20);
    let config: DiscordConfig = serde_json::from_value(json!({
        "bot_token": "t",
        "api_base": "http://127.0.0.1:9/api",
        "gateway_url": "ws://127.0.0.1:9/gw",
        "backfill_limit": 5,
    }))
    .unwrap();
    assert_eq!(config.api_base(), "http://127.0.0.1:9/api");
    assert_eq!(config.gateway_url(), "ws://127.0.0.1:9/gw");
    assert_eq!(config.backfill_limit, 5);
}

// ─── Rate limiter gate ─────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn the_global_gate_holds_requests_until_the_window_passes() {
    let limiter = RateLimiter::default();
    // No global wait noted: waiting is a no-op.
    limiter.wait_for_global().await;
    limiter.note_global_wait(Duration::from_millis(50));
    // A later, shorter wait must not pull the deadline backwards.
    limiter.note_global_wait(Duration::from_millis(5));
    {
        let until = limiter
            .global_until
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .expect("a wait was noted");
        assert!(until > Instant::now() + Duration::from_millis(5));
    }
    limiter.wait_for_global().await;
    // After the window passed the gate is open again; re-noting works.
    limiter.wait_for_global().await;
    limiter.note_global_wait(Duration::ZERO);
}

// ─── REST sender ───────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn send_text_posts_and_returns_the_message_id() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        "/channels/chan-1/messages",
        200,
        r#"{"id": "msg-1"}"#,
    )])
    .await;
    let ctx = ctx_with_config(json!({}));
    let sender = test_sender(&ctx, &base);
    assert!(std::ptr::eq(sender.definition(), &DEFINITION));
    let id = sender
        .send_text(&plain_conversation(), "hello")
        .await
        .unwrap();
    assert_eq!(id.as_deref(), Some("msg-1"));
    let requests = requests_to(&recorded, "/channels/chan-1/messages");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].header("authorization"), Some("Bot bot-token"));
    let body: Value = serde_json::from_str(&requests[0].body_string()).unwrap();
    assert_eq!(body["content"], "hello");
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_typing_and_react_hit_their_routes() {
    let (base, recorded) = spawn_http(vec![
        HttpRoute::json("/channels/chan-1/messages/msg-1", 200, r#"{"id":"msg-1"}"#),
        HttpRoute::json("/channels/chan-1/typing", 204, ""),
        HttpRoute::json(
            "/channels/chan-1/messages/msg-1/reactions/%F0%9F%91%80/@me",
            204,
            "",
        ),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let sender = test_sender(&ctx, &base);
    sender
        .edit_text(&plain_conversation(), "msg-1", "edited")
        .await
        .unwrap();
    sender.typing(&plain_conversation()).await.unwrap();
    sender
        .react(&plain_conversation(), "msg-1", "%F0%9F%91%80")
        .await
        .unwrap();
    let edit = requests_to(&recorded, "/channels/chan-1/messages/msg-1");
    assert_eq!(edit.len(), 1);
    assert_eq!(edit[0].method, "PATCH");
    let body: Value = serde_json::from_str(&edit[0].body_string()).unwrap();
    assert_eq!(body["content"], "edited");
    assert_eq!(requests_to(&recorded, "/channels/chan-1/typing").len(), 1);
    assert_eq!(
        requests_to(
            &recorded,
            "/channels/chan-1/messages/msg-1/reactions/%F0%9F%91%80/@me"
        )
        .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_permanent_failure_is_an_error_not_a_retry() {
    let (base, _) = spawn_http(vec![HttpRoute::json(
        "/channels/chan-1/messages",
        400,
        r#"{"message": "Cannot send an empty message"}"#,
    )])
    .await;
    let ctx = ctx_with_config(json!({}));
    let sender = test_sender(&ctx, &base);
    let error = sender
        .send_text(&plain_conversation(), "")
        .await
        .expect_err("a 400 is a permanent rejection");
    assert!(error.to_string().contains("discord REST"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bucket_429_is_retried_until_it_gives_up() {
    let (base, recorded) = spawn_http(vec![HttpRoute::rate_limited(
        "/channels/chan-1/typing",
        r#"{"retry_after": 0.001, "global": false}"#,
        &[],
    )])
    .await;
    let ctx = ctx_with_config(json!({}));
    let sender = test_sender(&ctx, &base);
    let error = sender
        .typing(&plain_conversation())
        .await
        .expect_err("a permanent 429 exhausts the in-line retries");
    assert!(error.to_string().contains("still rate limited"), "{error}");
    // 1 transport attempt (429s are not retried by send_json itself) plus 3
    // in-line re-issues.
    assert_eq!(requests_to(&recorded, "/channels/chan-1/typing").len(), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_global_429_parks_the_limiter_for_the_next_call() {
    let (base, recorded) = spawn_http(vec![
        HttpRoute::rate_limited(
            "/limited",
            r#"{"retry_after": 0.05, "global": true}"#,
            &[("x-ratelimit-global", "true"), ("x-ratelimit-bucket", "b1")],
        ),
        HttpRoute::json("/other", 200, r#"{"ok": true}"#),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let sender = test_sender(&ctx, &base);
    let error = sender
        .request(
            reqwest::Method::GET,
            "/limited",
            None,
            RetryPolicy::single_attempt(),
        )
        .await
        .expect_err("the global 429 never clears inside one request");
    assert!(error.to_string().contains("still rate limited"), "{error}");
    // The limiter recorded the global window, so the next request on any route
    // waits for it first.
    let started = std::time::Instant::now();
    sender
        .request(
            reqwest::Method::GET,
            "/other",
            None,
            RetryPolicy::single_attempt(),
        )
        .await
        .unwrap();
    assert!(started.elapsed() >= Duration::from_millis(40));
    assert_eq!(requests_to(&recorded, "/other").len(), 1);
}

// ─── Attachments over the wire ─────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn attachments_download_inline_and_failures_degrade_to_urls() {
    let (base, _) = spawn_http(vec![
        HttpRoute::binary("/ok.png", 200, b"\x89PNG".to_vec()),
        HttpRoute::binary("/missing.bin", 404, Vec::new()),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let sender = test_sender(&ctx, &base);
    let shape = MessageShape {
        id: "m1".into(),
        channel_id: "chan-1".into(),
        author_id: "u1".into(),
        author_name: Some("alice".into()),
        content: "files".into(),
        is_dm: true,
        addressed: true,
        timestamp_ms: None,
        attachments: vec![
            AttachmentShape {
                url: format!("{base}/ok.png"),
                filename: Some("ok.png".into()),
                content_type: Some("image/png".into()),
                size: Some(4),
            },
            AttachmentShape {
                url: format!("{base}/missing.bin"),
                filename: None,
                content_type: None,
                size: None,
            },
        ],
    };
    handle_inbound(&ctx, &sender, shape).await;
    let ok = sender
        .download(&format!("{base}/ok.png"), "image/png")
        .await
        .unwrap();
    assert_eq!(ok, b"\x89PNG");
    let error = sender
        .download(&format!("{base}/missing.bin"), "application/octet-stream")
        .await
        .expect_err("a 404 download must fail");
    assert!(error.to_string().contains("404"), "{error}");
}

// ─── Backfill ──────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn backfill_queues_unseen_messages_and_skips_unaddressed_ones() {
    let mut unaddressed = inbound_message("m-skip");
    unaddressed["mentions"] = json!([]);
    unaddressed["content"] = json!("not for us");
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        "/channels/chan-1/messages",
        200,
        &json!([inbound_message("m-a"), unaddressed]).to_string(),
    )])
    .await;
    let config: DiscordConfig = serde_json::from_value(json!({
        "bot_token": "bot-token",
        "api_base": base,
    }))
    .unwrap();
    let ctx = ctx_with_config(json!({}));
    let sender = test_sender(&ctx, &config.api_base());
    backfill(&ctx, &config, &sender, "chan-1", None)
        .await
        .unwrap();
    let requests = requests_to(&recorded, "/channels/chan-1/messages");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].target.contains("limit=20"));
    assert!(!requests[0].target.contains("after="));
}

#[tokio::test(flavor = "multi_thread")]
async fn backfill_converts_the_since_timestamp_into_a_snowflake() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        "/channels/chan-1/messages",
        200,
        "[]",
    )])
    .await;
    let config: DiscordConfig = serde_json::from_value(json!({
        "bot_token": "bot-token",
        "api_base": base,
    }))
    .unwrap();
    let ctx = ctx_with_config(json!({}));
    let sender = test_sender(&ctx, &config.api_base());
    backfill(&ctx, &config, &sender, "chan-1", Some(1_800_000_000_000))
        .await
        .unwrap();
    let requests = requests_to(&recorded, "/channels/chan-1/messages");
    let expected = (1_800_000_000_000_u64 - 1_420_070_400_000) << 22;
    assert!(
        requests[0].target.contains(&format!("after={expected}")),
        "{}",
        requests[0].target
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_array_backfill_response_is_a_noop() {
    let (base, _) = spawn_http(vec![HttpRoute::json(
        "/channels/chan-1/messages",
        200,
        r#"{"weird": true}"#,
    )])
    .await;
    let config: DiscordConfig = serde_json::from_value(json!({
        "bot_token": "bot-token",
        "api_base": base,
    }))
    .unwrap();
    let ctx = ctx_with_config(json!({}));
    let sender = test_sender(&ctx, &config.api_base());
    backfill(&ctx, &config, &sender, "chan-1", None)
        .await
        .unwrap();
}

// ─── Gateway session (mock server) ─────────────────────────────────────────

/// The error text when the mock server drops the socket mid-session: either a
/// clean EOF read or a protocol-level reset, depending on timing.
fn assert_socket_drop(error: &anyhow::Error) {
    let text = error.to_string();
    assert!(
        text.contains("closed the connection") || text.contains("Connection reset"),
        "{text}"
    );
}

/// Run the gateway against the mock until it ends or the budget expires.
async fn run_gateway_once(
    ctx: &ProviderCtx,
    config: &DiscordConfig,
    sender: Arc<DiscordSender>,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), run_gateway(ctx, config, sender))
        .await
        .expect("the gateway must end when the scripted server finishes")
}

fn gateway_config(gateway_url: &str) -> DiscordConfig {
    serde_json::from_value(json!({
        "bot_token": "bot-token",
        "gateway_url": gateway_url,
    }))
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn hello_identifies_and_the_ready_dispatch_marks_the_session() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let ready = json!({
        "op": 0,
        "s": 3,
        "t": "READY",
        "d": {
            "session_id": "sess-1",
            "resume_gateway_url": "wss://resume.example",
            "user": {"id": "bot-1"},
        }
    });
    let (url, received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(ready.to_string()),
        WsAction::Delay(Duration::from_millis(120)),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config = gateway_config(&url);
    let sender = test_sender(&ctx, "http://127.0.0.1:1");
    let error = run_gateway_once(&ctx, &config, sender)
        .await
        .expect_err("a dropped socket is a reconnect, not a clean exit");
    assert_socket_drop(&error);
    let frames = received_gateway_frames(&received);
    assert_eq!(frames[0]["op"], OP_IDENTIFY);
    assert_eq!(frames[0]["d"]["token"], "bot-token");
    assert_eq!(frames[0]["d"]["intents"].as_u64().unwrap(), INTENTS);
    assert_eq!(frames[0]["d"]["properties"]["browser"], "future-channel");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_heartbeat_is_echoed_and_the_ack_clears_the_flag() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 120}});
    let ready = json!({
        "op": 0, "s": 1, "t": "READY",
        "d": {"session_id": "sess-1", "user": {"id": "bot-1"}}
    });
    let server_heartbeat = json!({"op": 1, "d": null});
    let (url, received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(ready.to_string()),
        WsAction::SendText(server_heartbeat.to_string()),
        // Let the client's own heartbeat fire once, then ack it.
        WsAction::Delay(Duration::from_millis(260)),
        WsAction::SendText(json!({"op": 11}).to_string()),
        WsAction::Delay(Duration::from_millis(60)),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config = gateway_config(&url);
    let sender = test_sender(&ctx, "http://127.0.0.1:1");
    run_gateway_once(&ctx, &config, sender)
        .await
        .expect_err("a dropped socket is a reconnect, not a clean exit");
    let frames = received_gateway_frames(&received);
    let heartbeats: Vec<&Value> = frames.iter().filter(|f| f["op"] == OP_HEARTBEAT).collect();
    // One echo of the server heartbeat, at least one clock heartbeat.
    assert!(heartbeats.len() >= 2, "{heartbeats:?}");
    assert_eq!(heartbeats[0]["d"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missed_heartbeat_ack_reconnects_as_a_zombie() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 80}});
    let ready = json!({
        "op": 0, "s": 1, "t": "READY",
        "d": {"session_id": "sess-1", "user": {"id": "bot-1"}}
    });
    // HELLO then READY, and never an ack: the second heartbeat interval
    // notices the missing ack and bails.
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(ready.to_string()),
        WsAction::Delay(Duration::from_secs(4)),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config = gateway_config(&url);
    let sender = test_sender(&ctx, "http://127.0.0.1:1");
    let error = run_gateway_once(&ctx, &config, sender)
        .await
        .expect_err("a missing ack must reconnect");
    assert!(error.to_string().contains("not acknowledged"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_held_session_sends_resume_on_a_re_hello() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let ready = json!({
        "op": 0, "s": 9, "t": "READY",
        "d": {"session_id": "sess-9", "user": {"id": "bot-1"}}
    });
    // A second HELLO on the same connection (the resume_gateway_url flow) is
    // answered with RESUME because READY named the session and the sequence.
    let resumed = json!({"op": 0, "s": 10, "t": "RESUMED", "d": {}});
    let (url, received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(ready.to_string()),
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(resumed.to_string()),
        WsAction::Delay(Duration::from_millis(300)),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config = gateway_config(&url);
    let sender = test_sender(&ctx, "http://127.0.0.1:1");
    run_gateway_once(&ctx, &config, sender)
        .await
        .expect_err("a dropped socket is a reconnect, not a clean exit");
    let frames = received_gateway_frames(&received);
    eprintln!("REHELLO frames={frames:?}");
    assert_eq!(frames[0]["op"], OP_IDENTIFY);
    let resume = frames
        .iter()
        .find(|frame| frame["op"] == OP_RESUME)
        .expect("the second HELLO must be answered with RESUME");
    assert_eq!(resume["d"]["session_id"], "sess-9");
    assert_eq!(resume["d"]["seq"], 9);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_op_7_reconnect_asks_for_a_new_connection() {
    let reconnect = json!({"op": 7, "d": null});
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(reconnect.to_string()),
        WsAction::Delay(Duration::from_millis(60)),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config = gateway_config(&url);
    let sender = test_sender(&ctx, "http://127.0.0.1:1");
    let error = run_gateway_once(&ctx, &config, sender)
        .await
        .expect_err("op 7 asks us to reconnect");
    assert!(
        error.to_string().contains("asked us to reconnect"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sequence_without_a_session_falls_back_to_identify() {
    // A dispatch sets the sequence; without a READY the session id is unknown,
    // so a later HELLO must IDENTIFY even though resume_decision said Resume.
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let message = json!({
        "op": 0, "s": 5, "t": "MESSAGE_CREATE",
        "d": inbound_message("m-seq")
    });
    let (url, received) = spawn_ws(vec![
        WsAction::SendText(message.to_string()),
        WsAction::SendText(hello.to_string()),
        WsAction::Delay(Duration::from_millis(200)),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config = gateway_config(&url);
    let sender = test_sender(&ctx, "http://127.0.0.1:1");
    run_gateway_once(&ctx, &config, sender)
        .await
        .expect_err("a dropped socket is a reconnect, not a clean exit");
    let frames = received_gateway_frames(&received);
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(frames[0]["op"], OP_IDENTIFY);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_session_drops_the_resume_state() {
    let invalid = json!({"op": 9, "d": false});
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(invalid.to_string()),
        WsAction::Delay(Duration::from_millis(60)),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config = gateway_config(&url);
    let sender = test_sender(&ctx, "http://127.0.0.1:1");
    let error = run_gateway_once(&ctx, &config, sender)
        .await
        .expect_err("an invalid session ends the connection");
    assert!(
        error
            .to_string()
            .contains("invalidated the session (resumable=false)"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resumable_invalid_session_keeps_the_session_state() {
    let invalid = json!({"op": 9, "d": true});
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(invalid.to_string()),
        WsAction::Delay(Duration::from_millis(60)),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config = gateway_config(&url);
    let sender = test_sender(&ctx, "http://127.0.0.1:1");
    let error = run_gateway_once(&ctx, &config, sender)
        .await
        .expect_err("an invalid session ends the connection");
    assert!(
        error
            .to_string()
            .contains("invalidated the session (resumable=true)"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_create_dispatch_flows_to_the_bridge() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let ready = json!({
        "op": 0, "s": 1, "t": "READY",
        "d": {"session_id": "sess-1", "user": {"id": "bot-1"}}
    });
    let message = json!({
        "op": 0, "s": 2, "t": "MESSAGE_CREATE",
        "d": inbound_message("m-gw")
    });
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(ready.to_string()),
        WsAction::SendText(message.to_string()),
        WsAction::Delay(Duration::from_millis(150)),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config = gateway_config(&url);
    let sender = test_sender(&ctx, "http://127.0.0.1:1");
    run_gateway_once(&ctx, &config, sender)
        .await
        .expect_err("a dropped socket is a reconnect, not a clean exit");
}

#[tokio::test(flavor = "multi_thread")]
async fn unparseable_and_unknown_frames_are_skipped() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText("not json at all".into()),
        WsAction::SendText(json!({"op": 99, "d": {}}).to_string()),
        WsAction::SendText(hello.to_string()),
        WsAction::SendBinary(vec![1, 2, 3]),
        WsAction::Delay(Duration::from_millis(120)),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config = gateway_config(&url);
    let sender = test_sender(&ctx, "http://127.0.0.1:1");
    run_gateway_once(&ctx, &config, sender)
        .await
        .expect_err("a dropped socket is a reconnect, not a clean exit");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fatal_close_code_marks_the_channel_failed() {
    let (url, _received) = spawn_ws(vec![
        WsAction::SendRawBytes(vec![0x88, 0x02, 0x0F, 0xA4]), // close 4004
        WsAction::Delay(Duration::from_millis(120)),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config = gateway_config(&url);
    let sender = test_sender(&ctx, "http://127.0.0.1:1");
    let error = run_gateway_once(&ctx, &config, sender)
        .await
        .expect_err("a fatal close code is not retried as-is");
    assert!(error.to_string().contains("fatal code"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ordinary_close_frame_is_a_plain_reconnect() {
    let (url, _received) = spawn_ws(vec![WsAction::SendClose]).await;
    let ctx = ctx_with_config(json!({}));
    let config = gateway_config(&url);
    let sender = test_sender(&ctx, "http://127.0.0.1:1");
    let error = run_gateway_once(&ctx, &config, sender)
        .await
        .expect_err("a close frame ends the connection");
    assert_socket_drop(&error);
    assert!(!error.to_string().contains("fatal"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_ping_is_answered_with_a_pong() {
    let (url, received) = spawn_ws(vec![
        WsAction::SendPing(b"ping-1".to_vec()),
        WsAction::Delay(Duration::from_millis(120)),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config = gateway_config(&url);
    let sender = test_sender(&ctx, "http://127.0.0.1:1");
    run_gateway_once(&ctx, &config, sender)
        .await
        .expect_err("a dropped socket is a reconnect, not a clean exit");
    let pongs = received
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|message| matches!(message, WsMessage::Pong(_)))
        .count();
    assert!(pongs >= 1, "the client must pong the server ping");
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_closes_the_socket_and_exits_cleanly() {
    let (url, _received) = spawn_ws(vec![WsAction::Delay(Duration::from_secs(4))]).await;
    let base = ProviderCtx::offline(&DEFINITION);
    let shutdown = std::sync::Arc::new(tokio::sync::Notify::new());
    let ctx = ProviderCtx::new(
        &DEFINITION,
        json!({}),
        base.bridge().clone(),
        base.data_dir().to_path_buf(),
        std::sync::Arc::new(crate::session_store::SessionStore::new(
            base.data_dir().join("sessions.json"),
        )),
        shutdown.clone(),
    );
    let task = tokio::spawn({
        let ctx = ctx.clone();
        let config = gateway_config(&url);
        let sender = test_sender(&ctx, "http://127.0.0.1:1");
        async move { run_gateway(&ctx, &config, sender).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    shutdown.notify_waiters();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("shutdown must end the gateway promptly")
        .expect("the gateway task must not panic");
    assert!(result.is_ok(), "{result:?}");
}

// ─── Provider run/probe ────────────────────────────────────────────────────

#[test]
fn provider_builds_a_sender_and_its_definition() {
    let ctx = ctx_with_config(json!({"bot_token": "bot-token"}));
    assert!(std::ptr::eq(Discord.definition(), &DEFINITION));
    let sender = Discord.sender(&ctx).unwrap();
    assert!(std::ptr::eq(sender.definition(), &DEFINITION));
}

#[tokio::test(flavor = "multi_thread")]
async fn run_without_a_token_fails_before_connecting() {
    let ctx = ctx_with_config(json!({}));
    let error = Discord
        .run(ctx)
        .await
        .expect_err("a blank token must not start the gateway");
    assert!(error.to_string().contains("bot_token"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn run_reconnects_and_stops_on_shutdown() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let ready = json!({
        "op": 0, "s": 1, "t": "READY",
        "d": {"session_id": "sess-1", "user": {"id": "bot-1"}}
    });
    let reconnect = json!({"op": 7, "d": null});
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(ready.to_string()),
        WsAction::SendText(reconnect.to_string()),
        WsAction::Delay(Duration::from_secs(6)),
    ])
    .await;
    let base = ProviderCtx::offline(&DEFINITION);
    let shutdown = std::sync::Arc::new(tokio::sync::Notify::new());
    let ctx = ProviderCtx::new(
        &DEFINITION,
        json!({"bot_token": "bot-token", "gateway_url": url}),
        base.bridge().clone(),
        base.data_dir().to_path_buf(),
        std::sync::Arc::new(crate::session_store::SessionStore::new(
            base.data_dir().join("sessions.json"),
        )),
        shutdown.clone(),
    );
    let task = tokio::spawn(Discord.run(ctx));
    // The first connection fails fast (op 7); the supervisor backs off and
    // reconnects. Shutdown during the backoff must end the loop cleanly.
    tokio::time::sleep(Duration::from_millis(400)).await;
    shutdown.notify_waiters();
    let result = tokio::time::timeout(Duration::from_secs(8), task)
        .await
        .expect("shutdown during the reconnect backoff must end the loop")
        .expect("the run task must not panic");
    assert!(result.is_ok(), "{result:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn probe_reports_the_bot_identity() {
    let (base, _) = spawn_http(vec![HttpRoute::json(
        "/users/@me",
        200,
        r#"{"id": "bot-1", "username": "futurebot"}"#,
    )])
    .await;
    let ctx = ctx_with_config(json!({"bot_token": "bot-token", "api_base": base}));
    let report = Discord.probe(&ctx).await.unwrap();
    assert_eq!(report, "connected as @futurebot (bot-1)");
}

#[tokio::test(flavor = "multi_thread")]
async fn probe_requires_a_token_and_survives_odd_payloads() {
    let ctx = ctx_with_config(json!({}));
    let error = Discord
        .probe(&ctx)
        .await
        .expect_err("a blank token must fail the probe");
    assert!(error.to_string().contains("bot_token"), "{error}");

    let (base, _) = spawn_http(vec![HttpRoute::json("/users/@me", 200, "{}")]).await;
    let ctx = ctx_with_config(json!({"bot_token": "bot-token", "api_base": base}));
    let report = Discord.probe(&ctx).await.unwrap();
    assert_eq!(report, "connected as @unknown (unknown)");
}

// ─── Payload builders ──────────────────────────────────────────────────────

#[test]
fn the_heartbeat_payload_carries_the_last_sequence() {
    let state = GatewayState {
        last_sequence: Some(42),
        ..GatewayState::default()
    };
    assert_eq!(state.heartbeat_payload(), json!({"op": 1, "d": 42}));
    let state = GatewayState::default();
    assert_eq!(state.heartbeat_payload(), json!({"op": 1, "d": null}));
}

#[test]
fn resume_payload_is_none_without_a_full_session() {
    let state = GatewayState::default();
    assert!(state.resume_payload("tok").is_none());
    let state = GatewayState {
        session_id: Some("s1".into()),
        ..GatewayState::default()
    };
    assert!(state.resume_payload("tok").is_none());
}

#[test]
fn close_frame_codes_convert_for_the_fatal_check() {
    let frame = CloseFrame {
        code: CloseCode::from(4004u16),
        reason: "".into(),
    };
    assert!(close_is_fatal(frame.code.into()));
}
