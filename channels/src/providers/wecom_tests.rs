//! Unit tests for the WeCom provider.
//!
//! What the bridge does not decide: the callback handshake, the signature over
//! the ciphertext, the envelope that says who a callback was meant for, the
//! message XML, the token cache, and which `errcode` values are worth another
//! attempt. Nothing here contacts the platform.
//!
//! The signature is pinned against a value computed outside this crate
//! (`the_signature_is_sha1_over_the_sorted_values`), so it is a check on the
//! implementation rather than on itself.

use super::*;
use crate::test_support::{requests_to, spawn_http, HttpRoute};

// ─── Fixtures ───────────────────────────────────────────────────────────────

const CORP_ID: &str = "ww1234567890abcdef";
const AGENT_ID: i64 = 1_000_002;
const SECRET: &str = "app-secret";
const TOKEN: &str = "callback-token";
/// A 43-character encoding key, the shape the console shows.
const ENCODING_KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
const MEMBER: &str = "zhangsan";

fn key() -> AesKey {
    AesKey::parse(ENCODING_KEY).expect("the fixture key parses")
}

/// The plaintext envelope: 16 random bytes, a 4-byte length, the message, and
/// the id of the corp it is meant for.
fn envelope(message: &str, corp_id: &str) -> Vec<u8> {
    let mut plain = vec![0x7fu8; 16];
    plain.extend_from_slice(&(message.len() as u32).to_be_bytes());
    plain.extend_from_slice(message.as_bytes());
    plain.extend_from_slice(corp_id.as_bytes());
    plain
}

/// An encrypted callback body, the way the platform sends one.
fn encrypted(message: &str) -> String {
    encrypted_with(&key(), message, CORP_ID)
}

fn encrypted_for(message: &str, corp_id: &str) -> String {
    encrypted_with(&key(), message, corp_id)
}

/// The base64 ciphertext for a message, under an arbitrary key.
fn encrypted_with(key: &AesKey, message: &str, corp_id: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .encode(cipher::encrypt(key, &envelope(message, corp_id)))
}

/// The outer callback envelope, which carries the ciphertext.
fn outer_body(encrypt: &str) -> Vec<u8> {
    format!(
        "<xml><ToUserName><![CDATA[{CORP_ID}]]></ToUserName>\
         <Encrypt><![CDATA[{encrypt}]]></Encrypt><AgentID><![CDATA[{AGENT_ID}]]></AgentID></xml>"
    )
    .into_bytes()
}

/// Percent-encode a query value.
///
/// The platform URL-encodes the callback parameters, and it has to: the base64
/// ciphertext contains `+`, which a query string reads as a space.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn callback_request(encrypt: &str, timestamp: &str, nonce: &str) -> WebhookRequest {
    let signature = callback_signature(TOKEN, timestamp, nonce, encrypt);
    WebhookRequest {
        method: "POST".into(),
        path: "/webhooks/wecom".into(),
        query: format!(
            "timestamp={}&nonce={}&msg_signature={}",
            encode(timestamp),
            encode(nonce),
            encode(&signature)
        ),
        headers: std::collections::HashMap::new(),
        body: outer_body(encrypt),
    }
}

fn verify_request(echostr: &str, timestamp: &str, nonce: &str) -> WebhookRequest {
    let signature = callback_signature(TOKEN, timestamp, nonce, echostr);
    WebhookRequest {
        method: "GET".into(),
        path: "/webhooks/wecom".into(),
        query: format!(
            "timestamp={}&nonce={}&echostr={}&msg_signature={}",
            encode(timestamp),
            encode(nonce),
            encode(echostr),
            encode(&signature)
        ),
        headers: std::collections::HashMap::new(),
        body: Vec::new(),
    }
}

/// One decrypted message, as the platform builds it.
fn message_xml(fields: &[(&str, &str)]) -> String {
    let mut out = String::from("<xml>");
    for (name, value) in fields {
        out.push_str(&format!("<{name}><![CDATA[{value}]]></{name}>"));
    }
    out.push_str("</xml>");
    out
}

/// A message stamped with an explicit `CreateTime`.
fn message_with(id: &str, text: &str, created: &str) -> String {
    message_xml(&[
        ("ToUserName", CORP_ID),
        ("FromUserName", MEMBER),
        ("CreateTime", created),
        ("MsgType", "text"),
        ("Content", text),
        ("MsgId", id),
        ("AgentID", "1000002"),
    ])
}

/// A message the bridge will treat as current.
///
/// A fixed past timestamp would be discarded as a stale replay before it ever
/// reached the agent, which is not what these tests are about.
fn text_message(id: &str, text: &str) -> String {
    message_with(
        id,
        text,
        &(crate::bridge::dedup::now_ms() / 1000).to_string(),
    )
}

fn config_with_base(base: &str) -> WecomConfig {
    WecomConfig {
        corp_id: CORP_ID.into(),
        agent_id: AGENT_ID,
        secret: SECRET.into(),
        token: TOKEN.into(),
        encoding_aes_key: ENCODING_KEY.into(),
        webhook: WebhookConfig::default(),
        api_base: base.to_string(),
    }
}

fn config_value(config: &WecomConfig) -> Value {
    json!({
        "corp_id": config.corp_id,
        "agent_id": config.agent_id,
        "secret": config.secret,
        "token": config.token,
        "encoding_aes_key": config.encoding_aes_key,
        "webhook": { "addr": config.webhook.addr, "path": config.webhook.path },
        "api_base": config.api_base,
    })
}

fn sender_against(base: &str) -> WecomSender {
    WecomSender::new(reqwest::Client::new(), &config_with_base(base))
}

fn conversation() -> ConversationRef {
    ConversationRef {
        id: MEMBER.into(),
        thread_id: None,
        kind: ChatKind::Direct,
    }
}

