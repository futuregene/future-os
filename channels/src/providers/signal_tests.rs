//! Tests for the Signal provider: envelope parsing, addressing, sending and
//! error classification. Nothing here reaches a daemon; the HTTP shapes are
//! served by the crate's mock server.

use super::*;
use crate::test_support::{requests_to, spawn_http, temp_dir, wait_until, HttpRoute};

fn identity(number: &str) -> AccountIdentity {
    AccountIdentity::new(&SignalConfig {
        number: number.to_string(),
        ..SignalConfig::default()
    })
}

fn config_for(base: &str) -> SignalConfig {
    SignalConfig {
        http_url: base.to_string(),
        number: "+15550001234".into(),
        poll_interval_ms: 50,
        ..SignalConfig::default()
    }
}

/// A provider context with this channel's block over an offline bridge.
fn ctx_with(block: Value) -> ProviderCtx {
    let data_dir = temp_dir("signal-ctx");
    let sessions = Arc::new(crate::session_store::SessionStore::new(
        data_dir.join("sessions.json"),
    ));
    let agent_cfg = crate::config::AgentConfig {
        // Unroutable loopback port: tests must never reach a real agent.
        grpc_addr: "http://127.0.0.1:1".into(),
        cwd: data_dir.to_string_lossy().into_owned(),
        ..crate::config::AgentConfig::default()
    };
    let bridge = crate::bridge::Bridge::new(
        Arc::new(agent_cfg),
        crate::policy::AccessPolicyConfig {
            dm_policy: "open".into(),
            group_policy: "open".into(),
            require_mention: false,
            ..crate::policy::AccessPolicyConfig::default()
        },
        data_dir.clone(),
        Arc::new(crate::status::StatusBoard::new(
            data_dir.join("status.json"),
        )),
    );
    ProviderCtx::new(
        &DEFINITION,
        block,
        bridge,
        data_dir,
        sessions,
        Arc::new(tokio::sync::Notify::new()),
    )
}

/// One `dataMessage` envelope as a daemon delivers it.
fn envelope(source: &str, text: &str, extra: Value) -> Value {
    let now = crate::bridge::dedup::now_ms();
    let mut data = serde_json::json!({ "message": text, "timestamp": now });
    if let (Some(object), Some(extra)) = (data.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            object.insert(key.clone(), value.clone());
        }
    }
    serde_json::json!({
        "envelope": {
            "sourceNumber": source,
            "sourceName": "Alice",
            "sourceUuid": "uuid-alice",
            "timestamp": now,
            "dataMessage": data,
        }
    })
}

// ─── configuration ─────────────────────────────────────────────────────────

#[test]
fn config_defaults_point_at_a_local_daemon() {
    let config: SignalConfig = serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(config.http_url, "http://127.0.0.1:8080");
    assert!(config.number.is_empty());
    assert_eq!(config.receive_timeout_s(), 25);
    assert_eq!(config.poll_interval_ms, 1_000);
}

#[test]
fn a_trailing_slash_in_the_base_url_does_not_double_up() {
    let config = SignalConfig {
        http_url: "http://127.0.0.1:8080/".into(),
        ..SignalConfig::default()
    };
    assert_eq!(config.base(), "http://127.0.0.1:8080");
}

#[test]
fn a_long_poll_is_clamped_below_the_http_client_budget() {
    let config = SignalConfig {
        receive_timeout_s: 600,
        ..SignalConfig::default()
    };
    assert_eq!(config.receive_timeout_s(), MAX_RECEIVE_TIMEOUT_S);
}

