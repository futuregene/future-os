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
pub mod session_store;

use crate::config::AgentConfig;
use anyhow::Result;
use std::sync::Arc;
use tracing::{error, info, warn};

pub struct FeishuChannel;

impl FeishuChannel {
    /// Start the Feishu channel. Connects WebSocket and enters the event loop.
    /// Auto-reconnects on disconnect.
    pub async fn run(agent_cfg: Arc<AgentConfig>, ch_cfg: crate::config::FeishuChannelConfig) -> Result<()> {
        let feishu_cfg = config::FeishuConfig::from_channel_config(&ch_cfg);

        let ws_client = feishu_ws::FeishuWsClient::new(
            feishu_cfg.api_base(),
            &feishu_cfg.app_id,
            &feishu_cfg.app_secret,
        );

        loop {
            let bridge = bridge::Bridge::new(agent_cfg.clone(), feishu_cfg.clone()).await?;
            let bridge = Arc::new(bridge);
            let b = bridge.clone();

            let result = ws_client.connect_and_listen(move |event| {
                let b = b.clone();
                tokio::spawn(async move {
                    if let Err(e) = b.handle_event(event).await {
                        error!("Error handling event: {}", e);
                    }
                });
            }).await;

            match result {
                Ok(()) => info!("WebSocket closed cleanly, reconnecting..."),
                Err(e) => {
                    warn!("WebSocket error: {}. Reconnecting in 5s...", e);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    }
}