const TOKEN_PATH: &str = "/cgi-bin/gettoken";
const SEND_PATH: &str = "/cgi-bin/message/send";
const AGENT_PATH: &str = "/cgi-bin/agent/get";

fn token_route() -> HttpRoute {
    HttpRoute::json(
        TOKEN_PATH,
        200,
        r#"{"errcode":0,"errmsg":"ok","access_token":"tok-1","expires_in":7200}"#,
    )
}

fn send_ok() -> HttpRoute {
    HttpRoute::json(
        SEND_PATH,
        200,
        r#"{"errcode":0,"errmsg":"ok","msgid":"MSG-1"}"#,
    )
}

fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        text: body.to_string(),
        body: serde_json::from_str(body).unwrap_or(Value::Null),
        headers: std::collections::HashMap::new(),
        retry_after: None,
    }
}

// ─── Definition and configuration ───────────────────────────────────────────

#[test]
fn the_definition_describes_the_app_api_honestly() {
    let definition = &DEFINITION;
    assert_eq!(definition.id, "wecom");
    assert_eq!(definition.maturity, Maturity::Preview);
    // Text in and out; the app API cannot rewrite a sent message, has no typing
    // signal or reactions, and this provider carries no attachments.
    assert!(definition.capabilities.receive);
    assert!(definition.capabilities.send);
    assert!(!definition.capabilities.edit);
    assert!(!definition.capabilities.threads);
    assert!(!definition.capabilities.typing);
    assert!(!definition.capabilities.reactions);
    assert!(!definition.capabilities.media_in);
    assert!(!definition.capabilities.media_out);
    // A self-built app receives members' direct messages only.
    assert!(!definition.capabilities.mention_gate);
}

#[test]
fn the_message_limit_is_in_bytes_because_the_platform_counts_bytes() {
    assert_eq!(DEFINITION.max_text_len, 2048);
    assert_eq!(DEFINITION.length_unit, LengthUnit::Bytes);
    // A 2048-character answer of multi-byte characters does not fit, and the
    // splitter has to be told the unit to know that.
    let split = crate::transport::text::chunk(
        &"中".repeat(1000),
        DEFINITION.max_text_len,
        DEFINITION.length_unit,
    );
    assert!(
        split.len() > 1,
        "multi-byte text must be split, not truncated"
    );
    for piece in &split {
        assert!(piece.len() <= DEFINITION.max_text_len, "{}", piece.len());
    }
}

#[test]
fn defaults_cover_the_origin_and_webhook_route() {
    let config = WecomConfig::default();
    assert_eq!(config.api_base(), "https://qyapi.weixin.qq.com");
    assert_eq!(config.webhook_addr(), "127.0.0.1:8790");
    assert_eq!(config.webhook_path(), "/webhooks/wecom");

    // A trailing slash on the configured origin would double up in the URL.
    let config = WecomConfig {
        api_base: "https://example.test/".into(),
        webhook: WebhookConfig {
            addr: "127.0.0.1:99".into(),
            path: "/custom".into(),
        },
        ..WecomConfig::default()
    };
    assert_eq!(config.api_base(), "https://example.test");
    assert_eq!(config.webhook_addr(), "127.0.0.1:99");
    assert_eq!(config.webhook_path(), "/custom");
}

#[test]
fn credentials_and_callback_secrets_are_required() {
    assert!(config_with_base("").require_credentials("wecom").is_ok());

    for broken in [
        WecomConfig::default(),
        WecomConfig {
            corp_id: CORP_ID.into(),
            ..WecomConfig::default()
        },
        WecomConfig {
            corp_id: CORP_ID.into(),
            secret: SECRET.into(),
            ..WecomConfig::default()
        },
    ] {
        let error = broken.require_credentials("wecom").expect_err("incomplete");
        let message = error.to_string();
        assert!(message.contains("providers.wecom"), "{message}");
        assert!(message.contains("agent_id"), "{message}");
    }

    assert!(config_with_base("")
        .require_callback_secrets("wecom")
        .is_ok());
    for broken in [
        WecomConfig::default(),
        WecomConfig {
            token: TOKEN.into(),
            ..WecomConfig::default()
        },
    ] {
        let error = broken
            .require_callback_secrets("wecom")
            .expect_err("incomplete");
        let message = error.to_string();
        assert!(message.contains("encoding_aes_key"), "{message}");
    }
}

// ─── The signature ──────────────────────────────────────────────────────────

#[test]
fn the_signature_is_sha1_over_the_sorted_values() {
    // SHA-1 of "1234ENCnoncetoken" — the four values sorted lexicographically
    // and concatenated. Computed with an independent implementation outside
    // this crate, so it checks the Rust code rather than agreeing with it.
    assert_eq!(
        callback_signature("token", "1234", "nonce", "ENC"),
        "cd62513bc2df15751b2584322bf17314fa844362"
    );
    // Order of the arguments must not matter: the platform sorts them, and so
    // must we, or the two sides would disagree.
    assert_eq!(
        callback_signature("ENC", "nonce", "1234", "token"),
        callback_signature("token", "1234", "nonce", "ENC")
    );
    // A different ciphertext is a different signature.
    assert_ne!(
        callback_signature("token", "1234", "nonce", "ENC"),
        callback_signature("token", "1234", "nonce", "OTHER")
    );
}

#[test]
fn verification_accepts_only_the_matching_signature() {
    let good = callback_signature(TOKEN, "1234", "nonce", "ENC");
    assert!(verify_callback(TOKEN, "1234", "nonce", "ENC", &good));

    // A different token, timestamp, nonce or payload all invalidate it.
    assert!(!verify_callback("other", "1234", "nonce", "ENC", &good));
    assert!(!verify_callback(TOKEN, "9999", "nonce", "ENC", &good));
    assert!(!verify_callback(TOKEN, "1234", "other", "ENC", &good));
    assert!(!verify_callback(TOKEN, "1234", "nonce", "OTHER", &good));
    // An absent signature is not a wildcard.
    assert!(!verify_callback(TOKEN, "1234", "nonce", "ENC", ""));
}

