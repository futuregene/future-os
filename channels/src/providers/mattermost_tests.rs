// Unit tests for the Mattermost provider.
//
// These cover the platform-specific decisions the bridge does not make:
// the two-step `posted` event decoding (post and mentions arrive as
// JSON-encoded strings), both thread shapes (`root_id` present and absent),
// the mention gate, the channel allowlist, error classification, the auth
// challenge frame, and that splitting stays inside the 4000-character post
// limit without breaking a code fence.

use super::*;
use crate::test_support::{spawn_http, spawn_ws, HttpRoute, WsAction};
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
    let code = (0..200)
        .map(|i| format!("let line_{i} = compute({i});\n"))
        .collect::<String>();
    let text = format!("intro\n```rust\n{code}```\noutro\n");
    let input_len = text.chars().count();
    assert!(
        input_len > DEFINITION.max_text_len,
        "fixture is {input_len} chars, under the {}-char limit — the split would not be real",
        DEFINITION.max_text_len
    );
    let chunks = crate::transport::chunk(&text, DEFINITION.max_text_len, DEFINITION.length_unit);
    assert!(
        chunks.len() > 1,
        "a {input_len}-char reply must split at the {}-char limit",
        DEFINITION.max_text_len
    );
    for chunk in &chunks {
        let size = chunk.chars().count();
        assert!(
            size <= DEFINITION.max_text_len,
            "chunk of {size} chars exceeds the limit"
        );
        let fences = chunk.matches("```").count();
        assert_eq!(fences % 2, 0, "unbalanced fence in {chunk:?}");
    }
    // Chunks are not literal substrings: the splitter closes a fence it had
    // to cut and reopens it on the next chunk, so a naive join gains fence
    // markers. Rejoining after removing the inserted close/open pair (and
    // condensing whitespace, which the splitter may shift around the cut)
    // must reproduce the input.
    let rejoined = chunks.join("").replace("```\n```", "");
    let condensed = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    assert_eq!(condensed(&rejoined), condensed(&text));
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

// ─── REST API (mock HTTP server) ────────────────────────────────────────────

fn api_for(base: &str) -> MattermostApi {
    MattermostApi {
        base: base.to_string(),
        token: "tok-1".into(),
        http: reqwest::Client::new(),
    }
}

fn me_body() -> &'static str {
    r#"{"id":"bot-id","username":"mybot"}"#
}

#[tokio::test(flavor = "multi_thread")]
async fn users_me_reads_the_bots_identity() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json("/users/me", 200, me_body())]).await;
    let (id, username) = api_for(&base).me().await.unwrap();
    assert_eq!(id, "bot-id");
    assert_eq!(username, "mybot");
    // The bearer token must ride on the request, or a real server answers 401.
    let requests = requests_to(&recorded, "/users/me");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].header("Authorization"), Some("Bearer tok-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_http_error_status_names_the_endpoint_and_the_platform_message() {
    let (base, _) = spawn_http(vec![HttpRoute::json(
        "/users/me",
        401,
        r#"{"id":"api.context.invalid_token","message":"Invalid or expired token"}"#,
    )])
    .await;
    let error = api_for(&base).me().await.unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("/users/me"), "{message}");
    assert!(message.contains("Invalid or expired token"), "{message}");
    assert_eq!(classify_failure(401), ErrorClass::Permanent);
}

use crate::test_support::requests_to;

// ─── Sender (create / edit / react) ─────────────────────────────────────────

fn sender_for(base: &str) -> MattermostSender {
    MattermostSender { api: api_for(base) }
}

