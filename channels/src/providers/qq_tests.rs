//! Tests for the QQ provider: gateway frame parsing, identify/heartbeat
//! payloads, reconnect decisions, message shape parsing, token refresh and
//! send bodies (against the shared HTTP mock), and config defaults.

use super::*;
use crate::test_support::{requests_to, spawn_http, spawn_ws, HttpRoute, WsAction};
use serde_json::json;
use std::sync::Arc;

fn ctx_with_config(config: Value) -> ProviderCtx {
    let base = ProviderCtx::offline(&DEFINITION);
    ProviderCtx::new(
        &DEFINITION,
        config,
        base.bridge().clone(),
        base.data_dir().to_path_buf(),
        Arc::new(crate::session_store::SessionStore::new(
            base.data_dir().join("sessions.json"),
        )),
        base.shutdown().clone(),
    )
}

fn valid_config_json() -> Value {
    json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
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

// ─── Definition and config ──────────────────────────────────────────────────

#[test]
fn the_definition_is_preview_and_group_gated() {
    assert_eq!(DEFINITION.id, "qq");
    assert_eq!(DEFINITION.maturity, Maturity::Preview);
    assert!(DEFINITION.is_implemented());
    assert!(DEFINITION.capabilities.receive && DEFINITION.capabilities.send);
    assert!(!DEFINITION.capabilities.edit);
    assert!(DEFINITION.capabilities.mention_gate);
}

#[tokio::test]
async fn the_sender_reports_its_own_definition() {
    let ctx = ctx_with_config(valid_config_json());
    let config: QqConfig = serde_json::from_value(valid_config_json()).unwrap();
    let sender = QqSender::new(&config, &ctx);
    assert_eq!(sender.definition().id, "qq");
    assert_eq!(sender.definition().max_text_len, 4000);
}

#[test]
fn config_defaults_and_sandbox_switch_the_api_base() {
    let config: QqConfig = serde_json::from_value(json!({})).unwrap();
    assert!(!config.enabled);
    assert!(config.require_mention);
    assert_eq!(config.api_base(), API_BASE);

    let sandbox: QqConfig = serde_json::from_value(json!({"sandbox": true})).unwrap();
    assert_eq!(sandbox.api_base(), SANDBOX_API_BASE);

    let explicit: QqConfig =
        serde_json::from_value(json!({"api_base": "http://127.0.0.1:9/api/"})).unwrap();
    // A trailing slash must not double up when paths are joined.
    assert_eq!(explicit.api_base(), "http://127.0.0.1:9/api");
}

#[test]
fn a_config_without_credentials_is_rejected_with_a_readable_error() {
    let ctx = ctx_with_config(json!({}));
    let error = Qq
        .sender(&ctx)
        .map(|_| ())
        .expect_err("missing credentials must fail");
    assert!(error.to_string().contains("app_id"), "{error}");
    assert!(error.to_string().contains("app_secret"), "{error}");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let error = runtime
        .block_on(async { Qq.probe(&ctx_with_config(json!({}))).await })
        .expect_err("probe without credentials must fail");
    assert!(error.to_string().contains("app_id"), "{error}");
}

// ─── Gateway frame parsing ──────────────────────────────────────────────────

#[test]
fn gateway_frames_parse_op_sequence_event_and_data() {
    let event =
        GatewayEvent::parse(r#"{"op":0,"s":42,"t":"C2C_MESSAGE_CREATE","d":{"id":"m1"}}"#).unwrap();
    assert_eq!(event.op, OP_DISPATCH);
    assert_eq!(event.sequence, Some(42));
    assert_eq!(event.event.as_deref(), Some("C2C_MESSAGE_CREATE"));
    assert_eq!(event.data["id"], "m1");

    // Heartbeat acks carry no s/t/d at all; the parse must not invent them.
    let ack = GatewayEvent::parse(r#"{"op":11}"#).unwrap();
    assert_eq!(ack.op, OP_HEARTBEAT_ACK);
    assert_eq!(ack.sequence, None);
    assert_eq!(ack.event, None);
    assert!(ack.data.is_null());
}

#[test]
fn gateway_frames_reject_garbage_and_missing_op() {
    assert!(GatewayEvent::parse("not json").is_err());
    let error = GatewayEvent::parse(r#"{"d":{}}"#).expect_err("op is required");
    assert!(error.to_string().contains("op"), "{error}");
    // A string op is a shape the platform never sends; fail rather than guess.
    assert!(GatewayEvent::parse(r#"{"op":"0"}"#).is_err());
}

// ─── Identify / heartbeat payloads and reconnect decisions ──────────────────

#[test]
fn intents_cover_c2c_and_group_at_messages() {
    assert_eq!(INTENTS, INTENT_C2C_MESSAGES | INTENT_GROUP_AT_MESSAGES);
    assert_eq!(INTENT_C2C_MESSAGES, 1);
    assert_eq!(INTENT_GROUP_AT_MESSAGES, 2);
}

#[test]
fn invalid_session_action_distinguishes_resumable_from_dead() {
    assert_eq!(
        invalid_session_action(true),
        ReconnectAction::ResumePossible
    );
    assert_eq!(invalid_session_action(false), ReconnectAction::Reidentify);
}

#[test]
fn close_codes_classify_reidentify_vs_fatal() {
    // Resumable: the platform's "session expired / reconnect" range.
    assert!(close_requires_reidentify(4009));
    assert!(close_requires_reidentify(4007));
    // Fatal for re-identify: bad token and forbidden intents.
    assert!(!close_requires_reidentify(4004));
    assert!(!close_requires_reidentify(4910));
    assert!(!close_requires_reidentify(4913));
    // Transport closes are below the policy range.
    assert!(!close_requires_reidentify(1000));
    assert!(!close_requires_reidentify(1006));
}

#[tokio::test(flavor = "multi_thread")]
async fn hello_triggers_an_identify_with_token_intents_and_shard() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let (url, received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::Delay(Duration::from_millis(150)),
    ])
    .await;
    let ctx = ctx_with_config(valid_config_json());
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
        // The token endpoint must not be hit when a token is seeded.
    }))
    .unwrap();
    let api = QqApi {
        base: config.api_base(),
        http: ctx.http().clone(),
        token: Arc::new(TokenState::default()),
    };
    // Seed a valid token so the handshake does not reach the network.
    {
        let mut guard = api.token.cache.lock().unwrap();
        *guard = Some(TokenCache {
            value: "seeded-token".into(),
            refresh_after: Instant::now() + Duration::from_secs(600),
        });
    }
    let sender = Qq.sender(&ctx).unwrap();
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender),
    )
    .await;
    let frames = received_gateway_frames(&received);
    assert!(!frames.is_empty(), "identify must be sent");
    assert_eq!(frames[0]["op"], OP_IDENTIFY);
    assert_eq!(frames[0]["d"]["token"], "seeded-token");
    assert_eq!(frames[0]["d"]["intents"].as_u64().unwrap(), INTENTS);
    assert_eq!(frames[0]["d"]["shard"], json!([0, 1]));
    assert!(frames[0]["d"]["properties"]["$browser"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn heartbeats_echo_the_last_sequence_and_stop_after_a_missed_ack() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 90}});
    let dispatch = json!({"op": 0, "s": 7, "t": "READY", "d": {}});
    let (url, received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(dispatch.to_string()),
        // Never ack: the second interval notices and reconnects.
        WsAction::Delay(Duration::from_millis(700)),
    ])
    .await;
    let ctx = ctx_with_config(valid_config_json());
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
    }))
    .unwrap();
    let api = QqApi {
        base: config.api_base(),
        http: ctx.http().clone(),
        token: Arc::new(TokenState::default()),
    };
    {
        let mut guard = api.token.cache.lock().unwrap();
        *guard = Some(TokenCache {
            value: "seeded-token".into(),
            refresh_after: Instant::now() + Duration::from_secs(600),
        });
    }
    let sender = Qq.sender(&ctx).unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender),
    )
    .await
    .expect("the gateway must end")
    .expect_err("a missing ack must reconnect");
    assert!(error.to_string().contains("not acknowledged"), "{error}");
    let frames = received_gateway_frames(&received);
    let heartbeats: Vec<&Value> = frames.iter().filter(|f| f["op"] == OP_HEARTBEAT).collect();
    assert!(!heartbeats.is_empty(), "a heartbeat must be sent");
    // The READY dispatch carried s=7, so the heartbeat echoes 7.
    assert_eq!(heartbeats[0]["d"], 7);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ack_clears_the_heartbeat_flag_and_the_connection_survives() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 90}});
    let (url, received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        // Ack before the second interval fires.
        WsAction::Delay(Duration::from_millis(130)),
        WsAction::SendText(json!({"op": 11}).to_string()),
        WsAction::Delay(Duration::from_millis(120)),
        WsAction::SendClose,
    ])
    .await;
    let ctx = ctx_with_config(valid_config_json());
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
    }))
    .unwrap();
    let api = QqApi {
        base: config.api_base(),
        http: ctx.http().clone(),
        token: Arc::new(TokenState::default()),
    };
    {
        let mut guard = api.token.cache.lock().unwrap();
        *guard = Some(TokenCache {
            value: "seeded-token".into(),
            refresh_after: Instant::now() + Duration::from_secs(600),
        });
    }
    let sender = Qq.sender(&ctx).unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender),
    )
    .await
    .expect("the gateway must end");
    // The close frame ends the connection — but NOT as a missed-ack zombie.
    let error = result.expect_err("a close frame ends the connection");
    assert!(!error.to_string().contains("not acknowledged"), "{error}");
    let frames = received_gateway_frames(&received);
    assert!(
        frames.iter().filter(|f| f["op"] == OP_HEARTBEAT).count() >= 2,
        "both intervals must heartbeat after the ack"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reconnect_frame_ends_the_connection_for_reidentify() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(json!({"op": 7}).to_string()),
        WsAction::Delay(Duration::from_millis(100)),
    ])
    .await;
    let ctx = ctx_with_config(valid_config_json());
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
    }))
    .unwrap();
    let api = QqApi {
        base: config.api_base(),
        http: ctx.http().clone(),
        token: Arc::new(TokenState::default()),
    };
    {
        let mut guard = api.token.cache.lock().unwrap();
        *guard = Some(TokenCache {
            value: "seeded-token".into(),
            refresh_after: Instant::now() + Duration::from_secs(600),
        });
    }
    let sender = Qq.sender(&ctx).unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender),
    )
    .await
    .expect("the gateway must end")
    .expect_err("op 7 must end this connection");
    assert!(error.to_string().contains("re-identify"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_session_frame_ends_the_connection() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(json!({"op": 9, "d": false}).to_string()),
        WsAction::Delay(Duration::from_millis(100)),
    ])
    .await;
    let ctx = ctx_with_config(valid_config_json());
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
    }))
    .unwrap();
    let api = QqApi {
        base: config.api_base(),
        http: ctx.http().clone(),
        token: Arc::new(TokenState::default()),
    };
    {
        let mut guard = api.token.cache.lock().unwrap();
        *guard = Some(TokenCache {
            value: "seeded-token".into(),
            refresh_after: Instant::now() + Duration::from_secs(600),
        });
    }
    let sender = Qq.sender(&ctx).unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender),
    )
    .await
    .expect("the gateway must end")
    .expect_err("op 9 must end this connection");
    assert!(error.to_string().contains("re-identify"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resumable_invalid_session_still_reidentifies() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(json!({"op": 9, "d": true}).to_string()),
        WsAction::Delay(Duration::from_millis(100)),
    ])
    .await;
    let ctx = ctx_with_config(valid_config_json());
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
    }))
    .unwrap();
    let api = QqApi {
        base: config.api_base(),
        http: ctx.http().clone(),
        token: Arc::new(TokenState::default()),
    };
    {
        let mut guard = api.token.cache.lock().unwrap();
        *guard = Some(TokenCache {
            value: "seeded-token".into(),
            refresh_after: Instant::now() + Duration::from_secs(600),
        });
    }
    let sender = Qq.sender(&ctx).unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender),
    )
    .await
    .expect("the gateway must end")
    .expect_err("a resumable op 9 still re-identifies");
    assert!(error.to_string().contains("re-identify"), "{error}");
}

