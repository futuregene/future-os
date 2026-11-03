//! DingTalk/Lark channel bridge.
//!
//! Connects to DingTalk via WebSocket stream mode, receives messages,
//! forwards them to FutureAgent via gRPC, and streams responses back.

pub mod bridge;
pub mod card;
pub mod config;
pub mod dingtalk_rest;
pub mod dingtalk_ws;

use crate::bridge::Shutdown;
use crate::config::{AgentConfig, DingtalkChannelConfig};
use anyhow::Result;
use std::sync::Arc;
use tracing::{info, warn};

const AGENT_RECONNECT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(20);
const WEBSOCKET_RECONNECT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

async fn wait_or_shutdown(delay: std::time::Duration, shutdown: &Shutdown) -> bool {
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
        shutdown: std::sync::Arc<crate::bridge::Shutdown>,
    ) -> Result<()> {
        // This channel's own state directory, the same layout the framework
        // channels and Feishu use. The bridge is rebuilt on every reconnect, so
        // the conversation → session map has to live here rather than in the
        // bridge.
        Self::run_with_data_dir(
            agent_cfg,
            ch_cfg,
            crate::data_root().join("dingtalk"),
            shutdown,
        )
        .await
    }

    /// [`DingtalkChannel::run`] with an explicit state directory: production
    /// passes `<future home>/channels/dingtalk`, tests a temp dir (so they never
    /// read or write the real `~/.future`).
    pub(crate) async fn run_with_data_dir(
        agent_cfg: Arc<AgentConfig>,
        ch_cfg: DingtalkChannelConfig,
        data_dir: std::path::PathBuf,
        shutdown: std::sync::Arc<crate::bridge::Shutdown>,
    ) -> Result<()> {
        let dt_cfg = config::DingtalkConfig {
            sender_allowlist: ch_cfg.sender_allowlist.clone(),
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
                    result = bridge::DingtalkBridge::new(
                        agent_cfg.clone(),
                        dt_cfg.clone(),
                        data_dir.clone(),
                    ) => result,
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
                    b.enqueue_event(event);
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
            sender_allowlist: vec!["*".into()],
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
    async fn run_stops_when_shutdown_landed_before_the_agent_connection() {
        // The arm a Ctrl-C during startup takes: the signal is already fired
        // when `run` reaches its first `select!`, so the shutdown branch is the
        // only ready one (a refused loopback connect cannot complete on its
        // first poll) and the channel returns without ever building a bridge.
        ts::ensure_crypto_provider();
        let shutdown = crate::bridge::Shutdown::new();
        shutdown.trigger();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            DingtalkChannel::run(
                agent_cfg("127.0.0.1:1"),
                ch_cfg("http://127.0.0.1:1"),
                shutdown,
            ),
        )
        .await
        .expect("a shutdown that already fired must end the channel");
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_keeps_retrying_without_agent_until_shutdown() {
        ts::ensure_crypto_provider();
        let shutdown = crate::bridge::Shutdown::new();
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
        shutdown.trigger();
        let result = tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("shutdown must interrupt Agent retry backoff")
            .expect("task join");
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_retries_the_agent_connection_after_the_backoff_elapses() {
        // The reconnect backoff is 20 s, and the only way to observe that the
        // loop *retries* (rather than sitting on its first attempt) is to let
        // one elapse: this is the arm a long Agent outage takes, and it is what
        // makes "keeps retrying" more than "can be cancelled".
        ts::ensure_crypto_provider();
        let shutdown = crate::bridge::Shutdown::new();
        let handle = tokio::spawn(DingtalkChannel::run(
            agent_cfg("127.0.0.1:1"),
            ch_cfg("http://127.0.0.1:1"),
            shutdown.clone(),
        ));
        // One full backoff plus margin: the first attempt has failed and the
        // loop has come back around by now.
        tokio::time::sleep(Duration::from_secs(23)).await;
        assert!(
            !handle.is_finished(),
            "the channel must still be retrying after a full backoff"
        );
        shutdown.trigger();
        let result = tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("shutdown must interrupt the retry")
            .expect("task join");
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_connects_then_shuts_down() {
        ts::ensure_crypto_provider();
        let (ws_url, _) = ts::spawn_ws(vec![WsAction::Delay(Duration::from_secs(30))]).await;
        let (base, _) = ts::spawn_http(vec![gateway_route(&format!("{}/stream", ws_url))]).await;
        let (addr, _) = ts::spawn_mock_grpc(MockState::default()).await;
        let shutdown = crate::bridge::Shutdown::new();
        let sd = shutdown.clone();
        let handle = tokio::spawn(DingtalkChannel::run(agent_cfg(&addr), ch_cfg(&base), sd));
        tokio::time::sleep(Duration::from_millis(500)).await;
        shutdown.trigger();
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
        let shutdown = crate::bridge::Shutdown::new();
        let sd = shutdown.clone();
        let handle = tokio::spawn(DingtalkChannel::run(agent_cfg(&addr), ch_cfg(&base), sd));
        tokio::time::sleep(Duration::from_millis(500)).await;
        shutdown.trigger();
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("run must return after shutdown")
            .expect("task join");
        assert!(result.is_ok(), "run returned: {:?}", result.err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_keeps_the_conversation_session_across_a_reconnect() {
        // h4-C3: `run` rebuilds the bridge at the head of every outer-loop
        // iteration and the conversation → session map lived only inside it, so
        // one clean WS close (the arms below show reconnects are the normal
        // state of this channel) forgot every chat: the next message created a
        // brand-new agent session, dropping the conversation and any /model or
        // /effort the user had set. docs/wiki/en/DingTalk.md promises
        // "reconnection is automatic and transparent — sessions are preserved".
        ts::ensure_crypto_provider();
        // The mapping is persisted now, so the channel gets a temp state dir:
        // this test must not read or write the real `~/.future`.
        let data_dir = ts::temp_dir("dingtalk-reconnect-sessions");

        let frame = |message_id: &str, text: &str| {
            serde_json::json!({
                "type": "CALLBACK",
                "headers": {"messageId": message_id, "topic": "/v1.0/im/bot/messages/get"},
                "data": serde_json::json!({
                    "senderId": "user-1",
                    "conversationId": "cid-reconnect",
                    "msgtype": "text",
                    "text": {"content": text},
                    "sessionWebhook": "http://127.0.0.1:1/unreachable"
                }).to_string()
            })
            .to_string()
        };
        let (ws_url, _) = ts::spawn_ws_per_connection(vec![
            vec![
                WsAction::SendText(frame("mid-before", "hello")),
                // The ACK is what the client sends once the event reached the
                // bridge; the delay lets the first turn create its session
                // before the close tears the bridge down.
                WsAction::WaitForReceived {
                    count: 2,
                    timeout: Duration::from_secs(5),
                },
                WsAction::Delay(Duration::from_millis(300)),
                WsAction::SendClose,
            ],
            vec![
                WsAction::SendText(frame("mid-after", "again")),
                WsAction::Delay(Duration::from_secs(30)),
            ],
        ])
        .await;
        let (base, _) = ts::spawn_http(vec![gateway_route(&format!("{}/stream", ws_url))]).await;
        let (addr, grpc) = ts::spawn_mock_grpc(MockState::default()).await;
        let shutdown = crate::bridge::Shutdown::new();
        let handle = tokio::spawn(DingtalkChannel::run_with_data_dir(
            agent_cfg(&addr),
            ch_cfg(&base),
            data_dir,
            shutdown.clone(),
        ));

        // Both messages reach the Agent: the reconnect happened in between and
        // the second one was still answered.
        assert!(
            ts::wait_until(
                || ts::recorded_of(&grpc, "prompt").len() >= 2,
                Duration::from_secs(10)
            )
            .await,
            "the second message must be processed after the reconnect"
        );
        assert_eq!(
            ts::recorded_of(&grpc, "new_session").len(),
            1,
            "one chat keeps one agent session across a reconnect"
        );

        shutdown.trigger();
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("run must return after shutdown")
            .expect("task join");
        assert!(result.is_ok());
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
        let shutdown = crate::bridge::Shutdown::new();
        let sd = shutdown.clone();
        let handle = tokio::spawn(DingtalkChannel::run(agent_cfg(&addr), ch_cfg(&base), sd));
        tokio::time::sleep(Duration::from_millis(300)).await;
        shutdown.trigger();
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("shutdown cancels the backoff sleep")
            .expect("task join");
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_backoff_elapses_and_retries() {
        // The other edge of the retry delay: the 5 s backoff elapses because
        // nothing shuts down during it, so the loop comes back around and opens
        // the gateway again (the sibling above cancels the same wait).
        ts::ensure_crypto_provider();
        let (base, log) = ts::spawn_http(vec![HttpRoute::json(
            "/v1.0/gateway/connections/open",
            500,
            "{}",
        )])
        .await;
        let (addr, _) = ts::spawn_mock_grpc(MockState::default()).await;
        let shutdown = crate::bridge::Shutdown::new();
        let sd = shutdown.clone();
        let handle = tokio::spawn(DingtalkChannel::run(agent_cfg(&addr), ch_cfg(&base), sd));
        // One full backoff plus margin: the open has been retried by now.
        tokio::time::sleep(Duration::from_millis(5_600)).await;
        assert!(
            !handle.is_finished(),
            "the channel must still be retrying after a full backoff"
        );
        let attempts = ts::requests_to(&log, "/v1.0/gateway/connections/open").len();
        assert!(
            attempts >= 2,
            "the loop must have retried the gateway open (attempts: {attempts})"
        );
        shutdown.trigger();
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("shutdown must end the channel")
            .expect("task join");
        assert!(result.is_ok(), "{result:?}");
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
        let shutdown = crate::bridge::Shutdown::new();
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
        shutdown.trigger();
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
        let shutdown = crate::bridge::Shutdown::new();
        let sd = shutdown.clone();
        let handle = tokio::spawn(DingtalkChannel::run(agent_cfg(&addr), ch_cfg(&base), sd));
        tokio::time::sleep(Duration::from_millis(800)).await;
        shutdown.trigger();
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("run must return after shutdown")
            .expect("task join");
        assert!(result.is_ok());
    }
}