#[test]
fn a_missing_number_is_a_configuration_error_naming_the_channel() {
    let ctx = ctx_with(serde_json::json!({ "enabled": true, "number": "" }));
    let error = match Signal.sender(&ctx) {
        Ok(_) => panic!("a sender must not be built without a number"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(message.contains("providers.signal"), "{message}");
    assert!(message.contains("`number`"), "{message}");
}

#[test]
fn a_missing_daemon_url_is_a_configuration_error_naming_the_channel() {
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": "", "number": "+15550001234",
    }));
    let error = match Signal.sender(&ctx) {
        Ok(_) => panic!("a sender must not be built without a daemon URL"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(message.contains("providers.signal"), "{message}");
    assert!(message.contains("`http_url`"), "{message}");
}

#[test]
fn a_malformed_config_block_names_the_channel() {
    let ctx = ctx_with(serde_json::json!({ "enabled": true, "receive_timeout_s": "soon" }));
    let error = ctx.config::<SignalConfig>().expect_err("must fail");
    assert!(error.to_string().contains("providers.signal"), "{error}");
}

#[test]
fn the_declaration_is_usable_and_names_its_external_dependency() {
    assert_eq!(DEFINITION.maturity, Maturity::Preview);
    assert!(DEFINITION.is_implemented());
    assert!(DEFINITION
        .requires
        .iter()
        .any(|need| need.contains("signal-cli")));
    // The sample must be valid JSON with every field the provider reads.
    let sample: Value = serde_json::from_str(DEFINITION.config_example).expect("valid JSON");
    for key in [
        "http_url",
        "number",
        "uuid",
        "receive_timeout_s",
        "poll_interval_ms",
    ] {
        assert!(sample.get(key).is_some(), "config_example is missing {key}");
    }
}

// ─── targets and conversation ids ──────────────────────────────────────────

#[test]
fn direct_and_group_conversation_ids_round_trip() {
    assert_eq!(
        Target::parse("+15550001234"),
        Target::Direct("+15550001234".into())
    );
    let group = Target::Group("abc/def+gh=".into());
    assert_eq!(group.conversation_id(), "group:abc/def+gh=");
    assert_eq!(Target::parse(&group.conversation_id()), group);
    // A group id must never be mistaken for a phone number.
    assert_ne!(
        Target::parse(&group.conversation_id()),
        Target::parse("abc/def+gh=")
    );
    // A direct target's id is the recipient itself: no prefix, no rewriting.
    assert_eq!(
        Target::Direct("+15550009999".into()).conversation_id(),
        "+15550009999"
    );
}

#[test]
fn send_bodies_use_recipients_for_a_direct_message_and_group_id_for_a_group() {
    let direct = Target::Direct("+15550009999".into()).send_body("+15550001234", "hello");
    assert_eq!(direct["message"], "hello");
    assert_eq!(direct["number"], "+15550001234");
    assert_eq!(direct["recipients"][0], "+15550009999");
    assert!(direct.get("group-id").is_none());

    let group = Target::Group("groupid".into()).send_body("+15550001234", "hello");
    assert_eq!(group["group-id"], "groupid");
    assert!(group.get("recipients").is_none());

    // The same split exists in the parameter shape the RPC methods take.
    let direct = Target::Direct("+15550009999".into()).rpc_params("+15550001234");
    assert_eq!(direct["account"], "+15550001234");
    assert_eq!(direct["recipient"][0], "+15550009999");
    assert!(direct.get("groupId").is_none());
    let group = Target::Group("groupid".into()).rpc_params("+15550001234");
    assert_eq!(group["groupId"], "groupid");
    assert!(group.get("recipient").is_none());
}

#[test]
fn phone_numbers_are_percent_encoded_in_a_url_path() {
    assert_eq!(encode_segment("+15550001234"), "%2B15550001234");
    assert_eq!(encode_segment("abc/def+gh="), "abc%2Fdef%2Bgh%3D");
    // Unreserved characters stay readable.
    assert_eq!(encode_segment("Plain-1.2_3~4"), "Plain-1.2_3~4");
}

// ─── envelope parsing ──────────────────────────────────────────────────────

#[test]
fn a_direct_envelope_is_addressed_and_uses_the_number_as_the_sender_id() {
    let raw = envelope("+15550009999", "hello", serde_json::json!({}));
    let inbound = parse_envelope(&raw, &identity("+15550001234"), "http://d").expect("inbound");
    assert_eq!(inbound.conversation.kind, ChatKind::Direct);
    assert_eq!(inbound.conversation.id, "+15550009999");
    assert_eq!(inbound.sender.id, "+15550009999");
    assert_eq!(inbound.sender.display.as_deref(), Some("Alice"));
    assert!(inbound.addressed_to_bot);
    assert_eq!(inbound.text, "hello");
    assert!(inbound.created_at_ms.is_some());
    // The dedup key is unique per message even inside one millisecond.
    assert!(inbound.message_id.contains("+15550009999"));
    assert!(inbound.raw.is_some());
}

#[test]
fn a_group_envelope_becomes_its_own_conversation() {
    let raw = envelope(
        "+15550009999",
        "hello everyone",
        serde_json::json!({ "groupInfo": { "groupId": "grp==", "type": "DELIVER" } }),
    );
    let inbound = parse_envelope(&raw, &identity("+15550001234"), "http://d").expect("inbound");
    assert_eq!(inbound.conversation.kind, ChatKind::Group);
    assert_eq!(inbound.conversation.id, "group:grp==");
    // Nobody mentioned the bot, so a group message without a mention is not
    // addressed and the mention gate applies.
    assert!(!inbound.addressed_to_bot);
}

#[test]
fn the_alternate_group_shape_is_also_a_group() {
    let raw = envelope(
        "+15550009999",
        "hi",
        serde_json::json!({ "groupV2": { "id": "newstyle==", "revision": 3 } }),
    );
    let inbound = parse_envelope(&raw, &identity("+15550001234"), "http://d").expect("inbound");
    assert_eq!(inbound.conversation.id, "group:newstyle==");
}

#[test]
fn a_group_mention_of_our_number_is_addressed() {
    let raw = envelope(
        "+15550009999",
        "\u{fffc} please summarise",
        serde_json::json!({
            "groupInfo": { "groupId": "grp==" },
            "mentions": [{ "start": 0, "length": 1, "number": "+15550001234", "uuid": "uuid-bot" }],
        }),
    );
    let inbound = parse_envelope(&raw, &identity("+15550001234"), "http://d").expect("inbound");
    assert!(inbound.addressed_to_bot);
    // The mention placeholder is not text anyone typed.
    assert_eq!(inbound.text, "please summarise");
    assert!(!inbound.text.contains('\u{fffc}'));
}

#[test]
fn a_mention_of_somebody_else_is_not_addressed() {
    let raw = envelope(
        "+15550009999",
        "\u{fffc} hello",
        serde_json::json!({
            "groupInfo": { "groupId": "grp==" },
            "mentions": [{ "start": 0, "length": 1, "number": "+15550005555" }],
        }),
    );
    let inbound = parse_envelope(&raw, &identity("+15550001234"), "http://d").expect("inbound");
    assert!(!inbound.addressed_to_bot);
}

#[test]
fn our_uuid_recognises_a_mention_the_daemon_could_not_resolve() {
    let identity = AccountIdentity::new(&SignalConfig {
        number: "+15550001234".into(),
        uuid: "uuid-bot".into(),
        ..SignalConfig::default()
    });
    let raw = envelope(
        "+15550009999",
        "hi",
        serde_json::json!({
            "groupInfo": { "groupId": "grp==" },
            "mentions": [{ "start": 0, "length": 1, "uuid": "UUID-BOT" }],
        }),
    );
    let inbound = parse_envelope(&raw, &identity, "http://d").expect("inbound");
    assert!(inbound.addressed_to_bot);
}

#[test]
fn a_quote_of_our_own_message_is_addressed() {
    let raw = envelope(
        "+15550009999",
        "what do you think?",
        serde_json::json!({
            "groupInfo": { "groupId": "grp==" },
            "quote": { "id": 1, "authorNumber": "+15550001234", "text": "earlier" },
        }),
    );
    let inbound = parse_envelope(&raw, &identity("+15550001234"), "http://d").expect("inbound");
    assert!(inbound.addressed_to_bot);
}

#[test]
fn a_quote_of_somebody_else_is_not_addressed() {
    let raw = envelope(
        "+15550009999",
        "what do you think?",
        serde_json::json!({
            "groupInfo": { "groupId": "grp==" },
            "quote": { "id": 1, "authorNumber": "+15550005555" },
        }),
    );
    let inbound = parse_envelope(&raw, &identity("+15550001234"), "http://d").expect("inbound");
    assert!(!inbound.addressed_to_bot);
}

#[test]
fn events_without_a_prompt_are_dropped() {
    let now = crate::bridge::dedup::now_ms();
    let cases = [
        // A reaction carries no text and no attachment.
        serde_json::json!({"envelope": {"sourceNumber": "+1", "timestamp": now,
            "dataMessage": {"timestamp": now, "reaction": {"emoji": "👍", "isRemove": false}}}}),
        // An edit is the platform's own concept, not a new prompt.
        serde_json::json!({"envelope": {"sourceNumber": "+1", "timestamp": now,
            "dataMessage": {"timestamp": now, "message": ""}}}),
        // A receipt, a typing indicator and a sync copy of our own send.
        serde_json::json!({"envelope": {"sourceNumber": "+1", "timestamp": now,
            "receiptMessage": {"when": now, "isDelivery": true}}}),
        serde_json::json!({"envelope": {"sourceNumber": "+1", "timestamp": now,
            "typingMessage": {"action": "STARTED"}}}),
        serde_json::json!({"envelope": {"sourceNumber": "+1", "timestamp": now,
            "syncMessage": {"sentMessage": {"message": "from my phone", "timestamp": now}}}}),
    ];
    for case in cases {
        assert!(
            parse_envelope(&case, &identity("+15550001234"), "http://d").is_none(),
            "should be dropped: {case}"
        );
    }
}

#[test]
fn an_envelope_without_a_sender_is_dropped() {
    let raw = serde_json::json!({"envelope": {"dataMessage": {"message": "hi", "timestamp": 1}}});
    assert!(parse_envelope(&raw, &identity("+1"), "http://d").is_none());
}

#[test]
fn a_bare_envelope_is_accepted_next_to_the_wrapped_shape() {
    let now = crate::bridge::dedup::now_ms();
    let bare = serde_json::json!({"sourceNumber": "+15550009999", "timestamp": now,
        "dataMessage": {"message": "hello", "timestamp": now}});
    let inbound = parse_envelope(&bare, &identity("+15550001234"), "http://d").expect("inbound");
    assert_eq!(inbound.text, "hello");
}

#[test]
fn the_uuid_stands_in_when_a_daemon_hides_the_number() {
    let now = crate::bridge::dedup::now_ms();
    let raw = serde_json::json!({"envelope": {"sourceUuid": "uuid-alice", "timestamp": now,
        "dataMessage": {"message": "hello", "timestamp": now}}});
    let inbound = parse_envelope(&raw, &identity("+15550001234"), "http://d").expect("inbound");
    assert_eq!(inbound.sender.id, "uuid-alice");
    assert_eq!(inbound.conversation.id, "uuid-alice");
}

#[test]
fn attachment_kinds_follow_the_content_type() {
    let attachments = vec![
        serde_json::json!({"contentType": "image/png", "filename": "shot.png", "id": "a b/c"}),
        serde_json::json!({"contentType": "audio/ogg", "id": "b"}),
        serde_json::json!({"contentType": "video/mp4", "id": "c"}),
        serde_json::json!({"contentType": "application/pdf", "id": "d"}),
        // No content type at all.
        serde_json::json!({"id": "e"}),
        // No id: nothing can be fetched, so there is nothing to record.
        serde_json::json!({"contentType": "image/png"}),
    ];
    let refs = attachment_refs(&attachments, "http://daemon");
    assert_eq!(refs.len(), 5);
    assert_eq!(refs[0].kind, MediaKind::Image);
    assert_eq!(refs[0].filename.as_deref(), Some("shot.png"));
    assert_eq!(
        refs[0].url.as_deref(),
        Some("http://daemon/api/v1/attachments/a%20b%2Fc")
    );
    assert!(refs[0].data.is_none());
    assert_eq!(refs[1].kind, MediaKind::Audio);
    assert_eq!(refs[2].kind, MediaKind::Video);
    assert_eq!(refs[3].kind, MediaKind::Document);
    assert_eq!(refs[4].kind, MediaKind::Unknown);
}

#[test]
fn an_envelope_without_a_timestamp_still_gets_a_unique_identity() {
    // The daemon normally stamps every envelope; when it does not, the dedup
    // key still has to distinguish two messages.
    let bare = serde_json::json!({"envelope": {
        "sourceNumber": "+15550009999",
        "dataMessage": {"message": "hello"}}});
    let first = parse_envelope(&bare, &identity("+15550001234"), "http://d").expect("inbound");
    let second = parse_envelope(&bare, &identity("+15550001234"), "http://d").expect("inbound");
    assert!(first.created_at_ms.is_none());
    assert_ne!(first.message_id, second.message_id);
    assert_eq!(first.text, "hello");
}

#[test]
fn an_attachment_only_message_is_not_dropped() {
    let now = crate::bridge::dedup::now_ms();
    let raw = serde_json::json!({"envelope": {"sourceNumber": "+15550009999", "timestamp": now,
        "dataMessage": {"timestamp": now, "attachments": [
            {"contentType": "image/png", "id": "a"}]}}});
    let inbound = parse_envelope(&raw, &identity("+15550001234"), "http://d").expect("inbound");
    assert!(inbound.text.is_empty());
    assert_eq!(inbound.media.len(), 1);
    assert_eq!(inbound.images().len(), 1);
}

#[test]
fn receive_answers_are_read_as_arrays_and_as_single_objects() {
    assert_eq!(parse_envelopes("[]").len(), 0);
    assert_eq!(parse_envelopes("[{\"a\":1},{\"a\":2}]").len(), 2);
    assert_eq!(parse_envelopes("{\"a\":1}").len(), 1);
    // A garbage answer is "no envelopes", never a panic.
    assert!(parse_envelopes("not json").is_empty());
    assert!(parse_envelopes("\"\"").is_empty());
}

// ─── error classification ──────────────────────────────────────────────────

#[test]
fn an_unregistered_account_or_a_rejected_number_is_permanent() {
    for (status, message) in [
        (400u16, "Account is not registered"),
        (400, "UNREGISTERED"),
        (400, "\"+15550001234\" is not a valid number"),
        (400, "unknown group"),
        (401, "unauthorized"),
        (403, "forbidden"),
        (400, "not a member of group"),
    ] {
        assert_eq!(
            classify_failure(status, message),
            ErrorClass::Permanent,
            "{message} should be permanent"
        );
    }
}

#[test]
fn timeouts_and_rate_limits_stay_retryable() {
    for (status, message) in [
        (408u16, "request timeout"),
        (429, "rate limit exceeded"),
        (503, "daemon busy"),
        (500, "connection refused while sending"),
        // A plain 500 with no useful body is the shared rule.
        (500, ""),
    ] {
        assert_eq!(
            classify_failure(status, message),
            ErrorClass::Transient,
            "{message} should be retryable"
        );
    }
}

#[test]
fn an_unclassified_status_falls_back_to_the_shared_http_rule() {
    assert_eq!(
        classify_failure(404, "no such route"),
        ErrorClass::Permanent
    );
    assert_eq!(classify_failure(502, "bad gateway"), ErrorClass::Transient);
}

// ─── sending ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn sending_posts_the_message_and_returns_the_daemon_timestamp() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        "/v2/send",
        200,
        r#"{"timestamp":1700000000123}"#,
    )])
    .await;
    let ctx =
        ctx_with(serde_json::json!({"enabled": true, "http_url": base, "number": "+15550001234"}));
    let sender = Signal.sender(&ctx).unwrap();
    let conversation = ConversationRef {
        id: "+15550009999".into(),
        thread_id: None,
        kind: ChatKind::Direct,
    };
    let id = sender.send_text(&conversation, "hello").await.unwrap();
    assert_eq!(id.as_deref(), Some("1700000000123"));

    let sent = requests_to(&recorded, "/v2/send");
    assert_eq!(sent.len(), 1, "{sent:?}");
    let body: Value = serde_json::from_str(&sent[0].body_string()).unwrap();
    assert_eq!(body["message"], "hello");
    assert_eq!(body["number"], "+15550001234");
    assert_eq!(body["recipients"][0], "+15550009999");
}

