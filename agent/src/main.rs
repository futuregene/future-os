//! future-agent — Rust agent backend (gRPC server entry point)

use anyhow::Result;
use chrono::Local;
use clap::Parser;
use future_agent::{
    Engine, EngineConfig, Manager, ModelRegistry, ServerSession, AGENTS_SKILLS_DIR, APP_SKILLS_DIR,
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
}

fn main() -> Result<()> {
    // Initialise tracing with timestamps before anything else.
    tracing_subscriber::fmt()
        .with_timer(tracing_subscriber::fmt::time::SystemTime::default())
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Build model registry BEFORE tokio runtime starts.
    // Registry::new() uses reqwest::blocking::Client internally,
    // which creates a nested runtime that cannot be dropped in async context.
    let model_registry = ModelRegistry::new();

    // Launch async portion
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main(model_registry))
}

async fn async_main(model_registry: ModelRegistry) -> Result<()> {
    let cli = Cli::parse();

    let cwd = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .to_string_lossy()
        .to_string();

    let all_models = model_registry.all_models();

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
    let resolved_model = future_agent::models::get_default_model()
        .or_else(|| {
            all_models
                .iter()
                .find(|m| !m.api_key.is_empty() || auth_store.get(&m.provider).is_some())
                .map(|m| m.id.clone())
        })
        .unwrap_or_default();
    if resolved_model.is_empty() {
        tracing::warn!("future-agent: no model configured yet — starting the gRPC server \
             anyway so a client can log in and pick a model. Add an API key via \
             'future auth login' or the desktop app, or configure a provider in \
             ~/.future/agent/models.json."
        );
    }

    // Resolve model config
    let model_config = model_registry.resolve(&resolved_model);

    let engine_model = model_config
        .as_ref()
        .map(|m| m.id.clone())
        .unwrap_or_else(|| resolved_model.clone());

    let base_url = model_config
        .as_ref()
        .map(|m| m.base_url.clone())
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

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
    let broadcaster: Arc<future_agent::rpc::SseBroadcaster> =
        Arc::new(future_agent::rpc::SseBroadcaster::new());
    let approval_gate = future_agent::rpc::ApprovalGate::default();
    let mut server_session = ServerSession::new(
        future_agent::utils::generate_id(),
        Arc::new(tokio::sync::RwLock::new(engine.agent_loop)),
        manager,
        &cwd,
        event_bus.clone(),
        broadcaster.clone(),
        approval_gate.clone(),
    );
    server_session.model = resolved_model.clone();
    *server_session.compaction_model.write().unwrap() = resolved_model.clone();

    server_session.set_steering_mode(&settings.steering_mode);
    server_session.set_follow_up_mode(&settings.follow_up_mode);
    if !settings.default_permission_level.is_empty() {
        server_session.set_permission_level(&settings.default_permission_level);
    }
    server_session.set_auto_compaction(settings.compaction_enabled());
    server_session.set_auto_retry(settings.retry_enabled());

    let session = Arc::new(std::sync::RwLock::new(server_session));

    let app_state = future_agent::rpc::AppState {
        session: session.clone(),
        sessions: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        active_session_id: Arc::new(std::sync::RwLock::new(String::new())),
        welcome_version: future_agent::utils::VERSION.to_string(),
        welcome_cwd: cwd.clone(),
        welcome_skills: Arc::new(std::sync::RwLock::new(skill_names.clone())),
        welcome_context: Arc::new(std::sync::RwLock::new(context_lines)),
        welcome_exts: vec![],
        explicit_session: false,
        broadcaster: broadcaster.clone(),
        event_bus: event_bus.clone(),
        approval_gate,
        verbose: cli.verbose,
    };

    future_agent::grpc::serve(app_state, grpc_host, grpc_port).await?;
    Ok(())
}
