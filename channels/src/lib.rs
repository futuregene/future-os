//! future_channel — FutureAgent Channel Bridge library.
//!
//! Reads `~/.future/channels/config.json` and starts the enabled channels.
//! Every channel talks to the FutureAgent over gRPC. The same entry point
//! (`run`) is used by the standalone `future-channel` binary and, embedded,
//! by the `future` CLI (`future channel`).
//!
//! There are two kinds of channel here:
//!
//! * **Framework channels** live in [`providers`]. Each one implements the
//!   [`providers::Provider`] trait and gets duplicate filtering, access policy,
//!   session routing, per-conversation queueing, streaming replies and approval
//!   routing from [`bridge::Bridge`] — so a new IM is one file, not a new
//!   pipeline.
//! * **Self-bridged channels** (`feishu`, `dingtalk`) predate the framework and
//!   keep their own bridges, which carry platform behaviour the framework does
//!   not model yet (interactive cards, streaming card elements, approval
//!   buttons). They are declared in [`providers::native`] so the CLI and the
//!   docs describe the whole product.

#![allow(dead_code)]

pub mod bridge;
pub mod cli_cmd;
pub mod config;
pub mod delivery;
pub mod dingtalk;
pub mod feishu;
pub mod grpc_client;
pub mod outbox;
pub mod policy;
pub mod providers;
pub mod session_store;
pub mod status;
pub mod tls;
pub mod transport;

#[cfg(test)]
pub(crate) mod test_support;

use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{info, warn};

use bridge::{Bridge, ProviderCtx};
use config::AgentConfig;
use policy::AccessPolicyConfig;
use providers::registry;
use providers::traits::Provider;
use session_store::SessionStore;
use status::{ChannelState, StatusBoard, StatusSnapshot};
use transport::ws::Backoff;

/// How often the published status snapshot is refreshed.
const STATUS_FLUSH_INTERVAL: Duration = Duration::from_secs(2);

/// How often the durable outbound queue is retried.
const OUTBOX_DRAIN_INTERVAL: Duration = Duration::from_secs(30);

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

    // Diagnostics run before (and instead of) the bridge: `future channel
    // status` must answer whether or not a bridge is running, and must not
    // start one.
    if let Some(command) = args.first() {
        if cli_cmd::is_subcommand(command) {
            return cli_cmd::run(args);
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_async())
}

