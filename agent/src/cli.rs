//! `future-agent` CLI entry — moved out of `main.rs` so the same code can be
//! run either as the standalone `future-agent` binary or embedded in the
//! `future` CLI (`future agent <args>`). The behavior is identical in both
//! cases; `Cli::parse_from` is fed a synthetic argv whose program name is
//! always `future-agent`, so help/error text matches the standalone binary.

mod shutdown;

#[cfg(test)]
use crate::service::config::parse_tcp_bind;
use crate::Manager;
use anyhow::{Context, Result};
use clap::Parser;
use std::path::Path;

/// Transport crates that log per-frame / per-handshake detail at DEBUG. Raising
/// the root level to `debug` for `--verbose` would otherwise bury the Agent's
/// own logs under every HTTP/2 frame h2 sends and every TLS record rustls
/// processes; pin them to WARN so only their failures show up. `RUST_LOG` still
/// overrides this filter entirely.
const NOISY_TRANSPORT_TARGETS: &[&str] = &["h2", "rustls", "tonic", "tower"];

fn default_log_filter(verbose: bool) -> tracing_subscriber::EnvFilter {
    if !verbose {
        return tracing_subscriber::EnvFilter::new("info");
    }
    let mut directives = vec!["debug".to_owned()];
    directives.extend(
        NOISY_TRANSPORT_TARGETS
            .iter()
            .map(|target| format!("{target}=warn")),
    );
    tracing_subscriber::EnvFilter::new(directives.join(","))
}

#[cfg(test)]
#[test]
fn verbose_default_filter_enables_grpc_debug_events() {
    for verbose in [false, true] {
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(default_log_filter(verbose))
            .with_writer(std::io::sink)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            assert_eq!(tracing::enabled!(tracing::Level::DEBUG), verbose);
            // The transport crates stay quiet even in verbose mode; the
            // `enabled!` macro requires a literal target, so assert one by one.
            assert!(!tracing::enabled!(target: "h2", tracing::Level::DEBUG));
            assert!(!tracing::enabled!(target: "rustls", tracing::Level::DEBUG));
            assert!(!tracing::enabled!(target: "tonic", tracing::Level::DEBUG));
            assert!(!tracing::enabled!(target: "tower", tracing::Level::DEBUG));
            assert!(tracing::enabled!(target: "h2", tracing::Level::WARN));
        });
    }
}

#[cfg(test)]
#[test]
fn tcp_bind_rejects_invalid_ports_and_accepts_bracketed_ipv6() {
    for value in [
        "localhost:1234",
        "127.0.0.1:70000",
        "0.0.0.0:",
        "abc",
        "::1:1234",
    ] {
        assert!(parse_tcp_bind(value).is_err(), "{value}");
    }
    assert_eq!(parse_tcp_bind(":1234").unwrap(), ("127.0.0.1".into(), 1234));
    assert_eq!(parse_tcp_bind("[::1]:1234").unwrap(), ("::1".into(), 1234));
}

#[derive(Parser)]
#[command(name = "future-agent")]
#[command(version = crate::utils::VERSION)]
pub struct Cli {
    /// Import legacy sessions and print a content-free report, then exit.
    #[arg(long)]
    migrate_sessions: bool,
    /// Explicitly retry one skipped legacy session while the Agent is stopped.
    #[arg(long, value_name = "SESSION_ID")]
    retry_session_import: Option<String>,
    /// Session directory for isolated migration verification (maintenance only).
    #[arg(long, value_name = "DIRECTORY")]
    migration_source: Option<std::path::PathBuf>,

    /// Internal Linux sandbox helper request. This is intentionally hidden and
    /// dispatched before singleton/config/runtime initialization.
    #[arg(long, hide = true, value_name = "REQUEST")]
    linux_sandbox_helper: Option<String>,
    #[arg(long, hide = true, requires = "linux_sandbox_helper")]
    linux_sandbox_host_version: Option<String>,

    /// Probe the current platform sandbox and print a stable JSON result
    /// without starting the agent server.
    #[arg(
        long,
        default_value_t = false,
        conflicts_with_all = ["probe_windows_sandbox", "reset_windows_sandbox"]
    )]
    probe_sandbox: bool,

    /// Verify the complete Windows unelevated sandbox pipeline and print a
    /// machine-readable result without starting the agent server.
    #[arg(
        long,
        default_value_t = false,
        conflicts_with_all = ["probe_sandbox", "reset_windows_sandbox"]
    )]
    probe_windows_sandbox: bool,

    /// Revoke all tracked FutureOS Windows sandbox ACL entries. Intended for
    /// Settings reset and the installer/uninstaller maintenance path.
    #[arg(
        long,
        default_value_t = false,
        conflicts_with_all = ["probe_sandbox", "probe_windows_sandbox"]
    )]
    reset_windows_sandbox: bool,

    /// FutureOS home directory: the root that owns all Agent state (normally
    /// ~/.future). Point a second Agent at another directory to run a fully
    /// isolated instance beside the default one — its own singleton lock,
    /// database, sessions, logs and local IPC endpoint. FUTURE_HOME has the
    /// same effect.
    #[arg(long, value_name = "DIR")]
    home: Option<std::path::PathBuf>,

    /// Explicit TCP gRPC address for remote/development compatibility.
    /// Omit this option to use secure per-user local IPC.
    #[arg(long)]
    grpc_addr: Option<String>,

    /// Enable verbose logging (show gRPC requests, LLM calls, tool execution)
    #[arg(long, default_value_t = false)]
    verbose: bool,

    /// Append logs (without ANSI colors) to a file. Accepts an optional path;
    /// when omitted, defaults to ~/.future/agent/logs/agent.log. Parent
    /// directories are created if missing. Can also be enabled via
    /// FUTURE_AGENT_LOG_FILE (a path, or empty for the default location).
    #[arg(
        long,
        env = "FUTURE_AGENT_LOG_FILE",
        value_name = "PATH",
        num_args = 0..=1,
        default_missing_value = ""
    )]
    log_file: Option<String>,

    /// When file logging is enabled, keep only the newest N lines (trimmed at
    /// startup and as the file grows). 0 disables trimming.
    #[arg(
        long,
        env = "FUTURE_AGENT_LOG_MAX_LINES",
        value_name = "N",
        default_value_t = crate::logfile::DEFAULT_MAX_LINES
    )]
    log_max_lines: usize,

    /// Enable CPU profiling and write a flamegraph SVG to the given path on
    /// shutdown.  Profiling starts immediately and runs until the agent exits.
    #[arg(long, value_name = "PATH")]
    profile: Option<String>,

    /// Profile for N seconds then exit automatically (for benchmarking).
    /// Implies --profile with a default path when --profile is not also set.
    #[arg(long, value_name = "N")]
    profile_seconds: Option<u64>,

    /// Enable heap (memory) profiling and write a dhat report to the given
    /// path on shutdown.  Requires the `dhat-heap` build feature
    /// (`cargo build --release --features dhat-heap`); without it this flag
    /// is ignored with a warning.
    #[arg(long, value_name = "PATH")]
    profile_heap: Option<String>,
}

