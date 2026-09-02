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

const AGENT_RECONNECT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(20);
const WEBSOCKET_RECONNECT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

async fn wait_or_shutdown(delay: std::time::Duration, shutdown: &tokio::sync::Notify) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        _ = shutdown.notified() => true,
    }
}

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
            let bridge = loop {
                let result = tokio::select! {
                    result = bridge::DingtalkBridge::new(agent_cfg.clone(), dt_cfg.clone()) => result,
                    _ = shutdown.notified() => {
                        info!("DingTalk channel shutting down");
                        return Ok(());
                    }
                };
                match result {
                    Ok(bridge) => break Arc::new(bridge),
                    Err(error) => {
                        warn!(
                            "Failed to connect DingTalk channel to Agent: {}. Retrying in 20s...",
                            error
                        );
                        if wait_or_shutdown(AGENT_RECONNECT_INTERVAL, &shutdown).await {
                            info!("DingTalk channel shutting down");
                            return Ok(());
                        }
                    }
                }
            };

            let b = bridge.clone();
            let monitored_bridge = bridge.clone();
            let sd = shutdown.clone();

            let retry_delay = tokio::select! {
                result = ws_client.connect_and_listen(move |event| {
                    let b = b.clone();
                    tokio::spawn(async move {
                        if let Err(e) = b.handle_event(event).await {
                            error!("DingTalk event error: {}", e);
                        }
                    });
                }) => match result {
                    Ok(()) => {
                        info!("DingTalk WebSocket closed cleanly, reconnecting...");
                        None
                    }
                    Err(error) => {
                        warn!("DingTalk WebSocket error: {}. Reconnecting in 5s...", error);
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
                    info!("DingTalk channel shutting down");
                    return Ok(());
                }
            };

            if let Some(delay) = retry_delay {
                if wait_or_shutdown(delay, &shutdown).await {
                    info!("DingTalk channel shutting down");
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
    async fn run_keeps_retrying_without_agent_until_shutdown() {
        ts::ensure_crypto_provider();
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let handle = tokio::spawn(DingtalkChannel::run(
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
    async fn run_agent_disconnect_backoff_is_interrupted_by_shutdown() {
        ts::ensure_crypto_provider();
        let (ws_url, _) = ts::spawn_ws(vec![WsAction::Delay(Duration::from_secs(30))]).await;
        let (base, _) = ts::spawn_http(vec![gateway_route(&format!("{}/stream", ws_url))]).await;
        let (addr, _) = ts::spawn_mock_grpc(MockState {
            stream_status_error: true,
            ..Default::default()
        })
        .await;
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let handle = tokio::spawn(DingtalkChannel::run(
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
