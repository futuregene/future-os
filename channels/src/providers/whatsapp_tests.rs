//! Unit tests for the WhatsApp provider.
//!
//! The platform-specific decisions the bridge does not make: the endpoint
//! verification handshake, signature verification over the raw body, payload
//! parsing (including the shapes that must be ignored), attachment resolution,
//! the outbound request bodies, and error classification.
//!
//! Outbound tests run against [`crate::test_support::spawn_http`]; nothing here
//! contacts the platform, and the one end-to-end test drives a mock agent.

use super::*;
use crate::test_support::{requests_to, spawn_http, HttpRoute};

// ─── Fixtures ───────────────────────────────────────────────────────────────

const APP_SECRET: &str = "app-secret";
const VERIFY_TOKEN: &str = "verify-me";
const PHONE_NUMBER_ID: &str = "5550001111";
const CUSTOMER: &str = "15551230000";

fn webhook_get(query: &str) -> WebhookRequest {
    WebhookRequest {
        method: "GET".into(),
        path: "/webhooks/whatsapp".into(),
        query: query.into(),
        headers: std::collections::HashMap::new(),
        body: Vec::new(),
    }
}

fn webhook_post(body: &[u8], signature: Option<&str>) -> WebhookRequest {
    let mut headers = std::collections::HashMap::new();
    if let Some(signature) = signature {
        headers.insert("x-hub-signature-256".to_string(), signature.to_string());
    }
    WebhookRequest {
        method: "POST".into(),
        path: "/webhooks/whatsapp".into(),
        query: String::new(),
        headers,
        body: body.to_vec(),
    }
}

/// The signature the platform would send for `body`.
fn sign(secret: &str, body: &[u8]) -> String {
    format!("sha256={}", hmac_sha256_hex(secret.as_bytes(), body))
}

fn text_message(id: &str, body: &str) -> Value {
    text_message_at(1_700_000_000, id, body)
}

/// A message stamped `seconds` ago-by-the-clock: the bridge drops anything
/// older than its freshness window, so end-to-end tests need a live one.
fn text_message_at(seconds: i64, id: &str, body: &str) -> Value {
    json!({
        "from": CUSTOMER,
        "id": id,
        "timestamp": seconds.to_string(),
        "type": "text",
        "text": { "body": body },
    })
}

/// A message the bridge will treat as current.
fn live_message(id: &str, body: &str) -> Value {
    text_message_at(crate::bridge::dedup::now_ms() / 1000, id, body)
}

/// A whole delivery envelope around one or more messages.
fn delivery(messages: Vec<Value>) -> Value {
    json!({
        "object": "whatsapp_business_account",
        "entry": [{
            "id": "WABA-ID",
            "changes": [{
                "field": "messages",
                "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                        "display_phone_number": "15550001111",
                        "phone_number_id": PHONE_NUMBER_ID,
                    },
                    "contacts": [{ "profile": { "name": "Ada" }, "wa_id": CUSTOMER }],
                    "messages": messages,
                },
            }],
        }],
    })
}

fn config_with_base(base: &str) -> WhatsappConfig {
    WhatsappConfig {
        phone_number_id: PHONE_NUMBER_ID.into(),
        access_token: "tok".into(),
        verify_token: VERIFY_TOKEN.into(),
        app_secret: APP_SECRET.into(),
        webhook: WebhookConfig::default(),
        api_base: base.to_string(),
        api_version: "v21.0".into(),
    }
}

fn sender_against(base: &str) -> WhatsappSender {
    WhatsappSender::new(reqwest::Client::new(), &config_with_base(base))
}

fn conversation() -> ConversationRef {
    ConversationRef {
        id: CUSTOMER.into(),
        thread_id: None,
        kind: ChatKind::Direct,
    }
}

fn messages_path() -> String {
    format!("/v21.0/{PHONE_NUMBER_ID}/messages")
}