// When built with the `dhat-heap` feature, route every allocation through
// dhat's tracking allocator.  Without an active `dhat::Profiler` this is a
// thin pass-through to the system allocator, and the shim is compiled out
// entirely in normal builds.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// Parse `future-agent` args (argv without the program name) and run the
/// agent to completion. Both startup and runtime failures are returned to the
/// standalone/unified CLI entry point, which reports exit code 1 after process
/// guards (singleton lock, Windows permission cleanup, profilers) are dropped.
pub fn run_from_args(args: &[String]) -> Result<()> {
    let mut argv = vec!["future-agent".to_string()];
    // Keep helper dispatch deterministic even for a renamed standalone binary.
    let args = if args.first().map(String::as_str) == Some("agent")
        && args.get(1).map(String::as_str) == Some("--linux-sandbox-helper")
    {
        &args[1..]
    } else {
        args
    };
    argv.extend_from_slice(args);
    let cli = Cli::parse_from(argv);
    // Applied before anything reads a FutureOS path (sandbox helper dispatch,
    // singleton lock, logging, state) and before the threads that would make
    // mutating the process environment unsound. Child processes — sandboxed
    // shells and skills calling the `future` CLI — inherit the same home.
    if let Some(expected) = &cli.linux_sandbox_host_version {
        anyhow::ensure!(
            expected == crate::utils::VERSION,
            "Linux sandbox helper build mismatch; restart FutureOS after updating"
        );
    }
    if let Some(_request) = cli.linux_sandbox_helper.as_deref() {
        #[cfg(target_os = "linux")]
        crate::sandbox::linux::helper::run_helper_request(_request);
        #[cfg(not(target_os = "linux"))]
        anyhow::bail!("Linux sandbox helper is unavailable on this platform");
    }
    apply_future_home(&cli)?;
    if cli.probe_sandbox {
        let result = crate::sandbox::platform_sandbox_probe_product()?;
        println!("{}", serde_json::to_string(&result)?);
        return Ok(());
    }
    if cli.probe_windows_sandbox {
        let result = crate::sandbox::probe_windows_sandbox_host()?;
        if result.diagnostic().is_some() {
            tracing::debug!(code = result.code, "Windows sandbox host probe unavailable");
        }
        println!("{}", serde_json::to_string(&result)?);
        return Ok(());
    }
    if cli.reset_windows_sandbox {
        let removed = crate::sandbox::reset_windows_sandbox_capabilities()?;
        println!("{{\"removedCapabilities\":{removed}}}");
        return Ok(());
    }
    // Maintenance commands above are deliberately sessionless and must remain
    // usable while the server is running. Only the long-lived server owns the
    // user-scoped singleton lock.
    if cli.migration_source.is_some() && !cli.migrate_sessions && cli.retry_session_import.is_none()
    {
        anyhow::bail!("--migration-source requires --migrate-sessions or --retry-session-import");
    }
    let config = crate::service::AgentConfig {
        verbose: cli.verbose,
        grpc_addr: cli.grpc_addr.clone(),
        ..crate::service::AgentConfig::default()
    };
    crate::service::AgentService::with_instance(config, |service| {
        if cli.migrate_sessions || cli.retry_session_import.is_some() {
            let manager = Manager::new(
                cli.migration_source
                    .clone()
                    .unwrap_or_else(|| crate::utils::default_session_dir("")),
            );
            manager.initialize()?;
            if let Some(id) = cli.retry_session_import.as_deref() {
                manager.retry_legacy_import(id)?;
            }
            let report = manager.import_records()?;
            println!("{}", serde_json::to_string(&report)?);
            if let Some(id) = cli.retry_session_import.as_deref() {
                if !report
                    .iter()
                    .any(|record| record.session_id == id && record.status == "imported")
                {
                    anyhow::bail!("session import is still skipped; see the migration report");
                }
            }
            return Ok(());
        }
        // Environment hydration must precede the signal thread as well as Tokio's
        // worker threads: mutating the process environment is only sound here.
        crate::sandbox::hydrate_from_login_shell();
        // Outlive runtime destruction, profiler output, and Windows ACE cleanup.
        let (_shutdown_guard, shutdown_request) = shutdown::ShutdownGuard::start()?;
        service.run_with_cleanup(|| run(cli, shutdown_request, service))
    })
}

