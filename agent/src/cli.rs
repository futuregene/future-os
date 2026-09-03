//! `future-agent` CLI entry — moved out of `main.rs` so the same code can be
//! run either as the standalone `future-agent` binary or embedded in the
//! `future` CLI (`future agent <args>`). The behavior is identical in both
//! cases; `Cli::parse_from` is fed a synthetic argv whose program name is
//! always `future-agent`, so help/error text matches the standalone binary.

use crate::{Engine, EngineConfig, Manager, ModelRegistry};
use anyhow::Result;
use chrono::Local;
use clap::Parser;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

type AgentInstanceGuard = fd_lock::RwLockWriteGuard<'static, File>;

struct CleanupGuard<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> CleanupGuard<F> {
    fn new(cleanup: F) -> Self {
        Self(Some(cleanup))
    }
}

impl<F: FnOnce()> Drop for CleanupGuard<F> {
    fn drop(&mut self) {
        if let Some(cleanup) = self.0.take() {
            cleanup();
        }
    }
}

/// Hold a user-scoped process lock for the full server lifetime. The gRPC port
/// is not a sufficient singleton boundary because a second agent can choose a
/// different `--grpc-addr` while still sharing sessions, approval state, and
/// Windows sandbox capability metadata with the first one.
fn acquire_agent_instance_lock() -> Result<AgentInstanceGuard> {
    acquire_agent_instance_lock_at(&crate::utils::default_config_dir().join("agent-instance.lock"))
}

fn acquire_agent_instance_lock_at(path: &Path) -> Result<AgentInstanceGuard> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        // Do not truncate until after the exclusive lock is held; a rejected
        // second process must not erase the running Agent's diagnostic PID.
        .truncate(false)
        .open(path)?;
    // The lock object must outlive its write guard. This function runs once per
    // server process, so retaining the tiny allocation until process exit is
    // intentional; the OS releases the file lock even after a crash/force-kill.
    let lock = Box::leak(Box::new(fd_lock::RwLock::new(file)));
    let mut guard = lock.try_write().map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            anyhow::anyhow!(
                "Future Agent is already running for this user (lock: {})",
                path.display()
            )
        } else {
            anyhow::Error::from(error)
        }
    })?;
    guard.seek(SeekFrom::Start(0))?;
    guard.set_len(0)?;
    writeln!(guard, "{}", std::process::id())?;
    guard.flush()?;
    Ok(guard)
}

/// Map of live server sessions shared with the shutdown paths.
type SessionsMap = Arc<
    parking_lot::RwLock<
        std::collections::HashMap<String, Arc<parking_lot::RwLock<crate::rpc::ServerSession>>>,
    >,
>;

/// Load the first readable project-context file (CLAUDE.md / AGENTS.md /
/// GEMINI.md) in `cwd`. A file that exists but cannot be read (e.g. it is a
/// directory) is skipped, falling through to the next candidate name.
fn load_project_context(cwd: &str) -> String {
    for fname in &["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
        let p = std::path::Path::new(cwd).join(fname);
        if p.exists() {
            if let Ok(content) = std::fs::read_to_string(&p) {
                return content;
            }
        }
    }
    String::new()
}

/// Abort every live session (SIGINT / profile-timer shutdown path).
fn abort_all_sessions(sessions: &SessionsMap) {
    for s in sessions.read().values() {
        s.read().abort();
    }
}

#[derive(Parser)]
#[command(name = "future-agent")]
#[command(version = crate::utils::VERSION)]
pub struct Cli {
    /// Internal Linux sandbox helper request. This is intentionally hidden and
    /// dispatched before singleton/config/runtime initialization.
    #[arg(long, hide = true, value_name = "REQUEST")]
    linux_sandbox_helper: Option<String>,

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