/// Where channel data lives: `~/.future/channels`.
pub fn data_root() -> std::path::PathBuf {
    config::home_dir().join(".future").join("channels")
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
            warn!("{}", e);
            return Ok(());
        }
    };

    let agent_cfg = Arc::new(config.agent.clone());
    let mut handles = Vec::new();
    let shutdown = Arc::new(Notify::new());
    let status = Arc::new(StatusBoard::new(StatusSnapshot::default_path()));
    let root = data_root();

    // ── Feishu ─────────────────────────────────────────────────────────

    if let Some(ref feishu_cfg) = config.feishu {
        if feishu_cfg.enabled {
            if feishu_cfg.app_id.is_empty() || feishu_cfg.app_secret.is_empty() {
                anyhow::bail!("Feishu channel enabled but app_id/app_secret missing");
            }
            info!("Starting Feishu channel...");
            status.set_started("feishu");
            let agent = agent_cfg.clone();
            let fcfg = feishu_cfg.clone();
            let sd = shutdown.clone();
            handles.push(tokio::spawn(async move {
                // inspect_err, not if-let: rustfmt explodes single-line
                // if-lets and the Ok-edge brace is unreachable in tests
                // (a channel run only returns on error or shutdown abort).
                let _ = feishu::FeishuChannel::run(agent, fcfg, sd)
                    .await
                    .inspect_err(|e| tracing::error!("Feishu channel exited: {}", e));
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
            status.set_started("dingtalk");
            let agent = agent_cfg.clone();
            let dcfg = dt_cfg.clone();
            let sd = shutdown.clone();
            handles.push(tokio::spawn(async move {
                let _ = dingtalk::DingtalkChannel::run(agent, dcfg, sd)
                    .await
                    .inspect_err(|e| tracing::error!("DingTalk channel exited: {}", e));
            }));
        }
    }

    // ── Framework channels ────────────────────────────────────────────

    let mut started = 0usize;
    for entry in registry::all() {
        let definition = entry.definition;
        let id = definition.id;
        let Some(block) = config.provider_config(id) else {
            status.set_state(id, ChannelState::Disabled, None);
            continue;
        };
        if !config::ChannelConfig::provider_enabled(&block) {
            status.set_state(id, ChannelState::Disabled, None);
            continue;
        }
        if !definition.is_implemented() {
            // Enabled but not built: say so loudly instead of appearing to run.
            let reason = format!("the {id} channel is not implemented in this build");
            warn!("{}", reason);
            status.set_state(id, ChannelState::Unsupported, Some(reason));
            continue;
        }
        info!("Starting {} channel...", definition.display_name);
        started += 1;
        status.set_state(id, ChannelState::Starting, None);
        handles.push(spawn_provider(
            entry,
            block,
            agent_cfg.clone(),
            root.clone(),
            status.clone(),
            shutdown.clone(),
        ));
    }

    if handles.is_empty() {
        warn!(
            "No channels enabled. Edit {} and set a channel's 'enabled' to true.",
            cfg_path.display()
        );
    } else {
        info!("{} channel task(s) running", handles.len());
        let _ = started;
    }

    status.flush()?;
    let flusher = spawn_status_flusher(status.clone(), shutdown.clone());
    let outbox_drainer = spawn_outbox_drainer(
        outbox::Outbox::new(
            Arc::new(delivery::DeliveryQueue::load(
                delivery::DeliveryQueue::default_path(),
            )),
            config.clone(),
            agent_cfg.clone(),
            root.clone(),
            status.clone(),
        ),
        shutdown.clone(),
    );

    tokio::signal::ctrl_c().await?;
    info!("Shutting down...");
    shutdown.notify_waiters();
    for h in handles {
        h.abort();
    }
    flusher.abort();
    outbox_drainer.abort();
    // Leave a truthful snapshot: nothing is running any more.
    for entry in registry::all() {
        status.set_state(entry.definition.id, ChannelState::Disabled, None);
    }
    let _ = status.flush();
    Ok(())
}

/// Supervise one framework channel: run it, and restart it with backoff when
/// it stops or fails, until the process is asked to stop.
fn spawn_provider(
    entry: &'static registry::ProviderEntry,
    block: serde_json::Value,
    agent_cfg: Arc<AgentConfig>,
    root: std::path::PathBuf,
    status: Arc<StatusBoard>,
    shutdown: Arc<Notify>,
) -> tokio::task::JoinHandle<()> {
    let definition = entry.definition;
    let provider: Arc<dyn Provider> = Arc::from((entry.provider)());
    tokio::spawn(async move {
        let id = definition.id;
        let data_dir = root.join(id);
        let sessions = Arc::new(SessionStore::new(data_dir.join("sessions.json")));
        let mut backoff = Backoff::gateway();
        loop {
            if let Err(error) = std::fs::create_dir_all(&data_dir) {
                warn!(channel = id, %error, "cannot create the channel data directory");
            }
            // The policy block lives alongside the channel's own settings, so a
            // channel that only sets `enabled` gets the safe defaults.
            let policy =
                serde_json::from_value::<AccessPolicyConfig>(block.clone()).unwrap_or_default();
            let bridge = Bridge::new(agent_cfg.clone(), policy, data_dir.clone(), status.clone());
            let ctx = ProviderCtx::new(
                definition,
                block.clone(),
                bridge,
                data_dir.clone(),
                sessions.clone(),
                shutdown.clone(),
            );

            let result = tokio::select! {
                result = provider.run(ctx) => result,
                _ = shutdown.notified() => return,
            };
            match result {
                Ok(()) => info!(channel = id, "channel stopped; restarting"),
                Err(error) => {
                    warn!(channel = id, %error, "channel failed; restarting");
                    status.set_state(id, ChannelState::Error, Some(error.to_string()));
                }
            }

            let delay = backoff.next_delay();
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = shutdown.notified() => return,
            }
        }
    })
}

/// Publish the status snapshot on a timer so `future channel status` sees
/// live counters without the bridge having to flush on every event.
fn spawn_status_flusher(
    status: Arc<StatusBoard>,
    shutdown: Arc<Notify>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(STATUS_FLUSH_INTERVAL) => {
                    if let Err(error) = status.flush_if_due() {
                        tracing::debug!(%error, "cannot publish channel status");
                    }
                }
                _ = shutdown.notified() => return,
            }
        }
    })
}

/// Retry queued outbound messages: the bridge is the only long-lived process,
/// so it is also what makes a `--durable` send eventually arrive.
fn spawn_outbox_drainer(
    outbox: outbox::Outbox,
    shutdown: Arc<Notify>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let summary = outbox.drain_due().await;
            if summary.touched() > 0 {
                tracing::info!(
                    sent = summary.sent,
                    retried = summary.retried,
                    failed = summary.failed,
                    "drained the outbound queue"
                );
            }
            tokio::select! {
                _ = tokio::time::sleep(OUTBOX_DRAIN_INTERVAL) => {}
                _ = shutdown.notified() => return,
            }
        }
    })
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
        std::fs::write(dir.join("config.json"), r#"{"feishu": {"enabled": true}}"#).unwrap();
        let err = run(&[]).unwrap_err();
        assert!(
            err.to_string().contains("app_id/app_secret missing"),
            "{err}"
        );
    }

    #[test]
    fn data_root_follows_the_future_home() {
        let _guard = crate::test_support::home_lock();
        let home = crate::test_support::IsolatedHome::new("lib-data-root");
        let root = data_root();
        assert!(root.starts_with(&home.path), "{root:?}");
        assert!(root.ends_with("channels"));
    }
}