/// Apply the FutureOS home of this process: an explicit `--home`, or the
/// inherited `FUTURE_HOME`.
///
/// `--home` is validated strictly — a bad value must fail loudly instead of
/// silently starting against the default `~/.future` and sharing another
/// instance's lock, database and socket. `FUTURE_HOME` keeps the lenient rule
/// every client uses (empty/relative values are ignored), so an unrelated
/// empty variable can never stop the Agent from starting.
fn apply_future_home(cli: &Cli) -> Result<()> {
    if let Some(dir) = cli.home.as_deref() {
        return set_future_home(dir, "--home");
    }
    match future_rpc::home::future_home_override() {
        Some(inherited) => set_future_home(&inherited, future_rpc::home::FUTURE_HOME_ENV),
        None => Ok(()),
    }
}

/// Validate the directory, create it, and publish it for this process (and for
/// every child process it spawns). `source` names the input the user gave, so
/// the error points at the right one.
fn set_future_home(dir: &Path, source: &str) -> Result<()> {
    if !dir.is_absolute() {
        anyhow::bail!("{source} must be an absolute path: {}", dir.display());
    }
    std::fs::create_dir_all(dir)
        .with_context(|| format!("could not create FutureOS home {}", dir.display()))?;
    if !dir.is_dir() {
        anyhow::bail!("FutureOS home is not a directory: {}", dir.display());
    }
    std::env::set_var(future_rpc::home::FUTURE_HOME_ENV, dir);
    Ok(())
}

/// Test-only failure injection for the profiler error arms. The spawned
/// binary has no cfg(test), so integration tests steer via this env var.
/// Values: "build" (guard construction), "report" (report build), "write"
/// (flamegraph write).
#[cfg(not(windows))]
fn profiler_fail_at(stage: &str) -> bool {
    std::env::var("FUTURE_TEST_PROFILER_FAIL_AT").is_ok_and(|v| v == stage)
}