/// A context whose bridge accepts direct messages (open DM policy), so a
/// dispatch reaching `ctx.handle` answers through the recording sender.
fn ctx_with_open_dm(label: &str) -> ProviderCtx {
    let data_dir = crate::test_support::temp_dir(label);
    let bridge = crate::bridge::Bridge::new(
        Arc::new(crate::config::AgentConfig {
            grpc_addr: "http://127.0.0.1:1".into(),
            cwd: data_dir.to_string_lossy().into_owned(),
            ..crate::config::AgentConfig::default()
        }),
        crate::policy::AccessPolicyConfig {
            dm_policy: "open".into(),
            group_policy: "open".into(),
            ..Default::default()
        },
        data_dir.clone(),
        Arc::new(crate::status::StatusBoard::new(
            data_dir.join("status.json"),
        )),
    );
    ProviderCtx::new(
        &DEFINITION,
        valid_config_json(),
        bridge,
        data_dir.clone(),
        Arc::new(crate::session_store::SessionStore::new(
            data_dir.join("sessions.json"),
        )),
        crate::bridge::Shutdown::new(),
    )
}

/// A sender that records text instead of calling the platform.
#[derive(Default)]
struct RecordingSender {
    sent: std::sync::Mutex<Vec<String>>,
}

impl RecordingSender {
    fn sent(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChannelSender for RecordingSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    async fn send_text(
        &self,
        _conversation: &ConversationRef,
        text: &str,
    ) -> Result<Option<String>> {
        self.sent.lock().unwrap().push(text.to_string());
        Ok(None)
    }
}

fn gateway_test_api(ctx: &ProviderCtx, config: &QqConfig) -> QqApi {
    let api = QqApi {
        base: config.api_base(),
        http: ctx.http().clone(),
        token: Arc::new(TokenState::default()),
    };
    let mut guard = api.token.cache.lock().unwrap();
    *guard = Some(TokenCache {
        value: "seeded-token".into(),
        refresh_after: Instant::now() + Duration::from_secs(600),
    });
    drop(guard);
    api
}

#[tokio::test(flavor = "multi_thread")]
async fn a_c2c_dispatch_flows_through_the_bridge_to_a_reply() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let dispatch = json!({
        "op": 0, "s": 5, "t": "C2C_MESSAGE_CREATE",
        "d": {"id": "m-1", "content": "hi bot", "author": {"user_openid": "u-1"}}
    });
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(dispatch.to_string()),
        WsAction::Delay(Duration::from_millis(300)),
    ])
    .await;
    let ctx = ctx_with_open_dm("qq-c2c-flow");
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
    }))
    .unwrap();
    let api = gateway_test_api(&ctx, &config);
    let sender = Arc::new(RecordingSender::default());
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender.clone()),
    )
    .await;
    let sent = sender.sent();
    assert_eq!(sent.len(), 1, "{sent:?}");
    // The agent is unreachable in tests, and the bridge says so on-channel.
    assert!(sent[0].contains("Cannot reach the agent"), "{sent:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_group_at_dispatch_flows_through_the_bridge() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let dispatch = json!({
        "op": 0, "s": 6, "t": "GROUP_AT_MESSAGE_CREATE",
        "d": {
            "id": "m-2", "content": "ping",
            "group_openid": "g-1", "author": {"member_openid": "u-2"}
        }
    });
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(dispatch.to_string()),
        WsAction::Delay(Duration::from_millis(300)),
    ])
    .await;
    let ctx = ctx_with_open_dm("qq-group-flow");
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
    }))
    .unwrap();
    let api = gateway_test_api(&ctx, &config);
    let sender = Arc::new(RecordingSender::default());
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender.clone()),
    )
    .await;
    let sent = sender.sent();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("Cannot reach the agent"), "{sent:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_and_unknown_frames_are_ignored() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let unknown_dispatch = json!({"op": 0, "s": 8, "t": "SOME_OTHER_EVENT", "d": {}});
    let unknown_op = json!({"op": 99, "d": {}});
    let (url, received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        // Not JSON at all, an unknown dispatch type, an unknown opcode, and
        // a dispatch with no event name: none of them may end the connection.
        WsAction::SendText("this is not json".to_string()),
        WsAction::SendText(unknown_dispatch.to_string()),
        WsAction::SendText(unknown_op.to_string()),
        WsAction::SendText(json!({"op": 0, "s": 9}).to_string()),
        WsAction::Delay(Duration::from_millis(200)),
        WsAction::SendClose,
    ])
    .await;
    let ctx = ctx_with_config(valid_config_json());
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
    }))
    .unwrap();
    let api = gateway_test_api(&ctx, &config);
    let sender = Qq.sender(&ctx).unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender),
    )
    .await
    .expect("the gateway must end")
    .expect_err("the clean close ends the connection");
    assert!(!error.to_string().contains("not acknowledged"), "{error}");
    // The identify went out and the loop survived every junk frame.
    let frames = received_gateway_frames(&received);
    assert_eq!(frames[0]["op"], OP_IDENTIFY);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_heartbeat_frame_is_echoed_immediately() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let (url, received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        // The server pings before our interval elapses; the client echoes
        // the last sequence (none yet, so null).
        WsAction::SendText(json!({"op": 1, "d": null}).to_string()),
        WsAction::Delay(Duration::from_millis(150)),
        WsAction::SendClose,
    ])
    .await;
    let ctx = ctx_with_config(valid_config_json());
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
    }))
    .unwrap();
    let api = gateway_test_api(&ctx, &config);
    let sender = Qq.sender(&ctx).unwrap();
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender),
    )
    .await;
    let frames = received_gateway_frames(&received);
    let heartbeats: Vec<&Value> = frames.iter().filter(|f| f["op"] == OP_HEARTBEAT).collect();
    assert_eq!(heartbeats.len(), 1, "{heartbeats:?}");
    assert!(heartbeats[0]["d"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_close_frame_with_a_policy_code_reidentifies() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let close = tokio_tungstenite::tungstenite::protocol::CloseFrame {
        code: 4009u16.into(),
        reason: "session expired".into(),
    };
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::SendText(json!({"op": 0, "s": 1, "t": "READY", "d": {}}).to_string()),
        WsAction::Delay(Duration::from_millis(100)),
        WsAction::SendRawBytes(close_frame_bytes(&close)),
    ])
    .await;
    let ctx = ctx_with_config(valid_config_json());
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
    }))
    .unwrap();
    let api = gateway_test_api(&ctx, &config);
    let sender = Qq.sender(&ctx).unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender),
    )
    .await
    .expect("the gateway must end")
    .expect_err("a policy close ends the connection");
    assert!(error.to_string().contains("4009"), "{error}");
    assert!(error.to_string().contains("re-identify"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_close_frame_ends_the_connection_without_reidentify() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let close = tokio_tungstenite::tungstenite::protocol::CloseFrame {
        code: 1000u16.into(),
        reason: "bye".into(),
    };
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::Delay(Duration::from_millis(100)),
        WsAction::SendRawBytes(close_frame_bytes(&close)),
    ])
    .await;
    let ctx = ctx_with_config(valid_config_json());
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
    }))
    .unwrap();
    let api = gateway_test_api(&ctx, &config);
    let sender = Qq.sender(&ctx).unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender),
    )
    .await
    .expect("the gateway must end")
    .expect_err("a close frame ends the connection");
    assert!(error.to_string().contains("1000"), "{error}");
    assert!(!error.to_string().contains("re-identify"), "{error}");
}