#[test]
fn an_unconfigured_token_never_verifies() {
    let signature = callback_signature(TOKEN, "1234", "nonce", "ENC");
    assert!(
        !verify_callback("", "1234", "nonce", "ENC", &signature),
        "a provider without a token must reject everything"
    );
}

// ─── XML ────────────────────────────────────────────────────────────────────

#[test]
fn xml_reads_cdata_and_plain_values() {
    let xml = "<xml><A><![CDATA[one]]></A><B>two</B><C><![CDATA[]]></C></xml>";
    assert_eq!(xml_tag(xml, "A").as_deref(), Some("one"));
    assert_eq!(xml_tag(xml, "B").as_deref(), Some("two"));
    assert_eq!(xml_tag(xml, "C").as_deref(), Some(""));
    assert_eq!(xml_tag(xml, "missing"), None);
}

#[test]
fn xml_reads_an_unclosed_or_empty_document_as_absent() {
    assert_eq!(xml_tag("", "A"), None);
    // An open tag with no close tag is not a value.
    assert_eq!(xml_tag("<xml><A>value", "A"), None);
    // A value that looks like markup is returned as written.
    assert_eq!(
        xml_tag("<A><![CDATA[a <b> c]]></A>", "A").as_deref(),
        Some("a <b> c")
    );
}

// ─── The envelope ───────────────────────────────────────────────────────────

#[test]
fn a_callback_round_trips_through_the_envelope() {
    let message = text_message("MSG-1", "hello");
    let encrypted = encrypted(&message);
    assert_eq!(
        decrypt_callback(&key(), &encrypted, CORP_ID).expect("decrypt"),
        message
    );
}

#[test]
fn an_envelope_addressed_to_another_corp_is_rejected() {
    // A captured callback replayed at another tenant must not be usable, so the
    // receiver id is checked rather than ignored.
    let encrypted = encrypted_for(&text_message("MSG-1", "hi"), "ww-other-corp");
    let error = decrypt_callback(&key(), &encrypted, CORP_ID).expect_err("another corp");
    assert!(error.to_string().contains("another corp"), "{error}");
}

#[test]
fn malformed_envelopes_are_reported_with_their_reason() {
    let key = key();

    // Shorter than the prefix.
    let short = cipher::encrypt(&key, &[0u8; 8]);
    use base64::Engine;
    let short = base64::engine::general_purpose::STANDARD.encode(short);
    let error = decrypt_callback(&key, &short, CORP_ID).expect_err("short");
    assert!(error.to_string().contains("shorter than"), "{error}");

    // Declares more message bytes than follow it.
    let mut plain = vec![0u8; 16];
    plain.extend_from_slice(&999u32.to_be_bytes());
    plain.extend_from_slice(b"tiny");
    plain.extend_from_slice(CORP_ID.as_bytes());
    let lying = base64::engine::general_purpose::STANDARD.encode(cipher::encrypt(&key, &plain));
    let error = decrypt_callback(&key, &lying, CORP_ID).expect_err("long length");
    assert!(error.to_string().contains("declares 999 bytes"), "{error}");

    // The message is not UTF-8.
    let mut plain = vec![0u8; 16];
    plain.extend_from_slice(&2u32.to_be_bytes());
    plain.extend_from_slice(&[0xff, 0xfe]);
    plain.extend_from_slice(CORP_ID.as_bytes());
    let invalid = base64::engine::general_purpose::STANDARD.encode(cipher::encrypt(&key, &plain));
    let error = decrypt_callback(&key, &invalid, CORP_ID).expect_err("not utf8");
    assert!(error.to_string().contains("not UTF-8"), "{error}");

    // No receive id at all.
    let mut plain = vec![0u8; 16];
    plain.extend_from_slice(&2u32.to_be_bytes());
    plain.extend_from_slice(b"hi");
    let bare = base64::engine::general_purpose::STANDARD.encode(cipher::encrypt(&key, &plain));
    let error = decrypt_callback(&key, &bare, CORP_ID).expect_err("no receive id");
    assert!(error.to_string().contains("no receive id"), "{error}");
}

#[test]
fn a_ciphertext_that_is_not_base64_is_rejected() {
    let error = decrypt_callback(&key(), "!!!not base64!!!", CORP_ID).expect_err("not base64");
    assert!(error.to_string().contains("not base64"), "{error}");
}

#[test]
fn a_callback_encrypted_with_another_key_is_rejected() {
    // A *fixed* plaintext, not the live fixture. A wrong key is caught either by
    // the padding check or — when the final byte happens to look like valid
    // padding — by the envelope's own length check, and which one it is depends
    // on the bytes. Taking the timestamp from the clock made the ciphertext
    // change every second, which turned this into a coin flip that passed
    // locally and failed on CI; the bytes have to be pinned for it to be a test.
    let other = AesKey::parse("WlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlo").expect("parse");
    let payload = encrypted_with(
        &other,
        &message_with("MSG-1", "hello", "1700000000"),
        CORP_ID,
    );
    let error = decrypt_callback(&key(), &payload, CORP_ID).expect_err("another key");
    let message = error.to_string();
    assert!(
        message.contains("padding") || message.contains("declares") || message.contains("UTF-8"),
        "a wrong key must be reported as damage rather than accepted: {message}"
    );
}

// ─── Message parsing ────────────────────────────────────────────────────────