/// The full agent entry point — the former `main()` body.
fn run(
    cli: Cli,
    shutdown_request: shutdown::ShutdownRequest,
    service: &crate::service::AgentService<'_>,
) -> Result<()> {
    // Resolve profile path early (before the runtime starts).
    // --profile-seconds alone implies CPU profiling with a default path —
    // but NOT when --profile-heap is set: running the CPU sampler during a
    // heap profile pollutes the report (pprof's collector alone allocates
    // ~68 MB) and skews every measurement.
    let profile_path: Option<std::path::PathBuf> = cli
        .profile
        .as_deref()
        .or_else(|| {
            if cli.profile_seconds.is_some() && cli.profile_heap.is_none() {
                Some("agent-profile.svg")
            } else {
                None
            }
        })
        .map(std::path::PathBuf::from);

    // Start CPU profiling if requested.  The guard lives in run() so it
    // covers the entire agent lifetime including gRPC startup/shutdown.
    // ProfilerGuard is !Send so we keep it right here.
    #[cfg(not(windows))]
    let profiler_guard = match &profile_path {
        Some(_path) => {
            let profile_target = _path.display();
            tracing::info!("CPU profiling enabled, writing flamegraph to {profile_target}");
            let build_result = if profiler_fail_at("build") {
                Err(pprof::Error::CreatingError)
            } else {
                pprof::ProfilerGuardBuilder::default()
                    .frequency(997) // prime to avoid lock-step with timers
                    .blocklist(&["libc", "libgcc", "pthread", "vdso"])
                    .build()
            };
            match build_result {
                Ok(g) => Some(g),
                Err(e) => {
                    tracing::warn!("Failed to start profiler: {e} — continuing without profiling");
                    None
                }
            }
        }
        None => None,
    };
    #[cfg(windows)]
    let profiler_guard: Option<()> = {
        if profile_path.is_some() {
            tracing::warn!(
                "Built-in CPU profiling is not available on Windows (pprof is Unix-only)."
            );
            tracing::info!(
                "To profile on Windows, use: make profile-quick (requires blondie + admin)"
            );
            tracing::info!("Or run externally: blondie flamegraph future-agent.exe --grpc-addr ... --profile-seconds N");
        }
        None
    };

    // Start heap profiling if requested.  The Profiler writes its report to
    // disk when dropped, so it must outlive the tokio runtime — it lives
    // here in run() and is dropped explicitly before any early exit.
    #[cfg(feature = "dhat-heap")]
    let _heap_profiler = cli.profile_heap.as_ref().map(|path| {
        tracing::info!("Heap profiling enabled → will write dhat report to {path}");
        dhat::Profiler::builder().file_name(path).build()
    });
    #[cfg(not(feature = "dhat-heap"))]
    if cli.profile_heap.is_some() {
        tracing::warn!(
            "--profile-heap requires a build with --features dhat-heap — ignoring the flag"
        );
    }

    // Initialise tracing with timestamps. The console layer keeps ANSI colors;
    // the optional file layer writes through LogMirror, which shares one
    // mutexed File with the raw streaming prints (eprint_log!) — so the log
    // file ends up identical to the console output, minus ANSI colors.
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| default_log_filter(cli.verbose));

    let console_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time::SystemTime);

    // Resolve the log file target: an explicit path if given, otherwise the
    // default ~/.future/agent/logs/agent.log when the flag/env is present
    // without a value.
    let log_file = cli.log_file.as_deref().map(|p| {
        if p.is_empty() {
            crate::utils::default_config_dir().join("logs/agent.log")
        } else {
            std::path::PathBuf::from(p)
        }
    });

    let file_layer = match &log_file {
        Some(path) => {
            let mirror = crate::logfile::init(path, cli.log_max_lines)?;
            Some(
                tracing_subscriber::fmt::layer()
                    .with_target(false)
                    .with_timer(tracing_subscriber::fmt::time::SystemTime)
                    .with_ansi(false)
                    .with_writer(mirror),
            )
        }
        None => None,
    };

    use tracing_subscriber::prelude::*;
    tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer)
        .with(file_layer)
        .init();

    if let Some(path) = &log_file {
        tracing::info!(
            "file logging enabled: {} (keeping last {} lines)",
            path.display(),
            cli.log_max_lines
        );
    }

    // Build model registry BEFORE tokio runtime starts.
    // Registry::new() uses reqwest::blocking::Client internally,
    // which creates a nested runtime that cannot be dropped in async context.
    // Wrap in Arc<RwLock> so AppState can share the cached registry and
    // get_state_internal avoids repeated blocking network I/O.
    let prepared = service.prepare(crate::service::StartupCancellation::default())?;

    // Launch async portion
    // 2 MB thread stack (Rust's default) is enough for async I/O while
    // leaving headroom for deep serde/JSON recursion (was 4 MB).
    // On a 32-core machine this still saves 64 MB virtual memory vs 4 MB.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(2 * 1024 * 1024)
        .build()?;
    let run_result = runtime.block_on(async_main(prepared, cli, shutdown_request));
    // A FailedStopping report retains the instance. Do not let a detached
    // blocking task prevent the CLI host from reaching its process-exit path.
    runtime.shutdown_timeout(std::time::Duration::from_secs(1));

    // Write profiling after host runtime teardown. A FailedStopping report
    // can still retain blocking work until process exit; it never authorizes
    // releasing the service instance. Build before the profiler guard drops.
    #[cfg(not(windows))]
    if let (Some(guard), Some(path)) = (profiler_guard, profile_path) {
        tracing::info!("Writing CPU profile flamegraph to {}", path.display());
        let report_result = if profiler_fail_at("report") {
            Err(pprof::Error::NotRunning)
        } else {
            guard.report().build()
        };
        match report_result {
            Ok(report) => {
                let file = std::fs::File::create(&path)
                    .map_err(|e| {
                        tracing::error!("Cannot create profile file {}: {}", path.display(), e);
                        e
                    })
                    .ok();
                if let Some(f) = file {
                    let write_result = if profiler_fail_at("write") {
                        Err(std::io::Error::other("injected flamegraph write failure").into())
                    } else {
                        report.flamegraph(f)
                    };
                    if let Err(e) = write_result {
                        tracing::error!("Failed to write flamegraph: {e}");
                    } else {
                        let sz = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        tracing::info!(
                            "Flamegraph written: {} ({:.1} KB)",
                            path.display(),
                            sz as f64 / 1024.0
                        );
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to build profiling report: {e}");
            }
        }
    }
    #[cfg(windows)]
    let _ = (profiler_guard, profile_path);

    // Propagate async_main's failure so the standalone/unified entry point can
    // report exit code 1 *after* process-lifetime cleanup guards have run.
    if let Err(e) = run_result {
        tracing::error!("Agent exited with error: {e}");
        return Err(e);
    }
    Ok(())
}

async fn async_main(
    prepared: crate::service::PreparedAgent<'_, '_>,
    cli: Cli,
    shutdown_request: shutdown::ShutdownRequest,
) -> Result<()> {
    let running = prepared.initialize().await?;
    let handle = running.handle();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let server = running.serve(Some(ready_tx));
    tokio::pin!(server);
    // A zero-duration timer can run on another worker before `serve` is first
    // polled. Admit the listener before enabling host shutdown sources, using
    // the same readiness boundary as the embedded host rather than a delay.
    tokio::select! {
        biased;
        result = &mut server => return result,
        ready = ready_rx => {
            ready.context("Agent serving ended before readiness notification")?;
        }
    }
    // If --profile-seconds is set, spawn a task that signals shutdown after N
    // seconds via a oneshot so the flamegraph gets written by run().
    // When --profile-seconds is not set, use pending() so the select branch
    // never fires.
    let profile_rx: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
        if let Some(secs) = cli.profile_seconds {
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            let handle = handle.clone();
            tokio::spawn(async move {
                tracing::info!(
                    "Profile timer: agent will auto-shutdown after {} seconds",
                    secs
                );
                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                tracing::info!("Profile timer expired — shutting down for flamegraph capture");
                tracing::info!("Profile timer completed — draining active streams");
                handle.request_shutdown(std::time::Duration::from_secs(5));
                let _ = tx.send(());
            });
            Box::pin(async move {
                let _ = rx.await;
            })
        } else {
            Box::pin(std::future::pending())
        };

    tokio::select! {
        result = &mut server => return result,
        signal = shutdown_request => {
            signal.context("Ctrl-C listener stopped unexpectedly")?
                .context("Could not listen for Ctrl-C")?;
            handle.request_shutdown(std::time::Duration::from_secs(4));
            tracing::info!("Ctrl-C received — press Ctrl-C again to force exit");
        }
        _ = profile_rx => {
            // profile timer handled inside the future
        }
    }
    let report = handle.shutdown(std::time::Duration::from_secs(4));
    tokio::pin!(report);
    tokio::select! {
        report = &mut report => {
            anyhow::ensure!(report.status == crate::service::ShutdownStatus::Stopped, "Agent failed to stop: {}", report.errors.join("; "));
            server.await
        }
        result = &mut server => {
            result?;
            let report = report.await;
            anyhow::ensure!(report.status == crate::service::ShutdownStatus::Stopped, "Agent failed to stop: {}", report.errors.join("; "));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_project_context_skips_unreadable_candidates() {
        // AGENTS.md exists but is a DIRECTORY (read fails) → CLAUDE.md.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("AGENTS.md")).unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "agent notes").unwrap();
        let context = crate::prompt::load_project_context(&dir.path().to_string_lossy(), true);
        assert_eq!(context.content, "agent notes");
    }

    #[test]
    fn load_project_context_empty_when_nothing_readable() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            crate::prompt::load_project_context(&dir.path().to_string_lossy(), true)
                .content
                .is_empty()
        );
    }

    #[test]
    fn home_flag_parses_the_directory() {
        let cli = Cli::parse_from([
            "future-agent",
            "--home",
            "/tmp/futureos-instance-b",
            "--grpc-addr",
            "127.0.0.1:0",
        ]);
        assert_eq!(
            cli.home.as_deref(),
            Some(std::path::Path::new("/tmp/futureos-instance-b"))
        );
    }

    #[test]
    fn home_flag_rejects_relative_and_non_directory_paths() {
        assert!(set_future_home(Path::new("relative/future-home"), "--home").is_err());
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-directory");
        std::fs::write(&file, "x").unwrap();
        assert!(set_future_home(&file, "--home").is_err());
    }

    /// A `FUTURE_HOME` the Agent cannot use is ignored, not fatal — the same
    /// rule clients apply, so an empty export cannot stop the Agent.
    #[test]
    fn unusable_inherited_future_home_is_ignored() {
        let _guard = crate::test_support::home_env_lock();
        let previous = std::env::var_os(future_rpc::home::FUTURE_HOME_ENV);
        std::env::set_var(future_rpc::home::FUTURE_HOME_ENV, "relative/future-home");
        let cli = Cli::parse_from(["future-agent"]);
        assert!(cli.home.is_none());
        apply_future_home(&cli).unwrap();
        assert_eq!(future_rpc::home::future_home_override(), None);
        crate::test_support::restore_env(future_rpc::home::FUTURE_HOME_ENV, &previous);
    }

    /// An inherited absolute `FUTURE_HOME` is applied (and created) exactly
    /// like the flag, so a client-launched sidecar lands in the same home.
    #[test]
    fn inherited_future_home_is_applied() {
        let _guard = crate::test_support::home_env_lock();
        let previous = std::env::var_os(future_rpc::home::FUTURE_HOME_ENV);
        let dir = tempfile::tempdir().unwrap();
        let inherited = dir.path().join("instance-b");
        std::env::set_var(future_rpc::home::FUTURE_HOME_ENV, &inherited);
        let cli = Cli::parse_from(["future-agent"]);
        apply_future_home(&cli).unwrap();
        assert!(inherited.is_dir());
        assert_eq!(crate::utils::future_home(), inherited);
        crate::test_support::restore_env(future_rpc::home::FUTURE_HOME_ENV, &previous);
    }

    /// `--home` moves every FutureOS path of the process (lock, database,
    /// sessions, and — on Unix — the local IPC endpoint), which is what makes a
    /// second Agent instance isolated instead of a second writer to the same
    /// state. The override is process-global, hence the shared home lock.
    #[test]
    fn home_flag_selects_an_isolated_futureos_root() {
        let _guard = crate::test_support::home_env_lock();
        let previous_home = std::env::var_os(future_rpc::home::FUTURE_HOME_ENV);
        let previous_socket = std::env::var_os("FUTURE_AGENT_SOCKET");
        std::env::remove_var("FUTURE_AGENT_SOCKET");

        let dir = tempfile::tempdir().unwrap();
        let isolated = dir.path().join("instance-b");
        let cli = Cli::parse_from(["future-agent", "--home", isolated.to_str().unwrap()]);
        apply_future_home(&cli).unwrap();

        assert_eq!(crate::utils::future_home(), isolated);
        assert_eq!(crate::utils::default_config_dir(), isolated.join("agent"));
        assert_eq!(
            crate::utils::default_session_dir("/any/cwd"),
            isolated.join("agent").join("sessions")
        );
        assert_eq!(
            crate::utils::default_config_dir().join("agent-instance.lock"),
            isolated.join("agent").join("agent-instance.lock")
        );
        #[cfg(unix)]
        assert_eq!(
            future_rpc::transport::local_socket_path(),
            isolated.join("run").join("agent.sock")
        );

        crate::test_support::restore_env(future_rpc::home::FUTURE_HOME_ENV, &previous_home);
        crate::test_support::restore_env("FUTURE_AGENT_SOCKET", &previous_socket);
    }

    #[test]
    fn without_a_home_override_the_root_is_dot_future_under_the_user_home() {
        let _home = crate::test_support::TestHome::new();
        let previous = std::env::var_os(future_rpc::home::FUTURE_HOME_ENV);
        std::env::remove_var(future_rpc::home::FUTURE_HOME_ENV);
        // Compare against the resolved home (TestHome canonicalizes it) rather
        // than the raw temp path, which is a symlink on macOS.
        assert_eq!(
            crate::utils::future_home(),
            crate::utils::home_dir().join(".future")
        );
        assert_eq!(
            crate::utils::default_config_dir(),
            crate::utils::future_home().join("agent")
        );
        crate::test_support::restore_env(future_rpc::home::FUTURE_HOME_ENV, &previous);
    }

    #[test]
    fn mismatched_helper_rejects_before_home_validation_or_initialization() {
        let error = run_from_args(&[
            "--linux-sandbox-helper".into(),
            "fd:999".into(),
            "--linux-sandbox-host-version".into(),
            "not-this-build".into(),
            "--home".into(),
            "relative-root".into(),
        ])
        .unwrap_err();
        assert!(error.to_string().contains("helper build mismatch"));
    }

    /// The Linux sandbox helper is a Linux-only entry point; asking for it from
    /// another platform must fail loudly rather than silently skipping the flag.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn the_linux_sandbox_helper_flag_is_rejected_off_linux() {
        let error = run_from_args(&["--linux-sandbox-helper".to_string(), "probe".to_string()])
            .unwrap_err();
        assert!(
            error.to_string().contains("unavailable on this platform"),
            "unexpected error: {error}"
        );
    }

    /// The hidden helper is dispatched by stripping an `agent` prefix, so a
    /// renamed standalone binary still reaches it deterministically. That
    /// rewrite is platform-independent, and off Linux the helper itself is then
    /// refused — which is what proves the rewrite ran rather than the flag being
    /// silently accepted.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn the_agent_prefixed_helper_is_rewritten_then_refused_off_linux() {
        let error = run_from_args(&[
            "agent".to_string(),
            "--linux-sandbox-helper".to_string(),
            "probe".to_string(),
        ])
        .unwrap_err();
        assert!(
            error.to_string().contains("unavailable on this platform"),
            "unexpected error: {error}"
        );
    }

    /// `--reset-windows-sandbox` is a sessionless maintenance command: it must
    /// run without taking the singleton lock (the installer runs it while the
    /// Agent is up) and print a machine-readable count. Run against an isolated
    /// home so no real capability state is touched.
    #[cfg(target_os = "windows")]
    #[test]
    fn reset_windows_sandbox_is_a_sessionless_maintenance_command() {
        let _home = crate::test_support::TestHome::new();
        assert!(run_from_args(&["--reset-windows-sandbox".to_string()]).is_ok());
    }

    /// `--migration-source` only means something with one of the two migration
    /// commands; alone it must be rejected before any state is touched.
    #[test]
    fn a_migration_source_without_a_migration_command_is_rejected() {
        let error = run_from_args(&[
            "--migration-source".to_string(),
            "/tmp/legacy-sessions".to_string(),
        ])
        .unwrap_err();
        assert!(
            error.to_string().contains("--migrate-sessions"),
            "unexpected error: {error}"
        );
    }

    /// `agent-instance.json` is a convenience for the installer, never a
    /// requirement: when it cannot be written the Agent must still start and do
    /// its job (the lock is what makes it a singleton).
    #[test]
    fn unbootable_instance_metadata_does_not_stop_startup() {
        let home = crate::test_support::TestHome::new();
        let config = crate::utils::default_config_dir();
        std::fs::create_dir_all(&config).unwrap();
        // A directory where the metadata file belongs: `fs::write` cannot win.
        std::fs::create_dir_all(config.join("agent-instance.json")).unwrap();
        let source = home.path().join("legacy-sessions");
        std::fs::create_dir_all(&source).unwrap();
        let result = run_from_args(&[
            "--migrate-sessions".to_string(),
            "--migration-source".to_string(),
            source.to_string_lossy().to_string(),
        ]);
        assert!(
            result.is_ok(),
            "startup continued without metadata: {result:?}"
        );
    }
}