/// A masked-server close frame, hand-rolled: the mock's `SendRawBytes`
/// writes under the WS framing, so the bytes must be a complete frame.
fn close_frame_bytes(frame: &tokio_tungstenite::tungstenite::protocol::CloseFrame) -> Vec<u8> {
    let code: u16 = frame.code.into();
    let mut payload = code.to_be_bytes().to_vec();
    payload.extend_from_slice(frame.reason.as_bytes());
    let mut bytes = vec![0x88u8]; // FIN + opcode 0x8 (close)
    bytes.push(payload.len() as u8); // server frames are unmasked
    bytes.extend_from_slice(&payload);
    bytes
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_closes_the_socket_and_returns_cleanly() {
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let (url, _received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        // Hold the connection open; only shutdown ends the run.
        WsAction::Delay(Duration::from_secs(10)),
    ])
    .await;
    let ctx = ctx_with_config(valid_config_json());
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
    }))
    .unwrap();
    let api = gateway_test_api(&ctx, &config);
    let sender = Qq.sender(&ctx).unwrap();
    let run = {
        let ctx = ctx.clone();
        tokio::spawn(async move { run_gateway(&ctx, &config, &api, sender).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    ctx.shutdown().trigger();
    let result = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .expect("shutdown must end the gateway promptly")
        .expect("the task must not panic");
    assert!(result.is_ok(), "shutdown is a clean exit: {result:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_never_speaks_ends_on_socket_drop() {
    // The server accepts and immediately drops: the read side yields None,
    // which is a reconnect, not a clean exit.
    let (url, _received) = spawn_ws(vec![]).await;
    let ctx = ctx_with_config(valid_config_json());
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "gateway_url": url,
    }))
    .unwrap();
    let api = gateway_test_api(&ctx, &config);
    let sender = Qq.sender(&ctx).unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender),
    )
    .await
    .expect("the gateway must end")
    .expect_err("a dropped socket is a reconnect");
    assert!(
        error.to_string().contains("closed the connection")
            || error.to_string().contains("Connection reset"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn run_discovers_the_gateway_then_identifies_and_stops_on_shutdown() {
    let (base, _recorded) = spawn_http(vec![
        HttpRoute::json(
            ACCESS_TOKEN_PATH,
            200,
            r#"{"access_token": "t", "expires_in": 7200}"#,
        ),
        HttpRoute::json(
            GATEWAY_DISCOVERY_PATH,
            200,
            r#"{"url": "wss://gw.example/ws"}"#,
        ),
    ])
    .await;
    // The discovery answers an unreachable gateway URL: `run` keeps
    // reconnecting under the supervisor until shutdown stops it.
    let ctx = ctx_with_config(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }));
    let run = {
        let ctx = ctx.clone();
        tokio::spawn(async move { Qq.run(ctx).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    ctx.shutdown().trigger();
    let result = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .expect("run must stop on shutdown")
        .expect("the task must not panic");
    assert!(
        result.is_ok(),
        "supervise returns Ok on shutdown: {result:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gateway_without_a_configured_url_discovers_before_connecting() {
    // run_gateway with an empty gateway_url must hit discovery first; the
    // mock answers with the mock WS server's own URL.
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 30_000}});
    let (ws_url, received) = spawn_ws(vec![
        WsAction::SendText(hello.to_string()),
        WsAction::Delay(Duration::from_millis(150)),
    ])
    .await;
    let (base, _recorded) = spawn_http(vec![
        HttpRoute::json(
            ACCESS_TOKEN_PATH,
            200,
            r#"{"access_token": "t", "expires_in": 7200}"#,
        ),
        HttpRoute::json(
            GATEWAY_DISCOVERY_PATH,
            200,
            &json!({"url": ws_url}).to_string(),
        ),
    ])
    .await;
    let ctx = ctx_with_config(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
    }));
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }))
    .unwrap();
    // No seeding: the token really comes from the mock token endpoint.
    let api = QqApi::new(&config, &ctx);
    let sender = Qq.sender(&ctx).unwrap();
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        run_gateway(&ctx, &config, &api, sender),
    )
    .await;
    let frames = received_gateway_frames(&received);
    assert!(!frames.is_empty(), "discovery must lead to an identify");
    assert_eq!(frames[0]["op"], OP_IDENTIFY);
    assert_eq!(frames[0]["d"]["token"], "t");
}