    /// gRPC server address (host:port, e.g., 127.0.0.1:50051)
    #[arg(long, default_value = "127.0.0.1:50051")]
    grpc_addr: String,

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
    argv.extend_from_slice(args);
    let cli = Cli::parse_from(argv);
    if let Some(_request) = cli.linux_sandbox_helper.as_deref() {
        #[cfg(target_os = "linux")]
        crate::sandbox::linux::helper::run_helper_request(_request);
        #[cfg(not(target_os = "linux"))]
        anyhow::bail!("Linux sandbox helper is unavailable on this platform");
    }
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
    let _instance_guard = acquire_agent_instance_lock()?;
    run_agent_lifecycle(
        cleanup_windows_sandbox_on_startup,
        || run(cli),
        cleanup_windows_sandbox_on_exit,
    )
}

fn run_agent_lifecycle<T>(
    startup_cleanup: impl FnOnce(),
    run_server: impl FnOnce() -> Result<T>,
    exit_cleanup: impl FnOnce(),
) -> Result<T> {
    startup_cleanup();
    // RAII makes cleanup cover successful shutdown, server/config errors, and
    // unwindable panics. Process abort/force-exit cannot run destructors and is
    // intentionally recovered by the next startup cleanup instead.
    let _exit_cleanup = CleanupGuard::new(exit_cleanup);
    run_server()
}