#[tokio::test]
async fn a_group_reply_names_the_group_instead_of_a_recipient() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json("/v2/send", 200, "{}")]).await;
    let ctx =
        ctx_with(serde_json::json!({"enabled": true, "http_url": base, "number": "+15550001234"}));
    let sender = Signal.sender(&ctx).unwrap();
    let conversation = ConversationRef {
        id: "group:grp==".into(),
        thread_id: None,
        kind: ChatKind::Group,
    };
    sender.send_text(&conversation, "hi all").await.unwrap();
    let sent = requests_to(&recorded, "/v2/send");
    let body: Value = serde_json::from_str(&sent[0].body_string()).unwrap();
    assert_eq!(body["group-id"], "grp==");
    assert!(body.get("recipients").is_none());
}

#[tokio::test]
async fn a_daemon_without_the_new_send_route_still_gets_the_message() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        "/api/v1/send",
        200,
        r#"{"timestamp":1}"#,
    )])
    .await;
    let ctx =
        ctx_with(serde_json::json!({"enabled": true, "http_url": base, "number": "+15550001234"}));
    let sender = Signal.sender(&ctx).unwrap();
    let conversation = ConversationRef {
        id: "+15550009999".into(),
        thread_id: None,
        kind: ChatKind::Direct,
    };
    let id = sender.send_text(&conversation, "hello").await.unwrap();
    assert_eq!(id.as_deref(), Some("1"));
    // The old route was tried first, and the fallback carried the message.
    assert_eq!(requests_to(&recorded, "/v2/send").len(), 1);
    assert_eq!(requests_to(&recorded, "/api/v1/send").len(), 1);
}