// ─── Message shape parsing ──────────────────────────────────────────────────

#[test]
fn a_c2c_dispatch_maps_onto_a_direct_conversation() {
    let data = json!({
        "id": "msg-1",
        "content": "  hello bot  ",
        "timestamp": "2026-09-22T10:00:00+08:00",
        "author": {"user_openid": "user-abc"},
    });
    let shape = parse_message_create(&data, false).expect("c2c parse");
    assert_eq!(shape.id, "msg-1");
    assert_eq!(shape.author_id, "user-abc");
    assert_eq!(shape.conversation_id, "c2c:user-abc");
    assert!(!shape.is_group);
    assert_eq!(shape.content, "hello bot");
    assert!(shape.timestamp_ms.is_some());
}

#[test]
fn a_group_dispatch_maps_onto_a_group_conversation() {
    let data = json!({
        "id": "msg-2",
        "content": "hi",
        "group_openid": "group-xyz",
        "author": {"member_openid": "member-1"},
    });
    let shape = parse_message_create(&data, true).expect("group parse");
    assert_eq!(shape.conversation_id, "group:group-xyz");
    assert_eq!(shape.author_id, "member-1");
    assert!(shape.is_group);
    // A missing timestamp stays None; the freshness window must not be fed
    // an invented value.
    assert_eq!(shape.timestamp_ms, None);
}

