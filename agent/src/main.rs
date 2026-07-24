//! future-agent — Rust agent backend (gRPC server entry point)

use anyhow::Result;
use chrono::Local;
use clap::Parser;
use future_agent::{
    Engine, EngineConfig, Manager, ModelRegistry, AGENTS_SKILLS_DIR, APP_SKILLS_DIR,
    PROJECT_SKILLS_DIR,
};
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "future-agent")]
#[command(version = future_agent::utils::VERSION)]
struct Cli {
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
        default_value_t = future_agent::logfile::DEFAULT_MAX_LINES
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Resolve profile path early (before the runtime starts).
    let profile_path: Option<std::path::PathBuf> = cli
        .profile
        .as_deref()
        .or_else(|| {
            if cli.profile_seconds.is_some() {
                Some("agent-profile.svg")
            } else {
                None
            }
        })
        .map(std::path::PathBuf::from);

    // Start CPU profiling if requested.  The guard lives in main() so it
    // covers the entire agent lifetime including gRPC startup/shutdown.
    // ProfilerGuard is !Send so we keep it right here.
    #[cfg(not(windows))]
    let profiler_guard = match &profile_path {
        Some(_path) => {
            tracing::info!(
                "CPU profiling enabled → will write flamegraph to {}",
                _path.display()
            );
            match pprof::ProfilerGuardBuilder::default()
                .frequency(997) // prime to avoid lock-step with timers
                .blocklist(&["libc", "libgcc", "pthread", "vdso"])
                .build()
            {
                Ok(g) => Some(g),
                Err(e) => {
                    tracing::warn!(
                        "Failed to start profiler: {} — continuing without profiling",
                        e
                    );
                    None
                }
            }
        }
        None => None,
    };
    #[cfg(windows)]
    let profiler_guard: Option<()> = {
        if profile_path.is_some() {
            tracing::warn!("CPU profiling is not supported on Windows — ignoring --profile flag");
        }
        None
    };

