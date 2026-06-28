//! FutureAgent Channel Bridge — unified binary for all channels.
//!
//! Reads ~/.future/channels/config.json and starts enabled channels.
//! Each channel connects to the FutureAgent via gRPC.

#![allow(dead_code)]

mod config;
mod dingtalk;
mod feishu;
mod grpc_client;

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
    let shutdown = Arc::new(tokio::sync::Notify::new());

    // ── Feishu ─────────────────────────────────────────────────────────

    if let Some(ref feishu_cfg) = config.feishu {
        if feishu_cfg.enabled {
            if feishu_cfg.app_id.is_empty() || feishu_cfg.app_secret.is_empty() {
                anyhow::bail!("Feishu channel enabled but app_id/app_secret missing");
            }
            info!("Starting Feishu channel...");
            let agent = agent_cfg.clone();
            let fcfg = feishu_cfg.clone();
            let sd = shutdown.clone();
            handles.push(tokio::spawn(async move {
                if let Err(e) = feishu::FeishuChannel::run(agent, fcfg, sd).await {
                    tracing::error!("Feishu channel exited: {}", e);
                }
            }));
        }
    }

    // ── DingTalk ──────────────────────────────────────────────────────

    if let Some(ref dt_cfg) = config.dingtalk {
        if dt_cfg.enabled {
            if dt_cfg.client_id.is_empty() || dt_cfg.client_secret.is_empty() {
                anyhow::bail!("DingTalk channel enabled but client_id/client_secret missing");
            }
            info!("Starting DingTalk channel...");
            let agent = agent_cfg.clone();
            let dcfg = dt_cfg.clone();
            let sd = shutdown.clone();
            handles.push(tokio::spawn(async move {
                if let Err(e) = dingtalk::DingtalkChannel::run(agent, dcfg, sd).await {
                    tracing::error!("DingTalk channel exited: {}", e);
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
    shutdown.notify_waiters();
    for h in handles {
        h.abort();
    }
    Ok(())
}
