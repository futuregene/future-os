//! DingTalk/Lark channel bridge.
//!
//! Connects to DingTalk via WebSocket stream mode, receives messages,
//! forwards them to FutureAgent via gRPC, and streams responses back.

pub mod bridge;
pub mod card;
pub mod config;
pub mod dingtalk_rest;
pub mod dingtalk_ws;

use crate::config::{AgentConfig, DingtalkChannelConfig};
use anyhow::Result;
use std::sync::Arc;
use tracing::{error, info, warn};

pub struct DingtalkChannel;

impl DingtalkChannel {
    /// Start the DingTalk channel. Connects WebSocket and enters the event loop.
    /// Auto-reconnects on disconnect.
    pub async fn run(
        agent_cfg: Arc<AgentConfig>,
        ch_cfg: DingtalkChannelConfig,
        shutdown: Arc<tokio::sync::Notify>,
    ) -> Result<()> {
        let dt_cfg = config::DingtalkConfig {
            client_id: ch_cfg.client_id.clone(),
            client_secret: ch_cfg.client_secret.clone(),
            domain: ch_cfg.domain.clone(),
        };

        let ws_client = dingtalk_ws::DingtalkWsClient::new(
            &dt_cfg.domain,
            &dt_cfg.client_id,
            &dt_cfg.client_secret,
        );

        loop {
            let bridge = bridge::DingtalkBridge::new(agent_cfg.clone(), dt_cfg.clone()).await?;
            let bridge = Arc::new(bridge);
            let b = bridge.clone();
            let sd = shutdown.clone();

            let result = tokio::select! {
                r = ws_client.connect_and_listen(move |event| {
                    let b = b.clone();
                    tokio::spawn(async move {
                        if let Err(e) = b.handle_event(event).await {
                            error!("DingTalk event error: {}", e);
                        }
                    });
                }) => r,
                _ = sd.notified() => {
                    info!("DingTalk channel shutting down");
                    return Ok(());
                }
            };

            match result {
                Ok(()) => info!("DingTalk WebSocket closed cleanly, reconnecting..."),
                Err(e) => {
                    warn!("DingTalk WebSocket error: {}. Reconnecting in 5s...", e);
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {},
                        _ = shutdown.notified() => {
                            info!("DingTalk channel shutting down");
                            return Ok(());
                        }
                    }
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

    fn ch_cfg(domain: &str) -> DingtalkChannelConfig {
        DingtalkChannelConfig {
            enabled: true,
            client_id: "id".into(),
            client_secret: "secret".into(),
            domain: domain.into(), // full URL → base_url verbatim
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

    fn gateway_route(ws_url: &str) -> HttpRoute {
        HttpRoute::json(
            "/v1.0/gateway/connections/open",
            200,
            &serde_json::json!({"endpoint": ws_url, "ticket": "t"}).to_string(),
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_fails_fast_without_agent() {
        ts::ensure_crypto_provider();
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let err = DingtalkChannel::run(
            agent_cfg("127.0.0.1:1"),
            ch_cfg("http://127.0.0.1:1"),
            shutdown,
        )
        .await
        .err()
        .unwrap();
        assert!(!err.to_string().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_connects_then_shuts_down() {
        ts::ensure_crypto_provider();
        let (ws_url, _) = ts::spawn_ws(vec![WsAction::Delay(Duration::from_secs(30))]).await;
        let (base, _) = ts::spawn_http(vec![gateway_route(&format!("{}/stream", ws_url))]).await;
        let (addr, _) = ts::spawn_mock_grpc(MockState::default()).await;
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let sd = shutdown.clone();
        let handle = tokio::spawn(DingtalkChannel::run(agent_cfg(&addr), ch_cfg(&base), sd));
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
        let (base, _) = ts::spawn_http(vec![gateway_route(&format!("{}/stream", ws_url))]).await;
        let (addr, _) = ts::spawn_mock_grpc(MockState::default()).await;
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let sd = shutdown.clone();
        let handle = tokio::spawn(DingtalkChannel::run(agent_cfg(&addr), ch_cfg(&base), sd));
        tokio::time::sleep(Duration::from_millis(500)).await;
        shutdown.notify_waiters();
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("run must return after shutdown")
            .expect("task join");
        assert!(result.is_ok(), "run returned: {:?}", result.err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_backoff_interrupted_by_shutdown() {
        ts::ensure_crypto_provider();
        // Gateway open fails → warn + 5s backoff; shutdown cancels it.
        let (base, _) = ts::spawn_http(vec![HttpRoute::json(
            "/v1.0/gateway/connections/open",
            500,
            "{}",
        )])
        .await;
        let (addr, _) = ts::spawn_mock_grpc(MockState::default()).await;
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let sd = shutdown.clone();
        let handle = tokio::spawn(DingtalkChannel::run(agent_cfg(&addr), ch_cfg(&base), sd));
        tokio::time::sleep(Duration::from_millis(300)).await;
        shutdown.notify_waiters();
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("shutdown cancels the backoff sleep")
            .expect("task join");
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_dispatches_events_to_the_bridge() {
        ts::ensure_crypto_provider();
        // A CALLBACK bot-message frame through the WS: the bridge spawns the
        // handler (closure + spawn + error-log arms). new_session fails, so
        // the prompt path errors and gets logged.
        let frame = serde_json::json!({
            "type": "CALLBACK",
            "headers": {"messageId": "mid-ev", "topic": "/v1.0/im/bot/messages/get"},
            "data": serde_json::json!({
                "senderId": "user-1",
                "conversationId": "cid-1",
                "msgtype": "text",
                "text": {"content": "/frobnicate"},
                "sessionWebhook": "http://127.0.0.1:1/unreachable"
            }).to_string()
        })
        .to_string();
        // Plus a stale event (handle_event Ok → if-let-Err false path).
        let old_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            - 120_000;
        let stale_frame = serde_json::json!({
            "type": "CALLBACK",
            "headers": {"messageId": "mid-stale", "topic": "/v1.0/im/bot/messages/get"},
            "data": serde_json::json!({
                "senderId": "user-1",
                "conversationId": "cid-1",
                "msgtype": "text",
                "text": {"content": "old"},
                "createAt": old_ms
            }).to_string()
        })
        .to_string();
        let (ws_url, _) = ts::spawn_ws(vec![
            WsAction::SendText(frame),
            WsAction::SendText(stale_frame),
            WsAction::Delay(Duration::from_millis(500)),
            WsAction::SendClose,
        ])
        .await;
        let (base, _) = ts::spawn_http(vec![gateway_route(&format!("{}/stream", ws_url))]).await;
        let mut state = MockState::default();
        state.fail_commands.insert("new_session".into());
        let (addr, _) = ts::spawn_mock_grpc(state).await;
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let sd = shutdown.clone();
        let handle = tokio::spawn(DingtalkChannel::run(agent_cfg(&addr), ch_cfg(&base), sd));
        tokio::time::sleep(Duration::from_millis(800)).await;
        shutdown.notify_waiters();
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("run must return after shutdown")
            .expect("task join");
        assert!(result.is_ok());
    }
}
