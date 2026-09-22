// Unit tests for the Mattermost provider.
//
// These cover the platform-specific decisions the bridge does not make:
// the two-step `posted` event decoding (post and mentions arrive as
// JSON-encoded strings), both thread shapes (`root_id` present and absent),
// the mention gate, the channel allowlist, error classification, the auth
// challenge frame, and that splitting stays inside the 4000-character post
// limit without breaking a code fence.

use super::*;
use crate::test_support::{spawn_ws, WsAction};
use serde_json::json;

fn posted_event(post: &Value, mentions: &[&str], channel_type: &str) -> Value {
    json!({
        "event": "posted",
        "data": {
            // On the wire these are JSON-encoded strings, not nested values.
            "post": post.to_string(),
            "mentions": serde_json::to_string(&mentions).unwrap(),
            "channel_type": channel_type,
            "sender_name": "@alice",
        }
    })
}

fn channel_post(root_id: &str) -> Value {
    json!({
        "id": "post1",
        "user_id": "user-a",
        "channel_id": "chan1",
        "message": "@mybot how do I deploy?",
        "root_id": root_id,
        "create_at": 1_700_000_000_000_i64,
    })
}

// ─── Splitting ─────────────────────────────────────────────────────────────

#[test]
fn a_long_reply_is_split_without_cutting_a_code_block() {
    let code = (0..80)
        .map(|i| format!("let line_{i} = compute({i});\n"))
        .collect::<String>();
    let text = format!("intro\n```rust\n{code}```\noutro\n");
    let chunks = crate::transport::chunk(&text, DEFINITION.max_text_len, DEFINITION.length_unit);
    assert!(chunks.len() > 1);
    for chunk in &chunks {
        assert!(chunk.chars().count() <= 4000);
        let fences = chunk.matches("```").count();
        assert_eq!(fences % 2, 0, "unbalanced fence in {chunk:?}");
    }
}

#[test]
fn a_short_reply_is_not_split() {
    let chunks = crate::transport::chunk("hello", DEFINITION.max_text_len, DEFINITION.length_unit);
    assert_eq!(chunks, vec!["hello"]);
}

// ─── Websocket event parsing ───────────────────────────────────────────────

#[test]
fn a_posted_event_decodes_its_stringified_fields() {
    let event = posted_event(&channel_post(""), &["bot-id"], "O");
    let posted = parse_ws_event(&event).expect("posted event must parse");
    assert_eq!(posted.post["id"], "post1");
    assert_eq!(posted.mentions, vec!["bot-id"]);
    assert_eq!(posted.channel_type, "O");
    assert_eq!(posted.sender_name, "@alice");
}

#[test]
fn non_posted_events_are_ignored() {
    for event in [
        json!({"event": "hello", "data": {"server_version": "9.0"}}),
        json!({"event": "typing", "data": {}}),
        json!({"event": "post_edited", "data": {"post": "{}"}}),
        json!({"status": "OK"}),
    ] {
        assert!(parse_ws_event(&event).is_none(), "{event}");
    }
}

#[test]
fn a_malformed_post_string_is_dropped_not_panicked() {
    let event = json!({
        "event": "posted",
        "data": { "post": "not json", "channel_type": "O" }
    });
    assert!(parse_ws_event(&event).is_none());
}

#[test]
fn a_post_arriving_as_an_object_is_tolerated() {
    // The documented shape is a string; tolerating the object keeps the bot
    // alive if the envelope ever changes.
    let event = json!({
        "event": "posted",
        "data": { "post": channel_post(""), "mentions": ["bot-id"], "channel_type": "O" }
    });
    let posted = parse_ws_event(&event).expect("object-shaped post must parse");
    assert_eq!(posted.post["id"], "post1");
    assert_eq!(posted.mentions, vec!["bot-id"]);
}

// ─── Inbound normalization ─────────────────────────────────────────────────

#[test]
fn a_top_level_channel_post_becomes_a_channel_conversation() {
    let event = posted_event(&channel_post(""), &["bot-id"], "O");
    let posted = parse_ws_event(&event).unwrap();
    let inbound = parse_posted(&posted, "bot-id", &HashSet::new())
        .unwrap()
        .expect("a fresh post must normalize");
    assert_eq!(inbound.message_id, "post1");
    assert_eq!(inbound.sender.id, "user-a");
    assert_eq!(inbound.sender.display.as_deref(), Some("@alice"));
    assert_eq!(inbound.conversation.id, "chan1");
    assert_eq!(inbound.conversation.thread_id, None);
    assert_eq!(inbound.conversation.kind, ChatKind::Channel);
    assert_eq!(inbound.text, "@mybot how do I deploy?");
    assert!(inbound.addressed_to_bot);
    assert_eq!(inbound.created_at_ms, Some(1_700_000_000_000));
}