#[test]
fn a_text_message_becomes_a_direct_conversation() {
    let inbound = parse_message(&text_message("MSG-7", "hello there")).expect("a prompt");
    assert_eq!(inbound.message_id, "MSG-7");
    assert_eq!(inbound.sender.id, MEMBER);
    assert_eq!(inbound.text, "hello there");
    // A self-built app only receives members' direct messages.
    assert_eq!(inbound.conversation.kind, ChatKind::Direct);
    assert_eq!(inbound.conversation.id, MEMBER);
    assert_eq!(inbound.conversation.thread_id, None);
    // The app is addressed by construction, which is what the DM policy uses.
    assert!(inbound.addressed_to_bot);
    assert!(inbound.media.is_empty());
    assert!(inbound.raw.is_some());
}

#[test]
fn the_platform_timestamp_becomes_milliseconds() {
    // The platform counts seconds; the bridge counts milliseconds.
    let inbound = parse_message(&message_with("MSG-1", "hi", "1700000000")).expect("a prompt");
    assert_eq!(inbound.created_at_ms, Some(1_700_000_000_000));

    for seconds in ["", "not-a-number"] {
        let inbound = parse_message(&message_with("MSG-1", "hi", seconds)).expect("a prompt");
        assert_eq!(
            inbound.created_at_ms, None,
            "an unusable timestamp is absent, not invented: {seconds:?}"
        );
    }
}

#[test]
fn anything_that_is_not_a_prompt_is_dropped() {
    // Not text: the provider carries no attachments, so an image would become
    // an empty prompt unless it is dropped.
    let image = message_xml(&[
        ("FromUserName", MEMBER),
        ("MsgType", "image"),
        ("PicUrl", "https://example.test/a.png"),
    ]);
    assert!(parse_message(&image).is_none());

    // Text with no content.
    let empty = text_message("MSG-1", "");
    assert!(parse_message(&empty).is_none());
    let whitespace = text_message("MSG-1", "   \n ");
    assert!(parse_message(&whitespace).is_none());

    // No sender.
    let anonymous = message_xml(&[("MsgType", "text"), ("Content", "hi")]);
    assert!(parse_message(&anonymous).is_none());

    // Not a message at all.
    assert!(parse_message("<xml></xml>").is_none());
    assert!(parse_message("").is_none());
}

#[test]
fn a_message_without_an_id_still_gets_a_unique_one() {
    // The platform sends `MsgId` for messages, but a payload without one must
    // not be dropped: the id falls back to the pair that identifies it, with
    // empty parts standing in when the timestamp is missing too.
    let without_id = message_xml(&[
        ("FromUserName", MEMBER),
        ("CreateTime", "1700000000"),
        ("MsgType", "text"),
        ("Content", "hi"),
    ]);
    let inbound = parse_message(&without_id).expect("a prompt");
    assert_eq!(inbound.message_id, format!("{MEMBER}:1700000000000"));
    assert_eq!(inbound.created_at_ms, Some(1_700_000_000_000));

    let bare = message_xml(&[
        ("FromUserName", MEMBER),
        ("MsgType", "text"),
        ("Content", "hi"),
    ]);
    let inbound = parse_message(&bare).expect("a prompt");
    assert_eq!(inbound.message_id, format!("{MEMBER}:0"));

    // An empty id is treated as absent rather than used.
    let blank = message_xml(&[
        ("FromUserName", MEMBER),
        ("MsgType", "text"),
        ("Content", "hi"),
        ("MsgId", ""),
    ]);
    let inbound = parse_message(&blank).expect("a prompt");
    assert_eq!(inbound.message_id, format!("{MEMBER}:0"));
}

// ─── The callback handler ───────────────────────────────────────────────────

#[test]
fn a_signed_callback_yields_its_message() {
    let message = text_message("MSG-1", "hello");
    let request = callback_request(&encrypted(&message), "1234", "nonce");
    assert_eq!(
        read_callback(&request, TOKEN, &key(), CORP_ID).expect("accepted"),
        message
    );
}

#[test]
fn a_callback_without_an_encrypt_element_is_rejected() {
    let mut request = callback_request(&encrypted("x"), "1234", "nonce");
    request.body = b"<xml><ToUserName>nothing</ToUserName></xml>".to_vec();
    let response = read_callback(&request, TOKEN, &key(), CORP_ID).expect_err("no encrypt");
    assert_eq!(response.status, 400);
}

#[test]
fn a_callback_with_a_forged_signature_is_rejected_before_decryption() {
    let message = text_message("MSG-1", "hello");
    let encrypt = encrypted(&message);

    // Signature for a different payload.
    let mut request = callback_request(&encrypt, "1234", "nonce");
    request.query = "timestamp=1234&nonce=nonce&msg_signature=v0=deadbeef".into();
    let response = read_callback(&request, TOKEN, &key(), CORP_ID).expect_err("forged");
    assert_eq!(
        response.status, 401,
        "an unverified payload is never decrypted"
    );

    // Correct signature, but over a payload that was then changed: this is the
    // case a signature exists to catch.
    let mut request = callback_request(&encrypt, "1234", "nonce");
    request.body = outer_body(&encrypted(&text_message("MSG-2", "tampered")));
    let response = read_callback(&request, TOKEN, &key(), CORP_ID).expect_err("tampered");
    assert_eq!(response.status, 401);

    // Missing signature and missing query parameters.
    let mut request = callback_request(&encrypt, "1234", "nonce");
    request.query = String::new();
    let response = read_callback(&request, TOKEN, &key(), CORP_ID).expect_err("no signature");
    assert_eq!(response.status, 401);

    // A different token.
    let request = callback_request(&encrypt, "1234", "nonce");
    let response =
        read_callback(&request, "other-token", &key(), CORP_ID).expect_err("other token");
    assert_eq!(response.status, 401);
}

#[test]
fn a_signed_callback_whose_key_does_not_fit_is_rejected() {
    // The signature is genuine, so this is a key mismatch rather than an
    // attacker: it must be reported, not silently accepted.
    let request = callback_request(&encrypted("x"), "1234", "nonce");
    let other = AesKey::parse("WlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlo").expect("parse");
    let response = read_callback(&request, TOKEN, &other, CORP_ID).expect_err("wrong key");
    assert_eq!(response.status, 400);
}