#[tokio::test(flavor = "multi_thread")]
async fn send_text_posts_the_body_and_returns_the_post_id() {
    let (base, recorded) =
        spawn_http(vec![HttpRoute::json("/posts", 201, r#"{"id":"post-new"}"#)]).await;
    let conversation = ConversationRef {
        id: "chan1".into(),
        thread_id: Some("root-9".into()),
        kind: ChatKind::Channel,
    };
    let id = sender_for(&base)
        .send_text(&conversation, "hello thread")
        .await
        .unwrap();
    assert_eq!(id.as_deref(), Some("post-new"));
    let requests = requests_to(&recorded, "/posts");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_str(&requests[0].body_string()).unwrap();
    assert_eq!(body["channel_id"], "chan1");
    assert_eq!(body["root_id"], "root-9");
    assert_eq!(body["message"], "hello thread");
}

#[tokio::test(flavor = "multi_thread")]
async fn send_text_surfaces_a_permanent_rejection() {
    let (base, _) = spawn_http(vec![HttpRoute::json(
        "/posts",
        403,
        r#"{"message":"no write access"}"#,
    )])
    .await;
    let error = sender_for(&base)
        .send_text(&ConversationRef::default(), "hi")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no write access"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_text_rewrites_the_post_in_place() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        "/posts/post-1",
        200,
        r#"{"id":"post-1"}"#,
    )])
    .await;
    sender_for(&base)
        .edit_text(&ConversationRef::default(), "post-1", "edited")
        .await
        .unwrap();
    let requests = requests_to(&recorded, "/posts/post-1");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "PUT");
    let body: Value = serde_json::from_str(&requests[0].body_string()).unwrap();
    assert_eq!(body["message"], "edited");
}