/// The read receipts among the recorded send requests.
///
/// The same endpoint also carries replies and reactions, so the body has to
/// say which is which.
fn read_receipts(recorded: &crate::test_support::RecordedRequests) -> Vec<String> {
    requests_to(recorded, &messages_path())
        .into_iter()
        .filter(|request| request.body_string().contains(r#""status":"read""#))
        .map(|request| request.body_string())
        .collect()
}

// ─── Configuration ──────────────────────────────────────────────────────────

#[test]
fn the_definition_describes_the_cloud_api_honestly() {
    assert_eq!(DEFINITION.id, "whatsapp");
    assert_eq!(DEFINITION.maturity, Maturity::Preview);
    assert!(DEFINITION.is_implemented());
    // 4096 is the platform's documented text limit.
    assert_eq!(DEFINITION.max_text_len, 4096);
    assert_eq!(DEFINITION.length_unit, LengthUnit::Chars);
    let capabilities = DEFINITION.capabilities;
    assert!(capabilities.receive && capabilities.send);
    assert!(!capabilities.edit, "a sent message cannot be rewritten");
    assert!(capabilities.media_in && capabilities.reactions);
    // One customer per number: there is no group to gate mentions in.
    assert!(!capabilities.mention_gate);
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
        assert!(piece.chars().count() <= 4096);
    }
}

#[test]
fn defaults_cover_the_origin_version_and_webhook_route() {
    let empty = WhatsappConfig::default();
    assert_eq!(empty.api_base(), "https://graph.facebook.com");
    assert_eq!(empty.api_version(), "v21.0");
    assert_eq!(empty.webhook_addr(), "127.0.0.1:8788");
    assert_eq!(empty.webhook_path(), "/webhooks/whatsapp");
    // A trailing slash in a configured base must not double up in the path.
    let configured = WhatsappConfig {
        api_base: "https://example.test/".into(),
        api_version: "v19.0".into(),
        webhook: WebhookConfig {
            addr: "127.0.0.1:9".into(),
            path: "/wa".into(),
        },
        ..WhatsappConfig::default()
    };
    assert_eq!(configured.api_base(), "https://example.test");
    assert_eq!(configured.api_version(), "v19.0");
    assert_eq!(configured.webhook_addr(), "127.0.0.1:9");
    assert_eq!(configured.webhook_path(), "/wa");
}

#[test]
fn credentials_and_webhook_secrets_are_required() {
    let complete = config_with_base("");
    assert!(complete.require_credentials("whatsapp").is_ok());
    assert!(complete.require_webhook_secrets("whatsapp").is_ok());

    let no_token = WhatsappConfig {
        access_token: String::new(),
        ..config_with_base("")
    };
    let error = no_token.require_credentials("whatsapp").unwrap_err();
    assert!(error.to_string().contains("access_token"), "{error}");

    let no_number = WhatsappConfig {
        phone_number_id: String::new(),
        ..config_with_base("")
    };
    assert!(no_number.require_credentials("whatsapp").is_err());

    // Receiving without a secret would accept anyone's POST, so it is refused
    // rather than silently trusted.
    let unverifiable = WhatsappConfig {
        app_secret: String::new(),
        ..config_with_base("")
    };
    assert!(unverifiable.require_webhook_secrets("whatsapp").is_err());
    let no_verify_token = WhatsappConfig {
        verify_token: String::new(),
        ..config_with_base("")
    };
    assert!(no_verify_token.require_webhook_secrets("whatsapp").is_err());
}

// ─── Endpoint verification ──────────────────────────────────────────────────

#[test]
fn verification_echoes_the_challenge_when_the_token_matches() {
    let request = webhook_get(&format!(
        "hub.mode=subscribe&hub.verify_token={VERIFY_TOKEN}&hub.challenge=1158201444"
    ));
    let response = verify_endpoint(&request, VERIFY_TOKEN);
    assert_eq!(response.status, 200);
    assert_eq!(String::from_utf8_lossy(&response.body), "1158201444");
    assert!(response.content_type.starts_with("text/plain"));
}

#[test]
fn verification_rejects_a_wrong_or_missing_token_without_leaking_the_challenge() {
    for query in [
        "hub.mode=subscribe&hub.verify_token=nope&hub.challenge=1158201444",
        "hub.mode=subscribe&hub.challenge=1158201444",
        "hub.mode=subscribe&hub.verify_token=&hub.challenge=1158201444",
    ] {
        let response = verify_endpoint(&webhook_get(query), VERIFY_TOKEN);
        assert_eq!(response.status, 403, "{query}");
        assert!(!String::from_utf8_lossy(&response.body).contains("1158201444"));
    }
}

#[test]
fn verification_rejects_an_unexpected_mode_and_a_missing_challenge() {
    let wrong_mode = webhook_get("hub.mode=unsubscribe&hub.verify_token=verify-me");
    assert_eq!(verify_endpoint(&wrong_mode, VERIFY_TOKEN).status, 403);
    let no_challenge = webhook_get(&format!(
        "hub.mode=subscribe&hub.verify_token={VERIFY_TOKEN}"
    ));
    assert_eq!(verify_endpoint(&no_challenge, VERIFY_TOKEN).status, 400);
}

#[test]
fn verification_is_off_for_a_request_that_is_not_a_get() {
    // The same path also takes deliveries; a POST must not be answered with a
    // challenge.
    let mut request = webhook_get(&format!(
        "hub.mode=subscribe&hub.verify_token={VERIFY_TOKEN}&hub.challenge=42"
    ));
    request.method = "POST".into();
    assert_eq!(verify_endpoint(&request, VERIFY_TOKEN).status, 404);
}

// ─── Signature verification ─────────────────────────────────────────────────

#[test]
fn a_correctly_signed_delivery_is_accepted() {
    let body = serde_json::to_vec(&delivery(vec![text_message("wamid.1", "hi")])).unwrap();
    let request = webhook_post(&body, Some(&sign(APP_SECRET, &body)));
    assert!(verify_signature(&request, APP_SECRET));
    // The platform's own SDKs accept a bare digest too; both forms verify.
    let bare = webhook_post(&body, Some(&hmac_sha256_hex(APP_SECRET.as_bytes(), &body)));
    assert!(verify_signature(&bare, APP_SECRET));
}

#[test]
fn a_missing_signature_is_rejected() {
    let body = br#"{"entry":[]}"#;
    assert!(!verify_signature(&webhook_post(body, None), APP_SECRET));
    // An empty header is not a signature either.
    assert!(!verify_signature(&webhook_post(body, Some("")), APP_SECRET));
}

#[test]
fn a_signature_from_another_secret_or_over_other_bytes_is_rejected() {
    let body = br#"{"entry":[]}"#;
    assert!(!verify_signature(
        &webhook_post(body, Some(&sign("wrong-secret", body))),
        APP_SECRET
    ));
    // The digest covers the body as sent.
    assert!(!verify_signature(
        &webhook_post(body, Some(&sign(APP_SECRET, br#"{"entry":[] }"#))),
        APP_SECRET
    ));
    assert!(!verify_signature(
        &webhook_post(br#"{"entry":[{"id":"x"}]}"#, Some(&sign(APP_SECRET, body))),
        APP_SECRET
    ));
}

#[test]
fn a_tampered_body_invalidates_its_signature() {
    // The same JSON with different whitespace is different bytes, and the
    // digest covers bytes: verification has to run against the raw body.
    let signed = br#"{"entry":[{"id":"WABA"}]}"#;
    let respaced = br#"{ "entry": [ { "id": "WABA" } ] }"#;
    assert_eq!(
        serde_json::from_slice::<Value>(signed).unwrap(),
        serde_json::from_slice::<Value>(respaced).unwrap(),
        "the two bodies mean the same thing"
    );
    assert!(verify_signature(
        &webhook_post(signed, Some(&sign(APP_SECRET, signed))),
        APP_SECRET
    ));
    assert!(!verify_signature(
        &webhook_post(respaced, Some(&sign(APP_SECRET, signed))),
        APP_SECRET
    ));
    // And a body whose content was changed after signing never verifies.
    let tampered = br#"{"entry":[{"id":"WABA"}],"extra":true}"#;
    assert!(!verify_signature(
        &webhook_post(tampered, Some(&sign(APP_SECRET, signed))),
        APP_SECRET
    ));
}

#[test]
fn an_unconfigured_secret_never_verifies() {
    let body = br#"{"entry":[]}"#;
    assert!(!verify_signature(
        &webhook_post(body, Some(&sign("", body))),
        ""
    ));
}

// ─── Event parsing ──────────────────────────────────────────────────────────

#[test]
fn a_text_message_becomes_a_direct_conversation() {
    let inbounds = parse_deliveries(&delivery(vec![text_message("wamid.A", "hello there")]));
    assert_eq!(inbounds.len(), 1);
    let inbound = &inbounds[0];
    assert_eq!(inbound.message_id, "wamid.A");
    assert_eq!(inbound.text, "hello there");
    assert_eq!(inbound.conversation.id, CUSTOMER);
    assert_eq!(inbound.conversation.kind, ChatKind::Direct);
    assert!(inbound.conversation.thread_id.is_none());
    // The sender is the customer's WhatsApp id, and their profile name rides
    // along for display.
    assert_eq!(inbound.sender.id, CUSTOMER);
    assert_eq!(inbound.sender.display.as_deref(), Some("Ada"));
    // One-to-one by construction.
    assert!(inbound.addressed_to_bot);
    // Seconds in the payload become milliseconds in the envelope, so the
    // freshness filter sees a live message.
    assert_eq!(inbound.created_at_ms, Some(1_700_000_000_000));
    assert_eq!(inbound.raw.as_ref().unwrap()["type"], "text");
}

#[test]
fn a_timestamp_that_is_missing_or_unparsable_is_left_absent() {
    let mut no_timestamp = text_message("wamid.A", "hi");
    no_timestamp.as_object_mut().unwrap().remove("timestamp");
    let inbounds = parse_deliveries(&delivery(vec![no_timestamp]));
    assert_eq!(inbounds[0].created_at_ms, None);

    let mut garbage = text_message("wamid.B", "hi");
    garbage["timestamp"] = json!("not-a-number");
    let inbounds = parse_deliveries(&delivery(vec![garbage]));
    assert_eq!(inbounds[0].created_at_ms, None);
}

#[test]
fn an_interactive_reply_uses_the_label_the_customer_tapped() {
    let button = json!({
        "from": CUSTOMER,
        "id": "wamid.B",
        "timestamp": "1700000000",
        "type": "interactive",
        "interactive": {
            "type": "button_reply",
            "button_reply": { "id": "yes-1", "title": "Yes, please" },
        },
    });
    let inbounds = parse_deliveries(&delivery(vec![button]));
    assert_eq!(inbounds[0].text, "Yes, please");
    // The platform's own id stays reachable for a later callback.
    assert_eq!(
        inbounds[0].raw.as_ref().unwrap()["interactive"]["button_reply"]["id"],
        "yes-1"
    );

    let list = json!({
        "from": CUSTOMER,
        "id": "wamid.C",
        "timestamp": "1700000000",
        "type": "interactive",
        "interactive": {
            "type": "list_reply",
            "list_reply": { "id": "row-2", "title": "Talk to billing" },
        },
    });
    assert_eq!(
        parse_deliveries(&delivery(vec![list]))[0].text,
        "Talk to billing"
    );
}

#[test]
fn a_quick_reply_button_message_uses_its_text() {
    let quick_reply = json!({
        "from": CUSTOMER,
        "id": "wamid.D",
        "timestamp": "1700000000",
        "type": "button",
        "button": { "payload": "stop-all", "text": "Stop" },
    });
    let inbounds = parse_deliveries(&delivery(vec![quick_reply]));
    assert_eq!(inbounds[0].text, "Stop");
}

#[test]
fn an_image_carries_its_caption_and_a_resolvable_reference() {
    let image = json!({
        "from": CUSTOMER,
        "id": "wamid.E",
        "timestamp": "1700000000",
        "type": "image",
        "image": {
            "id": "MEDIA-1",
            "mime_type": "image/jpeg",
            "sha256": "abc",
            "caption": "what is this?",
        },
    });
    let inbounds = parse_deliveries(&delivery(vec![image]));
    assert_eq!(inbounds[0].text, "what is this?");
    let media = &inbounds[0].media;
    assert_eq!(media.len(), 1);
    assert_eq!(media[0].kind, MediaKind::Image);
    assert_eq!(media[0].content_type.as_deref(), Some("image/jpeg"));
    // The bytes are two authenticated calls away, so the id is carried as a
    // placeholder until they are fetched.
    assert_eq!(media[0].url.as_deref(), Some("wa-media:MEDIA-1"));
    assert!(media[0].data.is_none());
}

#[test]
fn media_types_map_onto_the_bridge_vocabulary() {
    let document = json!({
        "from": CUSTOMER,
        "id": "wamid.F",
        "timestamp": "1700000000",
        "type": "document",
        "document": {
            "id": "MEDIA-2",
            "mime_type": "application/pdf",
            "filename": "invoice.pdf",
            "caption": "here it is",
        },
    });
    let inbounds = parse_deliveries(&delivery(vec![document]));
    assert_eq!(inbounds[0].media[0].kind, MediaKind::Document);
    assert_eq!(
        inbounds[0].media[0].filename.as_deref(),
        Some("invoice.pdf")
    );

    let voice = json!({
        "from": CUSTOMER,
        "id": "wamid.G",
        "timestamp": "1700000000",
        "type": "audio",
        "audio": { "id": "MEDIA-3", "mime_type": "audio/ogg; codecs=opus" },
    });
    let inbounds = parse_deliveries(&delivery(vec![voice]));
    assert_eq!(inbounds[0].media[0].kind, MediaKind::Audio);
    // A voice note has no words, so the attachment is the whole message.
    assert!(inbounds[0].text.is_empty());

    assert_eq!(media_kind("sticker", None), MediaKind::Image);
    assert_eq!(media_kind("voice", None), MediaKind::Audio);
    assert_eq!(
        media_kind("document", Some("image/png")),
        MediaKind::Document
    );
    assert_eq!(media_kind("unknown", Some("image/png")), MediaKind::Image);
    assert_eq!(media_kind("unknown", Some("video/mp4")), MediaKind::Video);
    assert_eq!(media_kind("unknown", Some("audio/mpeg")), MediaKind::Audio);
    assert_eq!(media_kind("unknown", None), MediaKind::Unknown);
}

#[test]
fn delivery_receipts_produce_no_prompts() {
    let statuses = json!({
        "object": "whatsapp_business_account",
        "entry": [{
            "id": "WABA-ID",
            "changes": [{
                "field": "messages",
                "value": {
                    "messaging_product": "whatsapp",
                    "metadata": { "phone_number_id": PHONE_NUMBER_ID },
                    "statuses": [{
                        "id": "wamid.SENT",
                        "status": "delivered",
                        "timestamp": "1700000000",
                        "recipient_id": CUSTOMER,
                    }],
                },
            }],
        }],
    });
    assert!(parse_deliveries(&statuses).is_empty());
}

#[test]
fn message_types_with_nothing_to_answer_are_dropped() {
    let reaction = json!({
        "from": CUSTOMER,
        "id": "wamid.H",
        "timestamp": "1700000000",
        "type": "reaction",
        "reaction": { "message_id": "wamid.A", "emoji": "👍" },
    });
    assert!(parse_deliveries(&delivery(vec![reaction])).is_empty());

    let location = json!({
        "from": CUSTOMER,
        "id": "wamid.I",
        "timestamp": "1700000000",
        "type": "location",
        "location": { "latitude": 1.0, "longitude": 2.0 },
    });
    assert!(parse_deliveries(&delivery(vec![location])).is_empty());

    // An empty text body is not a prompt either.
    assert!(parse_deliveries(&delivery(vec![text_message("wamid.J", "   ")])).is_empty());

    // A message with no type at all is malformed, not a prompt.
    let typeless = json!({ "from": CUSTOMER, "id": "wamid.K", "timestamp": "1700000000" });
    assert!(parse_deliveries(&delivery(vec![typeless])).is_empty());

    // Neither is one without a sender.
    let anonymous = json!({ "id": "wamid.L", "timestamp": "1700000000", "type": "text", "text": { "body": "hi" } });
    assert!(parse_deliveries(&delivery(vec![anonymous])).is_empty());
}

#[test]
fn several_messages_across_entries_are_all_parsed() {
    let body = json!({
        "object": "whatsapp_business_account",
        "entry": [
            { "id": "WABA", "changes": [{ "field": "messages", "value": {
                "contacts": [{ "profile": { "name": "Ada" }, "wa_id": CUSTOMER }],
                "messages": [text_message("wamid.1", "first")],
            } }] },
            { "id": "WABA", "changes": [
                { "field": "messages", "value": {
                    "contacts": [],
                    "messages": [text_message("wamid.2", "second")],
                } },
                { "field": "message_template_status_update", "value": { "event": "APPROVED" } },
            ] },
        ],
    });
    let inbounds = parse_deliveries(&body);
    assert_eq!(inbounds.len(), 2);
    assert_eq!(inbounds[0].text, "first");
    assert_eq!(inbounds[1].text, "second");
    // No contact entry covered the second message, so it arrives without a
    // display name rather than with the wrong one.
    assert_eq!(inbounds[1].sender.display, None);
}

#[test]
fn a_delivery_without_entries_yields_nothing() {
    assert!(parse_deliveries(&json!({})).is_empty());
    assert!(parse_deliveries(&json!({ "entry": [] })).is_empty());
    assert!(parse_deliveries(&json!({ "entry": [{ "id": "x" }] })).is_empty());
    assert!(parse_deliveries(&Value::Null).is_empty());
}

// ─── Outbound ───────────────────────────────────────────────────────────────

fn ok_send_body(message_id: &str) -> String {
    json!({
        "messaging_product": "whatsapp",
        "contacts": [{ "input": CUSTOMER, "wa_id": CUSTOMER }],
        "messages": [{ "id": message_id, "message_status": "accepted" }],
    })
    .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn send_text_posts_the_documented_body_and_returns_the_message_id() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        &messages_path(),
        200,
        &ok_send_body("wamid.OUT"),
    )])
    .await;
    let sender = sender_against(&base);
    let id = sender.send_text(&conversation(), "hello!").await.unwrap();
    assert_eq!(id.as_deref(), Some("wamid.OUT"));

    let sent = requests_to(&recorded, &messages_path());
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].header("authorization"), Some("Bearer tok"));
    let body: Value = serde_json::from_str(&sent[0].body_string()).unwrap();
    assert_eq!(body["messaging_product"], "whatsapp");
    assert_eq!(body["recipient_type"], "individual");
    assert_eq!(body["to"], CUSTOMER);
    assert_eq!(body["type"], "text");
    assert_eq!(body["text"]["body"], "hello!");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_without_a_recipient_is_refused_without_a_request() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(&messages_path(), 200, "{}")]).await;
    let sender = sender_against(&base);
    let empty = ConversationRef {
        id: "  ".into(),
        ..conversation()
    };
    let error = sender.send_text(&empty, "hi").await.unwrap_err();
    assert!(error.to_string().contains("recipient"), "{error}");
    assert!(requests_to(&recorded, &messages_path()).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reaction_targets_the_original_message() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        &messages_path(),
        200,
        &ok_send_body("wamid.REACT"),
    )])
    .await;
    let sender = sender_against(&base);
    sender
        .react(&conversation(), "wamid.A", "👍")
        .await
        .unwrap();
    let sent = requests_to(&recorded, &messages_path());
    let body: Value = serde_json::from_str(&sent[0].body_string()).unwrap();
    assert_eq!(body["type"], "reaction");
    assert_eq!(body["reaction"]["message_id"], "wamid.A");
    assert_eq!(body["reaction"]["emoji"], "👍");
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_is_an_honest_no_op_for_a_bot() {
    // The Cloud API shows "typing…" only for a human agent, so nothing is sent
    // and the capability stays off.
    let (base, recorded) = spawn_http(vec![HttpRoute::json(&messages_path(), 200, "{}")]).await;
    let sender = sender_against(&base);
    assert!(sender.typing(&conversation()).await.is_ok());
    assert!(recorded.lock().unwrap().is_empty());
    assert!(!DEFINITION.capabilities.typing);
    let error = sender
        .edit_text(&conversation(), "wamid.A", "changed")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not supported"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_receipt_names_the_message() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(&messages_path(), 200, "{}")]).await;
    let sender = sender_against(&base);
    sender.mark_read("wamid.A").await.unwrap();
    let sent = requests_to(&recorded, &messages_path());
    let body: Value = serde_json::from_str(&sent[0].body_string()).unwrap();
    assert_eq!(body["status"], "read");
    assert_eq!(body["message_id"], "wamid.A");
    assert_eq!(body["messaging_product"], "whatsapp");
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
fn known_error_codes_are_classified() {
    assert_eq!(classify_error_code(190), Some(ErrorClass::Permanent));
    assert_eq!(classify_error_code(102), Some(ErrorClass::Permanent));
    assert_eq!(classify_error_code(131047), Some(ErrorClass::Permanent));
    assert_eq!(classify_error_code(131026), Some(ErrorClass::Permanent));
    assert_eq!(classify_error_code(131052), Some(ErrorClass::Permanent));
    // A number that is not on WhatsApp at all.
    assert_eq!(classify_error_code(133010), Some(ErrorClass::Permanent));
    assert_eq!(classify_error_code(131009), Some(ErrorClass::Permanent));
    assert_eq!(classify_error_code(131048), Some(ErrorClass::Transient));
    assert_eq!(classify_error_code(131056), Some(ErrorClass::Transient));
    assert_eq!(classify_error_code(130429), Some(ErrorClass::Transient));
    assert_eq!(classify_error_code(80007), Some(ErrorClass::Transient));
    assert_eq!(classify_error_code(4), Some(ErrorClass::Transient));
    assert_eq!(classify_error_code(999), None);
}

#[test]
fn a_closed_service_window_is_reported_as_permanent() {
    let body = json!({
        "error": {
            "message": "Message failed to send because more than 24 hours have passed since the customer last replied",
            "type": "OAuthException",
            "code": 131047,
            "fbtrace_id": "abc",
        }
    })
    .to_string();
    let error = send_error("messages", &response(400, &body));
    let text = error.to_string();
    assert!(text.contains("permanent"), "{text}");
    assert!(text.contains("24 hours"), "{text}");
}

#[test]
fn a_rate_limit_is_reported_as_transient() {
    let body = json!({
        "error": { "message": "Rate limit hit", "type": "OAuthException", "code": 130429 }
    })
    .to_string();
    let error = send_error("messages", &response(429, &body));
    let text = error.to_string();
    assert!(text.contains("transient"), "{text}");
    assert!(text.contains("Rate limit hit"), "{text}");
}

#[test]
fn an_unclassified_error_falls_back_to_the_http_status() {
    let unavailable = send_error("messages", &response(500, "{}"));
    assert!(
        unavailable.to_string().contains("transient"),
        "{unavailable}"
    );

    let bad_request = send_error("messages", &response(400, r#"{"error":{"code":100}}"#));
    assert!(
        bad_request.to_string().contains("permanent"),
        "{bad_request}"
    );

    // A 401 with no body still yields a message an operator can act on.
    let unauthorized = send_error("messages", &response(401, ""));
    assert!(unauthorized.to_string().contains("401"), "{unauthorized}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_send_surfaces_the_platform_error() {
    let body = json!({
        "error": { "message": "Invalid OAuth access token", "code": 190 }
    })
    .to_string();
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(&messages_path(), 401, &body)]).await;
    let sender = sender_against(&base);
    let error = sender.send_text(&conversation(), "hi").await.unwrap_err();
    let text = error.to_string();
    assert!(text.contains("permanent"), "{text}");
    assert!(text.contains("Invalid OAuth access token"), "{text}");
}

// ─── Attachments ────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn an_attachment_is_resolved_through_the_media_endpoint() {
    // The signed CDN URL the lookup returns points somewhere else on the
    // network, so it is a second mock server rather than a path on this one.
    let (cdn, cdn_recorded) = spawn_http(vec![HttpRoute::binary(
        "/cdn/photo",
        200,
        b"\x89PNG".to_vec(),
    )])
    .await;
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        "/v21.0/MEDIA-1",
        200,
        &json!({
            "id": "MEDIA-1",
            "url": format!("{cdn}/cdn/photo"),
            "mime_type": "image/jpeg",
            "file_name": "photo.jpg",
            "file_size": 4,
        })
        .to_string(),
    )])
    .await;
    let sender = sender_against(&base);
    let mut media = vec![MediaRef {
        kind: MediaKind::Image,
        url: Some("wa-media:MEDIA-1".into()),
        ..MediaRef::default()
    }];
    sender.hydrate(&mut media).await;
    assert_eq!(media[0].data.as_deref(), Some(&b"\x89PNG"[..]));
    assert_eq!(media[0].content_type.as_deref(), Some("image/jpeg"));
    assert_eq!(media[0].filename.as_deref(), Some("photo.jpg"));
    // The signed CDN URL is fetched with the same bearer token the lookup used.
    let lookup = requests_to(&recorded, "/v21.0/MEDIA-1");
    assert_eq!(lookup[0].header("authorization"), Some("Bearer tok"));
    let download = requests_to(&cdn_recorded, "/cdn/photo");
    assert_eq!(download.len(), 1);
    assert_eq!(download[0].header("authorization"), Some("Bearer tok"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_download_keeps_the_reference_and_does_not_panic() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        "/v21.0/MEDIA-2",
        404,
        r#"{"error":{"message":"Media not found","code":100}}"#,
    )])
    .await;
    let sender = sender_against(&base);
    let mut media = vec![MediaRef {
        kind: MediaKind::Image,
        filename: Some("photo.jpg".into()),
        url: Some("wa-media:MEDIA-2".into()),
        ..MediaRef::default()
    }];
    sender.hydrate(&mut media).await;
    assert!(media[0].data.is_none(), "no bytes were fetched");
    assert_eq!(media[0].filename.as_deref(), Some("photo.jpg"));
    assert_eq!(media[0].url.as_deref(), Some("wa-media:MEDIA-2"));
    assert_eq!(requests_to(&recorded, "/v21.0/MEDIA-2").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_image_attachment_is_not_downloaded() {
    // Bytes on a document would reach the model as an image, so only images are
    // fetched.
    let (base, recorded) = spawn_http(vec![HttpRoute::json("/v21.0/MEDIA-4", 200, "{}")]).await;
    let sender = sender_against(&base);
    let mut media = vec![MediaRef {
        kind: MediaKind::Document,
        filename: Some("invoice.pdf".into()),
        url: Some("wa-media:MEDIA-4".into()),
        ..MediaRef::default()
    }];
    sender.hydrate(&mut media).await;
    assert!(media[0].data.is_none());
    assert!(recorded.lock().unwrap().is_empty(), "no request was made");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_url_reference_is_left_alone() {
    let (base, recorded) = spawn_http(vec![]).await;
    let sender = sender_against(&base);
    let mut media = vec![MediaRef {
        kind: MediaKind::Image,
        url: Some("https://example.test/a.png".into()),
        ..MediaRef::default()
    }];
    sender.hydrate(&mut media).await;
    assert!(media[0].data.is_none());
    assert!(recorded.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_media_lookup_without_a_url_is_a_failure_not_a_panic() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::json("/v21.0/MEDIA-3", 200, "{}")]).await;
    let sender = sender_against(&base);
    let error = sender.fetch_media("MEDIA-3").await.unwrap_err();
    assert!(error.to_string().contains("no url"), "{error}");
}

// ─── Provider wiring ────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn probing_reports_the_number_and_its_verified_name() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        &format!("/v21.0/{PHONE_NUMBER_ID}"),
        200,
        &json!({ "display_phone_number": "15550001111", "verified_name": "Acme Support" })
            .to_string(),
    )])
    .await;
    let ctx = ctx_with_config(json!({
        "enabled": true,
        "phone_number_id": PHONE_NUMBER_ID,
        "access_token": "tok",
        "verify_token": VERIFY_TOKEN,
        "app_secret": APP_SECRET,
        "api_base": base,
    }));
    let summary = Whatsapp.probe(&ctx).await.unwrap();
    assert!(summary.contains("15550001111"), "{summary}");
    assert!(summary.contains("Acme Support"), "{summary}");
    let probe = requests_to(&recorded, &format!("/v21.0/{PHONE_NUMBER_ID}"));
    assert_eq!(probe.len(), 1);
    assert_eq!(probe[0].method, "GET");
    assert!(probe[0].target.contains("fields="));
}

