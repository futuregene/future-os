//! Unit tests for the Linq provider.
//!
//! The platform-specific decisions the bridge does not make: signature
//! verification (both the current signed-request scheme and the deprecated one),
//! replay rejection, event parsing across both documented payload versions,
//! mention detection in group chats, the two outbound shapes, attachment
//! fetching, and the numbered error table.
//!
//! Outbound tests run against [`crate::test_support::spawn_http`]; nothing here
//! contacts the platform, and the one end-to-end test drives a mock agent.

use super::*;
use crate::test_support::{requests_to, spawn_http, HttpRoute};

// ─── Fixtures ───────────────────────────────────────────────────────────────

const SECRET: &str = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw7Jxx2Oll+OE=";
const CHAT_ID: &str = "8f392755-6865-4b18-880a-227f9d8b458f";
const MESSAGE_ID: &str = "89e3566e-1d13-49e5-a8ee-48490d5bfeb7";
const CUSTOMER: &str = "+12025559876";

/// Seconds since the epoch, for signatures made "now".
fn now_seconds() -> i64 {
    crate::bridge::dedup::now_ms() / 1000
}

fn webhook_post(body: &[u8]) -> WebhookRequest {
    let mut headers = std::collections::HashMap::new();
    headers.insert("content-type".to_string(), "application/json".to_string());
    WebhookRequest {
        method: "POST".into(),
        path: "/webhooks/linq".into(),
        query: String::new(),
        headers,
        body: body.to_vec(),
    }
}

/// Sign a body the way the platform does: `{id}.{timestamp}.{body}` under the
/// base64-decoded secret, base64-encoded, prefixed `v1,`.
fn signed_request(body: &[u8], id: &str, timestamp: &str, secret: &str) -> WebhookRequest {
    let digest = hmac_sha256(&signing_key(secret), &signed_content(id, timestamp, body));
    let mut request = webhook_post(body);
    request
        .headers
        .insert("webhook-id".to_string(), id.to_string());
    request
        .headers
        .insert("webhook-timestamp".to_string(), timestamp.to_string());
    request.headers.insert(
        "webhook-signature".to_string(),
        format!("v1,{}", STANDARD.encode(digest)),
    );
    request
}

/// A `message.received` delivery in the current payload version.
fn received_message(data: Value) -> Value {
    json!({
        "api_version": "v3",
        "webhook_version": "2026-02-03",
        "event_type": "message.received",
        "event_id": "2915e81c-5068-4796-ace2-21d2c94ad298",
        "created_at": "2026-02-05T19:31:13.736Z",
        "trace_id": "8af9171a45022df2eb74ba4e4c83be0f",
        "partner_id": "partner-1",
        "data": merge(
            json!({
                "chat": { "id": CHAT_ID, "is_group": false },
                "id": MESSAGE_ID,
                "direction": "inbound",
                "sender_handle": { "handle": CUSTOMER, "id": "e604375a-5913-483a-8278-c631e8f0ffda" },
                "parts": [{ "type": "text", "value": "Hello!" }],
                "sent_at": "2026-02-05T19:31:13.074Z",
            }),
            data,
        ),
    })
}

/// Shallow-merge two objects (the second wins), for fixture overrides.
fn merge(base: Value, overrides: Value) -> Value {
    let mut base = base;
    if let (Some(base), Some(overrides)) = (base.as_object_mut(), overrides.as_object()) {
        for (key, value) in overrides {
            base.insert(key.clone(), value.clone());
        }
    }
    base
}

fn sender_against(base: &str, from: &str) -> LinqSender {
    LinqSender::new(
        reqwest::Client::new(),
        &LinqConfig {
            api_key: "key".into(),
            from: from.into(),
            webhook: WebhookConfig::default(),
            api_base: base.to_string(),
            api_version: "v3".into(),
        },
    )
}

fn chat_conversation() -> ConversationRef {
    ConversationRef {
        id: CHAT_ID.into(),
        thread_id: None,
        kind: ChatKind::Direct,
    }
}

fn handle_conversation() -> ConversationRef {
    ConversationRef {
        id: CUSTOMER.into(),
        thread_id: None,
        kind: ChatKind::Direct,
    }
}

fn ctx_with_config(config: Value) -> ProviderCtx {
    use std::sync::Arc;
    ProviderCtx::new(
        &DEFINITION,
        config,
        crate::bridge::Bridge::offline(),
        crate::test_support::temp_dir("linq-ctx"),
        Arc::new(crate::session_store::SessionStore::new(
            crate::test_support::temp_dir("linq-sessions").join("sessions.json"),
        )),
        crate::bridge::Shutdown::new(),
    )
}

/// A context wired to a mock agent with an open policy, for the tests that
/// drive `run` and `deliver` end to end.
fn ctx_with_agent(grpc_addr: &str, data_dir: &std::path::Path, config: Value) -> ProviderCtx {
    use std::sync::Arc;
    let sessions = Arc::new(crate::session_store::SessionStore::new(
        data_dir.join("sessions.json"),
    ));
    let agent_cfg = crate::config::AgentConfig {
        grpc_addr: grpc_addr.to_string(),
        cwd: data_dir.to_string_lossy().into_owned(),
        ..crate::config::AgentConfig::default()
    };
    ProviderCtx::new(
        &DEFINITION,
        config,
        crate::bridge::Bridge::new(
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
        ),
        data_dir.to_path_buf(),
        sessions,
        crate::bridge::Shutdown::new(),
    )
}

// ─── Configuration ──────────────────────────────────────────────────────────

#[test]
fn the_definition_describes_the_api_honestly() {
    assert_eq!(DEFINITION.id, "linq");
    assert_eq!(DEFINITION.maturity, Maturity::Preview);
    assert!(DEFINITION.is_implemented());
    assert_eq!(DEFINITION.length_unit, LengthUnit::Chars);
    // A chat is a conversation, not a thread.
    assert!(!DEFINITION.capabilities.threads);
    // Mentions are reported on a text part, so a group gate can be enforced.
    assert!(DEFINITION.capabilities.mention_gate);
    assert!(DEFINITION.capabilities.media_in);
}

