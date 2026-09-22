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
use tracing::{info, warn};

use bridge::{Bridge, ProviderCtx, Shutdown};
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
    match args.first() {
        Some(command) if cli_cmd::is_subcommand(command) => return cli_cmd::run(args),
        _ => {}
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

/// What a started bridge is holding, so `run_async` and tests share one path.
pub struct Started {
    /// One task per channel (or per supervisor), plus the background helpers.
    pub handles: Vec<tokio::task::JoinHandle<()>>,
    pub status: Arc<StatusBoard>,
    pub shutdown: Arc<Shutdown>,
    /// How many framework channels were started (diagnostics and tests).
    pub started_providers: usize,
}

impl Started {
    /// Stop everything and leave a snapshot that says so.
    ///
    /// Called on Ctrl-C, and by tests so a started bridge never outlives them.
    pub async fn stop(self) {
        // `notify_waiters` wakes the tasks already parked on the signal, but a
        // supervisor that is mid-session registers its wait *after* this point
        // and would then wait for a second signal. `notify_one` stores a permit
        // so that next registration completes immediately; the aborts below are
        // what actually guarantees a prompt stop.
        self.shutdown.trigger();
        for handle in self.handles {
            handle.abort();
        }
        for entry in registry::all() {
            self.status
                .set_state(entry.definition.id, ChannelState::Disabled, None);
        }
        let _ = self.status.flush();
    }
}

/// Start every enabled channel described by `config`.
///
/// Split out of the process entry point because this is the part worth testing:
/// which channels start, which are reported as `unsupported`, and what the
/// published snapshot says. It never installs a signal handler and never blocks.
pub fn start_all(
    config: &config::ChannelConfig,
    root: std::path::PathBuf,
    status: Arc<StatusBoard>,
) -> Result<Started> {
    let agent_cfg = Arc::new(config.agent.clone());
    let shutdown = Shutdown::new();
    let mut handles = Vec::new();

    // The self-bridged channels are published too, so `future channel status`
    // shows an off channel as off rather than as one that never reported.
    for definition in providers::native_definitions() {
        status.set_state(definition.id, ChannelState::Disabled, None);
    }

    // ── Feishu ─────────────────────────────────────────────────────────

    if let Some(feishu_cfg) = config.feishu.as_ref().filter(|cfg| cfg.enabled) {
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

    // ── DingTalk ──────────────────────────────────────────────────────

    if let Some(dt_cfg) = config.dingtalk.as_ref().filter(|cfg| cfg.enabled) {
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

    // ── Framework channels ────────────────────────────────────────────

    let mut started_providers = 0usize;
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
            let reason = definition
                .ensure_usable()
                .err()
                .unwrap_or_else(|| "unsupported".to_string());
            warn!("{reason}");
            status.set_state(id, ChannelState::Unsupported, Some(reason));
            continue;
        }
        info!("Starting {} channel...", definition.display_name);
        started_providers += 1;
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

    Ok(Started {
        handles,
        status,
        shutdown,
        started_providers,
    })
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

    let status = Arc::new(StatusBoard::new(StatusSnapshot::default_path()));
    let root = data_root();
    let started = start_all(&config, root.clone(), status.clone())?;

    if started.handles.is_empty() {
        let guidance = format!(
            "No channels enabled. Edit {} and set a channel's 'enabled' to true.",
            cfg_path.display()
        );
        warn!("{guidance}");
    } else {
        info!("{} channel task(s) running", started.handles.len());
    }

    status.flush()?;
    let flusher = spawn_status_flusher(status.clone(), started.shutdown.clone());
    let outbox_drainer = spawn_outbox_drainer(
        outbox::Outbox::new(
            Arc::new(delivery::DeliveryQueue::load(
                delivery::DeliveryQueue::default_path(),
            )),
            config.clone(),
            Arc::new(config.agent.clone()),
            root,
            status.clone(),
        ),
        started.shutdown.clone(),
    );

    tokio::signal::ctrl_c().await?;
    info!("Shutting down...");
    flusher.abort();
    outbox_drainer.abort();
    started.stop().await;
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
    shutdown: Arc<Shutdown>,
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
                let message = format!("cannot create the channel data directory: {error}");
                warn!(channel = id, "{message}");
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
    shutdown: Arc<Shutdown>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(STATUS_FLUSH_INTERVAL) => {
                    if let Err(error) = status.flush_if_due() {
                        let message = format!("cannot publish channel status: {error}");
                        tracing::debug!("{message}");
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
    shutdown: Arc<Shutdown>,
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

    fn config_with(entries: &[(&str, bool)]) -> config::ChannelConfig {
        let mut config = config::ChannelConfig::default();
        for (id, enabled) in entries {
            config
                .providers
                .insert((*id).to_string(), serde_json::json!({ "enabled": enabled }));
        }
        config
    }

    async fn start(config: config::ChannelConfig, label: &str) -> (Started, std::path::PathBuf) {
        let root = crate::test_support::temp_dir(label);
        let status = Arc::new(StatusBoard::new(root.join("status.json")));
        let started = start_all(&config, root.clone(), status).expect("start");
        (started, root)
    }

    #[tokio::test]
    async fn starting_publishes_a_state_for_every_channel() {
        let config = config_with(&[("cli", true), ("telegram", true), ("slack", false)]);
        let expected_running = providers::registry::implemented()
            .iter()
            .filter(|entry| matches!(entry.definition.id, "cli" | "telegram"))
            .count();

        let root = crate::test_support::temp_dir("lib-start-states");
        let status = Arc::new(StatusBoard::new(root.join("status.json")));
        let started = start_all(&config, root.clone(), status).expect("start");
        // Whatever the build implements, every enabled-and-implemented channel
        // spawns a supervisor and everything else is reported instead.
        assert_eq!(started.started_providers, expected_running);
        started.status.flush().unwrap();

        let snapshot = StatusSnapshot::load(&root.join("status.json"));
        let report = cli_cmd::ListReport::build(&config);
        let published = snapshot.channels.len();
        assert_eq!(
            published,
            report.channels.len(),
            "every registered channel must be published"
        );
        for row in &report.channels {
            let entry = snapshot.channels.get(row.id).expect("published above");
            match row.configured {
                // Enabled but not built: reported, never silently skipped.
                "unsupported" => assert_eq!(
                    entry.state,
                    Some(ChannelState::Unsupported),
                    "{} must be reported as unsupported",
                    row.id
                ),
                "not-configured" | "disabled" => {
                    assert_eq!(entry.state, Some(ChannelState::Disabled), "{}", row.id)
                }
                // Enabled and implemented: a supervisor is running it.
                _ => assert!(
                    matches!(
                        entry.state,
                        Some(ChannelState::Starting) | Some(ChannelState::Running)
                    ),
                    "{} should be running, got {:?}",
                    row.id,
                    entry.state
                ),
            }
        }

        started.stop().await;
        let after = StatusSnapshot::load(&root.join("status.json"));
        assert_eq!(
            after.channels.get("cli").unwrap().state,
            Some(ChannelState::Disabled),
            "stopping leaves a truthful snapshot"
        );
    }

    #[tokio::test]
    async fn a_provider_that_fails_to_start_is_reported_and_retried() {
        // A configured channel whose credentials are missing fails on its first
        // attempt: the failure is published and the supervisor keeps retrying
        // instead of dropping the channel.
        let config = config_with(&[("telegram", true)]);
        let root = crate::test_support::temp_dir("lib-start-failing");
        let status = Arc::new(StatusBoard::new(root.join("status.json")));
        let started = start_all(&config, root.clone(), status).expect("start");
        let failed = crate::test_support::wait_until(
            || {
                let snapshot = StatusSnapshot::load(&root.join("status.json"));
                snapshot
                    .channels
                    .get("telegram")
                    .and_then(|entry| entry.state.clone())
                    .map(|state| state == ChannelState::Error)
                    .unwrap_or(false)
            },
            std::time::Duration::from_secs(10),
        )
        .await;
        assert!(failed, "a provider that cannot start must be reported");
        started.stop().await;
        let after = StatusSnapshot::load(&root.join("status.json"));
        assert_eq!(
            after.channels.get("telegram").unwrap().state,
            Some(ChannelState::Disabled)
        );
    }

    #[tokio::test]
    async fn a_status_that_cannot_be_written_is_not_fatal() {
        // The flusher runs on a timer and must survive an unwritable snapshot
        // path: losing diagnostics must never take the bridge down.
        let root = crate::test_support::temp_dir("lib-status-unwritable");
        // A regular file where the snapshot's directory should be.
        std::fs::write(root.join("blocked"), b"not a directory").unwrap();
        let status = Arc::new(StatusBoard::new(root.join("blocked").join("status.json")));
        status.set_state("cli", ChannelState::Running, None);
        // A transition publishes immediately; the failure is reported rather
        // than panicking (the state change above already went through that path).
        let error = status
            .flush()
            .map(|_| ())
            .expect_err("publishing into a file must fail")
            .to_string();
        assert!(!error.is_empty());
        // The periodic flush reports rather than panics.
        let shutdown = Shutdown::new();
        let flusher = spawn_status_flusher(status.clone(), shutdown.clone());
        shutdown.trigger();
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(5), flusher).await;
        assert!(stopped.is_ok(), "the flusher must stop on shutdown");
    }

    #[tokio::test]
    async fn a_data_directory_that_cannot_be_created_is_reported_not_fatal() {
        // A file where the channel's data directory should be: the provider
        // still starts (it reports the problem) instead of taking the process
        // down before it can say anything.
        let config = config_with(&[("cli", true)]);
        let root = crate::test_support::temp_dir("lib-start-wedged-dir");
        std::fs::write(root.join("cli"), b"not a directory").unwrap();
        let status = Arc::new(StatusBoard::new(root.join("status.json")));
        let started = start_all(&config, root, status).expect("start");
        assert_eq!(started.started_providers, 1);
        started.stop().await;
    }

    #[tokio::test]
    async fn the_flusher_survives_an_unwritable_snapshot() {
        // The flusher ticks on a timer: a write failure is reported and the task
        // keeps running, because losing diagnostics must not stop the bridge.
        let root = crate::test_support::temp_dir("lib-flusher-unwritable");
        std::fs::write(root.join("blocked"), b"not a directory").unwrap();
        let status = Arc::new(StatusBoard::new(root.join("blocked").join("status.json")));
        // A counter change is written by the periodic check rather than a state
        // transition, so the flusher's own error path runs.
        status.count_inbound("cli", crate::status::now_unix());
        let shutdown = Shutdown::new();
        let flusher = spawn_status_flusher(status.clone(), shutdown.clone());
        // Long enough for one tick (the interval is two seconds).
        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
        shutdown.trigger();
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(5), flusher).await;
        assert!(stopped.is_ok(), "the flusher must stop on shutdown");
    }

    #[tokio::test]
    async fn starting_with_nothing_enabled_is_not_an_error() {
        let (started, _root) = start(config::ChannelConfig::default(), "lib-start-empty").await;
        assert!(started.handles.is_empty());
        assert_eq!(started.started_providers, 0);
        started.stop().await;
    }

    #[tokio::test]
    async fn the_native_bridges_start_with_the_shared_snapshot() {
        let mut config = config::ChannelConfig::default();
        config.feishu = Some(config::FeishuChannelConfig {
            enabled: true,
            app_id: "app".into(),
            app_secret: "secret".into(),
            ..Default::default()
        });
        config.dingtalk = Some(config::DingtalkChannelConfig {
            enabled: true,
            client_id: "id".into(),
            client_secret: "secret".into(),
            ..Default::default()
        });
        let root = crate::test_support::temp_dir("lib-start-native");
        let status = Arc::new(StatusBoard::new(root.join("status.json")));
        let started = start_all(&config, root.clone(), status).expect("start");
        // Both self-bridged channels spawned a supervisor, and both are
        // published as started before their transports connect.
        assert_eq!(started.handles.len(), 2);
        started.status.flush().unwrap();
        let snapshot = StatusSnapshot::load(&root.join("status.json"));
        assert_eq!(
            snapshot.channels.get("feishu").unwrap().state,
            Some(ChannelState::Running)
        );
        assert_eq!(
            snapshot.channels.get("dingtalk").unwrap().state,
            Some(ChannelState::Running)
        );
        started.stop().await;
    }

    #[tokio::test]
    async fn a_native_bridge_without_credentials_refuses_to_start() {
        let mut config = config::ChannelConfig::default();
        config.feishu = Some(config::FeishuChannelConfig {
            enabled: true,
            ..Default::default()
        });
        let root = crate::test_support::temp_dir("lib-start-feishu-nocreds");
        let status = Arc::new(StatusBoard::new(root.join("status.json")));
        let error = start_all(&config, root, status)
            .err()
            .expect("must refuse")
            .to_string();
        assert!(error.contains("app_id/app_secret"), "{error}");

        let mut config = config::ChannelConfig::default();
        config.dingtalk = Some(config::DingtalkChannelConfig {
            enabled: true,
            ..Default::default()
        });
        let root = crate::test_support::temp_dir("lib-start-dingtalk-nocreds");
        let status = Arc::new(StatusBoard::new(root.join("status.json")));
        let error = start_all(&config, root, status)
            .err()
            .expect("must refuse")
            .to_string();
        assert!(error.contains("client_id/client_secret"), "{error}");
    }

    #[tokio::test]
    async fn stopping_cancels_a_running_channel_task() {
        // The supervisor loops until shutdown, so a started provider task is
        // still pending here; stopping must abort it rather than leak it.
        let config = config_with(&[("cli", true)]);
        let (started, _root) = start(config, "lib-stop-cancels").await;
        let handles = started.handles.len();
        assert!(handles >= 1);
        started.stop().await;
    }

    #[tokio::test]
    async fn the_status_flusher_publishes_and_then_stops() {
        let root = crate::test_support::temp_dir("lib-status-flusher");
        let status = Arc::new(StatusBoard::new(root.join("status.json")));
        let shutdown = Shutdown::new();
        status.count_inbound("cli", crate::status::now_unix());
        let flusher = spawn_status_flusher(status.clone(), shutdown.clone());
        // The flusher writes on its own timer; force one write to observe the
        // effect without sleeping for the interval.
        status.flush().unwrap();
        assert!(root.join("status.json").exists());
        // `notify_one` stores a permit, so the signal survives the flusher
        // being between waits; `notify_waiters` would be lost.
        shutdown.trigger();
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(2), flusher).await;
        assert!(stopped.is_ok(), "the flusher must stop on shutdown");
    }

    #[tokio::test]
    async fn the_outbox_drainer_retries_a_queued_message_and_stops() {
        let root = crate::test_support::temp_dir("lib-outbox-drainer");
        let config = config_with(&[("telegram", true)]);
        let queue = Arc::new(crate::delivery::DeliveryQueue::load(
            root.join("deliveries.json"),
        ));
        queue
            .enqueue(
                "telegram",
                crate::bridge::ConversationRef {
                    id: "c1".into(),
                    thread_id: None,
                    kind: crate::bridge::ChatKind::Direct,
                },
                "hello",
            )
            .unwrap();
        let outbox = outbox::Outbox::new(
            queue.clone(),
            config,
            Arc::new(config::AgentConfig {
                grpc_addr: "http://127.0.0.1:1".into(),
                ..config::AgentConfig::default()
            }),
            root.clone(),
            Arc::new(StatusBoard::new(root.join("status.json"))),
        );
        let shutdown = Shutdown::new();
        let drainer = spawn_outbox_drainer(outbox, shutdown.clone());
        // Give the first pass a moment to record the failed attempt.
        let recorded = crate::test_support::wait_until(
            || !queue.pending().is_empty() && queue.pending()[0].attempts > 0,
            std::time::Duration::from_secs(5),
        )
        .await;
        assert!(recorded, "the drainer must attempt a due message");
        shutdown.trigger();
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(2), drainer).await;
        assert!(stopped.is_ok(), "the drainer must stop on shutdown");
    }
}