/// Once the singleton is held, no other Future Agent for this user can own a
/// live restricted process tree. Reclaim ACEs left by a previous crash before
/// accepting commands; the separate capability lock still fails safely if an
/// installer/maintenance process is concurrently touching the state.
fn cleanup_windows_sandbox_on_startup() {
    #[cfg(target_os = "windows")]
    match crate::sandbox::reset_windows_sandbox_capabilities() {
        Ok(removed) if removed > 0 => {
            eprintln!("Future Agent: cleaned {removed} stale Windows sandbox permission(s)")
        }
        Ok(_) => {}
        Err(error) => {
            eprintln!("Future Agent: could not clean stale Windows sandbox permissions: {error}")
        }
    }
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
pub(crate) fn run(cli: Cli) -> Result<()> {
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

    // Load the user's login-shell PATH/env BEFORE spawning any threads or the
    // tokio runtime — set_var is only sound while single-threaded. Fixes
    // "command not found" for user-installed tools (nvm/Homebrew/npm-global)
    // when the agent is launched from a GUI with a minimal inherited PATH.
    crate::sandbox::hydrate_from_login_shell();

    // Initialise tracing with timestamps. The console layer keeps ANSI colors;
    // the optional file layer writes through LogMirror, which shares one
    // mutexed File with the raw streaming prints (eprint_log!) — so the log
    // file ends up identical to the console output, minus ANSI colors.
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

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
    let model_registry = Arc::new(parking_lot::RwLock::new(ModelRegistry::new()));

    // Launch async portion
    // 2 MB thread stack (Rust's default) is enough for async I/O while
    // leaving headroom for deep serde/JSON recursion (was 4 MB).
    // On a 32-core machine this still saves 64 MB virtual memory vs 4 MB.
    let run_result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(2 * 1024 * 1024)
        .build()?
        .block_on(async_main(model_registry, cli));

    // Write profiling flamegraph on shutdown (after the runtime drops,
    // so all async tasks have settled).  ProfilerGuard stops sampling on
    // drop, so we must build the report BEFORE dropping the guard.
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
    model_registry: Arc<parking_lot::RwLock<ModelRegistry>>,
    cli: Cli,
) -> Result<()> {
    let cwd = crate::utils::home_dir().to_string_lossy().to_string();

    let all_models = model_registry.read().all_models();

    // Load settings
    let settings_path = std::path::PathBuf::from(crate::models::settings_path());
    let settings = match crate::config::load_settings(&settings_path) {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(
                "Failed to load settings from {}: {}. Falling back to defaults.",
                settings_path.display(),
                error
            );
            crate::Settings::default()
        }
    };

    // Load auth store
    let auth_store = crate::AuthStore::load();

    // Resolve the *initial* model: the settings default, else the first model
    // that has credentials (a built-in key or an auth.json entry for its
    // provider). This is only a starting point — clients (GUI/TUI) set their own
    // model per session via the `set_model` RPC, which rebuilds the registry and
    // reloads auth.json, so the initial choice is not authoritative.
    //
    // IMPORTANT — do NOT turn "nothing configured" back into `process::exit(1)`.
    // The agent runs as a Tauri sidecar that the GUI spawns and connects to over
    // gRPC. On a fresh install there is no auth.json yet, so no model resolves —
    // but the user logs in *from inside the GUI*, which needs the agent already
    // reachable to drive the flow. If the agent exited here, that first-run login
    // could never complete (the GUI spawns the sidecar exactly once at startup;
    // see desktop/src-tauri/src/agent_supervisor.rs), and the app would look broken
    // out of the box. So when nothing is configured we log a warning and start
    // the server anyway with an empty model. The endpoint stays unconfigured
    // until the first `set_model` call, which resolves base_url + api_key from a
    // freshly loaded auth.json (see agent/src/rpc/session.rs::set_model).
    let resolved_model = {
        // Prefer future/deepseek-v4-pro when the future provider is configured.
        let preferred = if auth_store.get("future").is_some()
            || all_models
                .iter()
                .any(|m| m.provider == "future" && !m.api_key.is_empty())
        {
            all_models
                .iter()
                .find(|m| m.provider == "future" && m.id == "deepseek-v4-pro")
                .map(|m| format!("{}/{}", m.provider, m.id))
        } else {
            None
        };
        preferred
            .or_else(crate::models::get_default_model)
            .or_else(|| {
                all_models
                    .iter()
                    .find(|m| !m.api_key.is_empty() || auth_store.get(&m.provider).is_some())
                    .map(|m| m.id.clone())
            })
            .unwrap_or_default()
    };
    if resolved_model.is_empty() {
        tracing::info!(
            "future-agent: no model configured yet — starting the gRPC server \
             anyway so a client can log in and pick a model. Add an API key via \
             'future auth login' or the desktop app, or configure a provider in \
             ~/.future/agent/models.json."
        );
    }

    // Resolve model config
    let model_config = model_registry.read().resolve(&resolved_model);

    let engine_model = model_config
        .as_ref()
        .map(|m| m.id.clone())
        .unwrap_or_else(|| resolved_model.clone());

    // Resolve base URL: models.json > auth.json baseUrl > built-in default
    let base_url = model_config
        .as_ref()
        .filter(|m| !m.base_url.is_empty())
        .map(|m| m.base_url.clone())
        .or_else(|| {
            model_config
                .as_ref()
                .and_then(|m| auth_store.base_url(&m.provider))
        })
        .unwrap_or_default();
    let base_url = if base_url.is_empty() {
        "https://api.openai.com/v1".to_string()
    } else {
        base_url
    };

    // Resolve API key from auth.json > model config
    let api_key = model_config
        .as_ref()
        .and_then(|m| auth_store.get(&m.provider))
        .or_else(|| {
            model_config
                .as_ref()
                .filter(|m| !m.api_key.is_empty())
                .map(|m| m.api_key.clone())
        })
        .unwrap_or_default();

    // Default thinking level (clients override per-session).

    // Honor each model's advertised output limit. Models without one retain
    // the existing reasoning/non-reasoning fallbacks.
    let max_tokens = model_config
        .as_ref()
        .map(crate::models::effective_max_tokens);

    // Build engine config from settings and model config
    let config = EngineConfig {
        cwd: cwd.clone(),
        max_turns: if settings.max_turns > 0 {
            settings.max_turns
        } else {
            50
        },
        thinking_level: "high".to_string(),
        compaction_reserve_tokens: settings.compaction_reserve_tokens(),
        compaction_keep_recent_tokens: settings.compaction_keep_recent_tokens(),
        ..EngineConfig::with_defaults()
    };

    // Build engine
    let engine = if let Some(model) = model_config.as_ref() {
        let target = crate::llm::schema::ResolvedModelTarget::from_model(
            model,
            api_key.clone(),
            None,
            max_tokens,
        )?;
        Engine::new_with_target(target, config)?
    } else {
        Engine::new(&base_url, &api_key, &engine_model, config, None, max_tokens)?
    };
    let mut engine = engine.with_tools(crate::coding_tools());

    // Always run gRPC server mode
    let (grpc_host, grpc_port) = if cli.grpc_addr.starts_with(':') {
        let port_str = &cli.grpc_addr[1..];
        ("127.0.0.1", port_str.parse().unwrap_or(50051))
    } else if cli.grpc_addr.contains(':') {
        let parts: Vec<&str> = cli.grpc_addr.split(':').collect();
        let host = parts.first().copied().unwrap_or("127.0.0.1");
        let port: u16 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(50051);
        (host, port)
    } else {
        match cli.grpc_addr.parse::<u16>() {
            Ok(port) => ("127.0.0.1", port),
            Err(_) => ("127.0.0.1", 50051),
        }
    };
    // Discover skills (global user-level dirs only — project/cwd-relative
    // skill dirs are intentionally not scanned).
    let skill_dirs = crate::global_skill_dirs();
    let skills = crate::discover_skills_cached(&skill_dirs);
    let skill_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();

    // Load project context
    let agent_content = load_project_context(&cwd);
    let context_lines: Vec<String> = if agent_content.is_empty() {
        vec![]
    } else {
        vec![agent_content.clone()]
    };

    // Build system prompt
    let today = Local::now().format("%Y-%m-%d").to_string();
    let system_prompt = crate::prompt::build_prompt(&crate::prompt::PromptOptions {
        working_directory: cwd.clone(),
        date: today,
        tools: engine.tools.clone(),
        skills: skills.clone(),
        agent_content,
        ..Default::default()
    });
    engine.agent_loop.verbose = cli.verbose;
    engine.agent_loop.system_prompt = system_prompt.clone();
    engine.agent_loop.config.system_prompt = system_prompt;

    let manager = Arc::new(Manager::default_for(&cwd));
    match manager.gc_orphan_run_data() {
        Ok(count) if count > 0 => {
            tracing::info!(count, "reclaimed orphan Agent run-data directories")
        }
        Ok(_) => {}
        Err(error) => tracing::warn!("failed to reclaim orphan Agent run data: {error:#}"),
    }
    let approval_gate = crate::rpc::ApprovalGate::default();
    // Template for minting per-session agent loops.  Sessions no longer
    // share one global loop — each hydrated/created session gets an
    // independent copy so concurrent runs, model switches and aborts stay
    // session-local.  The template itself never runs prompts.
    // Set the model_registry on the template so all session loops inherit
    // the cached registry via independent_copy() — avoids ~15% CPU overhead
    // from re-deserialising the model catalog on every prompt.
    let mut template_loop = engine.agent_loop.independent_copy();
    template_loop.model_registry = Some(model_registry.clone());
    let loop_template = Arc::new(template_loop);

    // The agent starts with ZERO sessions.  There is no privileged
    // "default"/"current" session — clients (TUI, GUI, CLI, channels)
    // create or switch to sessions explicitly, and the agent hydrates
    // them on demand.  Settings that used to be applied to the startup
    // default session are applied per-session in cmd_new_session.
    let app_state = crate::rpc::AppState {
        agent_instance_id: format!("agent_{}", uuid::Uuid::new_v4().simple()),
        sessions: Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new())),
        queue_budget: Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
        session_manager: manager,
        welcome_version: crate::utils::VERSION.to_string(),
        welcome_cwd: cwd.clone(),
        welcome_skills: Arc::new(parking_lot::RwLock::new(skill_names.clone())),
        welcome_context: Arc::new(parking_lot::RwLock::new(context_lines)),
        welcome_exts: vec![],
        explicit_session: false,
        approval_gate,
        verbose: cli.verbose,
        shutting_down: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        model_registry: model_registry.clone(),
        loop_template,
    };

    // Ctrl+C: set the shutting_down flag so new prompts are rejected, then
    // abort in-flight streams and exit immediately.
    let shutting_down = app_state.shutting_down.clone();
    let sessions = app_state.sessions.clone();

    let server = crate::grpc::serve(app_state, grpc_host, grpc_port);

    // If --profile-seconds is set, spawn a task that signals shutdown after N
    // seconds via a oneshot so the flamegraph gets written by run().
    // When --profile-seconds is not set, use pending() so the select branch
    // never fires.
    let profile_rx: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
        if let Some(secs) = cli.profile_seconds {
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            let shutting_down = shutting_down.clone();
            let sessions = sessions.clone();
            tokio::spawn(async move {
                tracing::info!(
                    "Profile timer: agent will auto-shutdown after {} seconds",
                    secs
                );
                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                tracing::info!("Profile timer expired — shutting down for flamegraph capture");
                shutting_down.store(true, std::sync::atomic::Ordering::SeqCst);
                abort_all_sessions(&sessions);
                let _ = tx.send(());
            });
            Box::pin(async move {
                let _ = rx.await;
                tracing::info!("Profile timer completed — draining active streams");
            })
        } else {
            Box::pin(std::future::pending())
        };

    tokio::select! {
        result = server => result?,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("SIGINT received — aborting active streams, exiting immediately");
            shutting_down.store(true, std::sync::atomic::Ordering::SeqCst);

            // Interrupt in-flight runs so the process exits promptly instead
            // of waiting for a long LLM stream to finish on its own.
            abort_all_sessions(&sessions);
        }
        _ = profile_rx => {
            // profile timer handled inside the future
        }
    }
    Ok(())
}