#[tokio::test(flavor = "multi_thread")]
async fn probing_without_credentials_fails_before_any_request() {
    let ctx = ctx_with_config(json!({ "enabled": true }));
    let error = Whatsapp.probe(&ctx).await.unwrap_err();
    assert!(error.to_string().contains("access_token"), "{error}");
    assert!(Whatsapp.sender(&ctx).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_probe_reports_the_platform_error() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        &format!("/v21.0/{PHONE_NUMBER_ID}"),
        400,
        r#"{"error":{"message":"Unsupported get request","code":100}}"#,
    )])
    .await;
    let ctx = ctx_with_config(json!({
        "enabled": true,
        "phone_number_id": PHONE_NUMBER_ID,
        "access_token": "tok",
        "verify_token": VERIFY_TOKEN,
        "app_secret": APP_SECRET,
        "api_base": base,
    }));
    let error = Whatsapp.probe(&ctx).await.unwrap_err();
    assert!(
        error.to_string().contains("Unsupported get request"),
        "{error}"
    );
}

#[test]
fn the_factory_builds_the_provider_it_declares() {
    let provider = provider();
    assert_eq!(provider.definition().id, DEFINITION.id);
    assert_eq!(provider.definition().maturity, Maturity::Preview);
}

// ─── End to end through the bridge ──────────────────────────────────────────