// ─── The verification handshake ─────────────────────────────────────────────

#[test]
fn verification_echoes_the_decrypted_challenge() {
    let challenge = "1616140317555161709";
    let request = verify_request(&encrypted(challenge), "1234", "nonce");
    let response = verify_endpoint(&request, TOKEN, &key(), CORP_ID);
    assert_eq!(response.status, 200);
    assert_eq!(String::from_utf8_lossy(&response.body), challenge);
}

#[test]
fn verification_rejects_a_forged_or_unusable_request() {
    let challenge = encrypted("1616140317555161709");
    let request = verify_request(&challenge, "1234", "nonce");

    // A forged signature over a real challenge.
    let mut forged = request.clone();
    forged.query = format!(
        "timestamp=1234&nonce=nonce&echostr={}&msg_signature=v0=deadbeef",
        encode(&challenge)
    );
    assert_eq!(verify_endpoint(&forged, TOKEN, &key(), CORP_ID).status, 401);

    // A different callback token.
    assert_eq!(
        verify_endpoint(&request, "other-token", &key(), CORP_ID).status,
        401
    );

    // No challenge to echo: rejected before the signature is even considered,
    // because there is nothing to answer with either way.
    let mut no_echo = request.clone();
    no_echo.query = "timestamp=1234&nonce=nonce".into();
    assert_eq!(
        verify_endpoint(&no_echo, TOKEN, &key(), CORP_ID).status,
        400
    );

    // A signed challenge that the key cannot open: the signature was computed
    // over the ciphertext, so this is a key mismatch.
    let other = AesKey::parse("WlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlo").expect("parse");
    assert_eq!(
        verify_endpoint(&request, TOKEN, &other, CORP_ID).status,
        400
    );

    // Only GET is the handshake.
    let mut posted = request.clone();
    posted.method = "POST".into();
    assert_eq!(verify_endpoint(&posted, TOKEN, &key(), CORP_ID).status, 404);
}

// ─── Error classification ───────────────────────────────────────────────────

#[test]
fn errcode_is_the_real_status_even_on_http_200() {
    // The platform answers 200 with a failure in the body. Trusting the status
    // would turn a rejected send into silence.
    let body = r#"{"errcode":40001,"errmsg":"invalid credential"}"#;
    let error = check_ok("message/send", &response(200, body)).expect_err("errcode 40001");
    assert!(error.to_string().contains("40001"), "{error}");

    assert!(check_ok(
        "message/send",
        &response(200, r#"{"errcode":0,"errmsg":"ok"}"#)
    )
    .is_ok());
    // No errcode at all is not evidence of failure.
    assert!(check_ok("message/send", &response(200, r#"{"msgid":"M1"}"#)).is_ok());

    // A transport-level failure is still a failure.
    let error = check_ok("message/send", &response(502, "bad gateway")).expect_err("502");
    assert!(error.to_string().contains("502"), "{error}");
}

#[test]
fn error_codes_are_classified_where_the_meaning_is_clear() {
    for code in [40014, 42001, 42007, 42009] {
        assert_eq!(
            classify_error_code(code),
            Some(ErrorClass::Transient),
            "{code} is a token problem, and another call fixes it"
        );
    }
    for code in [45009, 45047, -1] {
        assert_eq!(
            classify_error_code(code),
            Some(ErrorClass::Transient),
            "{code}"
        );
    }
    for code in [
        40001, 40013, 41001, 60011, 60020, 81013, 60111, 45002, 45003, 86001,
    ] {
        assert_eq!(
            classify_error_code(code),
            Some(ErrorClass::Permanent),
            "{code}"
        );
    }
    // Anything else falls back to the HTTP status rather than guessing.
    assert_eq!(classify_error_code(12345), None);
    assert!(!is_token_error(Some(12345)));
    assert!(is_token_error(Some(40014)));
    assert!(!is_token_error(None));
}

#[test]
fn a_classified_failure_reaches_the_delivery_queue() {
    // The queue stores the message text and nothing else, so a classification
    // that is not stamped into the text is lost, and a permanent failure would
    // be retried until the attempt cap.
    let permanent = send_error(
        "message/send",
        &response(200, r#"{"errcode":81013,"errmsg":"invalid user"}"#),
    );
    assert!(
        crate::delivery::is_permanent_error(&permanent.to_string()),
        "{permanent}"
    );

    let transient = send_error(
        "message/send",
        &response(200, r#"{"errcode":45009,"errmsg":"api freq out of limit"}"#),
    );
    assert!(
        !crate::delivery::is_permanent_error(&transient.to_string()),
        "a rate limit must stay retryable: {transient}"
    );
}

#[test]
fn a_failure_without_a_message_still_carries_its_code() {
    // The platform does not always include `errmsg`. The code is the part an
    // operator needs, so its absence must not leave an empty description.
    let error = send_error("message/send", &response(200, r#"{"errcode":40001}"#)).to_string();
    assert!(error.contains("errcode 40001"), "{error}");
    assert!(error.contains("permanent"), "{error}");
}

#[test]
fn errors_name_the_likely_cause() {
    let cases = [
        (40001, "app secret is wrong"),
        (40013, "app secret is wrong"),
        (40014, "fetched again"),
        (42001, "fetched again"),
        (60011, "may not call this API"),
        (60020, "may not call this API"),
        (81013, "not a member of this corp"),
        (45009, "call limit"),
        // No hint for a code we do not know.
        (12345, ""),
    ];
    for (code, expected) in cases {
        let body = format!(r#"{{"errcode":{code},"errmsg":"rejected"}}"#);
        let error = send_error("message/send", &response(200, &body)).to_string();
        assert!(error.contains("wecom `message/send` rejected"), "{error}");
        assert!(error.contains("rejected"), "{error}");
        if !expected.is_empty() {
            assert!(
                error.contains(expected),
                "{error} should mention {expected}"
            );
        }
    }
}

// ─── Outbound ───────────────────────────────────────────────────────────────

#[test]
fn the_access_token_is_fetched_once_and_reused() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let (base, recorded) = spawn_http(vec![token_route(), send_ok()]).await;
        let sender = sender_against(&base);

        let first = sender.access_token().await.expect("token");
        let second = sender.access_token().await.expect("token");
        assert_eq!(first, "tok-1");
        assert_eq!(second, "tok-1");
        assert_eq!(
            requests_to(&recorded, TOKEN_PATH).len(),
            1,
            "a cached token must not be fetched again on every use"
        );
        // The credential goes in the query string, the way this API takes it.
        let request = &requests_to(&recorded, TOKEN_PATH)[0];
        assert!(request.target.contains(CORP_ID), "{}", request.target);
        assert!(request.target.contains(SECRET), "{}", request.target);
    });
}

#[test]
fn a_token_that_is_about_to_expire_is_fetched_again() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        // `expires_in: 0` means the token is spent the moment it arrives, which
        // is the case the refresh margin has to handle. A zero lifetime also
        // must not overflow into "never expires".
        let (base, recorded) = spawn_http(vec![
            HttpRoute::json(
                TOKEN_PATH,
                200,
                r#"{"errcode":0,"access_token":"tok-1","expires_in":0}"#,
            ),
            send_ok(),
        ])
        .await;
        let sender = sender_against(&base);
        sender.access_token().await.expect("first");
        sender.access_token().await.expect("second");
        assert_eq!(
            requests_to(&recorded, TOKEN_PATH).len(),
            2,
            "an expired token must be replaced"
        );
    });
}

#[test]
fn a_send_posts_the_expected_body() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let (base, recorded) = spawn_http(vec![token_route(), send_ok()]).await;
        let sender = sender_against(&base);
        let id = sender
            .send_text(&conversation(), "hello there")
            .await
            .expect("send");
        assert_eq!(id.as_deref(), Some("MSG-1"));

        let request = &requests_to(&recorded, SEND_PATH)[0];
        assert!(
            request.target.contains("access_token=tok-1"),
            "{}",
            request.target
        );
        let body: Value = serde_json::from_str(&request.body_string()).expect("json");
        assert_eq!(body["touser"], MEMBER);
        assert_eq!(body["msgtype"], "text");
        assert_eq!(body["agentid"], AGENT_ID);
        assert_eq!(body["text"]["content"], "hello there");
    });
}