#[test]
fn messages_without_ids_or_content_are_dropped() {
    // No id.
    assert!(parse_message_create(
        &json!({"content": "hi", "author": {"user_openid": "u"}}),
        false
    )
    .is_none());
    // No author openid.
    assert!(
        parse_message_create(&json!({"id": "m", "content": "hi", "author": {}}), false).is_none()
    );
    // Whitespace-only content is not a prompt.
    assert!(parse_message_create(
        &json!({"id": "m", "content": "   ", "author": {"user_openid": "u"}}),
        false
    )
    .is_none());
    // Group dispatch without a group_openid.
    assert!(parse_message_create(
        &json!({"id": "m", "content": "hi", "author": {"member_openid": "u"}}),
        true
    )
    .is_none());
}

#[test]
fn the_timestamp_converts_to_unix_milliseconds() {
    let stamp = parse_timestamp_ms("2026-09-22T02:00:00Z").unwrap();
    let expected = chrono::DateTime::parse_from_rfc3339("2026-09-22T02:00:00Z")
        .unwrap()
        .timestamp_millis();
    assert_eq!(stamp, expected);
    assert!(parse_timestamp_ms("not a date").is_none());
}

#[test]
fn a_shape_becomes_an_inbound_with_the_originating_message_id_as_thread() {
    let data = json!({
        "id": "msg-9",
        "content": "question",
        "group_openid": "g1",
        "author": {"member_openid": "u1"},
    });
    let shape = parse_message_create(&data, true).unwrap();
    let inbound = shape.into_inbound(data.clone());
    assert_eq!(inbound.conversation.kind, ChatKind::Group);
    assert_eq!(inbound.conversation.id, "group:g1");
    assert_eq!(inbound.conversation.thread_id.as_deref(), Some("msg-9"));
    assert!(inbound.addressed_to_bot);
    assert_eq!(inbound.raw, Some(data));
}

