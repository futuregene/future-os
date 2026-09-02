//! Feishu/Lark channel bridge.
//!
//! Connects to Feishu via WebSocket long connection, receives messages,
//! forwards them to FutureAgent via gRPC, and streams responses back.

pub mod bridge;
pub mod card;
pub mod config;
pub mod feishu_rest;
pub mod feishu_ws;
pub mod policy;
pub mod prompt_loop;
pub mod session_store;

use crate::config::AgentConfig;
use anyhow::Result;
use std::sync::Arc;
use tracing::{error, info, warn};

const AGENT_RECONNECT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(20);
const WEBSOCKET_RECONNECT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

async fn wait_or_shutdown(delay: std::time::Duration, shutdown: &tokio::sync::Notify) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        _ = shutdown.notified() => true,
    }
}

pub struct FeishuChannel;

impl FeishuChannel {
    /// Start the Feishu channel. Connects WebSocket and enters the event loop.
    /// Auto-reconnects on disconnect. Checks `shutdown` to stop on Ctrl-C.
    pub async fn run(
        agent_cfg: Arc<AgentConfig>,
        ch_cfg: crate::config::FeishuChannelConfig,
        shutdown: Arc<tokio::sync::Notify>,
    ) -> Result<()> {
        let feishu_cfg = config::FeishuConfig::from_channel_config(&ch_cfg);

        let ws_client = feishu_ws::FeishuWsClient::new(
            &feishu_cfg.api_domain(),
            &feishu_cfg.app_id,
            &feishu_cfg.app_secret,
        );

        loop {
            let bridge = loop {
                let result = tokio::select! {
                    result = bridge::Bridge::new(agent_cfg.clone(), feishu_cfg.clone()) => result,
                    _ = shutdown.notified() => {
                        info!("Feishu channel shutting down");
                        return Ok(());
                    }
                };
                match result {
                    Ok(bridge) => break Arc::new(bridge),
                    Err(error) => {
                        warn!(
                            "Failed to connect Feishu channel to Agent: {}. Retrying in 20s...",
                            error
                        );
                        if wait_or_shutdown(AGENT_RECONNECT_INTERVAL, &shutdown).await {
                            info!("Feishu channel shutting down");
                            return Ok(());
                        }
                    }
                }
            };

            let b = bridge.clone();
            let monitored_bridge = bridge.clone();
            let sd = shutdown.clone();

            // Supervise both connections. If the Agent stream disappears,
            // rebuild the bridge after the fixed retry interval; this also
            // replaces tonic's stale transport before reconnecting Feishu.
            let retry_delay = tokio::select! {
                result = ws_client.connect_and_listen(move |event| {
                    let b = b.clone();
                    tokio::spawn(async move {
                        if let Err(e) = b.handle_event(event).await {
                            error!("Error handling event: {}", e);
                        }
                    });
                }) => match result {
                    Ok(()) => {
                        info!("WebSocket closed cleanly, reconnecting...");
                        None
                    }
                    Err(error) => {
                        warn!("WebSocket error: {}. Reconnecting in 5s...", error);
                        Some(WEBSOCKET_RECONNECT_INTERVAL)
                    }
                },
                result = monitored_bridge.wait_for_agent_disconnect() => {
                    match result {
                        Ok(()) => warn!("Agent connection monitor stopped unexpectedly"),
                        Err(error) => warn!("Agent connection lost: {}", error),
                    }
                    warn!("Reconnecting to Agent in 20s...");
                    Some(AGENT_RECONNECT_INTERVAL)
                },
                _ = sd.notified() => {
                    info!("Feishu channel shutting down");
                    return Ok(());
                }
            };

            if let Some(delay) = retry_delay {
                if wait_or_shutdown(delay, &shutdown).await {
                    info!("Feishu channel shutting down");
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{self as ts, HttpRoute, MockState, WsAction};
    use std::time::Duration;

    fn ch_cfg(domain: &str) -> crate::config::FeishuChannelConfig {
        crate::config::FeishuChannelConfig {
            enabled: true,
            app_id: "app".into(),
            app_secret: "secret".into(),
            domain: domain.into(), // full URL → api_domain() verbatim
            ..Default::default()
        }
    }

    fn agent_cfg(addr: &str) -> Arc<AgentConfig> {
        Arc::new(AgentConfig {
            grpc_addr: addr.into(),
            cwd: "/tmp".into(),
            model: String::new(),
            thinking_level: String::new(),
            permission_level: String::new(),
        })
    }

    fn http_routes(ws_url: &str) -> Vec<HttpRoute> {
        vec![
            HttpRoute::json(
                "/auth/v3/tenant_access_token/internal",
                200,
                r#"{"code":0,"tenant_access_token":"tok","expire":7200}"#,
            ),
            HttpRoute::json(
                "/bot/v3/info",
                200,
                r#"{"code":0,"bot":{"open_id":"ou_bot","app_name":"Bot"}}"#,
            ),
            HttpRoute::json(
                "/callback/ws/endpoint",
                200,
                &serde_json::json!({"code": 0, "data": {"URL": ws_url}}).to_string(),
            ),
        ]
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_keeps_retrying_without_agent_until_shutdown() {
        ts::ensure_crypto_provider();
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let handle = tokio::spawn(FeishuChannel::run(
            agent_cfg("127.0.0.1:1"),
            ch_cfg("http://127.0.0.1:1"),
            shutdown.clone(),
        ));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !handle.is_finished(),
            "channel must keep retrying the Agent"
        );
        shutdown.notify_waiters();
        let result = tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("shutdown must interrupt Agent retry backoff")
            .expect("task join");
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_connects_then_shuts_down() {
        ts::ensure_crypto_provider();
        // WS holds the connection open until the client goes away.
        let (ws_url, _) = ts::spawn_ws(vec![WsAction::Delay(Duration::from_secs(30))]).await;
        let (base, _) = ts::spawn_http(http_routes(&ws_url)).await;
        let (addr, _) = ts::spawn_mock_grpc(MockState::default()).await;
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let sd = shutdown.clone();
        let handle = tokio::spawn(FeishuChannel::run(agent_cfg(&addr), ch_cfg(&base), sd));
        // Give it a moment to connect, then shut down.
        tokio::time::sleep(Duration::from_millis(500)).await;
        shutdown.notify_waiters();
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("run must return after shutdown")
            .expect("task join");
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_reconnects_after_clean_close() {
        ts::ensure_crypto_provider();
        // First connection closes immediately (clean-close arm); later
        // connections hold open so shutdown lands in the select.
        let (ws_url, _) = ts::spawn_ws_per_connection(vec![
            vec![WsAction::SendClose],
            vec![WsAction::Delay(Duration::from_secs(30))],
        ])
        .await;
        let (base, _) = ts::spawn_http(http_routes(&ws_url)).await;
        let (addr, _) = ts::spawn_mock_grpc(MockState::default()).await;
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let sd = shutdown.clone();
        let handle = tokio::spawn(FeishuChannel::run(agent_cfg(&addr), ch_cfg(&base), sd));
        tokio::time::sleep(Duration::from_millis(500)).await;
        shutdown.notify_waiters();
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("run must return after shutdown")
            .expect("task join");
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_reconnect_backoff_interrupted_by_shutdown() {
        ts::ensure_crypto_provider();
        // WS bootstrap fails (500) → warn + 5s backoff; shutdown cancels it.
        let mut routes = http_routes("ws://unused/");
        routes.retain(|r| r.path != "/callback/ws/endpoint");
        routes.push(HttpRoute::json("/callback/ws/endpoint", 500, "{}"));
        let (base, _) = ts::spawn_http(routes).await;
        let (addr, _) = ts::spawn_mock_grpc(MockState::default()).await;
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let sd = shutdown.clone();
        let handle = tokio::spawn(FeishuChannel::run(agent_cfg(&addr), ch_cfg(&base), sd));
        tokio::time::sleep(Duration::from_millis(300)).await;
        shutdown.notify_waiters();
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("shutdown cancels the backoff sleep")
            .expect("task join");
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_agent_disconnect_backoff_is_interrupted_by_shutdown() {
        ts::ensure_crypto_provider();
        let (ws_url, _) = ts::spawn_ws(vec![WsAction::Delay(Duration::from_secs(30))]).await;
        let (base, _) = ts::spawn_http(http_routes(&ws_url)).await;
        let (addr, _) = ts::spawn_mock_grpc(MockState {
            stream_status_error: true,
            ..Default::default()
        })
        .await;
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let handle = tokio::spawn(FeishuChannel::run(
            agent_cfg(&addr),
            ch_cfg(&base),
            shutdown.clone(),
        ));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !handle.is_finished(),
            "channel must stay alive during Agent reconnect backoff"
        );
        shutdown.notify_waiters();
        let result = tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("shutdown must interrupt Agent reconnect backoff")
            .expect("task join");
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_dispatches_events_to_the_bridge() {
        ts::ensure_crypto_provider();
        // One real event frame through the WS: the default policy (allowlist,
        // empty) denies the DM; the deny reply hits an unregistered route
        // (404 → code -1) so handle_event errors → the error-log arm runs.
        use prost::Message as _;
        let event_json = serde_json::json!({
            "header": {"event_type": "im.message.receive_v1"},
            "event": {
                "sender": {"sender_id": {"open_id": "ou_stranger"}},
                "message": {
                    "message_id": "om_ev", "chat_id": "oc_1", "chat_type": "p2p",
                    "message_type": "text", "content": "{\"text\":\"hi\"}"
                }
            }
        })
        .to_string();
        let frame = super::feishu_ws::feishu_pb::WsFrame {
            seq_id: 1,
            log_id: 2,
            service: 0,
            method: 0,
            headers: vec![super::feishu_ws::feishu_pb::Header {
                key: "type".into(),
                value: "event".into(),
            }],
            payload: event_json.into_bytes(),
            payload_encoding: String::new(),
            payload_type: String::new(),
            log_id_new: String::new(),
        };
        let mut buf = Vec::new();
        frame.encode(&mut buf).unwrap();
        // A stale event too: handle_event returns Ok → the handler's
        // if-let-Err false path.
        let old_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            - 120_000;
        let stale_json = serde_json::json!({
            "header": {"event_type": "im.message.receive_v1"},
            "event": {
                "sender": {"sender_id": {"open_id": "ou_1"}},
                "message": {
                    "message_id": "om_stale", "chat_id": "oc_1", "chat_type": "p2p",
                    "message_type": "text", "content": "{\"text\":\"old\"}",
                    "create_time": old_ms.to_string()
                }
            }
        })
        .to_string();
        let stale_frame = super::feishu_ws::feishu_pb::WsFrame {
            seq_id: 2,
            log_id: 3,
            service: 0,
            method: 0,
            headers: vec![super::feishu_ws::feishu_pb::Header {
                key: "type".into(),
                value: "event".into(),
            }],
            payload: stale_json.into_bytes(),
            payload_encoding: String::new(),
            payload_type: String::new(),
            log_id_new: String::new(),
        };
        let mut buf2 = Vec::new();
        stale_frame.encode(&mut buf2).unwrap();
        let (ws_url, _) = ts::spawn_ws(vec![
            WsAction::SendBinary(buf),
            WsAction::SendBinary(buf2),
            WsAction::Delay(Duration::from_millis(400)),
            WsAction::SendClose,
        ])
        .await;
        let (base, _) = ts::spawn_http(http_routes(&ws_url)).await;
        let (addr, _) = ts::spawn_mock_grpc(MockState::default()).await;
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let sd = shutdown.clone();
        let handle = tokio::spawn(FeishuChannel::run(agent_cfg(&addr), ch_cfg(&base), sd));
        // Let the event flow through, then shut down.
        tokio::time::sleep(Duration::from_millis(800)).await;
        shutdown.notify_waiters();
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("run must return after shutdown")
            .expect("task join");
        assert!(result.is_ok());
    }
}