/// A context wired to a mock agent, with an open DM policy.
fn ctx_with_agent(grpc_addr: &str, base: &str, data_dir: &std::path::Path) -> ProviderCtx {
    ctx_with_agent_config(grpc_addr, base, data_dir, json!({}))
}

/// [`ctx_with_agent`] with extra `providers.whatsapp` keys merged in — the
/// webhook listener, for the tests that drive `run` over a real socket.
fn ctx_with_agent_config(
    grpc_addr: &str,
    base: &str,
    data_dir: &std::path::Path,
    extra: Value,
) -> ProviderCtx {
    use std::sync::Arc;
    let sessions = Arc::new(crate::session_store::SessionStore::new(
        data_dir.join("sessions.json"),
    ));
    let agent_cfg = crate::config::AgentConfig {
        grpc_addr: grpc_addr.to_string(),
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
            require_mention: false,
        },
        data_dir.to_path_buf(),
        Arc::new(crate::status::StatusBoard::new(
            data_dir.join("status.json"),
        )),
    );
    let mut config = json!({
        "enabled": true,
        "phone_number_id": PHONE_NUMBER_ID,
        "access_token": "tok",
        "verify_token": VERIFY_TOKEN,
        "app_secret": APP_SECRET,
        "api_base": base,
    });
    if let (Some(config), Some(extra)) = (config.as_object_mut(), extra.as_object()) {
        config.extend(extra.clone());
    }
    ProviderCtx::new(
        &DEFINITION,
        config,
        bridge,
        data_dir.to_path_buf(),
        sessions,
        crate::bridge::Shutdown::new(),
    )
}