#[tokio::test]
async fn a_daemon_with_neither_send_route_fails_with_both_paths_named() {
    let (base, _recorded) = spawn_http(Vec::new()).await;
    let ctx =
        ctx_with(serde_json::json!({"enabled": true, "http_url": base, "number": "+15550001234"}));
    let sender = Signal.sender(&ctx).unwrap();
    let error = sender
        .send_text(&ConversationRef::default(), "hello")
        .await
        .expect_err("no route can work");
    let message = error.to_string();
    assert!(message.contains("/v2/send"), "{message}");
    assert!(message.contains("/api/v1/send"), "{message}");
}

#[tokio::test]
async fn a_rejected_send_reports_whether_retrying_could_help() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        "/v2/send",
        400,
        r#"{"error":"Account is not registered"}"#,
    )])
    .await;
    let ctx =
        ctx_with(serde_json::json!({"enabled": true, "http_url": base, "number": "+15550001234"}));
    let sender = Signal.sender(&ctx).unwrap();
    let error = sender
        .send_text(&ConversationRef::default(), "hello")
        .await
        .expect_err("must fail");
    let message = error.to_string();
    assert!(message.contains("permanent"), "{message}");
    assert!(message.contains("not registered"), "{message}");
}

#[tokio::test]
async fn a_reply_longer_than_the_limit_is_split_across_sends() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json("/v2/send", 200, "{}")]).await;
    let ctx =
        ctx_with(serde_json::json!({"enabled": true, "http_url": base, "number": "+15550001234"}));
    let sender = Signal.sender(&ctx).unwrap();
    let long = "word ".repeat(1_200);
    sender
        .send_text(&ConversationRef::default(), &long)
        .await
        .unwrap();
    let sent = requests_to(&recorded, "/v2/send");
    assert!(
        sent.len() >= 2,
        "expected a split, got {} sends",
        sent.len()
    );
    for request in &sent {
        let body: Value = serde_json::from_str(&request.body_string()).unwrap();
        let piece = body["message"].as_str().unwrap();
        assert!(piece.chars().count() <= DEFINITION.max_text_len);
    }
}