#[test]
fn a_long_reply_is_split_within_the_platform_limit() {
    let pieces = crate::transport::chunk(
        &"x".repeat(9000),
        DEFINITION.max_text_len,
        DEFINITION.length_unit,
    );
    assert_eq!(pieces.len(), 3, "{pieces:?}");
    for piece in &pieces {
        assert!(piece.chars().count() <= DEFINITION.max_text_len);
    }
}

#[test]
fn defaults_cover_the_origin_version_and_webhook_route() {
    let empty = LinqConfig::default();
    assert_eq!(empty.api_base(), "https://api.linqapp.com/api/partner");
    assert_eq!(empty.api_version(), "v3");
    assert_eq!(empty.webhook_addr(), "127.0.0.1:8789");
    assert_eq!(empty.webhook_path(), "/webhooks/linq");
    let configured = LinqConfig {
        api_base: "https://example.test/".into(),
        api_version: "v4".into(),
        webhook: WebhookConfig {
            addr: "127.0.0.1:9".into(),
            path: "/linq".into(),
            signing_secret: SECRET.into(),
        },
        ..LinqConfig::default()
    };
    assert_eq!(configured.api_base(), "https://example.test");
    assert_eq!(configured.api_version(), "v4");
    assert_eq!(configured.webhook_addr(), "127.0.0.1:9");
    assert_eq!(configured.webhook_path(), "/linq");
}

#[test]
fn an_api_key_and_a_signing_secret_are_required() {
    let complete = LinqConfig {
        api_key: "key".into(),
        webhook: WebhookConfig {
            signing_secret: SECRET.into(),
            ..WebhookConfig::default()
        },
        ..LinqConfig::default()
    };
    assert!(complete.require_api_key("linq").is_ok());
    assert!(complete.require_signing_secret("linq").is_ok());

    let no_key = LinqConfig {
        ..LinqConfig::default()
    };
    let error = no_key.require_api_key("linq").unwrap_err();
    assert!(error.to_string().contains("api_key"), "{error}");
    // Receiving unsigned would accept anyone's POST, so it is refused.
    let unsigned = LinqConfig {
        api_key: "key".into(),
        ..LinqConfig::default()
    };
    let error = unsigned.require_signing_secret("linq").unwrap_err();
    assert!(error.to_string().contains("signing_secret"), "{error}");
}

// ─── Signature verification ─────────────────────────────────────────────────

#[test]
fn a_correctly_signed_delivery_is_accepted() {
    let body = serde_json::to_vec(&received_message(json!({}))).unwrap();
    let request = signed_request(&body, "evt-1", &now_seconds().to_string(), SECRET);
    assert!(verify_delivery(&request, SECRET, now_seconds()));
}

#[test]
fn a_missing_signature_is_rejected() {
    let body = br#"{"event_type":"message.received","data":{}}"#;
    let bare = webhook_post(body);
    assert!(!verify_delivery(&bare, SECRET, now_seconds()));

    // A header holding an empty value is not a signature.
    let mut empty = webhook_post(body);
    empty
        .headers
        .insert("webhook-signature".to_string(), String::new());
    empty
        .headers
        .insert("webhook-id".to_string(), "evt-1".to_string());
    empty
        .headers
        .insert("webhook-timestamp".to_string(), now_seconds().to_string());
    assert!(!verify_delivery(&empty, SECRET, now_seconds()));
}