fn ctx_with_config(config: Value) -> ProviderCtx {
    ProviderCtx::new(
        &DEFINITION,
        config,
        crate::bridge::Bridge::offline(),
        crate::test_support::temp_dir("whatsapp-ctx"),
        {
            use std::sync::Arc;
            Arc::new(crate::session_store::SessionStore::new(
                crate::test_support::temp_dir("whatsapp-sessions").join("sessions.json"),
            ))
        },
        crate::bridge::Shutdown::new(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn an_accepted_message_is_marked_read_and_a_denied_one_is_not() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(&messages_path(), 200, "{}")]).await;
    let (grpc, state) = crate::test_support::spawn_mock_grpc(Default::default()).await;
    let ctx = ctx_with_agent(&grpc, &base, &crate::test_support::temp_dir("whatsapp-e2e"));
    let sender = Arc::new(WhatsappSender::new(
        reqwest::Client::new(),
        &config_with_base(&base),
    ));

    deliver(
        &ctx,
        &sender,
        delivery(vec![live_message("wamid.E2E", "hello")]),
    )
    .await;
    // The message reached the queue, so the customer's own client is told it
    // was read.
    assert_eq!(
        read_receipts(&recorded).len(),
        1,
        "one read receipt for one accepted message"
    );
    assert_eq!(
        crate::test_support::recorded_of(&state, "new_session").len(),
        1
    );

    // An unreachable agent means the message was not accepted, so nothing is
    // marked read — the customer's inbox stays honest.
    let denied = ctx_with_agent(
        "http://127.0.0.1:1",
        &base,
        &crate::test_support::temp_dir("whatsapp-e2e-denied"),
    );
    deliver(
        &denied,
        &sender,
        delivery(vec![live_message("wamid.E2E-2", "hello again")]),
    )
    .await;
    assert_eq!(read_receipts(&recorded).len(), 1, "still only the first");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_duplicate_delivery_is_answered_without_touching_the_agent_twice() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(&messages_path(), 200, "{}")]).await;
    let (grpc, state) = crate::test_support::spawn_mock_grpc(Default::default()).await;
    let ctx = ctx_with_agent(&grpc, &base, &crate::test_support::temp_dir("whatsapp-dup"));
    let sender = Arc::new(WhatsappSender::new(
        reqwest::Client::new(),
        &config_with_base(&base),
    ));
    let body = delivery(vec![live_message("wamid.DUP", "hello")]);
    deliver(&ctx, &sender, body.clone()).await;
    deliver(&ctx, &sender, body).await;
    // The platform retries deliveries it thinks were lost; the bridge's dedup
    // keys on the message id, so the second copy runs no turn and no receipt.
    assert_eq!(
        crate::test_support::recorded_of(&state, "new_session").len(),
        1
    );
    assert_eq!(read_receipts(&recorded).len(), 1);
}