#[cfg(test)]
mod future_home_tests {
    use super::*;

    /// `--home` points at a directory; a path that exists as a *file* fails at
    /// the creation step with a message naming the input the user gave. This is
    /// also the evidence for the classification of the following
    /// `is_dir()` check: `create_dir_all` cannot succeed on an existing file, so
    /// that check is only a belt-and-braces guard (it can be reached only if the
    /// path is swapped between the two calls).
    #[test]
    fn a_home_path_that_is_a_file_fails_at_creation_and_names_the_input() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-directory");
        std::fs::write(&file, "occupied").unwrap();

        let error = set_future_home(&file, "--home").expect_err("a file is not a home");
        let message = format!("{error:#}");
        assert!(
            message.contains("could not create FutureOS home"),
            "creation fails first: {message}"
        );
        // The creation arm names the *path*; its two siblings name `source`.
        // Asserted as-is and recorded as F-11 in the module record rather than
        // "fixed" here, because changing a user-visible message is a behaviour
        // change and belongs with a deliberate decision.
        assert!(
            message.contains(&file.display().to_string()),
            "the message points at the path it could not use: {message}"
        );
        assert!(
            !message.contains("is not a directory"),
            "the later guard is not the one that fires: {message}"
        );

        // A relative path is refused before touching the filesystem.
        let error = set_future_home(std::path::Path::new("relative/home"), "--home")
            .expect_err("a relative home is refused");
        assert!(
            error.to_string().contains("must be an absolute path"),
            "{error}"
        );