#[tokio::test(flavor = "multi_thread")]
async fn react_posts_a_reaction_as_the_bot_user() {
    let (base, recorded) = spawn_http(vec![
        HttpRoute::json("/users/me", 200, me_body()),
        HttpRoute::json("/reactions", 200, r#"{"user_id":"bot-id"}"#),
    ])
    .await;
    sender_for(&base)
        .react(&ConversationRef::default(), "post-1", "eyes")
        .await
        .unwrap();
    let requests = requests_to(&recorded, "/reactions");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_str(&requests[0].body_string()).unwrap();
    assert_eq!(body["user_id"], "bot-id");
    assert_eq!(body["post_id"], "post-1");
    assert_eq!(body["emoji_name"], "eyes");
}

// ─── Provider wiring (sender / probe) ───────────────────────────────────────

fn ctx_with_config(config: Value, data_dir: &std::path::Path) -> ProviderCtx {
    ProviderCtx::new(
        &DEFINITION,
        config,
        crate::bridge::Bridge::offline(),
        data_dir.to_path_buf(),
        Arc::new(crate::session_store::SessionStore::new(
            data_dir.join("sessions.json"),
        )),
        crate::bridge::Shutdown::new(),
    )
}

fn config_value(base: &str, token: &str) -> Value {
    json!({
        "enabled": true,
        "base_url": base,
        // The mock server serves flat paths ("/users/me"), not "/api/v4/…",
        // so the API root is the base itself.
        "api_base": base,
        "token": token,
        "channel_allowlist": ["chan1"],
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn the_provider_builds_a_sender_with_a_token() {
    let dir = crate::test_support::temp_dir("mm-sender-build");
    let ctx = ctx_with_config(config_value("http://127.0.0.1:1", "tok-1"), &dir);
    let sender = Mattermost.sender(&ctx).expect("a token builds a sender");
    assert!(std::ptr::eq(sender.definition(), &DEFINITION));
}

#[tokio::test(flavor = "multi_thread")]
async fn sender_and_probe_refuse_a_missing_token() {
    let dir = crate::test_support::temp_dir("mm-no-token");
    let ctx = ctx_with_config(config_value("http://127.0.0.1:1", ""), &dir);
    let error = Mattermost
        .sender(&ctx)
        .map(|_| ())
        .expect_err("a missing token must not build a sender");
    assert!(error.to_string().contains("token is required"), "{error}");
    let error = Mattermost.probe(&ctx).await.unwrap_err();
    assert!(error.to_string().contains("token is required"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn probe_reports_the_bot_username() {
    let (base, _) = spawn_http(vec![HttpRoute::json("/users/me", 200, me_body())]).await;
    let dir = crate::test_support::temp_dir("mm-probe");
    let ctx = ctx_with_config(config_value(&base, "tok-1"), &dir);
    let summary = Mattermost.probe(&ctx).await.unwrap();
    assert_eq!(summary, "connected as @mybot");
}

#[tokio::test(flavor = "multi_thread")]
async fn probe_falls_back_to_the_user_id_without_a_username() {
    let (base, _) = spawn_http(vec![HttpRoute::json(
        "/users/me",
        200,
        r#"{"id":"bot-id"}"#,
    )])
    .await;
    let dir = crate::test_support::temp_dir("mm-probe-no-name");
    let ctx = ctx_with_config(config_value(&base, "tok-1"), &dir);
    let summary = Mattermost.probe(&ctx).await.unwrap();
    assert_eq!(summary, "connected (user id bot-id)");
}

// ─── Websocket session, extended ────────────────────────────────────────────

#[tokio::test]
async fn a_shutdown_signal_stops_the_session_cleanly() {
    // The server holds the socket open and says nothing; only the shutdown
    // notification may end the session, and it must end it without an error.
    let (url, _) = spawn_ws(vec![WsAction::Delay(std::time::Duration::from_secs(60))]).await;
    let ctx = crate::bridge::ProviderCtx::offline(&DEFINITION);
    let socket = ws::connect(&url, &[]).await.unwrap();
    let sender: Arc<dyn ChannelSender> = Arc::new(NullSender);
    let allowlist = HashSet::new();
    let shutdown = ctx.shutdown().clone();
    let handle = tokio::spawn(async move {
        websocket_session(&ctx, sender, "token-1", "bot-id", &allowlist, socket).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    shutdown.trigger();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
        .await
        .expect("shutdown must end the session");
    assert!(result.unwrap().is_ok(), "shutdown is a clean exit");
}

#[tokio::test]
async fn a_ping_is_answered_with_a_pong_and_garbage_frames_are_ignored() {
    let posted = posted_event(&channel_post(""), &["bot-id"], "O");
    let (url, received) = spawn_ws(vec![
        WsAction::SendPing(b"keepalive".to_vec()),
        WsAction::SendText("definitely not json".to_string()),
        WsAction::SendText(json!({"event":"typing","data":{}}).to_string()),
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
        .expect("the session ends when the server drops the socket")
        .expect_err("a dropped socket asks for a reconnect");
    let messages = received.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, WsMessage::Pong(data) if data == b"keepalive")),
        "the ping must be answered with a pong carrying the same payload: {messages:?}"
    );
}

#[tokio::test]
async fn a_server_close_frame_is_an_error_so_the_supervisor_reconnects() {
    let (url, _) = spawn_ws(vec![WsAction::SendClose]).await;
    let ctx = crate::bridge::ProviderCtx::offline(&DEFINITION);
    let socket = ws::connect(&url, &[]).await.unwrap();
    let sender: Arc<dyn ChannelSender> = Arc::new(NullSender);
    let allowlist = HashSet::new();
    let result = websocket_session(&ctx, sender, "token-1", "bot-id", &allowlist, socket).await;
    let error = result.expect_err("a close frame is not a clean shutdown");
    assert!(
        error.to_string().contains("closed by the server"),
        "{error}"
    );
}

// ─── Dispatch through the bridge ────────────────────────────────────────────

/// A sender that records reactions; delivery itself never happens in these
/// tests (the offline bridge denies, the mock-agent bridge accepts but the
/// sender is not consulted before the reaction).
struct RecordingSender {
    reactions: std::sync::Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl ChannelSender for RecordingSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
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
        _conversation: &ConversationRef,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        self.reactions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((message_id.to_string(), emoji.to_string()));
        Ok(())
    }
}

/// The fixture timestamp must sit inside the bridge's freshness window, or
/// dispatch drops the post as a stale replay before any assertion runs.
fn fresh_post(root_id: &str) -> Value {
    let mut post = channel_post(root_id);
    post["create_at"] = json!(crate::bridge::dedup::now_ms());
    post
}

#[tokio::test(flavor = "multi_thread")]
async fn an_accepted_post_is_acknowledged_with_an_eyes_reaction() {
    let (agent_addr, _shared) =
        crate::test_support::spawn_mock_grpc(crate::test_support::MockState::default()).await;
    let dir = crate::test_support::temp_dir("mm-dispatch-accepted");
    let sessions = Arc::new(crate::session_store::SessionStore::new(
        dir.join("sessions.json"),
    ));
    let agent_cfg = crate::config::AgentConfig {
        grpc_addr: agent_addr,
        cwd: dir.to_string_lossy().into_owned(),
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
        dir.to_path_buf(),
        Arc::new(crate::status::StatusBoard::new(dir.join("status.json"))),
    );
    let ctx = ProviderCtx::new(
        &DEFINITION,
        json!({"enabled": true}),
        bridge,
        dir.to_path_buf(),
        sessions,
        crate::bridge::Shutdown::new(),
    );
    let sender = Arc::new(RecordingSender {
        reactions: std::sync::Mutex::new(Vec::new()),
    });
    let event = posted_event(&fresh_post(""), &["bot-id"], "O");
    let posted = parse_ws_event(&event).unwrap();
    let dyn_sender: Arc<dyn ChannelSender> = sender.clone();
    dispatch_posted(&ctx, &dyn_sender, &posted, "bot-id", &HashSet::new()).await;
    // The ack may race the (unreachable-agent) turn; give the queue a moment.
    for _ in 0..50 {
        if !sender
            .reactions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let reactions = sender.reactions.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        reactions.as_slice(),
        &[("post1".to_string(), "eyes".to_string())],
        "an accepted post must be acknowledged"
    );
}

#[tokio::test]
async fn a_denied_post_gets_no_reaction() {
    // The offline bridge denies by default (empty DM allowlist), so dispatch
    // must not acknowledge.
    let ctx = crate::bridge::ProviderCtx::offline(&DEFINITION);
    let sender = Arc::new(RecordingSender {
        reactions: std::sync::Mutex::new(Vec::new()),
    });
    let event = posted_event(&fresh_post(""), &["bot-id"], "O");
    let posted = parse_ws_event(&event).unwrap();
    let dyn_sender: Arc<dyn ChannelSender> = sender.clone();
    dispatch_posted(&ctx, &dyn_sender, &posted, "bot-id", &HashSet::new()).await;
    assert!(sender
        .reactions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty());
}

#[tokio::test]
async fn a_post_the_gate_drops_never_reaches_the_bridge() {
    let ctx = crate::bridge::ProviderCtx::offline(&DEFINITION);
    let sender = Arc::new(RecordingSender {
        reactions: std::sync::Mutex::new(Vec::new()),
    });
    // The bot's own post is dropped inside parse_posted.
    let mut own = fresh_post("");
    own["user_id"] = json!("bot-id");
    let event = posted_event(&own, &[], "D");
    let posted = parse_ws_event(&event).unwrap();
    let dyn_sender: Arc<dyn ChannelSender> = sender.clone();
    dispatch_posted(&ctx, &dyn_sender, &posted, "bot-id", &HashSet::new()).await;
    assert!(sender
        .reactions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty());
}

#[test]
fn the_registry_entrypoint_builds_the_provider() {
    let provider = provider();
    assert!(std::ptr::eq(provider.definition(), &DEFINITION));
}

// ─── parse_ws_event / parse_posted remaining edges ─────────────────────────

#[test]
fn mentions_as_a_native_array_are_tolerated() {
    let event = json!({
        "event": "posted",
        "data": {
            "post": channel_post(""),
            "mentions": ["bot-id", "user-a"],
            "channel_type": "O",
        }
    });
    let posted = parse_ws_event(&event).expect("array mentions must parse");
    assert_eq!(posted.mentions, vec!["bot-id", "user-a"]);
}

#[test]
fn missing_mentions_and_missing_optional_fields_default() {
    let event = json!({
        "event": "posted",
        "data": { "post": channel_post("") }
    });
    let posted = parse_ws_event(&event).expect("sparse data must parse");
    assert!(posted.mentions.is_empty());
    assert!(posted.channel_type.is_empty());
    assert!(posted.sender_name.is_empty());
}

#[test]
fn a_post_without_data_or_a_valid_post_is_dropped() {
    assert!(parse_ws_event(&json!({"event": "posted"})).is_none());
    let event = json!({"event": "posted", "data": {"post": 42}});
    assert!(parse_ws_event(&event).is_none());
}

#[test]
fn a_post_without_an_id_or_a_sender_is_dropped() {
    for patch in [
        json!({"id": "", "user_id": "user-a", "channel_id": "chan1", "message": "hi"}),
        json!({"id": "p1", "user_id": "", "channel_id": "chan1", "message": "hi"}),
        json!({"user_id": "user-a", "channel_id": "chan1", "message": "hi"}),
    ] {
        let event = posted_event(&patch, &["bot-id"], "O");
        let posted = parse_ws_event(&event).unwrap();
        assert!(
            parse_posted(&posted, "bot-id", &HashSet::new())
                .unwrap()
                .is_none(),
            "{patch}"
        );
    }
}

#[test]
fn an_unknown_channel_type_is_a_channel_and_display_is_none_when_unnamed() {
    let mut post = channel_post("");
    post["create_at"] = Value::Null;
    let event = json!({
        "event": "posted",
        "data": {
            "post": post.to_string(),
            "mentions": "[]",
            "channel_type": "X",
            "sender_name": "",
        }
    });
    let posted = parse_ws_event(&event).unwrap();
    let inbound = parse_posted(&posted, "bot-id", &HashSet::new())
        .unwrap()
        .expect("a fresh post must normalize");
    assert_eq!(inbound.conversation.kind, ChatKind::Channel);
    assert_eq!(inbound.sender.display, None);
    assert_eq!(inbound.created_at_ms, None);
    assert!(!inbound.addressed_to_bot);
}

// ─── Run loop (mock HTTP + WS) ─────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn run_authenticates_then_handles_posts_until_shutdown() {
    let (base, _) = spawn_http(vec![HttpRoute::json("/users/me", 200, me_body())]).await;
    let hello = json!({"event": "hello", "data": {"server_version": "9.0"}});
    let posted = posted_event(&channel_post(""), &["bot-id"], "O");
    // The socket stays open until the test's own deadline; the run loop must
    // exit because shutdown fired, never because the socket ended.
    let (ws_url, received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(posted.to_string()),
        WsAction::Delay(std::time::Duration::from_secs(30)),
    ])
    .await;

    let dir = crate::test_support::temp_dir("mm-run");
    let shutdown = crate::bridge::Shutdown::new();
    let ctx = ProviderCtx::new(
        &DEFINITION,
        json!({
            "enabled": true,
            "base_url": base,
            "api_base": base,
            "token": "tok-1",
            "ws_url": ws_url,
        }),
        crate::bridge::Bridge::offline(),
        dir.to_path_buf(),
        Arc::new(crate::session_store::SessionStore::new(
            dir.join("sessions.json"),
        )),
        shutdown.clone(),
    );
    let run = tokio::spawn(async move { Mattermost.run(ctx).await });
    // Shutdown while the session is mid-read: the session's select hears the
    // notification and returns Ok, and the supervisor passes it through.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    shutdown.trigger();
    let result = tokio::time::timeout(std::time::Duration::from_secs(15), run)
        .await
        .expect("shutdown must end the run loop");
    assert!(result.unwrap().is_ok(), "shutdown is a clean exit");
    let messages = received.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let first = messages
        .first()
        .and_then(|m| m.to_text().ok())
        .expect("the client must send an auth challenge");
    let challenge: Value = serde_json::from_str(first).unwrap();
    assert_eq!(challenge["action"], "authentication_challenge");
    assert_eq!(challenge["data"]["token"], "tok-1");
}

#[tokio::test(flavor = "multi_thread")]
async fn run_marks_the_channel_running_once_the_socket_is_up() {
    let (base, _) = spawn_http(vec![HttpRoute::json("/users/me", 200, me_body())]).await;
    let hello = json!({"event": "hello", "data": {"server_version": "9.0"}});
    let (ws_url, _) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::Delay(std::time::Duration::from_secs(30)),
    ])
    .await;

    let dir = crate::test_support::temp_dir("mm-run-status");
    let status_path = dir.join("status.json");
    let shutdown = crate::bridge::Shutdown::new();
    let ctx = ProviderCtx::new(
        &DEFINITION,
        json!({
            "enabled": true,
            "base_url": base,
            "api_base": base,
            "token": "tok-1",
            "ws_url": ws_url,
        }),
        crate::bridge::Bridge::offline_at(dir.to_path_buf()),
        dir.to_path_buf(),
        Arc::new(crate::session_store::SessionStore::new(
            dir.join("sessions.json"),
        )),
        shutdown.clone(),
    );
    let run = tokio::spawn(async move { Mattermost.run(ctx).await });
    // `mark_running` fires right after the handshake and publishes the state.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut running = false;
    while std::time::Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&status_path) {
            if text.contains("\"running\"") {
                running = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    shutdown.trigger();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(15), run).await;
    assert!(
        running,
        "a connected run must publish the running state to {status_path:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn run_recovers_after_the_socket_drops_and_still_stops_on_shutdown() {
    let (base, _) = spawn_http(vec![HttpRoute::json("/users/me", 200, me_body())]).await;
    // First connection dies at once; every reconnect holds until the test
    // deadline. The supervisor must reconnect at least once (second auth
    // challenge) and then exit on shutdown.
    let hello = json!({"event": "hello", "data": {"server_version": "9.0"}});
    let (ws_url, received) = crate::test_support::spawn_ws_per_connection(vec![
        // The first connection authenticates and is then dropped by the
        // platform, so the reconnect has to authenticate again.
        vec![
            WsAction::SendText(hello.to_string()),
            WsAction::Delay(std::time::Duration::from_millis(200)),
            WsAction::SendClose,
        ],
        vec![
            WsAction::SendText(hello.to_string()),
            WsAction::Delay(std::time::Duration::from_secs(30)),
        ],
    ])
    .await;

    let dir = crate::test_support::temp_dir("mm-run-reconnect");
    let shutdown = crate::bridge::Shutdown::new();
    let ctx = ProviderCtx::new(
        &DEFINITION,
        json!({
            "enabled": true,
            "base_url": base,
            "api_base": base,
            "token": "tok-1",
            "ws_url": ws_url,
        }),
        crate::bridge::Bridge::offline(),
        dir.to_path_buf(),
        Arc::new(crate::session_store::SessionStore::new(
            dir.join("sessions.json"),
        )),
        shutdown.clone(),
    );
    let run = tokio::spawn(async move { Mattermost.run(ctx).await });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let challenges = loop {
        let count = {
            received
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .filter_map(|m| m.to_text().ok())
                .filter_map(|t| serde_json::from_str::<Value>(t).ok())
                .filter(|frame| frame["action"] == "authentication_challenge")
                .count()
        };
        if count >= 2 || std::time::Instant::now() >= deadline {
            break count;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    };
    assert!(
        challenges >= 2,
        "a dropped socket must lead to a re-authenticated reconnect, saw {challenges} challenge(s)"
    );
    shutdown.trigger();
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), run)
        .await
        .expect("shutdown must end the run loop");
    assert!(result.unwrap().is_ok(), "shutdown is a clean exit");
}

#[tokio::test(flavor = "multi_thread")]
async fn run_refuses_a_missing_token_and_fails_when_identity_fails() {
    let dir = crate::test_support::temp_dir("mm-run-no-token");
    let ctx = ctx_with_config(config_value("http://127.0.0.1:1", ""), &dir);
    let error = Mattermost.run(ctx).await.unwrap_err();
    assert!(error.to_string().contains("token is required"), "{error}");

    let (base, _) = spawn_http(vec![HttpRoute::json(
        "/users/me",
        401,
        r#"{"message":"Invalid or expired token"}"#,
    )])
    .await;
    let dir = crate::test_support::temp_dir("mm-run-bad-token");
    let ctx = ctx_with_config(config_value(&base, "tok-bad"), &dir);
    let error = Mattermost.run(ctx).await.unwrap_err();
    assert!(
        error.to_string().contains("Invalid or expired token"),
        "{error}"
    );
}