#[test]
fn a_signature_from_another_secret_or_over_other_bytes_is_rejected() {
    let body = serde_json::to_vec(&received_message(json!({}))).unwrap();
    let other_secret = "whsec_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    let foreign = signed_request(&body, "evt-1", &now_seconds().to_string(), other_secret);
    assert!(!verify_delivery(&foreign, SECRET, now_seconds()));

    // The digest covers the body as sent, so a tampered body fails.
    let signature = signed_request(&body, "evt-1", &now_seconds().to_string(), SECRET)
        .header("webhook-signature")
        .unwrap()
        .to_string();
    let mut tampered = webhook_post(br#"{"event_type":"message.received","data":{"id":"x"}}"#);
    tampered
        .headers
        .insert("webhook-id".to_string(), "evt-1".to_string());
    tampered
        .headers
        .insert("webhook-timestamp".to_string(), now_seconds().to_string());
    tampered
        .headers
        .insert("webhook-signature".to_string(), signature);
    assert!(!verify_delivery(&tampered, SECRET, now_seconds()));
}

#[test]
fn a_stale_or_undated_delivery_is_rejected() {
    let body = br#"{"event_type":"message.received","data":{}}"#;
    // A replay hours later must not be accepted, even though the digest is
    // genuine.
    let old = (now_seconds() - REPLAY_WINDOW_SECONDS - 60).to_string();
    assert!(!verify_delivery(
        &signed_request(body, "evt-1", &old, SECRET),
        SECRET,
        now_seconds()
    ));
    // A clock a little ahead is tolerated.
    let ahead = (now_seconds() + 30).to_string();
    assert!(verify_delivery(
        &signed_request(body, "evt-1", &ahead, SECRET),
        SECRET,
        now_seconds()
    ));
    // A timestamp that is not a number is not a signature of anything.
    assert!(!verify_delivery(
        &signed_request(body, "evt-1", "soon", SECRET),
        SECRET,
        now_seconds()
    ));
}

#[test]
fn a_delivery_without_the_id_or_timestamp_header_is_rejected() {
    let body = br#"{"event_type":"message.received","data":{}}"#;
    let mut no_id = signed_request(body, "evt-1", &now_seconds().to_string(), SECRET);
    no_id.headers.remove("webhook-id");
    assert!(!verify_delivery(&no_id, SECRET, now_seconds()));

    let mut no_timestamp = signed_request(body, "evt-1", &now_seconds().to_string(), SECRET);
    no_timestamp.headers.remove("webhook-timestamp");
    assert!(!verify_delivery(&no_timestamp, SECRET, now_seconds()));
}

#[test]
fn any_matching_candidate_in_the_signature_header_is_accepted() {
    // Rotation sends several candidates; one of them is the current key.
    let body = br#"{"event_type":"message.received","data":{}}"#;
    let timestamp = now_seconds().to_string();
    let other = STANDARD.encode(hmac_sha256(&signing_key("whsec_other"), b"nope"));
    let mut request = signed_request(body, "evt-1", &timestamp, SECRET);
    let good = request.header("webhook-signature").unwrap().to_string();
    request.headers.insert(
        "webhook-signature".to_string(),
        format!("v1,{other} v0,ignored {good}"),
    );
    assert!(verify_delivery(&request, SECRET, now_seconds()));
}

#[test]
fn the_deprecated_header_set_still_verifies() {
    let body = br#"{"event_type":"message.received","data":{}}"#;
    let mut legacy = webhook_post(body);
    legacy.headers.insert(
        "x-webhook-signature".to_string(),
        hmac_sha256_hex(SECRET.as_bytes(), body),
    );
    legacy
        .headers
        .insert("x-webhook-timestamp".to_string(), now_seconds().to_string());
    assert!(verify_delivery(&legacy, SECRET, now_seconds()));

    // Without the timestamp the digest still has to be right.
    let mut no_timestamp = webhook_post(body);
    no_timestamp.headers.insert(
        "x-webhook-signature".to_string(),
        hmac_sha256_hex(SECRET.as_bytes(), body),
    );
    assert!(verify_delivery(&no_timestamp, SECRET, now_seconds()));

    // A stale legacy timestamp is a replay like any other.
    let mut old = webhook_post(body);
    old.headers.insert(
        "x-webhook-signature".to_string(),
        hmac_sha256_hex(SECRET.as_bytes(), body),
    );
    old.headers.insert(
        "x-webhook-timestamp".to_string(),
        (now_seconds() - REPLAY_WINDOW_SECONDS - 1).to_string(),
    );
    assert!(!verify_delivery(&old, SECRET, now_seconds()));

    // A wrong digest fails.
    let mut wrong = webhook_post(body);
    wrong
        .headers
        .insert("x-webhook-signature".to_string(), "deadbeef".to_string());
    assert!(!verify_delivery(&wrong, SECRET, now_seconds()));
}

#[test]
fn the_current_scheme_is_not_downgraded_to_the_deprecated_one() {
    // Both headers present, only the legacy digest right: the delivery is
    // rejected rather than accepted through the weaker of the two.
    let body = br#"{"event_type":"message.received","data":{}}"#;
    let mut request = signed_request(body, "evt-1", &now_seconds().to_string(), SECRET);
    request.headers.insert(
        "webhook-signature".to_string(),
        "v1,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
    );
    request.headers.insert(
        "x-webhook-signature".to_string(),
        hmac_sha256_hex(SECRET.as_bytes(), body),
    );
    assert!(!verify_delivery(&request, SECRET, now_seconds()));
}

#[test]
fn an_unconfigured_secret_never_verifies() {
    let body = br#"{"event_type":"message.received","data":{}}"#;
    let request = signed_request(body, "evt-1", &now_seconds().to_string(), "");
    assert!(!verify_delivery(&request, "", now_seconds()));
}

#[test]
fn the_signing_key_accepts_a_prefixed_base64_secret_and_a_plain_one() {
    // The documented form: prefix plus base64 key bytes.
    assert_eq!(signing_key("whsec_AQID"), vec![1u8, 2, 3]);
    // A secret configured without the prefix is decoded the same way.
    assert_eq!(signing_key("AQID"), vec![1u8, 2, 3]);
    // Something that is not base64 at all is used verbatim rather than
    // silently turning into an empty key.
    assert_eq!(signing_key("not!base64!"), b"not!base64!".to_vec());
    assert_eq!(signing_key(""), b"".to_vec());
}

// ─── Event parsing ──────────────────────────────────────────────────────────

#[test]
fn a_received_message_becomes_a_direct_conversation() {
    let inbounds = parse_deliveries(&received_message(json!({})));
    assert_eq!(inbounds.len(), 1);
    let inbound = &inbounds[0];
    assert_eq!(inbound.message_id, MESSAGE_ID);
    assert_eq!(inbound.text, "Hello!");
    assert_eq!(inbound.conversation.id, CHAT_ID);
    assert_eq!(inbound.conversation.kind, ChatKind::Direct);
    assert!(inbound.conversation.thread_id.is_none());
    // Policy matches on the participant's handle, which is what an operator can
    // put in an allowlist.
    assert_eq!(inbound.sender.id, CUSTOMER);
    assert!(inbound.addressed_to_bot);
    assert_eq!(inbound.created_at_ms, Some(1_770_319_873_074));
}

#[test]
fn the_previous_payload_version_parses_the_same_way() {
    // The older version nests the same facts differently and reports the
    // direction as a boolean.
    let event = json!({
        "api_version": "v3",
        "webhook_version": "2025-01-01",
        "event_type": "message.received",
        "event_id": "2915e81c-5068-4796-ace2-21d2c94ad298",
        "created_at": "2026-02-05T19:31:13.736Z",
        "data": {
            "chat_id": CHAT_ID,
            "from": CUSTOMER,
            "from_handle": { "handle": CUSTOMER, "id": "e604375a-5913-483a-8278-c631e8f0ffda" },
            "is_from_me": false,
            "is_group": false,
            "message": {
                "id": MESSAGE_ID,
                "created_at": "2026-02-05T19:31:12.892Z",
                "sent_at": "2026-02-05T19:31:13.074Z",
                "parts": [{ "type": "text", "value": "Hello!" }],
            },
            "received_at": "2026-02-05T19:31:13.074Z",
        },
    });
    let inbounds = parse_deliveries(&event);
    assert_eq!(inbounds.len(), 1);
    assert_eq!(inbounds[0].text, "Hello!");
    assert_eq!(inbounds[0].message_id, MESSAGE_ID);
    assert_eq!(inbounds[0].conversation.id, CHAT_ID);
    assert_eq!(inbounds[0].sender.id, CUSTOMER);
    assert!(inbounds[0].addressed_to_bot);
    // `sent_at` is present in both versions and wins over the envelope.
    assert_eq!(inbounds[0].created_at_ms, Some(1_770_319_873_074));
}

#[test]
fn our_own_messages_are_never_prompts() {
    // Answering the events our own sends produce would make the bot talk to
    // itself.
    let outbound = received_message(json!({ "direction": "outbound" }));
    assert!(parse_deliveries(&outbound).is_empty());

    let mut legacy = received_message(json!({}));
    legacy["data"] = json!({
        "chat_id": CHAT_ID,
        "from": CUSTOMER,
        "is_from_me": true,
        "message": { "id": MESSAGE_ID, "parts": [{ "type": "text", "value": "hi" }] },
    });
    assert!(parse_deliveries(&legacy).is_empty());
}

#[test]
fn a_group_message_counts_as_addressed_only_when_it_mentions_this_line() {
    let group = json!({
        "chat": { "id": CHAT_ID, "is_group": true },
        "parts": [{ "type": "text", "value": "anyone around?" }],
    });
    let inbound = &parse_deliveries(&received_message(group))[0];
    assert_eq!(inbound.conversation.kind, ChatKind::Group);
    assert!(!inbound.addressed_to_bot);

    let mentioned = json!({
        "chat": { "id": CHAT_ID, "is_group": true },
        "parts": [{
            "type": "text",
            "value": "@line what is the weather?",
            "mentions": [
                { "handle": "+12025551234", "is_me": true, "range": [0, 5] },
                { "handle": "+12025559999", "is_me": false, "range": [6, 11] },
            ],
        }],
    });
    let inbound = &parse_deliveries(&received_message(mentioned))[0];
    assert!(inbound.addressed_to_bot);
    // The mention is markup, not words; the text arrives as written.
    assert_eq!(inbound.text, "@line what is the weather?");

    // A mention of somebody else is not a mention of this line.
    let other_mention = json!({
        "chat": { "id": CHAT_ID, "is_group": true },
        "parts": [{
            "type": "text",
            "value": "hey @friend",
            "mentions": [{ "handle": "+12025559999", "is_me": false, "range": [4, 11] }],
        }],
    });
    assert!(!parse_deliveries(&received_message(other_mention))[0].addressed_to_bot);
}

#[test]
fn a_direct_message_is_addressed_without_a_mention() {
    let inbound = &parse_deliveries(&received_message(json!({
        "parts": [{ "type": "text", "value": "hi" }],
    })))[0];
    assert!(inbound.addressed_to_bot);
}

#[test]
fn text_link_and_media_parts_are_all_understood() {
    let parts = json!({
        "parts": [
            { "type": "text", "value": "look at this" },
            { "type": "media", "url": "https://cdn.example.test/a.jpg", "filename": "a.jpg", "mime_type": "image/jpeg", "size_bytes": 10 },
            { "type": "link", "value": "https://example.test/article" },
            { "type": "app_clip", "value": "https://zero.example.test/pay" },
        ],
    });
    let inbound = &parse_deliveries(&received_message(parts))[0];
    // A link's content is its URL; an app clip is platform chrome, not words.
    assert_eq!(inbound.text, "look at this\nhttps://example.test/article");
    assert_eq!(inbound.media.len(), 1);
    assert_eq!(inbound.media[0].kind, MediaKind::Image);
    assert_eq!(inbound.media[0].filename.as_deref(), Some("a.jpg"));
    assert_eq!(
        inbound.media[0].url.as_deref(),
        Some("https://cdn.example.test/a.jpg")
    );
}

#[test]
fn media_kinds_come_from_the_declared_type() {
    assert_eq!(media_kind(Some("image/png")), MediaKind::Image);
    assert_eq!(media_kind(Some("video/mp4")), MediaKind::Video);
    assert_eq!(media_kind(Some("audio/mp4")), MediaKind::Audio);
    assert_eq!(media_kind(Some("application/pdf")), MediaKind::Document);
    assert_eq!(media_kind(None), MediaKind::Unknown);
}

#[test]
fn events_that_are_not_received_messages_are_ignored() {
    for event_type in [
        "message.sent",
        "message.delivered",
        "message.read",
        "message.failed",
        "reaction.added",
        "chat.typing_indicator.started",
        "participant.added",
    ] {
        let event = received_message(json!({}));
        let mut event = event;
        event["event_type"] = json!(event_type);
        assert!(parse_deliveries(&event).is_empty(), "{event_type}");
    }
    // A delivery with no event type at all is not a prompt either.
    assert!(parse_deliveries(&json!({ "data": { "id": MESSAGE_ID } })).is_empty());
    assert!(parse_deliveries(&Value::Null).is_empty());
}

#[test]
fn a_message_with_nothing_to_answer_is_dropped() {
    // No parts at all.
    let inbounds = parse_deliveries(&received_message(json!({ "parts": [] })));
    assert!(inbounds.is_empty());
    // Only whitespace.
    let inbounds = parse_deliveries(&received_message(json!({
        "parts": [{ "type": "text", "value": "   " }],
    })));
    assert!(inbounds.is_empty());
    // A media part with no URL is not a reference to anything.
    let inbounds = parse_deliveries(&received_message(json!({
        "parts": [{ "type": "media", "mime_type": "image/jpeg" }],
    })));
    assert_eq!(inbounds.len(), 1);
    assert!(inbounds[0].media[0].url.is_none());
    // No chat.
    let inbounds = parse_deliveries(&received_message(json!({ "chat": {} })));
    assert!(inbounds.is_empty());
    // An empty chat id is no chat either.
    let inbounds = parse_deliveries(&received_message(json!({ "chat": { "id": "" } })));
    assert!(inbounds.is_empty());
    // No message id: the bridge would have no way to tell a retry from a new
    // message, and answering one delivery twice is worse than not answering.
    let inbounds = parse_deliveries(&received_message(json!({ "id": "" })));
    assert!(inbounds.is_empty());
    // No sender handle at all.
    let inbounds = parse_deliveries(&received_message(json!({
        "sender_handle": {},
        "from": "",
    })));
    assert!(inbounds.is_empty());
}

#[test]
fn a_batched_delivery_is_tolerated() {
    let batch = json!([
        received_message(json!({ "id": "one" })),
        received_message(json!({ "id": "two" })),
    ]);
    let inbounds = parse_deliveries(&batch);
    assert_eq!(inbounds.len(), 2);
    assert_eq!(inbounds[0].message_id, "one");
    assert_eq!(inbounds[1].message_id, "two");
}

#[test]
fn a_timestamp_that_cannot_be_parsed_is_left_absent() {
    let mut message = received_message(json!({}));
    message["data"]["sent_at"] = json!("not-a-time");
    message["created_at"] = json!("also-not-a-time");
    let inbounds = parse_deliveries(&message);
    // The bridge treats a missing timestamp as "now"; inventing one risks
    // dropping a live message as a replay.
    assert_eq!(inbounds[0].created_at_ms, None);

    // The envelope is the fallback when the message carries no instant.
    let mut envelope_only = received_message(json!({}));
    envelope_only["data"]
        .as_object_mut()
        .unwrap()
        .remove("sent_at");
    let inbounds = parse_deliveries(&envelope_only);
    assert_eq!(inbounds[0].created_at_ms, Some(1_770_319_873_736));

    assert_eq!(
        parse_instant_ms("2026-02-05T19:31:13.074Z"),
        Some(1_770_319_873_074)
    );
    assert_eq!(parse_instant_ms(""), None);
}

// ─── Outbound ───────────────────────────────────────────────────────────────

fn ok_send_body() -> String {
    json!({
        "chat_id": CHAT_ID,
        "message": {
            "id": "69a37c7d-af4f-4b5e-af42-e28e98ce873a",
            "delivery_status": "pending",
            "is_read": false,
            "parts": [{ "type": "text", "value": "hello!" }],
        },
    })
    .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_goes_to_the_chat_and_returns_the_message_id() {
    let path = format!("/v3/chats/{CHAT_ID}/messages");
    let (base, recorded) = spawn_http(vec![HttpRoute::json(&path, 200, &ok_send_body())]).await;
    let sender = sender_against(&base, "");
    let id = sender
        .send_text(&chat_conversation(), "hello!")
        .await
        .unwrap();
    assert_eq!(id.as_deref(), Some("69a37c7d-af4f-4b5e-af42-e28e98ce873a"));

    let sent = requests_to(&recorded, &path);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].header("authorization"), Some("Bearer key"));
    let body: Value = serde_json::from_str(&sent[0].body_string()).unwrap();
    assert_eq!(body["message"]["parts"][0]["type"], "text");
    assert_eq!(body["message"]["parts"][0]["value"], "hello!");
    assert!(body.get("to").is_none(), "a chat send needs no recipient");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_send_to_a_bare_handle_opens_the_conversation() {
    // With a line configured, the chat is created on that line.
    let (base, recorded) =
        spawn_http(vec![HttpRoute::json("/v3/chats", 200, &ok_send_body())]).await;
    let sender = sender_against(&base, "+12025551234");
    sender
        .send_text(&handle_conversation(), "hi")
        .await
        .unwrap();
    let sent = requests_to(&recorded, "/v3/chats");
    let body: Value = serde_json::from_str(&sent[0].body_string()).unwrap();
    assert_eq!(body["from"], "+12025551234");
    assert_eq!(body["to"], json!([CUSTOMER]));
    assert_eq!(body["message"]["parts"][0]["value"], "hi");

    // Without one, the platform picks the line.
    let (base, recorded) =
        spawn_http(vec![HttpRoute::json("/v3/messages", 200, &ok_send_body())]).await;
    let sender = sender_against(&base, "");
    sender
        .send_text(&handle_conversation(), "hi")
        .await
        .unwrap();
    let sent = requests_to(&recorded, "/v3/messages");
    let body: Value = serde_json::from_str(&sent[0].body_string()).unwrap();
    assert_eq!(body["to"], json!([CUSTOMER]));
    assert!(body.get("from").is_none(), "no line is configured");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_with_no_target_is_refused_without_a_request() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json("/v3/messages", 200, "{}")]).await;
    let sender = sender_against(&base, "");
    let empty = ConversationRef {
        id: "  ".into(),
        ..chat_conversation()
    };
    let error = sender.send_text(&empty, "hi").await.unwrap_err();
    assert!(error.to_string().contains("no chat id"), "{error}");
    assert!(recorded.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_that_returns_no_message_id_reports_none() {
    // The API also answers a send with just a trace id; there is then nothing
    // to edit later, which is honest rather than an empty string.
    let path = format!("/v3/chats/{CHAT_ID}/messages");
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        &path,
        200,
        r#"{"success":true,"trace_id":"2eff5df5c6f688733c007523c4d61cd9"}"#,
    )])
    .await;
    let sender = sender_against(&base, "");
    assert!(sender
        .send_text(&chat_conversation(), "hi")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_and_typing_are_not_claimed() {
    let (base, recorded) = spawn_http(vec![]).await;
    let sender = sender_against(&base, "");
    let error = sender
        .edit_text(&chat_conversation(), "m1", "changed")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not supported"), "{error}");
    assert!(sender.typing(&chat_conversation()).await.is_ok());
    assert!(recorded.lock().unwrap().is_empty());
    assert!(!DEFINITION.capabilities.edit);
    assert!(!DEFINITION.capabilities.typing);
}

// ─── Error classification ───────────────────────────────────────────────────

fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        text: body.to_string(),
        body: serde_json::from_str(body).unwrap_or(Value::Null),
        headers: std::collections::HashMap::new(),
        retry_after: None,
    }
}