#[tokio::test]
async fn a_reaction_needs_the_numeric_timestamp_of_the_message() {
    let ctx = ctx_with(serde_json::json!({"enabled": true, "number": "+15550001234"}));
    let sender = Signal.sender(&ctx).unwrap();
    let error = sender
        .react(&ConversationRef::default(), "not-a-timestamp", "👍")
        .await
        .expect_err("must fail");
    assert!(error.to_string().contains("timestamp"), "{error}");
}

// ─── the daemon's JSON-RPC surface ─────────────────────────────────────────

/// A server that answers without a `Content-Length`, so the size of the body is
/// only knowable by reading to EOF.
async fn spawn_lengthless_server(body_len: usize) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut head = [0u8; 1024];
                let _ = socket.read(&mut head).await;
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                let chunk = vec![0u8; 64 * 1024];
                let mut written = 0usize;
                while written < body_len {
                    let take = chunk.len().min(body_len - written);
                    if socket.write_all(&chunk[..take]).await.is_err() {
                        return;
                    }
                    written += take;
                }
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://127.0.0.1:{}", addr.port())
}

#[tokio::test]
async fn a_typing_indicator_is_sent_through_the_rpc_route() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        "/api/v1/rpc",
        200,
        r#"{"jsonrpc":"2.0","id":"sendTyping","result":{}}"#,
    )])
    .await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234",
    }));
    let sender = Signal.sender(&ctx).unwrap();

    // A direct conversation addresses the recipient; a group names the group.
    // Both shapes come from the same helper, so both are checked here.
    sender
        .typing(&ConversationRef {
            id: "+15550009999".into(),
            thread_id: None,
            kind: ChatKind::Direct,
        })
        .await
        .unwrap();
    sender
        .typing(&ConversationRef {
            id: "group:grp==".into(),
            thread_id: None,
            kind: ChatKind::Group,
        })
        .await
        .unwrap();

    let calls = requests_to(&recorded, "/api/v1/rpc");
    assert_eq!(calls.len(), 2, "{calls:?}");
    let direct: Value = serde_json::from_str(&calls[0].body_string()).unwrap();
    assert_eq!(direct["method"], "sendTyping");
    assert_eq!(direct["params"]["account"], "+15550001234");
    assert_eq!(direct["params"]["recipient"][0], "+15550009999");
    assert_eq!(direct["params"]["stop"], false);
    let group: Value = serde_json::from_str(&calls[1].body_string()).unwrap();
    assert_eq!(group["params"]["groupId"], "grp==");
    assert!(group["params"].get("recipient").is_none());
}

#[tokio::test]
async fn a_reaction_names_the_message_and_its_author() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        "/api/v1/rpc",
        200,
        r#"{"jsonrpc":"2.0","id":"sendReaction","result":{}}"#,
    )])
    .await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234",
    }));
    let sender = Signal.sender(&ctx).unwrap();
    sender
        .react(&ConversationRef::default(), "1700000000123", "👍")
        .await
        .unwrap();

    let calls = requests_to(&recorded, "/api/v1/rpc");
    let body: Value = serde_json::from_str(&calls[0].body_string()).unwrap();
    assert_eq!(body["method"], "sendReaction");
    assert_eq!(body["params"]["reaction"], "👍");
    // A reaction targets a specific message by timestamp and author.
    assert_eq!(body["params"]["targetTimestamp"], 1700000000123i64);
    assert_eq!(body["params"]["targetAuthor"], "+15550001234");
}

