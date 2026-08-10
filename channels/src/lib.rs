//! future_channel — FutureAgent Channel Bridge library.
//!
//! Reads ~/.future/channels/config.json and starts enabled channels.
//! Each channel connects to the FutureAgent via gRPC. The same entry point
//! (`run`) is used by the standalone `future-channel` binary and, embedded,
//! by the `future` CLI (`future channel`).

#![allow(dead_code)]

pub mod config;
pub mod dingtalk;
pub mod feishu;
pub mod grpc_client;
pub mod tls;

#[cfg(test)]
pub(crate) mod test_support;

use anyhow::Result;
use std::sync::Arc;
use tracing::info;

/// Entry point — the former `main()` body. `args` is argv without the
/// program name (only `--version`/`-V` are inspected).
pub fn run(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("future-channel v{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // rustls-platform-verifier shares a rustls instance with reqwest and
    // tokio-tungstenite.  Both enable different default features on rustls
    // (aws-lc-rs vs. ring), so we must pin one provider explicitly.
    // install_default only errors when a provider is ALREADY installed —
    // e.g. when the channel bridge is embedded in the `future` CLI whose
    // agent set one up first — which is fine, so ignore the result.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_async())
}

async fn run_async() -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_flag_prints_and_exits_ok() {
        for flag in ["--version", "-V"] {
            run(&[flag.to_string()]).expect("--version is Ok");
        }
        // Mixed with other args still wins.
        run(&["--verbose".to_string(), "-V".to_string()]).expect("Ok");
    }

    /// The one in-process full run: crypto provider install + tracing init +
    /// runtime + config load are all process-global one-shots.
    #[test]
    fn run_with_enabled_channel_missing_credentials_bails() {
        let _guard = crate::test_support::home_lock();
        let home = crate::test_support::IsolatedHome::new("lib-run");
        let dir = home.path.join(".future").join("channels");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.json"),
            r#"{"feishu": {"enabled": true}}"#,
        )
        .unwrap();
        let err = run(&[]).unwrap_err();
        assert!(
            err.to_string().contains("app_id/app_secret missing"),
            "{err}"
        );
    }
}