#[test]
fn a_send_without_a_message_id_is_still_a_success() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        // Some replies carry no id; that is not a failure, it only means the
        // bridge has nothing to edit later (and it never edits here).
        let (base, _) = spawn_http(vec![
            token_route(),
            HttpRoute::json(SEND_PATH, 200, r#"{"errcode":0,"msgid":""}"#),
        ])
        .await;
        let sender = sender_against(&base);
        assert_eq!(
            sender.send_text(&conversation(), "hi").await.expect("send"),
            None
        );
    });
}

#[test]
fn a_rejected_token_is_dropped_so_the_next_send_refetches() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let (base, recorded) = spawn_http(vec![
            token_route(),
            HttpRoute::sequence(
                SEND_PATH,
                vec![
                    (200, r#"{"errcode":40014,"errmsg":"invalid access_token"}"#),
                    (200, r#"{"errcode":0,"msgid":"MSG-2"}"#),
                ],
            ),
        ])
        .await;
        let sender = sender_against(&base);

        let error = sender
            .send_text(&conversation(), "hi")
            .await
            .expect_err("a rejected token is not success");
        let message = error.to_string();
        assert!(message.contains("40014"), "{message}");
        assert!(message.contains("transient"), "{message}");

        // The dead token must not be reused, or every retry would fail the same
        // way while the code claims the failure is retryable.
        let id = sender
            .send_text(&conversation(), "hi again")
            .await
            .expect("the retry fetches a fresh token");
        assert_eq!(id.as_deref(), Some("MSG-2"));
        assert_eq!(
            requests_to(&recorded, TOKEN_PATH).len(),
            2,
            "the rejected token must be replaced"
        );
    });
}

#[test]
fn a_permanent_rejection_is_an_error_with_its_class() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let (base, recorded) = spawn_http(vec![
            token_route(),
            HttpRoute::json(
                SEND_PATH,
                200,
                r#"{"errcode":81013,"errmsg":"invalid user"}"#,
            ),
        ])
        .await;
        let sender = sender_against(&base);
        let error = sender
            .send_text(&conversation(), "hi")
            .await
            .expect_err("rejected");
        let message = error.to_string();
        assert!(message.contains("permanent"), "{message}");
        assert!(message.contains("81013"), "{message}");
        // A permanent rejection is not a token problem, so nothing is thrown
        // away: the next send reuses the token it already has.
        assert_eq!(requests_to(&recorded, TOKEN_PATH).len(), 1);
    });
}

#[test]
fn the_sender_refuses_a_conversation_without_a_recipient() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let (base, _) = spawn_http(vec![token_route(), send_ok()]).await;
        let sender = sender_against(&base);
        let empty = ConversationRef {
            id: "  ".into(),
            thread_id: None,
            kind: ChatKind::Direct,
        };
        let error = sender
            .send_text(&empty, "hi")
            .await
            .expect_err("no recipient");
        assert!(error.to_string().contains("no recipient"), "{error}");
    });
}