#[test]
fn the_published_bands_are_classified() {
    // Request and resource errors cannot be fixed by trying again.
    assert_eq!(classify_error_code(1001), Some(ErrorClass::Permanent));
    assert_eq!(classify_error_code(1002), Some(ErrorClass::Permanent));
    assert_eq!(classify_error_code(2001), Some(ErrorClass::Permanent));
    assert_eq!(classify_error_code(2004), Some(ErrorClass::Permanent));
    assert_eq!(classify_error_code(2024), Some(ErrorClass::Permanent));
    // …except the three the reference names as retryable.
    assert_eq!(classify_error_code(1007), Some(ErrorClass::Transient));
    assert_eq!(classify_error_code(2007), Some(ErrorClass::Transient));
    assert_eq!(classify_error_code(2009), Some(ErrorClass::Transient));
    // Server errors are worth another attempt.
    assert_eq!(classify_error_code(3006), Some(ErrorClass::Transient));
    assert_eq!(classify_error_code(3005), Some(ErrorClass::Transient));
    // Delivery and file errors are decided case by case.
    assert_eq!(classify_error_code(4005), Some(ErrorClass::Permanent));
    assert_eq!(classify_error_code(4006), Some(ErrorClass::Transient));
    assert_eq!(classify_error_code(5005), Some(ErrorClass::Permanent));
    assert_eq!(classify_error_code(5002), Some(ErrorClass::Transient));
    // Unknown codes fall back to the HTTP status.
    assert_eq!(classify_error_code(9999), None);
}

