//! FutureAgent Channel Bridge — unified binary for all channels.
//!
//! Reads ~/.future/channel/config.json and starts enabled channels.
//! Each channel connects to the FutureAgent via gRPC.

#![allow(dead_code)]

mod config;
mod grpc_client;
mod feishu;

use anyhow::Result;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg_path = config::ChannelConfig::default_path();
    info!("Loading config from {}", cfg_path.display());
    let config = match config::ChannelConfig::load() {
        Ok(c) => c,
        Err(e) => {
            if cfg_path.exists() {
                return Err(e);
            }
            // File doesn't exist — load() already wrote defaults
            tracing::warn!("{}", e);
            return Ok(());
        }
    };

    let agent_cfg = Arc::new(config.agent);
    let mut handles = Vec::new();

    // ── Feishu ─────────────────────────────────────────────────────────

    if let Some(ref feishu_cfg) = config.feishu {
        if feishu_cfg.enabled {
            if feishu_cfg.app_id.is_empty() || feishu_cfg.app_secret.is_empty() {
                anyhow::bail!("Feishu channel enabled but app_id/app_secret missing");
            }
            info!("Starting Feishu channel...");
            let agent = agent_cfg.clone();
            let fcfg = feishu_cfg.clone();
            handles.push(tokio::spawn(async move {
                if let Err(e) = feishu::FeishuChannel::run(agent, fcfg).await {
                    tracing::error!("Feishu channel exited: {}", e);
                }
            }));
        }
    }

    if handles.is_empty() {
        tracing::warn!(
            "No channels enabled. Edit {} and set a channel's 'enabled' to true.",
            cfg_path.display()
        );
    }

    tokio::signal::ctrl_c().await?;
    info!("Shutting down...");
    for h in handles {
        h.abort();
    }
    Ok(())
}