// ─── The webhook listener ───────────────────────────────────────────────────

/// A port nothing is bound to right now, so the provider's own listener can
/// take it. The window between releasing and rebinding is microseconds, and a
/// loss would surface as `run` failing to bind rather than as a silent pass.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("local_addr").port()
}

/// The first response, retrying until the server has come up.
async fn get_when_listening(client: &reqwest::Client, url: &str) -> reqwest::Response {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match client.get(url).send().await {
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

#[tokio::test(flavor = "multi_thread")]
async fn run_echoes_the_challenge_and_admits_only_signed_deliveries() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(&messages_path(), 200, "{}")]).await;
    let (grpc, state) = crate::test_support::spawn_mock_grpc(Default::default()).await;
    let port = free_port();
    let ctx = ctx_with_agent_config(
        &grpc,
        &base,
        &crate::test_support::temp_dir("whatsapp-run"),
        json!({ "webhook": { "addr": format!("127.0.0.1:{port}"), "path": "/hooks/whatsapp" } }),
    );
    let task = {
        let ctx = ctx.clone();
        tokio::spawn(async move { Whatsapp.run(ctx).await })
    };
    let url = format!("http://127.0.0.1:{port}/hooks/whatsapp");
    let client = reqwest::Client::new();

    // The one-time endpoint verification: the challenge comes back as plain
    // text, which is what marks the subscription verified at the platform.
    let verified = get_when_listening(
        &client,
        &format!("{url}?hub.mode=subscribe&hub.verify_token={VERIFY_TOKEN}&hub.challenge=abc123"),
    )
    .await;
    assert_eq!(verified.status(), 200);
    assert_eq!(verified.text().await.unwrap(), "abc123");
    // A wrong token never sees the challenge.
    let refused = client
        .get(format!(
            "{url}?hub.mode=subscribe&hub.verify_token=wrong&hub.challenge=abc123"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 403);

    // A delivery nobody signed is refused before it is parsed, so it never
    // becomes a prompt.
    let body = serde_json::to_vec(&delivery(vec![live_message("wamid.RUN-1", "hello")])).unwrap();
    let unsigned = client.post(&url).body(body.clone()).send().await.unwrap();
    assert_eq!(unsigned.status(), 401);
    assert_eq!(
        crate::test_support::recorded_of(&state, "new_session").len(),
        0
    );

    // The same delivery signed with the app secret is answered immediately and
    // then, off the request, runs the turn.
    let signed = client
        .post(&url)
        .header("x-hub-signature-256", sign(APP_SECRET, &body))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(signed.status(), 200);
    let delivered = crate::test_support::wait_until(
        || crate::test_support::recorded_of(&state, "new_session").len() == 1,
        std::time::Duration::from_secs(5),
    )
    .await;
    assert!(delivered, "the signed delivery never reached the agent");
    assert!(
        crate::test_support::wait_until(
            || !read_receipts(&recorded).is_empty(),
            std::time::Duration::from_secs(5),
        )
        .await,
        "an accepted message is marked read"
    );

    ctx.shutdown().trigger();
    let stopped = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
    assert!(stopped.is_ok(), "run() must return on shutdown");
    assert!(stopped.unwrap().unwrap().is_ok());
}