#[test]
fn an_unauthorized_key_is_reported_as_permanent() {
    let body = json!({
        "success": false,
        "error": {
            "status": 401,
            "code": 2004,
            "message": "Unauthorized - missing or invalid authentication token",
        },
        "trace_id": "2eff5df5c6f688733c007523c4d61cd9",
    })
    .to_string();
    let error = send_error("send message", &response(401, &body));
    let text = error.to_string();
    assert!(text.contains("permanent"), "{text}");
    assert!(text.contains("Unauthorized"), "{text}");
    // The trace id is what the platform's support asks for.
    assert!(text.contains("2eff5df5c6f688733c007523c4d61cd9"), "{text}");
}

#[test]
fn a_rate_limit_is_reported_as_transient_with_its_delay() {
    let body = json!({
        "success": false,
        "error": { "status": 429, "code": 1007, "message": "Rate limit exceeded", "retry_after": 12 },
    })
    .to_string();
    let error = send_error("send message", &response(429, &body));
    let text = error.to_string();
    assert!(text.contains("transient"), "{text}");
    assert!(text.contains("retry after 12s"), "{text}");
}

#[test]
fn an_unclassified_error_falls_back_to_the_http_status() {
    let unavailable = send_error("send message", &response(500, "{}"));
    assert!(
        unavailable.to_string().contains("transient"),
        "{unavailable}"
    );
    let bad_request = send_error("send message", &response(400, "{}"));
    assert!(
        bad_request.to_string().contains("permanent"),
        "{bad_request}"
    );
    // The queue keeps the message text, so the label is the only thing that
    // tells it a 400 with an empty body is not worth retrying.
    assert!(
        crate::delivery::is_permanent_error(&bad_request.to_string()),
        "{bad_request}"
    );
    assert!(
        !crate::delivery::is_permanent_error(&unavailable.to_string()),
        "a 500 must stay retryable: {unavailable}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_send_surfaces_the_platform_error() {
    let path = format!("/v3/chats/{CHAT_ID}/messages");
    let body = json!({
        "success": false,
        "error": { "status": 400, "code": 1002, "message": "Phone number must be in E.164 format" },
    })
    .to_string();
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(&path, 400, &body)]).await;
    let sender = sender_against(&base, "");
    let error = sender
        .send_text(&chat_conversation(), "hi")
        .await
        .unwrap_err();
    let text = error.to_string();
    assert!(text.contains("permanent"), "{text}");
    assert!(text.contains("E.164"), "{text}");
}

