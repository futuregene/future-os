//! DingTalk/Lark channel bridge.
//!
//! Connects to DingTalk via WebSocket stream mode, receives messages,
//! forwards them to FutureAgent via gRPC, and streams responses back.

pub mod bridge;
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