// ─── The remaining rejection paths ──────────────────────────────────────────

#[test]
fn a_change_without_a_value_contributes_no_messages() {
    // A change the platform sent without a value carries nothing to parse, but
    // the entries beside it still produce their messages.
    let body = json!({
        "entry": [
            { "id": "WABA-ID", "changes": [{ "field": "messages" }] },
            {
                "id": "WABA-ID",
                "changes": [{
                    "field": "messages",
                    "value": { "messages": [text_message("wamid.NOVALUE", "still here")] },
                }],
            },
        ],
    });
    let parsed = parse_deliveries(&body);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].text, "still here");
}

#[test]
fn a_recipient_that_cannot_receive_is_named_in_the_error() {
    let body = json!({
        "error": { "message": "Message undeliverable", "type": "OAuthException", "code": 131026 }
    })
    .to_string();
    let error = send_error("messages", &response(400, &body)).to_string();
    assert!(error.contains("permanent"), "{error}");
    assert!(error.contains("cannot receive"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_reaction_is_reported() {
    let body = json!({ "error": { "message": "Reaction not allowed", "code": 100 } }).to_string();
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(&messages_path(), 400, &body)]).await;
    let sender = sender_against(&base);
    let error = sender
        .react(&conversation(), "wamid.REACT", "👍")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("reaction"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_read_receipt_is_reported() {
    let body =
        json!({ "error": { "message": "Unsupported post request", "code": 100 } }).to_string();
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(&messages_path(), 400, &body)]).await;
    let sender = sender_against(&base);
    let error = sender.mark_read("wamid.RECEIPT").await.unwrap_err();
    assert!(error.to_string().contains("mark read"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_media_download_that_fails_keeps_the_reference() {
    // The lookup succeeds but the signed CDN URL behind it does not.
    let (cdn, _cdn_recorded) = spawn_http(vec![HttpRoute::json("/cdn/gone", 500, "{}")]).await;
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        "/v21.0/MEDIA-5",
        200,
        &json!({ "url": format!("{cdn}/cdn/gone"), "mime_type": "image/jpeg" }).to_string(),
    )])
    .await;
    let sender = sender_against(&base);
    let mut media = vec![MediaRef {
        kind: MediaKind::Image,
        url: Some("wa-media:MEDIA-5".into()),
        ..MediaRef::default()
    }];
    sender.hydrate(&mut media).await;
    assert!(media[0].data.is_none(), "no bytes came back");
    assert_eq!(media[0].url.as_deref(), Some("wa-media:MEDIA-5"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_attachment_is_refused() {
    // One byte past the limit the provider will hand to the model.
    let (cdn, _cdn_recorded) = spawn_http(vec![HttpRoute::binary(
        "/cdn/huge",
        200,
        vec![b'x'; MAX_DOWNLOAD_BYTES + 1],
    )])
    .await;
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        "/v21.0/MEDIA-6",
        200,
        &json!({ "url": format!("{cdn}/cdn/huge") }).to_string(),
    )])
    .await;
    let sender = sender_against(&base);
    let error = sender.fetch_media("MEDIA-6").await.unwrap_err();
    assert!(error.to_string().contains("above the"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_configured_provider_builds_a_working_sender() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        &messages_path(),
        200,
        &ok_send_body("wamid.SENDER"),
    )])
    .await;
    let ctx = ctx_with_config(json!({
        "enabled": true,
        "phone_number_id": PHONE_NUMBER_ID,
        "access_token": "tok",
        "api_base": base,
    }));
    let sender = Whatsapp.sender(&ctx).unwrap();
    assert_eq!(sender.definition().id, "whatsapp");
    // The sender carries the configuration it was built from.
    assert_eq!(
        sender
            .send_text(&conversation(), "hi")
            .await
            .unwrap()
            .as_deref(),
        Some("wamid.SENDER")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn probing_a_number_without_a_display_number_is_a_failure() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        &format!("/v21.0/{PHONE_NUMBER_ID}"),
        200,
        r#"{"verified_name":"Acme Support"}"#,
    )])
    .await;
    let ctx = ctx_with_config(json!({
        "enabled": true,
        "phone_number_id": PHONE_NUMBER_ID,
        "access_token": "tok",
        "api_base": base,
    }));
    let error = Whatsapp.probe(&ctx).await.unwrap_err();
    assert!(
        error.to_string().contains("display_phone_number"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_receipt_that_is_refused_does_not_fail_the_turn() {
    use std::sync::Arc;
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        &messages_path(),
        500,
        r#"{"error":{"message":"boom","code":1}}"#,
    )])
    .await;
    let (grpc, state) = crate::test_support::spawn_mock_grpc(Default::default()).await;
    let ctx = ctx_with_agent(
        &grpc,
        &base,
        &crate::test_support::temp_dir("whatsapp-receipt-fail"),
    );
    let sender = Arc::new(WhatsappSender::new(
        reqwest::Client::new(),
        &config_with_base(&base),
    ));

    deliver(
        &ctx,
        &sender,
        delivery(vec![live_message("wamid.RECEIPT-FAIL", "hello")]),
    )
    .await;
    // The turn stands even though the receipt was refused: an unread badge is
    // not worth losing an answer over.
    assert_eq!(
        crate::test_support::recorded_of(&state, "new_session").len(),
        1
    );
    assert_eq!(
        read_receipts(&recorded).len(),
        1,
        "the receipt was attempted"
    );
}