#[test]
fn a_thread_reply_keeps_its_root_as_the_thread() {
    let event = posted_event(&channel_post("root-9"), &["bot-id"], "O");
    let posted = parse_ws_event(&event).unwrap();
    let inbound = parse_posted(&posted, "bot-id", &HashSet::new())
        .unwrap()
        .expect("a thread reply must normalize");
    assert_eq!(inbound.conversation.thread_id.as_deref(), Some("root-9"));
    // A thread is its own conversation, so its session key differs from the
    // top-level channel's.
    let top = ConversationRef {
        id: "chan1".into(),
        thread_id: None,
        kind: ChatKind::Channel,
    };
    assert_ne!(
        inbound.conversation.key("mattermost"),
        top.key("mattermost")
    );
}

#[test]
fn a_direct_message_is_always_addressed() {
    let mut post = channel_post("");
    post["message"] = json!("hi there");
    let event = posted_event(&post, &[], "D");
    let posted = parse_ws_event(&event).unwrap();
    let inbound = parse_posted(&posted, "bot-id", &HashSet::new())
        .unwrap()
        .expect("a DM must normalize");
    assert_eq!(inbound.conversation.kind, ChatKind::Direct);
    assert!(inbound.addressed_to_bot, "a DM needs no mention");
}

// ─── Mention gate ──────────────────────────────────────────────────────────

#[test]
fn a_channel_post_without_a_mention_is_not_addressed() {
    let event = posted_event(&channel_post(""), &["someone-else"], "O");
    let posted = parse_ws_event(&event).unwrap();
    let inbound = parse_posted(&posted, "bot-id", &HashSet::new())
        .unwrap()
        .expect("a fresh post must normalize");
    assert!(!inbound.addressed_to_bot);
}

#[test]
fn a_channel_post_mentioning_the_bot_is_addressed() {
    let event = posted_event(&channel_post(""), &["other", "bot-id"], "P");
    let posted = parse_ws_event(&event).unwrap();
    let inbound = parse_posted(&posted, "bot-id", &HashSet::new())
        .unwrap()
        .unwrap();
    assert!(inbound.addressed_to_bot);
}

#[test]
fn a_group_dm_needs_a_mention() {
    let event = posted_event(&channel_post(""), &[], "G");
    let posted = parse_ws_event(&event).unwrap();
    let inbound = parse_posted(&posted, "bot-id", &HashSet::new())
        .unwrap()
        .unwrap();
    assert_eq!(inbound.conversation.kind, ChatKind::Group);
    assert!(!inbound.addressed_to_bot);
}

// ─── Drops ─────────────────────────────────────────────────────────────────

#[test]
fn the_bots_own_post_is_dropped() {
    let mut post = channel_post("");
    post["user_id"] = json!("bot-id");
    let event = posted_event(&post, &[], "D");
    let posted = parse_ws_event(&event).unwrap();
    assert!(parse_posted(&posted, "bot-id", &HashSet::new())
        .unwrap()
        .is_none());
}

#[test]
fn an_empty_message_is_dropped() {
    let mut post = channel_post("");
    post["message"] = json!("   ");
    let event = posted_event(&post, &["bot-id"], "O");
    let posted = parse_ws_event(&event).unwrap();
    assert!(parse_posted(&posted, "bot-id", &HashSet::new())
        .unwrap()
        .is_none());
}

#[test]
fn a_channel_outside_the_allowlist_is_dropped() {
    let allowlist: HashSet<String> = ["chan-other".to_string()].into_iter().collect();
    let event = posted_event(&channel_post(""), &["bot-id"], "O");
    let posted = parse_ws_event(&event).unwrap();
    assert!(parse_posted(&posted, "bot-id", &allowlist)
        .unwrap()
        .is_none());

    let allowlist: HashSet<String> = ["chan1".to_string()].into_iter().collect();
    let inbound = parse_posted(&posted, "bot-id", &allowlist).unwrap();
    assert!(inbound.is_some(), "an allowlisted channel must pass");
}

// ─── Outbound bodies ───────────────────────────────────────────────────────

#[test]
fn a_top_level_post_body_carries_no_root() {
    let conversation = ConversationRef {
        id: "chan1".into(),
        thread_id: None,
        kind: ChatKind::Channel,
    };
    let body = create_post_body(&conversation, "hello");
    assert_eq!(body["channel_id"], "chan1");
    assert_eq!(body["message"], "hello");
    assert!(body.get("root_id").is_none());
}