// ─── Attachments ────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn an_image_part_is_fetched_for_the_model() {
    let (cdn, cdn_recorded) = spawn_http(vec![HttpRoute::binary(
        "/a.jpg",
        200,
        b"\xff\xd8\xff".to_vec(),
    )])
    .await;
    let sender = sender_against(&cdn, "");
    let mut media = vec![MediaRef {
        kind: MediaKind::Image,
        filename: Some("a.jpg".into()),
        content_type: Some("image/jpeg".into()),
        url: Some(format!("{cdn}/a.jpg")),
        data: None,
    }];
    sender.hydrate(&mut media).await;
    assert_eq!(media[0].data.as_deref(), Some(&b"\xff\xd8\xff"[..]));
    assert_eq!(requests_to(&cdn_recorded, "/a.jpg").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn other_attachments_stay_references() {
    // Non-image bytes would reach the model as an image, so they are not
    // fetched; the reference in `raw` still says what arrived.
    let (base, recorded) = spawn_http(vec![]).await;
    let sender = sender_against(&base, "");
    let mut media = vec![MediaRef {
        kind: MediaKind::Document,
        filename: Some("invoice.pdf".into()),
        content_type: Some("application/pdf".into()),
        url: Some(format!("{base}/invoice.pdf")),
        data: None,
    }];
    sender.hydrate(&mut media).await;
    assert!(media[0].data.is_none());
    assert!(recorded.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_download_keeps_the_reference() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::json("/gone.jpg", 404, "{}")]).await;
    let sender = sender_against(&base, "");
    let mut media = vec![MediaRef {
        kind: MediaKind::Image,
        url: Some(format!("{base}/gone.jpg")),
        ..MediaRef::default()
    }];
    sender.hydrate(&mut media).await;
    assert!(media[0].data.is_none());
    assert_eq!(media[0].url.as_deref(), Some(&*format!("{base}/gone.jpg")));
}

// ─── Provider wiring ────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn probing_lists_the_accounts_lines() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        "/v3/phone_numbers",
        200,
        &json!({
            "phone_numbers": [
                { "id": "p1", "phone_number": "+12025551234", "reputation": { "status": "HEALTHY" } },
                { "id": "p2", "phone_number": "+12025559876", "reputation": { "status": "AT_RISK" } },
            ]
        })
        .to_string(),
    )])
    .await;
    let ctx = ctx_with_config(json!({ "enabled": true, "api_key": "key", "api_base": base }));
    let summary = Linq.probe(&ctx).await.unwrap();
    assert!(summary.contains("+12025551234"), "{summary}");
    assert!(summary.contains("HEALTHY"), "{summary}");
    let probe = requests_to(&recorded, "/v3/phone_numbers");
    assert_eq!(probe.len(), 1);
    assert_eq!(probe[0].method, "GET");
    assert_eq!(probe[0].header("authorization"), Some("Bearer key"));
}