#[test]
fn a_token_fetch_that_fails_is_reported() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        // A transport failure.
        let (base, _) = spawn_http(vec![HttpRoute::json(
            TOKEN_PATH,
            500,
            r#"{"errcode":-1,"errmsg":"system busy"}"#,
        )])
        .await;
        let error = sender_against(&base)
            .access_token()
            .await
            .expect_err("a 500 is not a token");
        assert!(error.to_string().contains("-1"), "{error}");

        // HTTP 200 with an error in the body.
        let (base, _) = spawn_http(vec![HttpRoute::json(
            TOKEN_PATH,
            200,
            r#"{"errcode":40001,"errmsg":"invalid credential"}"#,
        )])
        .await;
        let error = sender_against(&base)
            .access_token()
            .await
            .expect_err("errcode 40001 is not a token");
        assert!(error.to_string().contains("40001"), "{error}");

        // A success that carries no token at all: reported rather than stored.
        let (base, _) = spawn_http(vec![HttpRoute::json(
            TOKEN_PATH,
            200,
            r#"{"errcode":0,"errmsg":"ok"}"#,
        )])
        .await;
        let error = sender_against(&base)
            .access_token()
            .await
            .expect_err("no token");
        assert!(error.to_string().contains("no access_token"), "{error}");

        // An empty token is treated as absent too.
        let (base, _) = spawn_http(vec![HttpRoute::json(
            TOKEN_PATH,
            200,
            r#"{"errcode":0,"access_token":""}"#,
        )])
        .await;
        let error = sender_against(&base)
            .access_token()
            .await
            .expect_err("empty token");
        assert!(error.to_string().contains("no access_token"), "{error}");
    });
}

#[test]
fn a_send_that_cannot_reach_the_platform_is_reported() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        // No route at all for the send, so the request fails at the transport.
        let (base, _) = spawn_http(vec![token_route()]).await;
        let sender = sender_against(&base);
        let error = sender
            .send_text(&conversation(), "hi")
            .await
            .expect_err("unreachable");
        assert!(error.to_string().contains("message/send"), "{error}");
    });
}

// ─── The provider ───────────────────────────────────────────────────────────

fn ctx_with_config(config: Value, data_dir: &std::path::Path, agent_addr: &str) -> ProviderCtx {
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

#[test]
fn the_sender_needs_credentials_and_the_run_needs_secrets() {
    let dir = crate::test_support::temp_dir("wecom-provider-config");
    let ctx = ctx_with_config(
        config_value(&WecomConfig::default()),
        &dir,
        "http://127.0.0.1:1",
    );
    let provider = provider();
    // `ChannelSender` is a trait object, so its `Result` cannot be unwrapped
    // with `expect_err`; the interface is deliberately not given a `Debug`.
    let error = match provider.sender(&ctx) {
        Ok(_) => panic!("a sender without credentials must not be built"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("corp_id"), "{error}");

    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let no_secrets = WecomConfig {
            encoding_aes_key: String::new(),
            ..config_with_base("http://127.0.0.1:1")
        };
        let ctx = ctx_with_config(config_value(&no_secrets), &dir, "http://127.0.0.1:1");
        let error = provider.run(ctx).await.expect_err("no callback secrets");
        assert!(error.to_string().contains("encoding_aes_key"), "{error}");
    });
}

#[test]
fn a_complete_configuration_builds_a_sender() {
    let dir = crate::test_support::temp_dir("wecom-provider-sender");
    let ctx = ctx_with_config(
        config_value(&config_with_base("http://127.0.0.1:1")),
        &dir,
        "http://127.0.0.1:1",
    );
    let sender = provider()
        .sender(&ctx)
        .expect("a complete config builds a sender");
    assert_eq!(sender.definition().id, "wecom");
}

#[test]
fn startup_refuses_an_encoding_key_it_cannot_use() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        // A bad key is a configuration mistake worth reporting before the port
        // is open, rather than failing every callback afterwards.
        let dir = crate::test_support::temp_dir("wecom-provider-badkey");
        let config = WecomConfig {
            encoding_aes_key: "!!!not base64!!!".into(),
            ..config_with_base("http://127.0.0.1:1")
        };
        let ctx = ctx_with_config(config_value(&config), &dir, "http://127.0.0.1:1");
        let error = provider().run(ctx).await.expect_err("a bad key");
        assert!(error.to_string().contains("base64"), "{error}");
    });
}

#[test]
fn the_probe_names_the_agent_the_credentials_belong_to() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let (base, recorded) = spawn_http(vec![
            token_route(),
            HttpRoute::json(AGENT_PATH, 200, r#"{"errcode":0,"name":"Support bot"}"#),
        ])
        .await;
        let dir = crate::test_support::temp_dir("wecom-probe");
        let ctx = ctx_with_config(
            config_value(&config_with_base(&base)),
            &dir,
            "http://127.0.0.1:1",
        );
        let summary = provider().probe(&ctx).await.expect("probe");
        assert_eq!(summary, "connected as Support bot (agent 1000002)");

        // The agent id is part of the request, so the probe proves all three
        // credentials belong together rather than only the token.
        let request = &requests_to(&recorded, AGENT_PATH)[0];
        assert!(
            request.target.contains("agentid=1000002"),
            "{}",
            request.target
        );
    });
}

#[test]
fn the_probe_reports_an_agent_without_a_name_and_a_rejection() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let (base, _) = spawn_http(vec![
            token_route(),
            HttpRoute::json(AGENT_PATH, 200, r#"{"errcode":0,"name":""}"#),
        ])
        .await;
        let dir = crate::test_support::temp_dir("wecom-probe-noname");
        let ctx = ctx_with_config(
            config_value(&config_with_base(&base)),
            &dir,
            "http://127.0.0.1:1",
        );
        assert_eq!(
            provider().probe(&ctx).await.expect("probe"),
            "connected as agent 1000002"
        );

        // A rejected lookup is reported, never reported as success.
        let (base, _) = spawn_http(vec![
            token_route(),
            HttpRoute::json(
                AGENT_PATH,
                200,
                r#"{"errcode":60011,"errmsg":"no privilege"}"#,
            ),
        ])
        .await;
        let ctx = ctx_with_config(
            config_value(&config_with_base(&base)),
            &dir,
            "http://127.0.0.1:1",
        );
        let error = provider().probe(&ctx).await.expect_err("no privilege");
        assert!(error.to_string().contains("60011"), "{error}");
    });
}