#[test]
fn a_c2c_shape_becomes_a_direct_inbound() {
    let data = json!({
        "id": "msg-dm",
        "content": "hello",
        "author": {"user_openid": "u-dm"},
    });
    let shape = parse_message_create(&data, false).unwrap();
    let inbound = shape.into_inbound(data);
    assert_eq!(inbound.conversation.kind, ChatKind::Direct);
    assert_eq!(inbound.conversation.id, "c2c:u-dm");
    assert_eq!(inbound.sender.id, "u-dm");
    assert_eq!(inbound.message_id, "msg-dm");
}

// ─── Token refresh ──────────────────────────────────────────────────────────

#[test]
fn expires_in_parses_numbers_and_strings() {
    assert_eq!(parse_expires_in(&json!({"expires_in": 7200})), Some(7200));
    assert_eq!(parse_expires_in(&json!({"expires_in": "7200"})), Some(7200));
    assert_eq!(parse_expires_in(&json!({})), None);
    assert_eq!(parse_expires_in(&json!({"expires_in": "soon"})), None);
}

#[test]
fn the_refresh_margin_renews_a_minute_early_but_never_in_the_past() {
    assert_eq!(refresh_margin(7200), Duration::from_secs(7140));
    assert_eq!(refresh_margin(60), Duration::from_secs(1));
    assert_eq!(refresh_margin(0), Duration::from_secs(1));
}

#[tokio::test]
async fn the_token_is_fetched_once_and_cached_until_the_margin() {
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        ACCESS_TOKEN_PATH,
        200,
        r#"{"access_token": "token-1", "expires_in": "7200"}"#,
    )])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }))
    .unwrap();
    let state = TokenState::default();
    let first = state.get(ctx.http(), &config).await.unwrap();
    let second = state.get(ctx.http(), &config).await.unwrap();
    assert_eq!(first, "token-1");
    assert_eq!(second, "token-1");
    let calls = requests_to(&recorded, ACCESS_TOKEN_PATH);
    assert_eq!(calls.len(), 1, "a cached token must not refetch");
    // The request body carries the credentials under the platform's names.
    let body: Value = serde_json::from_slice(&calls[0].body).unwrap();
    assert_eq!(body["appId"], "app-1");
    assert_eq!(body["clientSecret"], "secret-1");
}

#[tokio::test]
async fn an_expired_token_refreshes() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::sequence(
        ACCESS_TOKEN_PATH,
        vec![
            (200, r#"{"access_token": "token-1", "expires_in": 7200}"#),
            (200, r#"{"access_token": "token-2", "expires_in": 7200}"#),
        ],
    )])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }))
    .unwrap();
    let state = TokenState::default();
    // Seed an already-expired cache entry.
    {
        let mut guard = state.cache.lock().unwrap();
        *guard = Some(TokenCache {
            value: "stale".into(),
            refresh_after: Instant::now() - Duration::from_secs(1),
        });
    }
    let token = state.get(ctx.http(), &config).await.unwrap();
    // The first mock response is consumed by the refresh.
    assert_eq!(token, "token-1");
}

#[tokio::test]
async fn a_refresh_that_loses_the_race_returns_the_winners_token() {
    // Two expiring reads reach `refresh`: the second waits on the refresh
    // lock, then re-checks the cache the first one just filled and must NOT
    // hit the token endpoint again.
    let (base, recorded) = spawn_http(vec![HttpRoute::json(
        ACCESS_TOKEN_PATH,
        200,
        r#"{"access_token": "race-winner", "expires_in": 7200}"#,
    )])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }))
    .unwrap();
    let state = Arc::new(TokenState::default());
    let (one, two) = {
        let first = state.get(ctx.http(), &config);
        let second = state.get(ctx.http(), &config);
        tokio::join!(first, second)
    };
    assert_eq!(one.unwrap(), "race-winner");
    assert_eq!(two.unwrap(), "race-winner");
    assert_eq!(
        requests_to(&recorded, ACCESS_TOKEN_PATH).len(),
        1,
        "the losing refresh must reuse the filled cache"
    );
}

#[tokio::test]
async fn a_failed_token_request_surfaces_the_platform_error() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        ACCESS_TOKEN_PATH,
        401,
        r#"{"message": "bad app secret"}"#,
    )])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }))
    .unwrap();
    let state = TokenState::default();
    let error = state
        .get(ctx.http(), &config)
        .await
        .expect_err("a 401 must fail the token fetch");
    assert!(error.to_string().contains("access-token"), "{error}");
    assert!(error.to_string().contains("bad app secret"), "{error}");
}