#[tokio::test(flavor = "multi_thread")]
async fn probing_checks_the_configured_line_is_assigned() {
    let numbers = json!({
        "phone_numbers": [
            { "id": "p1", "phone_number": "+12025551234", "reputation": { "status": "HEALTHY" } },
        ]
    })
    .to_string();
    let (base, _recorded) =
        spawn_http(vec![HttpRoute::json("/v3/phone_numbers", 200, &numbers)]).await;
    let ctx = ctx_with_config(json!({
        "enabled": true, "api_key": "key", "api_base": base, "from": "+12025551234",
    }));
    let summary = Linq.probe(&ctx).await.unwrap();
    assert!(summary.contains("is assigned"), "{summary}");

    // A line the account does not own is a configuration error worth catching
    // before the first send fails.
    let (base, _recorded) =
        spawn_http(vec![HttpRoute::json("/v3/phone_numbers", 200, &numbers)]).await;
    let ctx = ctx_with_config(json!({
        "enabled": true, "api_key": "key", "api_base": base, "from": "+19998887777",
    }));
    let error = Linq.probe(&ctx).await.unwrap_err();
    assert!(error.to_string().contains("not assigned"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn probing_fails_honestly_when_there_is_nothing_to_report() {
    // An account with no lines cannot send at all.
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        "/v3/phone_numbers",
        200,
        r#"{"phone_numbers":[]}"#,
    )])
    .await;
    let ctx = ctx_with_config(json!({ "enabled": true, "api_key": "key", "api_base": base }));
    let error = Linq.probe(&ctx).await.unwrap_err();
    assert!(error.to_string().contains("no phone numbers"), "{error}");

    // A rejected key is reported, not swallowed.
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        "/v3/phone_numbers",
        401,
        r#"{"success":false,"error":{"status":401,"code":2004,"message":"Unauthorized"}}"#,
    )])
    .await;
    let ctx = ctx_with_config(json!({ "enabled": true, "api_key": "key", "api_base": base }));
    let error = Linq.probe(&ctx).await.unwrap_err();
    assert!(error.to_string().contains("permanent"), "{error}");

    // Without a key there is nothing to probe with.
    let ctx = ctx_with_config(json!({ "enabled": true }));
    let error = Linq.probe(&ctx).await.unwrap_err();
    assert!(error.to_string().contains("api_key"), "{error}");
    assert!(Linq.sender(&ctx).is_err());
}

#[test]
fn the_factory_builds_the_provider_it_declares() {
    let provider = provider();
    assert_eq!(provider.definition().id, DEFINITION.id);
    assert_eq!(provider.definition().maturity, Maturity::Preview);
}

// ─── End to end through the bridge ──────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_verified_delivery_runs_a_turn_and_a_forged_one_does_not() {
    use std::sync::Arc;
    let (grpc, state) = crate::test_support::spawn_mock_grpc(Default::default()).await;
    let data_dir = crate::test_support::temp_dir("linq-e2e");
    let ctx = ctx_with_agent(
        &grpc,
        &data_dir,
        json!({ "enabled": true, "api_key": "key" }),
    );

    // The signature is checked before anything is parsed, so a delivery with a
    // body nobody signed never reaches the agent.
    let (base, _recorded) = spawn_http(vec![]).await;
    let sender = Arc::new(sender_against(&base, ""));
    let body = received_message(json!({}));
    assert!(!verify_delivery(
        &webhook_post(&serde_json::to_vec(&body).unwrap()),
        SECRET,
        now_seconds()
    ));
    assert_eq!(
        crate::test_support::recorded_of(&state, "new_session").len(),
        0
    );

    // The same body, signed, is delivered. Its instant is "now" so the
    // bridge's freshness window does not drop it as a replay.
    let mut live = body;
    live["data"]["sent_at"] =
        json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    let request = signed_request(
        &serde_json::to_vec(&live).unwrap(),
        "evt-1",
        &now_seconds().to_string(),
        SECRET,
    );
    assert!(verify_delivery(&request, SECRET, now_seconds()));
    deliver(&ctx, &sender, request.json()).await;
    assert_eq!(
        crate::test_support::recorded_of(&state, "new_session").len(),
        1
    );

    // A retry of the same delivery is deduplicated by its message id.
    deliver(&ctx, &sender, request.json()).await;
    assert_eq!(
        crate::test_support::recorded_of(&state, "new_session").len(),
        1
    );
}

// ─── The webhook listener and the remaining rejection paths ─────────────────

/// A port nothing is bound to right now, so the provider's own listener can
/// take it. The window between releasing and rebinding is microseconds, and a
/// loss would surface as `run` failing to bind rather than as a silent pass.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("local_addr").port()
}

/// The first response, retrying while the server is still coming up.
async fn post_when_listening(
    client: &reqwest::Client,
    url: &str,
    body: Vec<u8>,
) -> reqwest::Response {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match client.post(url).body(body.clone()).send().await {
            Ok(response) => return response,
            Err(error) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the webhook never listened: {error}"
                );
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }
    }
}

#[test]
fn a_signature_candidate_that_cannot_be_decoded_is_skipped() {
    let body = br#"{"event_type":"message.received","data":{}}"#;
    let mut request = signed_request(body, "evt-1", &now_seconds().to_string(), SECRET);
    let good = request.header("webhook-signature").unwrap().to_string();
    // A candidate that is not base64 is skipped rather than failing the whole
    // header: a rotation can leave unreadable stale entries beside a good one.
    request.headers.insert(
        "webhook-signature".to_string(),
        format!("v1,not!base64! {good}"),
    );
    assert!(verify_delivery(&request, SECRET, now_seconds()));

    // Nothing readable at all: the delivery is not verified.
    request.headers.insert(
        "webhook-signature".to_string(),
        "v1,not!base64!".to_string(),
    );
    assert!(!verify_delivery(&request, SECRET, now_seconds()));
}