#[tokio::test]
async fn a_refused_rpc_call_reports_why() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        "/api/v1/rpc",
        400,
        r#"{"error":"Account is not registered"}"#,
    )])
    .await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234",
    }));
    let sender = Signal.sender(&ctx).unwrap();
    let error = sender
        .typing(&ConversationRef::default())
        .await
        .expect_err("must fail");
    let message = error.to_string();
    assert!(message.contains("sendTyping"), "{message}");
    assert!(message.contains("permanent"), "{message}");
}

// ─── the receive route’s two shapes ────────────────────────────────────────

#[tokio::test]
async fn a_daemon_with_only_the_older_receive_route_is_still_polled() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        "/v1/receive/%2B15550001234",
        200,
        "[]",
    )])
    .await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234",
    }));
    let config: SignalConfig = ctx.config().unwrap();
    let sender = SignalSender::new(&ctx, &config);
    let envelopes = sender
        .receive(&config)
        .await
        .expect("the old route answers");
    assert!(envelopes.is_empty());
    // The newer path was tried first and is not there; the older one answered.
    assert_eq!(
        requests_to(&recorded, "/api/v1/receive/%2B15550001234").len(),
        1
    );
    assert_eq!(
        requests_to(&recorded, "/v1/receive/%2B15550001234").len(),
        1
    );
}

#[tokio::test]
async fn a_receive_failure_is_classified_for_the_caller() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        "/api/v1/receive/%2B15550001234",
        500,
        r#"{"error":"daemon is busy"}"#,
    )])
    .await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234",
    }));
    let config: SignalConfig = ctx.config().unwrap();
    let sender = SignalSender::new(&ctx, &config);
    let error = sender
        .receive(&config)
        .await
        .expect_err("a 500 is not a poll");
    let message = error.to_string();
    assert!(message.contains("receive"), "{message}");
    // A busy daemon is worth another try, so the text must say so.
    assert!(message.contains("transient"), "{message}");
    assert!(!crate::delivery::is_permanent_error(&message), "{message}");
}

#[tokio::test]
async fn a_daemon_with_neither_receive_route_says_so() {
    let (base, _recorded) = spawn_http(Vec::new()).await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234",
    }));
    let config: SignalConfig = ctx.config().unwrap();
    let sender = SignalSender::new(&ctx, &config);
    let error = sender
        .receive(&config)
        .await
        .expect_err("no route can poll");
    let message = error.to_string();
    assert!(message.contains("route"), "{message}");
}

// ─── attachments ───────────────────────────────────────────────────────────

#[tokio::test]
async fn an_attachment_above_the_size_cap_is_refused_from_its_length() {
    // The declared length is enough: nothing that big is read into memory.
    let (base, _recorded) = spawn_http(vec![HttpRoute::binary(
        "/api/v1/attachments/huge",
        200,
        vec![0u8; MAX_MEDIA_BYTES + 1],
    )])
    .await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234",
    }));
    let sender = SignalSender::new(&ctx, &ctx.config::<SignalConfig>().unwrap());
    let error = sender
        .download(&format!("{base}/api/v1/attachments/huge"))
        .await
        .expect_err("too large to keep");
    assert!(error.to_string().contains("larger than"), "{error}");
}

#[tokio::test]
async fn an_attachment_is_capped_even_when_the_length_is_hidden() {
    // A daemon that streams the body leaves the client to find the size by
    // reading it, so the cap is enforced a second time after the read.
    let base = spawn_lengthless_server(MAX_MEDIA_BYTES + 1).await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234",
    }));
    let sender = SignalSender::new(&ctx, &ctx.config::<SignalConfig>().unwrap());
    let error = sender
        .download(&format!("{base}/api/v1/attachments/streamed"))
        .await
        .expect_err("too large to keep");
    assert!(error.to_string().contains("larger than"), "{error}");
}

// ─── attachments ───────────────────────────────────────────────────────────

#[tokio::test]
async fn an_image_attachment_is_downloaded_into_model_input() {
    let (base, recorded) = spawn_http(vec![HttpRoute::binary(
        "/api/v1/attachments/abc",
        200,
        vec![0x89, 0x50, 0x4e, 0x47],
    )])
    .await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234",
    }));
    let sender = SignalSender::new(&ctx, &ctx.config::<SignalConfig>().unwrap());

    // The whole way from a daemon envelope to something the model can read.
    let now = crate::bridge::dedup::now_ms();
    let raw = serde_json::json!({"envelope": {
        "sourceNumber": "+15550009999", "timestamp": now,
        "dataMessage": {"timestamp": now, "attachments": [
            {"contentType": "image/png", "filename": "shot.png", "id": "abc"}]}}});
    let mut inbound =
        parse_envelope(&raw, &identity("+15550001234"), &sender.base).expect("inbound");
    assert!(inbound.images()[0].data.is_none(), "bytes arrive later");

    hydrate_media(&sender, &mut inbound).await;

    assert_eq!(
        inbound.media[0].data.as_deref(),
        Some([0x89, 0x50, 0x4e, 0x47].as_slice())
    );
    // The bytes are what the bridge turns into model input.
    assert_eq!(inbound.images().len(), 1);
    assert!(inbound.images()[0].data.is_some());
    assert_eq!(requests_to(&recorded, "/api/v1/attachments/abc").len(), 1);
}