/// A graceful standalone-agent exit has already stopped accepting work and
/// settled/aborted streams. Revoke the now-unused persistent Windows ACEs.
/// Failure is best-effort: metadata is retained so startup GC, Settings reset,
/// or uninstall can retry after a crash, active lease, or transient I/O error.
fn cleanup_windows_sandbox_on_exit() {
    #[cfg(target_os = "windows")]
    match crate::sandbox::reset_windows_sandbox_capabilities() {
        Ok(removed) => tracing::info!(removed, "Cleaned Windows sandbox permissions on exit"),
        Err(error) => {
            tracing::warn!(%error, "Could not clean Windows sandbox permissions on exit")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_project_context_skips_unreadable_candidates() {
        // CLAUDE.md exists but is a DIRECTORY (read fails) → the scan falls
        // through to AGENTS.md.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("CLAUDE.md")).unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "agent notes").unwrap();
        let content = load_project_context(&dir.path().to_string_lossy());
        assert_eq!(content, "agent notes");
    }

    #[test]
    fn load_project_context_empty_when_nothing_readable() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_project_context(&dir.path().to_string_lossy()).is_empty());
    }

    #[test]
    fn agent_instance_lock_is_exclusive_and_reusable_after_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-instance.lock");
        let first = acquire_agent_instance_lock_at(&path).expect("first agent owns lock");
        let error = acquire_agent_instance_lock_at(&path).expect_err("second agent is rejected");
        assert!(error.to_string().contains("already running"));
        drop(first);
        let _next = acquire_agent_instance_lock_at(&path).expect("lock released on exit");
    }

    #[test]
    fn agent_lifecycle_cleans_on_success_error_and_panic() {
        let success = std::cell::RefCell::new(Vec::new());
        let result: Result<i32> = run_agent_lifecycle(
            || success.borrow_mut().push("startup"),
            || {
                success.borrow_mut().push("run");
                Ok(7)
            },
            || success.borrow_mut().push("exit"),
        );
        assert_eq!(result.unwrap(), 7);
        assert_eq!(*success.borrow(), ["startup", "run", "exit"]);

        let error = std::cell::RefCell::new(Vec::new());
        let result: Result<()> = run_agent_lifecycle(
            || error.borrow_mut().push("startup"),
            || {
                error.borrow_mut().push("run");
                anyhow::bail!("server failed")
            },
            || error.borrow_mut().push("exit"),
        );
        assert_eq!(result.unwrap_err().to_string(), "server failed");
        assert_eq!(*error.borrow(), ["startup", "run", "exit"]);

        let panic_steps = std::cell::RefCell::new(Vec::new());
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = run_agent_lifecycle::<()>(
                || panic_steps.borrow_mut().push("startup"),
                || {
                    panic_steps.borrow_mut().push("run");
                    panic!("server panic")
                },
                || panic_steps.borrow_mut().push("exit"),
            );
        }));
        assert!(panic.is_err());
        assert_eq!(*panic_steps.borrow(), ["startup", "run", "exit"]);
    }
}