#[test]
fn a_thread_reply_body_carries_root_id() {
    let conversation = ConversationRef {
        id: "chan1".into(),
        thread_id: Some("root-9".into()),
        kind: ChatKind::Channel,
    };
    let body = create_post_body(&conversation, "reply");
    assert_eq!(body["root_id"], "root-9");
    assert_eq!(body["channel_id"], "chan1");
}

// ─── Error classification ──────────────────────────────────────────────────

#[test]
fn auth_failures_are_permanent_and_rate_limits_retryable() {
    for status in [401, 403] {
        assert_eq!(classify_failure(status), ErrorClass::Permanent, "{status}");
    }
    for status in [429, 500, 502, 503] {
        assert_eq!(classify_failure(status), ErrorClass::Transient, "{status}");
    }
}

// ─── URL derivation ────────────────────────────────────────────────────────

#[test]
fn urls_are_derived_from_the_base_url() {
    let config = MattermostConfig {
        base_url: "https://mm.example.com/".into(),
        ..MattermostConfig::default()
    };
    assert_eq!(config.api_base(), "https://mm.example.com/api/v4");
    assert_eq!(config.ws_url(), "wss://mm.example.com/api/v4/websocket");

    let config = MattermostConfig {
        base_url: "http://127.0.0.1:8065".into(),
        ..MattermostConfig::default()
    };
    assert_eq!(config.api_base(), "http://127.0.0.1:8065/api/v4");
    assert_eq!(config.ws_url(), "ws://127.0.0.1:8065/api/v4/websocket");
}

#[test]
fn explicit_urls_win_over_derived_ones() {
    let config = MattermostConfig {
        base_url: "https://mm.example.com".into(),
        api_base: "http://127.0.0.1:1/api/v4".into(),
        ws_url: "ws://127.0.0.1:2/ws".into(),
        ..MattermostConfig::default()
    };
    assert_eq!(config.api_base(), "http://127.0.0.1:1/api/v4");
    assert_eq!(config.ws_url(), "ws://127.0.0.1:2/ws");
}

// ─── Websocket session (mock server) ───────────────────────────────────────

#[tokio::test]
async fn the_session_authenticates_and_dispatches_a_posted_event() {
    let hello = json!({"event": "hello", "data": {"server_version": "9.0"}});
    let posted = posted_event(&channel_post(""), &["bot-id"], "O");
    let (url, received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(posted.to_string()),
        WsAction::Delay(std::time::Duration::from_millis(150)),
    ])
    .await;

    let ctx = crate::bridge::ProviderCtx::offline(&DEFINITION);
    let socket = ws::connect(&url, &[]).await.unwrap();
    let sender: Arc<dyn ChannelSender> = Arc::new(NullSender);
    let allowlist = HashSet::new();
    let session = websocket_session(&ctx, sender, "token-1", "bot-id", &allowlist, socket);
    tokio::time::timeout(std::time::Duration::from_secs(5), session)
        .await
        .expect("the session must end when the server drops the socket")
        .expect_err("a dropped socket is a reconnect, not a clean exit");

    // The first frame the client sends must be the auth challenge, and it
    // must carry the token — otherwise the server would close the stream.
    let messages = received.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let first = messages
        .first()
        .and_then(|m| m.to_text().ok())
        .expect("the client must send an auth challenge");
    let challenge: Value = serde_json::from_str(first).unwrap();
    assert_eq!(challenge["action"], "authentication_challenge");
    assert_eq!(challenge["data"]["token"], "token-1");
}

/// A sender that never talks to a platform; the session test only needs the
/// trait object, not real delivery (the offline bridge denies everything
/// before a send would happen).
struct NullSender;

#[async_trait]
impl ChannelSender for NullSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    async fn send_text(
        &self,
        _conversation: &ConversationRef,
        _text: &str,
    ) -> Result<Option<String>> {
        Ok(Some("null".into()))
    }
}

// ─── Definition ────────────────────────────────────────────────────────────

#[test]
fn the_definition_advertises_what_is_implemented() {
    assert!(DEFINITION.is_implemented());
    assert_eq!(DEFINITION.maturity, Maturity::Preview);
    assert_eq!(DEFINITION.max_text_len, 4000);
    let caps = DEFINITION.capabilities;
    assert!(caps.receive && caps.send && caps.edit && caps.threads);
    assert!(caps.mention_gate);
}