#[tokio::test]
async fn a_download_that_fails_keeps_the_reference_instead_of_dropping_it() {
    // No route for the attachment, so the daemon answers 404.
    let (base, _recorded) = spawn_http(Vec::new()).await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234",
    }));
    let sender = SignalSender::new(&ctx, &ctx.config::<SignalConfig>().unwrap());
    let now = crate::bridge::dedup::now_ms();
    let raw = serde_json::json!({"envelope": {
        "sourceNumber": "+15550009999", "timestamp": now,
        "dataMessage": {"timestamp": now, "message": "look", "attachments": [
            {"contentType": "image/png", "id": "gone"}]}}});
    let mut inbound =
        parse_envelope(&raw, &identity("+15550001234"), &sender.base).expect("inbound");

    hydrate_media(&sender, &mut inbound).await;

    // The message must still be answered; only the bytes are missing.
    assert!(inbound.media[0].data.is_none());
    assert_eq!(
        inbound.media[0].url.as_deref(),
        Some(format!("{base}/api/v1/attachments/gone").as_str())
    );
    assert_eq!(inbound.text, "look");
}

#[tokio::test]
async fn only_images_are_fetched_and_a_reference_is_never_fetched_twice() {
    let (base, recorded) = spawn_http(vec![HttpRoute::binary(
        "/api/v1/attachments/abc",
        200,
        vec![1, 2, 3],
    )])
    .await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234",
    }));
    let sender = SignalSender::new(&ctx, &ctx.config::<SignalConfig>().unwrap());
    let mut inbound = Inbound::new_direct("m1", "+15550009999", "+15550009999", "hi");
    inbound.media = vec![
        // A document is not model input, so it is not worth a round trip.
        MediaRef {
            kind: MediaKind::Document,
            url: Some(format!("{base}/api/v1/attachments/doc")),
            ..Default::default()
        },
        MediaRef {
            kind: MediaKind::Image,
            url: Some(format!("{base}/api/v1/attachments/abc")),
            ..Default::default()
        },
        // Already downloaded (the daemon may repeat an envelope).
        MediaRef {
            kind: MediaKind::Image,
            url: Some(format!("{base}/api/v1/attachments/abc")),
            data: Some(vec![9]),
            ..Default::default()
        },
    ];

    hydrate_media(&sender, &mut inbound).await;

    assert_eq!(
        inbound.media[0].data, None,
        "documents are left as references"
    );
    assert_eq!(inbound.media[1].data, Some(vec![1, 2, 3]));
    assert_eq!(inbound.media[2].data, Some(vec![9]), "already fetched");
    // Exactly one request: no document fetch, no second image fetch.
    assert_eq!(requests_to(&recorded, "/api/v1/attachments/abc").len(), 1);
    assert!(requests_to(&recorded, "/api/v1/attachments/doc").is_empty());
}

#[tokio::test]
async fn an_image_with_neither_bytes_nor_a_reference_is_left_alone() {
    let (base, recorded) = spawn_http(Vec::new()).await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234",
    }));
    let sender = SignalSender::new(&ctx, &ctx.config::<SignalConfig>().unwrap());
    let mut inbound = Inbound::new_direct("m1", "+15550009999", "+15550009999", "hi");
    // An image the provider could not even name a URL for: there is nothing to
    // fetch, and the message must survive untouched.
    inbound.media = vec![MediaRef {
        kind: MediaKind::Image,
        ..Default::default()
    }];

    hydrate_media(&sender, &mut inbound).await;

    assert_eq!(inbound.media.len(), 1);
    assert!(inbound.media[0].url.is_none());
    assert!(inbound.media[0].data.is_none());
    assert!(
        recorded.lock().unwrap().is_empty(),
        "nothing may be requested"
    );
}

#[tokio::test]
async fn a_sender_reports_the_signal_declaration() {
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "number": "+15550001234",
    }));
    let sender = Signal.sender(&ctx).unwrap();
    assert_eq!(sender.definition().id, "signal");
    // The bridge reads the split limit from here, so it must be the real one.
    assert_eq!(sender.definition().max_text_len, DEFINITION.max_text_len);
}

// ─── probing ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_probe_lists_accounts_without_consuming_queued_messages() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        "/api/v1/rpc",
        200,
        r#"{"jsonrpc":"2.0","id":"listAccounts","result":[{"number":"+15550009999"},{"number":"+15550001234","uuid":"uuid-bot"}]}"#,
    )])
    .await;
    let ctx =
        ctx_with(serde_json::json!({"enabled": true, "http_url": base, "number": "+15550001234"}));
    let summary = Signal.probe(&ctx).await.unwrap();
    assert!(summary.contains("connected"), "{summary}");
    assert!(summary.contains("+15550001234"), "{summary}");
    // Nothing a user sent may be consumed by a diagnostic.
    assert!(
        requests_to(&recorded, "/api/v1/receive/%2B15550001234").is_empty(),
        "a probe must not drain the inbox"
    );
}

#[tokio::test]
async fn a_probe_rejects_an_account_the_daemon_does_not_know() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        "/api/v1/rpc",
        200,
        r#"{"jsonrpc":"2.0","id":"listAccounts","result":[{"number":"+15550009999"}]}"#,
    )])
    .await;
    let ctx =
        ctx_with(serde_json::json!({"enabled": true, "http_url": base, "number": "+15550001234"}));
    let error = Signal
        .probe(&ctx)
        .await
        .expect_err("must not claim success");
    let message = error.to_string();
    assert!(message.contains("+15550001234"), "{message}");
    assert!(message.contains("+15550009999"), "{message}");
}