    // Load the user's login-shell PATH/env BEFORE spawning any threads or the
    // tokio runtime — set_var is only sound while single-threaded. Fixes
    // "command not found" for user-installed tools (nvm/Homebrew/npm-global)
    // when the agent is launched from a GUI with a minimal inherited PATH.
    future_agent::sandbox::hydrate_from_login_shell();

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
            future_agent::utils::default_config_dir().join("logs/agent.log")
        } else {
            std::path::PathBuf::from(p)
        }
    });

    let file_layer = match &log_file {
        Some(path) => {
            let mirror = future_agent::logfile::init(path, cli.log_max_lines)?;
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
    // 1 MB thread stack is sufficient for async I/O (was 4 MB).
    // On a 32-core machine this saves 96 MB virtual memory.
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(1 * 1024 * 1024)
        .build()?
        .block_on(async_main(model_registry, cli, profile_path.clone()))
        .inspect_err(|e| tracing::error!("Agent exited with error: {e}"))
        .ok();

    // Write profiling flamegraph on shutdown (after the runtime drops,
    // so all async tasks have settled).  ProfilerGuard stops sampling on
    // drop, so we must build the report BEFORE dropping the guard.
    #[cfg(not(windows))]
    if let (Some(guard), Some(path)) = (profiler_guard, profile_path) {
        tracing::info!("Writing CPU profile flamegraph to {}", path.display());
        match guard.report().build() {
            Ok(report) => {
                let file = std::fs::File::create(&path)
                    .map_err(|e| {
                        tracing::error!("Cannot create profile file {}: {}", path.display(), e);
                        e
                    })
                    .ok();
                if let Some(f) = file {
                    if let Err(e) = report.flamegraph(f) {
                        tracing::error!("Failed to write flamegraph: {}", e);
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
                tracing::error!("Failed to build profiling report: {}", e);
            }
        }
    }
    #[cfg(windows)]
    let _ = (profiler_guard, profile_path);

    Ok(())
}

async fn async_main(
    model_registry: Arc<parking_lot::RwLock<ModelRegistry>>,
    cli: Cli,
    _profile_path: Option<std::path::PathBuf>,
) -> Result<()> {
    let cwd = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .to_string_lossy()
        .to_string();

    let all_models = model_registry.read().all_models();

    // Load settings
    let settings_path = std::path::PathBuf::from(future_agent::models::settings_path());
    let settings = match future_agent::config::load_settings(&settings_path) {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(
                "Failed to load settings from {}: {}. Falling back to defaults.",
                settings_path.display(),
                error
            );
            future_agent::Settings::default()
        }
    };

    // Load auth store
    let auth_store = future_agent::AuthStore::load();

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
    // see gui/src-tauri/src/agent_supervisor.rs), and the app would look broken
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
            .or_else(future_agent::models::get_default_model)
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
        .and_then(|m| {
            if m.base_url.is_empty() {
                None
            } else {
                Some(m.base_url.clone())
            }
        })
        .or_else(|| auth_store.base_url(&resolved_model))
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
    let api_key = auth_store
        .get(&resolved_model)
        .or_else(|| {
            model_config
                .as_ref()
                .and_then(|m| auth_store.get(&m.provider))
        })
        .or_else(|| {
            model_config.as_ref().and_then(|m| {
                if m.api_key.is_empty() {
                    None
                } else {
                    Some(m.api_key.clone())
                }
            })
        })
        .or_else(|| auth_store.default_key())
        .unwrap_or_default();

    // Default thinking level (clients override per-session).

    // Parse thinking level map from model config
    let thinking_level_map: std::collections::HashMap<String, String> = model_config
        .as_ref()
        .map(|m| {
            m.thinking_level_map
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    // Cap max_tokens at 32000. For reasoning models without an explicit
    // max_tokens, default to 32000 so thinking has room to breathe without
    // starving the visible output.
    let max_tokens = model_config.as_ref().map(|m| {
        if m.max_tokens > 0 {
            std::cmp::min(m.max_tokens, 32000)
        } else if m.reasoning {
            32000
        } else {
            16384
        }
    });

    // Build engine config from settings and model config
    let config = EngineConfig {
        cwd: cwd.clone(),
        max_turns: if settings.max_turns > 0 {
            settings.max_turns
        } else {
            50
        },
        thinking_level: "high".to_string(),
        thinking_level_map,
        compat_thinking_format: model_config
            .as_ref()
            .and_then(|m| m.compat.get("thinkingFormat"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        compat_supports_reasoning_effort: model_config
            .as_ref()
            .and_then(|m| m.compat.get("supportsReasoningEffort"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        compat_requires_reasoning_on_assistant: model_config
            .as_ref()
            .and_then(|m| m.compat.get("requiresReasoningContentOnAssistantMessages"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        max_tokens_field: model_config
            .as_ref()
            .and_then(|m| m.compat.get("maxTokensField"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default(),
        compaction_reserve_tokens: settings.compaction_reserve_tokens(),
        compaction_keep_recent_tokens: settings.compaction_keep_recent_tokens(),
        ..EngineConfig::with_defaults()
    };

    // Build engine
    let mut engine = Engine::new(&base_url, &api_key, &engine_model, config, None, max_tokens)?
        .with_tools(future_agent::coding_tools());

    let event_bus = Arc::new(future_agent::EventBus::new());

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
    // Discover skills
    let skill_dirs = vec![
        APP_SKILLS_DIR.to_string(),
        format!("{}/{}", cwd, PROJECT_SKILLS_DIR),
        AGENTS_SKILLS_DIR.to_string(),
    ];
    let skills = future_agent::discover_skills(&skill_dirs).unwrap_or_default();
    let skill_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();

    // Load project context
    let mut agent_content = String::new();
    for fname in &["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
        let p = std::path::Path::new(&cwd).join(fname);
        if p.exists() {
            if let Ok(content) = std::fs::read_to_string(&p) {
                agent_content = content;
                break;
            }
        }
    }
    let context_lines: Vec<String> = if agent_content.is_empty() {
        vec![]
    } else {
        vec![agent_content.clone()]
    };

    // Build system prompt
    let today = Local::now().format("%Y-%m-%d").to_string();
    let system_prompt = future_agent::prompt::build_prompt(&future_agent::prompt::PromptOptions {
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
    let approval_gate = future_agent::rpc::ApprovalGate::default();
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
    let app_state = future_agent::rpc::AppState {
        sessions: Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new())),
        session_manager: manager,
        welcome_version: future_agent::utils::VERSION.to_string(),
        welcome_cwd: cwd.clone(),
        welcome_skills: Arc::new(parking_lot::RwLock::new(skill_names.clone())),
        welcome_context: Arc::new(parking_lot::RwLock::new(context_lines)),
        welcome_exts: vec![],
        explicit_session: false,
        event_bus: event_bus.clone(),
        approval_gate,
        verbose: cli.verbose,
        shutting_down: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        model_registry: model_registry.clone(),
        loop_template,
    };

    // Graceful shutdown on Ctrl+C: set the shutting_down flag so new prompts
    // are rejected, then wait up to 30 s for active streams to finish.
    let shutting_down = app_state.shutting_down.clone();
    let sessions = app_state.sessions.clone();
    let shutdown_timeout = std::time::Duration::from_secs(30);

    let server = future_agent::grpc::serve(app_state, grpc_host, grpc_port);

    // If --profile-seconds is set, spawn a task that signals shutdown after N
    // seconds via a oneshot so the flamegraph gets written by main().
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
                for s in sessions.read().values() {
                    s.read().abort();
                }
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
            tracing::info!("SIGINT received — aborting active streams, shutting down (max 30s)");
            shutting_down.store(true, std::sync::atomic::Ordering::SeqCst);

            // Actively interrupt in-flight runs. Without this, a long LLM
            // stream keeps running and the wait loop below only exits via the
            // 30 s timeout — making Ctrl-C look like a hang.
            for s in sessions.read().values() {
                s.read().abort();
            }

            // Wait for active streams to settle
            let deadline = tokio::time::Instant::now() + shutdown_timeout;
            loop {
                let any_streaming = sessions
                    .read()
                    .values()
                    .any(|s| s.read()
                        .is_streaming
                        .load(std::sync::atomic::Ordering::Relaxed));
                if !any_streaming {
                    tracing::info!("All streams finished — exiting");
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    tracing::warn!("Shutdown timeout (30s) — forcing exit with {} active stream(s)",
                        sessions.read().values()
                            .filter(|s| s.read().is_streaming.load(std::sync::atomic::Ordering::Relaxed))
                            .count());
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
        _ = profile_rx => {
            // profile timer handled inside the future
        }
    }
    Ok(())
}