        // A writable directory is accepted, and a missing one is created.
        let fresh = dir.path().join("nested/new-home");
        let previous = std::env::var_os(future_rpc::home::FUTURE_HOME_ENV);
        let _guard = crate::test_support::home_env_lock();
        set_future_home(&fresh, "--home").expect("a fresh absolute directory is accepted");
        assert!(fresh.is_dir());
        assert_eq!(
            std::env::var_os(future_rpc::home::FUTURE_HOME_ENV),
            Some(fresh.clone().into_os_string())
        );
        crate::test_support::restore_env(future_rpc::home::FUTURE_HOME_ENV, &previous);
    }
}

/// `async_main`'s final `select!` — the shutdown report racing the serve future
/// — and the agent's refusal to pass a drain that was cut short off as a clean
/// stop.
#[cfg(test)]
mod cut_short_drain_tests {
    /// The agent's last decision. `async_main` ends with a `select!` between the
    /// shutdown **report** completing and the serve future completing; its first
    /// arm runs `ensure!(report.status == Stopped, "Agent failed to stop: …")`,
    /// so an agent whose drain could not settle reports the stage that never
    /// settled and exits non-zero instead of leaving with a clean exit code.
    ///
    /// The first arm is taken whenever the report becomes ready while the serve
    /// future is still pending. The report can only be published early by the
    /// deadline watcher (`FailedStopping`), because the drain itself ends only
    /// after the listener stage — which cannot settle before the serve future
    /// finishes. So the fixture must keep a drain stage open past the 4 s
    /// deadline `async_main` itself requests. The drain cancels session runtime
    /// state (runs, shells, approvals) but **not** a platform fetch: one Future
    /// platform credential (`auth.json`'s `future.base_url`) pointing at an
    /// endpoint that accepts TCP and never answers leaves the cold-cache
    /// catalogue refresh (`models/future.rs`'s `spawn_future_models_refresh`,
    /// which holds a `process_background()` registration across its native
    /// fetch thread) outstanding, so the "runs and native background work" stage
    /// never settles. The listener stage is then never reached, the serve future
    /// is still pending when the report lands, and the first arm runs.
    ///
    /// The child is this test binary re-executed (the pattern
    /// `cli::shutdown::tests` uses) and becomes the agent through the same entry
    /// point `main` calls; a subprocess is required because the startup path
    /// installs process-global state. The control leg runs the identical
    /// fixture **without** the Future credential, which stops cleanly — that is
    /// what makes the failing leg attributable to the un-cancellable platform
    /// work rather than to the isolated home or the interrupt itself.
    #[cfg(unix)]
    #[test]
    fn a_cut_short_drain_is_reported_instead_of_a_clean_stop() {
        use std::time::Duration;

        const CHILD: &str = "FUTURE_TEST_CUT_SHORT_DRAIN_CHILD";
        const PORT: &str = "FUTURE_TEST_CUT_SHORT_DRAIN_PORT";
        const OUTCOME: &str = "FUTURE_TEST_CUT_SHORT_DRAIN_OUTCOME";
        const THIS_TEST: &str =
            "cli::cut_short_drain_tests::a_cut_short_drain_is_reported_instead_of_a_clean_stop";

        if std::env::var_os(CHILD).is_some() {
            // The child *is* the agent. `--grpc-addr` with no `--profile-seconds`
            // keeps it serving until the interrupt below.
            let port = std::env::var(PORT).expect("child gRPC port");
            let outcome = crate::cli::run_from_args(&[
                "--grpc-addr".to_string(),
                format!("127.0.0.1:{port}"),
            ]);
            // Announce the outcome before the Ctrl-C grace watchdog can force
            // the process out from under us. A *file*, not stdout: the libtest
            // harness captures this process's `print!` output and discards it
            // when the test exits. The parent always supplies the path, so a
            // missing one is a broken harness and `expect` is the honest answer.
            let outcome_path = std::env::var(OUTCOME).expect("child outcome path");
            let marker = match &outcome {
                Ok(()) => "clean-stop".to_string(),
                Err(error) => format!("failed-stop:{error:#}"),
            };
            let _ = std::fs::write(outcome_path, marker);
            std::process::exit(if outcome.is_ok() { 0 } else { 1 });
        }

        /// One leg of the fixture: an isolated FutureOS home, one real agent
        /// subprocess, one SIGINT. `platform_stalls` decides whether that home
        /// carries a Future platform credential pointing at an endpoint that
        /// accepts and never answers.
        async fn leg(
            platform_stalls: bool,
        ) -> (std::process::ExitStatus, Duration, Option<String>, String) {
            use std::time::{Duration, Instant};

            let home = tempfile::tempdir().expect("isolated home");
            let agent_dir = home.path().join(".future").join("agent");
            std::fs::create_dir_all(&agent_dir).expect("agent config dir");
            let held: std::sync::Arc<std::sync::Mutex<Vec<std::net::TcpStream>>> =
                Default::default();
            if platform_stalls {
                let stalling =
                    std::net::TcpListener::bind("127.0.0.1:0").expect("stalling endpoint");
                let stalling_addr = stalling.local_addr().expect("stalling address");
                let held_writer = held.clone();
                std::thread::spawn(move || {
                    // Hold every connection open and never answer: the fetch
                    // waits, which is the whole fixture. The loop ends when the
                    // listener errors, which is the only way it can stop.
                    while let Ok((stream, _)) = stalling.accept() {
                        held_writer.lock().expect("held").push(stream);
                    }
                });
                std::fs::write(
                    agent_dir.join("auth.json"),
                    format!(
                        r#"{{"future":{{"type":"api_key","key":"stall-key","base_url":"http://{stalling_addr}"}}}}"#
                    ),
                )
                .expect("auth.json");
                std::fs::write(
                    agent_dir.join("models.json"),
                    format!(
                        r#"{{"providers":{{"custom":{{"api":"openai-completions","baseUrl":"http://{stalling_addr}/v1","apiKey":"sk-custom","models":[{{"id":"stall-model","limit":{{"context":64000,"output":4096}}}}]}}}}}}"#
                    ),
                )
                .expect("models.json");
                std::fs::write(
                    agent_dir.join("settings.json"),
                    r#"{"defaultModel": "custom/stall-model", "maxTurns": 9}"#,
                )
                .expect("settings.json");
            }

            let port = std::net::TcpListener::bind("127.0.0.1:0")
                .expect("probe a free port")
                .local_addr()
                .expect("probed address")
                .port();
            let outcome_path = home.path().join("outcome.txt");
            let log_path = home.path().join("child.log");
            let log = std::fs::File::create(&log_path).expect("child log");
            let mut child =
                std::process::Command::new(std::env::current_exe().expect("test binary"))
                    .arg("--exact")
                    .arg(THIS_TEST)
                    .env(CHILD, "1")
                    .env(PORT, port.to_string())
                    .env(OUTCOME, &outcome_path)
                    .env("HOME", home.path())
                    .env("USERPROFILE", home.path())
                    // The child's home is the isolated one above, whichever way
                    // the FutureOS root is selected.
                    .env_remove(future_rpc::home::FUTURE_HOME_ENV)
                    .stdout(std::process::Stdio::from(
                        log.try_clone().expect("log clone"),
                    ))
                    .stderr(std::process::Stdio::from(log))
                    .spawn()
                    .expect("spawn child agent");

            // The agent publishes its listener before it can answer, so a
            // successful handshake is the readiness boundary — and it proves the
            // failure below is not "the agent never started".
            let address = format!("127.0.0.1:{port}");
            let deadline = Instant::now() + Duration::from_secs(30);
            while let Err(error) = future_rpc::connection::connect_verified(
                &address,
                future_rpc::connection::EXPECTED_AGENT_VERSION,
                Duration::from_secs(1),
                Duration::from_secs(1),
            )
            .await
            {
                assert!(
                    child.try_wait().expect("poll child").is_none(),
                    "the child agent exited before it served: {error}"
                );
                assert!(
                    Instant::now() < deadline,
                    "the child agent never served: {error}"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if platform_stalls {
                // The fixture needs the platform catalogue fetch in flight
                // before the interrupt, or the leg would be testing nothing.
                // This is a precondition rather than a wait: the refresh is
                // spawned during `prepare()` (before the listener binds), and in
                // the measured run the connection was already held when the
                // handshake returned -- the wait loop this replaced never
                // entered its body (its three lines read `DA:0`).
                let fetch_in_flight = !held.lock().expect("held").is_empty();
                assert!(fetch_in_flight, "the platform fetch must be in flight");
            }

            // Interrupt the agent: its drain now has either nothing to wait for
            // or a platform fetch it cannot cancel, and its report is due 4 s
            // later.
            let interrupted = Instant::now();
            // SAFETY: the pid is this test's own child and the signal is SIGINT,
            // exactly what a user's Ctrl-C delivers.
            unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
            let deadline = Instant::now() + Duration::from_secs(30);
            let status = loop {
                if let Some(status) = child.try_wait().expect("poll child") {
                    break status;
                }
                assert!(
                    Instant::now() < deadline,
                    "the child agent did not exit after the interrupt"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
            let mut output = String::new();
            std::io::Read::read_to_string(
                &mut std::fs::File::open(&log_path).expect("child log"),
                &mut output,
            )
            .expect("read child log");
            let outcome = std::fs::read_to_string(&outcome_path).ok();
            (status, interrupted.elapsed(), outcome, output)
        }

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("parent runtime");

        // Control leg first: the same home, agent and interrupt, but no Future
        // platform credential, so nothing in the drain can be left waiting.
        let (status, elapsed, outcome, output) = runtime.block_on(leg(false));
        assert_eq!(
            outcome.as_deref(),
            Some("clean-stop"),
            "a drain with nothing outstanding must stop cleanly \
             ({status:?} after {elapsed:?}): {output}"
        );
        assert_eq!(
            status.code(),
            Some(0),
            "control exit after {elapsed:?}: {output}"
        );

        // The fixture leg: the platform fetch is still waiting when the drain's
        // 4 s deadline expires.
        let (status, elapsed, outcome, output) = runtime.block_on(leg(true));
        // Diagnostic (shown on failure, and with `--nocapture`): which of the
        // two honest outcomes this run produced, plus the child's own log.
        println!("cut-short drain: outcome={outcome:?} status={status:?} elapsed={elapsed:?}");
        println!("cut-short drain child log:\n{output}");
        let reported = outcome.clone().unwrap_or_default();
        let forced_out = status.code() == Some(130);
        let reported_failure = reported.contains("failed-stop:")
            && reported.contains("Agent failed to stop")
            && reported.contains("Agent shutdown deadline exceeded during");
        // The refusal runs ~1 s ahead of the Ctrl-C grace watchdog, which
        // `std::process::exit(130)`s without unwinding. Both outcomes are the
        // same behaviour -- the drain did not settle and the process did not
        // exit 0 -- and the coverage of the refusal itself is measured from the
        // profile, not from these lines.
        let silent_but_forced = reported.is_empty() && forced_out;
        let refused = reported_failure || silent_but_forced;
        assert!(
            refused,
            "the cut-short drain neither failed nor was forced out"
        );
        assert!(
            elapsed >= Duration::from_secs(3),
            "the drain must reach its deadline"
        );
    }
}