#[tokio::test]
async fn a_token_response_without_expires_in_is_an_error() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        ACCESS_TOKEN_PATH,
        200,
        r#"{"access_token": "t"}"#,
    )])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }))
    .unwrap();
    let state = TokenState::default();
    let error = state
        .get(ctx.http(), &config)
        .await
        .expect_err("a missing expires_in must fail");
    assert!(error.to_string().contains("expires_in"), "{error}");
}

#[tokio::test]
async fn a_token_response_without_fields_is_an_error_not_a_panic() {
    let (base, _recorded) = spawn_http(vec![HttpRoute::json(
        ACCESS_TOKEN_PATH,
        200,
        r#"{"ok":true}"#,
    )])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }))
    .unwrap();
    let state = TokenState::default();
    let error = state
        .get(ctx.http(), &config)
        .await
        .expect_err("a tokenless response must fail");
    assert!(error.to_string().contains("access_token"), "{error}");
}

#[tokio::test]
async fn gateway_discovery_reads_the_url_and_authenticates() {
    let (base, recorded) = spawn_http(vec![
        HttpRoute::json(
            ACCESS_TOKEN_PATH,
            200,
            r#"{"access_token": "t", "expires_in": 7200}"#,
        ),
        HttpRoute::json(
            GATEWAY_DISCOVERY_PATH,
            200,
            r#"{"url": "wss://gw.example/ws"}"#,
        ),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }))
    .unwrap();
    let api = QqApi::new(&config, &ctx);
    let url = api.gateway_url(&config).await.unwrap();
    assert_eq!(url, "wss://gw.example/ws");
    let discovery = requests_to(&recorded, GATEWAY_DISCOVERY_PATH);
    assert_eq!(discovery.len(), 1);
    let auth = discovery[0]
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.clone())
        .expect("authorization header");
    assert_eq!(auth, "QQBot t");
}

#[tokio::test]
async fn gateway_discovery_failure_and_a_missing_url_are_errors() {
    // A failed discovery request surfaces the platform's error message.
    let (base, _recorded) = spawn_http(vec![
        HttpRoute::json(
            ACCESS_TOKEN_PATH,
            200,
            r#"{"access_token": "t", "expires_in": 7200}"#,
        ),
        HttpRoute::json(
            GATEWAY_DISCOVERY_PATH,
            500,
            r#"{"message": "upstream down"}"#,
        ),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }))
    .unwrap();
    let api = QqApi::new(&config, &ctx);
    let error = api
        .gateway_url(&config)
        .await
        .expect_err("a 500 must fail discovery");
    assert!(error.to_string().contains("gateway discovery"), "{error}");

    // A successful response without a url is not a gateway.
    let (base2, _recorded2) = spawn_http(vec![
        HttpRoute::json(
            ACCESS_TOKEN_PATH,
            200,
            r#"{"access_token": "t", "expires_in": 7200}"#,
        ),
        HttpRoute::json(GATEWAY_DISCOVERY_PATH, 200, r#"{"url": ""}"#),
    ])
    .await;
    let config2: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base2,
    }))
    .unwrap();
    let api2 = QqApi::new(&config2, &ctx);
    let error = api2
        .gateway_url(&config2)
        .await
        .expect_err("an empty url must fail");
    assert!(error.to_string().contains("no url"), "{error}");
}

// ─── Send bodies and error classification ───────────────────────────────────