#[test]
fn a_signed_request_without_its_signature_header_is_rejected() {
    // The id and the instant are there, but there is nothing to compare with.
    let mut request = webhook_post(br#"{"event_type":"message.received","data":{}}"#);
    request
        .headers
        .insert("webhook-id".to_string(), "evt-1".to_string());
    request
        .headers
        .insert("webhook-timestamp".to_string(), now_seconds().to_string());
    assert!(!verify_signed_request(&request, SECRET, now_seconds()));
}

#[test]
fn a_legacy_delivery_with_an_unreadable_timestamp_is_rejected() {
    let body = br#"{"event_type":"message.received","data":{}}"#;
    let mut request = webhook_post(body);
    request.headers.insert(
        "x-webhook-signature".to_string(),
        hmac_sha256_hex(SECRET.as_bytes(), body),
    );
    request
        .headers
        .insert("x-webhook-timestamp".to_string(), "yesterday".to_string());
    // An instant that cannot be read cannot be checked for replay, so the
    // delivery is refused rather than assumed fresh.
    assert!(!verify_legacy_request(&request, SECRET, now_seconds()));
}

#[test]
fn a_part_without_a_usable_value_contributes_nothing() {
    // A text part with no value, one whose value is not a string, and a link
    // preview with nothing to preview: each is skipped, not invented.
    for parts in [
        json!([{ "type": "text" }]),
        json!([{ "type": "text", "value": 7 }]),
        json!([{ "type": "link" }]),
    ] {
        let (text, media, mentioned) = parts_of(&json!({ "parts": parts }));
        assert!(text.is_empty(), "unexpected text {text:?}");
        assert!(media.is_empty());
        assert!(!mentioned);
    }
    // No parts array at all is not an error either.
    let (text, media, mentioned) = parts_of(&json!({ "parts": "not-an-array" }));
    assert!(text.is_empty());
    assert!(media.is_empty());
    assert!(!mentioned);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_image_without_a_url_is_left_alone() {
    let (base, recorded) = spawn_http(vec![]).await;
    let sender = sender_against(&base, "");
    let mut media = vec![MediaRef {
        kind: MediaKind::Image,
        url: None,
        ..MediaRef::default()
    }];
    sender.hydrate(&mut media).await;
    assert!(media[0].data.is_none());
    // Nothing to fetch means no request at all.
    assert!(recorded.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_attachment_is_refused() {
    // One byte past the limit the provider will hand to the model.
    let (base, _recorded) = spawn_http(vec![HttpRoute::binary(
        "/huge.jpg",
        200,
        vec![b'x'; MAX_DOWNLOAD_BYTES + 1],
    )])
    .await;
    let sender = sender_against(&base, "");
    let error = sender
        .download(&format!("{base}/huge.jpg"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("above the limit"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_configured_provider_builds_a_working_sender() {
    let path = format!("/v3/chats/{CHAT_ID}/messages");
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(&path, 200, &ok_send_body())]).await;
    let ctx = ctx_with_config(json!({ "enabled": true, "api_key": "key", "api_base": base }));
    let sender = Linq.sender(&ctx).unwrap();
    assert_eq!(sender.definition().id, "linq");
    // The sender carries the configuration it was built from.
    assert_eq!(
        sender
            .send_text(&chat_conversation(), "hello!")
            .await
            .unwrap()
            .as_deref(),
        Some("69a37c7d-af4f-4b5e-af42-e28e98ce873a")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn probing_a_line_without_a_number_is_a_failure() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        "/v3/phone_numbers",
        200,
        r#"{"phone_numbers":[{"id":"p1","reputation":{"status":"HEALTHY"}}]}"#,
    )])
    .await;
    let ctx = ctx_with_config(json!({ "enabled": true, "api_key": "key", "api_base": base }));
    let error = Linq.probe(&ctx).await.unwrap_err();
    assert!(error.to_string().contains("has no number"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn run_rejects_an_unsigned_delivery_and_runs_a_signed_one() {
    let (grpc, state) = crate::test_support::spawn_mock_grpc(Default::default()).await;
    let port = free_port();
    let ctx = ctx_with_agent(
        &grpc,
        &crate::test_support::temp_dir("linq-run"),
        json!({
            "enabled": true,
            "api_key": "key",
            "webhook": {
                "addr": format!("127.0.0.1:{port}"),
                "path": "/hooks/linq",
                "signing_secret": SECRET,
            },
        }),
    );
    let task = {
        let ctx = ctx.clone();
        tokio::spawn(async move { Linq.run(ctx).await })
    };
    let url = format!("http://127.0.0.1:{port}/hooks/linq");
    let client = reqwest::Client::new();

    // The signed instant has to be "now": the bridge drops anything older than
    // its freshness window as a replay.
    let mut live = received_message(json!({}));
    live["data"]["sent_at"] =
        json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    let body = serde_json::to_vec(&live).unwrap();

    // A delivery nobody signed is refused before it is parsed. The retry loop
    // also waits out the moment between spawning `run` and the listener
    // accepting connections.
    let unsigned = post_when_listening(&client, &url, body.clone()).await;
    assert_eq!(unsigned.status(), 401);
    assert_eq!(
        crate::test_support::recorded_of(&state, "new_session").len(),
        0
    );

    // The same body signed is answered immediately and then, off the request,
    // runs the turn.
    let request = signed_request(&body, "evt-run", &now_seconds().to_string(), SECRET);
    let mut signed_post = client.post(&url);
    for name in ["webhook-id", "webhook-timestamp", "webhook-signature"] {
        signed_post = signed_post.header(name, request.header(name).unwrap());
    }
    let signed = signed_post.body(body).send().await.unwrap();
    assert_eq!(signed.status(), 200);
    let delivered = crate::test_support::wait_until(
        || crate::test_support::recorded_of(&state, "new_session").len() == 1,
        std::time::Duration::from_secs(5),
    )
    .await;
    assert!(delivered, "the signed delivery never reached the agent");

    ctx.shutdown().trigger();
    let stopped = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
    assert!(stopped.is_ok(), "run() must return on shutdown");
    assert!(stopped.unwrap().unwrap().is_ok());
}