#[test]
fn the_probe_needs_credentials() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let dir = crate::test_support::temp_dir("wecom-probe-nocreds");
        let ctx = ctx_with_config(
            config_value(&WecomConfig::default()),
            &dir,
            "http://127.0.0.1:1",
        );
        let error = provider().probe(&ctx).await.expect_err("no credentials");
        assert!(error.to_string().contains("corp_id"), "{error}");
    });
}

#[test]
fn a_callback_reaches_the_agent_over_a_real_socket() {
    // The whole provider path: an encrypted, signed callback over HTTP is
    // verified, decrypted, parsed, and handed to the bridge, which prompts the
    // agent. The webhook server is the only part not exercised by the tests
    // above, and the answer path is covered by the sender tests.
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let (agent_addr, state) =
            crate::test_support::spawn_mock_grpc(crate::test_support::MockState::default()).await;
        // An ephemeral port: bind one, learn the number, release it. The
        // provider takes an address, not a listener, so this is how a test
        // avoids a fixed port that another test (or machine) may hold.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.local_addr().expect("addr").port()
        };
        let (base, _) = spawn_http(vec![token_route(), send_ok()]).await;
        let dir = crate::test_support::temp_dir("wecom-e2e");
        let config = WecomConfig {
            webhook: WebhookConfig {
                addr: format!("127.0.0.1:{port}"),
                path: "/webhooks/wecom".into(),
            },
            ..config_with_base(&base)
        };
        let ctx = ctx_with_config(config_value(&config), &dir, &format!("http://{agent_addr}"));
        let shutdown = ctx.shutdown().clone();
        let serving = tokio::spawn(async move { provider().run(ctx).await });

        let client = reqwest::Client::builder()
            .http1_only()
            .build()
            .expect("client");
        let url = format!("http://127.0.0.1:{port}/webhooks/wecom");

        // 1. The platform's one-time URL verification.
        let challenge = "1616140317555161709";
        let request = verify_request(&encrypted(challenge), "1234", "nonce");
        let ready = crate::test_support::wait_until(
            || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok(),
            std::time::Duration::from_secs(5),
        )
        .await;
        assert!(ready, "the callback server must start listening");
        let response = client
            .get(format!("{url}?{}", request.query))
            .send()
            .await
            .expect("verify");
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(response.text().await.expect("body"), challenge);

        // 2. A message callback.
        let message = text_message("MSG-1", "hello from wecom");
        let request = callback_request(&encrypted(&message), "1234", "nonce");
        let response = client
            .post(format!("{url}?{}", request.query))
            .body(request.body.clone())
            .send()
            .await
            .expect("callback");
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(
            response.text().await.expect("body"),
            "",
            "the platform's documented no-passive-reply answer"
        );

        // The message reached the agent: the callback was accepted, decrypted
        // and turned into a prompt.
        let prompted = crate::test_support::wait_until(
            || !crate::test_support::recorded_of(&state, "prompt").is_empty(),
            std::time::Duration::from_secs(10),
        )
        .await;
        assert!(prompted, "the callback must reach the agent as a prompt");

        // 3. A forged callback is refused, and does not prompt again.
        let before = crate::test_support::recorded_of(&state, "prompt").len();
        let mut forged = callback_request(&encrypted(&message), "1234", "nonce");
        forged.query = "timestamp=1234&nonce=nonce&msg_signature=v0=deadbeef".into();
        let response = client
            .post(format!("{url}?{}", forged.query))
            .body(forged.body.clone())
            .send()
            .await
            .expect("forged");
        assert_eq!(response.status().as_u16(), 401);
        assert_eq!(
            crate::test_support::recorded_of(&state, "prompt").len(),
            before,
            "a forged callback must not reach the agent"
        );

        shutdown.trigger();
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(5), serving).await;
        assert!(stopped.is_ok(), "the callback server must stop on shutdown");
        assert!(
            stopped.expect("stopped").is_ok(),
            "shutdown is a clean end, not an error"
        );
    });
}

#[test]
fn a_callback_that_carries_no_prompt_does_not_reach_the_agent() {
    // `deliver` runs after the HTTP answer, so it is driven directly here: an
    // image callback must be dropped with a reason rather than becoming an
    // empty prompt the agent cannot answer.
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let (agent_addr, state) =
            crate::test_support::spawn_mock_grpc(crate::test_support::MockState::default()).await;
        let (base, _) = spawn_http(vec![token_route(), send_ok()]).await;
        let dir = crate::test_support::temp_dir("wecom-deliver-image");
        let ctx = ctx_with_config(
            config_value(&config_with_base(&base)),
            &dir,
            &format!("http://{agent_addr}"),
        );
        let sender = Arc::new(WecomSender::new(
            reqwest::Client::new(),
            &config_with_base(&base),
        ));

        let image = message_xml(&[
            ("FromUserName", MEMBER),
            ("MsgType", "image"),
            ("PicUrl", "https://example.test/a.png"),
        ]);
        deliver(&ctx, &sender, &image).await;
        assert!(
            crate::test_support::recorded_of(&state, "prompt").is_empty(),
            "a callback with no text must not prompt the agent"
        );

        // A text one does, which is what makes the assertion above meaningful.
        let text = text_message("MSG-9", "hello");
        deliver(&ctx, &sender, &text).await;
        let prompted = crate::test_support::wait_until(
            || !crate::test_support::recorded_of(&state, "prompt").is_empty(),
            std::time::Duration::from_secs(10),
        )
        .await;
        assert!(prompted, "a text callback must reach the agent");
    });
}