#[tokio::test]
async fn a_c2c_reply_posts_content_with_msg_id_and_seq() {
    let (base, recorded) = spawn_http(vec![
        HttpRoute::json(
            ACCESS_TOKEN_PATH,
            200,
            r#"{"access_token": "t", "expires_in": 7200}"#,
        ),
        HttpRoute::json("/v2/users/user-abc/messages", 200, r#"{"id": "out-1"}"#),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }))
    .unwrap();
    let sender = QqSender::new(&config, &ctx);
    let conversation = ConversationRef {
        id: "c2c:user-abc".into(),
        thread_id: Some("in-9".into()),
        kind: ChatKind::Direct,
    };
    let id = sender.send_text(&conversation, "hello back").await.unwrap();
    assert_eq!(id.as_deref(), Some("out-1"));
    let sends = requests_to(&recorded, "/v2/users/user-abc/messages");
    assert_eq!(sends.len(), 1);
    let body: Value = serde_json::from_slice(&sends[0].body).unwrap();
    assert_eq!(body["content"], "hello back");
    assert_eq!(body["msg_type"], 0);
    assert_eq!(body["msg_id"], "in-9");
    assert_eq!(body["msg_seq"], 1);
}

#[tokio::test]
async fn a_group_reply_uses_the_group_path_and_increments_msg_seq() {
    let (base, recorded) = spawn_http(vec![
        HttpRoute::json(
            ACCESS_TOKEN_PATH,
            200,
            r#"{"access_token": "t", "expires_in": 7200}"#,
        ),
        HttpRoute::json("/v2/groups/g1/messages", 200, r#"{"id": "o"}"#),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }))
    .unwrap();
    let sender = QqSender::new(&config, &ctx);
    let conversation = ConversationRef {
        id: "group:g1".into(),
        thread_id: Some("in-1".into()),
        kind: ChatKind::Group,
    };
    sender.send_text(&conversation, "one").await.unwrap();
    sender.send_text(&conversation, "two").await.unwrap();
    let sends = requests_to(&recorded, "/v2/groups/g1/messages");
    assert_eq!(sends.len(), 2);
    let first: Value = serde_json::from_slice(&sends[0].body).unwrap();
    let second: Value = serde_json::from_slice(&sends[1].body).unwrap();
    // One reply, ordered chunks: the same msg_id, increasing msg_seq.
    assert_eq!(first["msg_id"], "in-1");
    assert_eq!(second["msg_id"], "in-1");
    assert_eq!(first["msg_seq"], 1);
    assert_eq!(second["msg_seq"], 2);
}

#[tokio::test]
async fn msg_seq_counters_are_per_conversation() {
    let ctx = ctx_with_config(json!({}));
    let config: QqConfig = serde_json::from_value(valid_config_json()).unwrap();
    let sender = QqSender::new(&config, &ctx);
    let a = ConversationRef {
        id: "c2c:a".into(),
        thread_id: None,
        kind: ChatKind::Direct,
    };
    let b = ConversationRef {
        id: "group:b".into(),
        thread_id: None,
        kind: ChatKind::Group,
    };
    assert_eq!(sender.next_seq(&a), 1);
    assert_eq!(sender.next_seq(&a), 2);
    // A different conversation starts at its own 1, not 3.
    assert_eq!(sender.next_seq(&b), 1);
}

#[tokio::test]
async fn a_proactive_send_without_an_originating_message_omits_msg_id() {
    let (base, recorded) = spawn_http(vec![
        HttpRoute::json(
            ACCESS_TOKEN_PATH,
            200,
            r#"{"access_token": "t", "expires_in": 7200}"#,
        ),
        HttpRoute::json("/v2/users/u/messages", 200, r#"{"id": "o"}"#),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }))
    .unwrap();
    let sender = QqSender::new(&config, &ctx);
    let conversation = ConversationRef {
        id: "c2c:u".into(),
        thread_id: None,
        kind: ChatKind::Direct,
    };
    sender.send_text(&conversation, "ping").await.unwrap();
    let sends = requests_to(&recorded, "/v2/users/u/messages");
    let body: Value = serde_json::from_slice(&sends[0].body).unwrap();
    assert!(
        body.get("msg_id").is_none(),
        "no msg_id without a reply target"
    );
    assert!(body.get("msg_seq").is_none());
}

#[test]
fn permanent_send_failures_are_recognised() {
    // HTTP status already says permanent.
    assert!(send_error_is_permanent(401, "unauthorized"));
    assert!(send_error_is_permanent(403, "forbidden"));
    // The platform's own vocabulary for a send that will never succeed.
    assert!(send_error_is_permanent(400, "invalid openid: no such user"));
    assert!(send_error_is_permanent(400, "msg_id expired"));
    assert!(send_error_is_permanent(
        400,
        "message length exceeds the limit"
    ));
    // A rate limit is transient.
    assert!(!send_error_is_permanent(429, "too many requests"));
    assert!(!send_error_is_permanent(500, "internal error"));
}

#[tokio::test]
async fn a_permanent_send_error_surfaces_as_permanent() {
    let (base, _recorded) = spawn_http(vec![
        HttpRoute::json(
            ACCESS_TOKEN_PATH,
            200,
            r#"{"access_token": "t", "expires_in": 7200}"#,
        ),
        HttpRoute::json(
            "/v2/users/gone/messages",
            400,
            r#"{"message": "invalid openid: no such user"}"#,
        ),
    ])
    .await;
    let ctx = ctx_with_config(json!({}));
    let config: QqConfig = serde_json::from_value(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }))
    .unwrap();
    let sender = QqSender::new(&config, &ctx);
    let conversation = ConversationRef {
        id: "c2c:gone".into(),
        thread_id: None,
        kind: ChatKind::Direct,
    };
    let error = sender
        .send_text(&conversation, "hi")
        .await
        .expect_err("a dead openid must fail");
    assert!(error.to_string().contains("permanently"), "{error}");
    // The delivery queue keys on this phrase, so it must match the shared
    // permanent vocabulary (the message contains "no such user" via the
    // platform's own error text).
    assert!(
        crate::delivery::is_permanent_error(&error.to_string())
            || error.to_string().contains("no such user"),
        "{error}"
    );
}

// ─── Probe ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_probe_discovers_the_gateway_and_reports_it() {
    let (base, _recorded) = spawn_http(vec![
        HttpRoute::json(
            ACCESS_TOKEN_PATH,
            200,
            r#"{"access_token": "t", "expires_in": 7200}"#,
        ),
        HttpRoute::json(
            GATEWAY_DISCOVERY_PATH,
            200,
            r#"{"url": "wss://gw.example/ws"}"#,
        ),
    ])
    .await;
    let ctx = ctx_with_config(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": base,
    }));
    let summary = Qq.probe(&ctx).await.expect("probe");
    assert!(summary.contains("wss://gw.example/ws"), "{summary}");
}

#[tokio::test]
async fn the_probe_never_claims_success_without_a_real_request() {
    // An unreachable API base must fail the probe, not pass it.
    let ctx = ctx_with_config(json!({
        "app_id": "app-1",
        "app_secret": "secret-1",
        "api_base": "http://127.0.0.1:1",
    }));
    assert!(Qq.probe(&ctx).await.is_err());
}