#[tokio::test]
async fn a_probe_never_claims_success_when_the_daemon_is_unreachable() {
    let ctx = ctx_with(
        serde_json::json!({"enabled": true, "http_url": "http://127.0.0.1:1", "number": "+15550001234"}),
    );
    let error = Signal.probe(&ctx).await.expect_err("must fail");
    assert!(error.to_string().contains("cannot reach"), "{error}");
}

#[test]
fn account_answers_are_read_from_both_shapes() {
    let modern: Value =
        serde_json::from_str(r#"{"result":[{"number":"+1","uuid":"u1"}]}"#).unwrap();
    assert_eq!(accounts_of(&modern).unwrap(), vec!["+1", "u1"]);
    let old: Value = serde_json::from_str(r#"["+1"]"#).unwrap();
    assert_eq!(accounts_of(&old).unwrap(), vec!["+1"]);
    // A non-array answer is "no information", not "no accounts".
    assert!(accounts_of(&serde_json::json!({"result": "nope"})).is_none());
    assert_eq!(
        accounts_of(&serde_json::json!({"result": []}))
            .unwrap()
            .len(),
        0
    );
    // Entries that are neither an object nor a string carry no account, and an
    // object with neither field contributes nothing either.
    let mixed: Value = serde_json::from_str(r#"{"result":[42,null,"+1"]}"#).unwrap();
    assert_eq!(accounts_of(&mixed).unwrap(), vec!["+1"]);
    let named: Value = serde_json::from_str(r#"{"result":[{"nickname":"home"},"+2"]}"#).unwrap();
    assert_eq!(accounts_of(&named).unwrap(), vec!["+2"]);
}

// ─── the receive loop ──────────────────────────────────────────────────────

/// The route a receive poll hits, with the number percent-encoded.
fn receive_route(body: &'static str) -> HttpRoute {
    HttpRoute::json("/api/v1/receive/%2B15550001234", 200, body)
}

#[tokio::test]
async fn a_received_envelope_reaches_the_sender() {
    let now = crate::bridge::dedup::now_ms();
    let envelope = format!(
        r#"[{{"envelope":{{"sourceNumber":"+15550009999","sourceName":"Alice","timestamp":{now},"dataMessage":{{"message":"hello","timestamp":{now}}}}}}}]"#
    );
    let (base, recorded) = spawn_http(vec![
        HttpRoute::json(
            "/api/v1/receive/%2B15550001234",
            200,
            Box::leak(envelope.into_boxed_str()),
        ),
        // The startup probe fails harmlessly (no such route) so the loop is
        // what is under test.
        HttpRoute::json("/v2/send", 200, "{}"),
    ])
    .await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234", "poll_interval_ms": 50,
    }));
    let task = {
        let ctx = ctx.clone();
        tokio::spawn(async move { Signal.run(ctx).await })
    };
    // The agent is unreachable in tests, so the turn fails and the bridge tells
    // the sender; that reply is the proof the envelope travelled the pipeline.
    let delivered = wait_until(
        || !requests_to(&recorded, "/v2/send").is_empty(),
        Duration::from_secs(5),
    )
    .await;
    ctx.shutdown().notify_waiters();
    let stopped = tokio::time::timeout(Duration::from_secs(5), task).await;
    assert!(delivered, "the received message never reached the sender");
    assert!(stopped.is_ok(), "the receive loop must stop on shutdown");
}

#[tokio::test]
async fn the_receive_loop_stops_promptly_on_shutdown() {
    let (base, recorded) = spawn_http(vec![receive_route("[]")]).await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234", "poll_interval_ms": 50,
    }));
    let task = {
        let ctx = ctx.clone();
        tokio::spawn(async move { Signal.run(ctx).await })
    };
    let polled = wait_until(
        || !requests_to(&recorded, "/api/v1/receive/%2B15550001234").is_empty(),
        Duration::from_secs(5),
    )
    .await;
    ctx.shutdown().notify_waiters();
    let result = tokio::time::timeout(Duration::from_secs(5), task).await;
    assert!(polled, "the loop never polled");
    assert!(result.is_ok(), "run() must return on shutdown");
    assert!(result.unwrap().unwrap().is_ok());
}

#[tokio::test]
async fn an_idle_daemon_is_polled_and_not_hammered() {
    let (base, recorded) = spawn_http(vec![receive_route("[]")]).await;
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": base, "number": "+15550001234", "poll_interval_ms": 200,
    }));
    let task = {
        let ctx = ctx.clone();
        tokio::spawn(async move { Signal.run(ctx).await })
    };
    tokio::time::sleep(Duration::from_millis(700)).await;
    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
    let polls = requests_to(&recorded, "/api/v1/receive/%2B15550001234").len();
    // An empty answer must not turn the loop into a busy one: ~3 polls fit in
    // 700ms at one per 200ms, far from the hundreds a hot loop would make.
    assert!(polls <= 6, "the poll loop is too hot: {polls} polls");
    assert!(polls >= 1, "the loop never polled");
}

#[tokio::test]
async fn an_unreachable_daemon_keeps_the_channel_retryable() {
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "http_url": "http://127.0.0.1:1", "number": "+15550001234",
    }));
    // `run` must not abort the channel on the first failure: the supervisor
    // would then reconnect in a tight loop.
    let task = {
        let ctx = ctx.clone();
        tokio::spawn(async move { Signal.run(ctx).await })
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!task.is_finished(), "run() gave up instead of retrying");
    ctx.shutdown().notify_waiters();
    let result = tokio::time::timeout(Duration::from_secs(5), task).await;
    assert!(result.is_ok(), "run() must stop on shutdown");
}
