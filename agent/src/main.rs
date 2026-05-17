//! future-agent — Rust agent backend (CLI entry point)

use anyhow::Result;
use chrono::Local;
use clap::Parser;
use future_agent::{
    Engine, EngineConfig, Manager, ModelRegistry, ServerSession, AGENTS_SKILLS_DIR,
    PROJECT_SKILLS_DIR, USER_SKILLS_DIR,
};
use std::env;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "future-agent")]
#[command(version = future_agent::utils::VERSION)]
struct Cli {
    /// Base URL for LLM API
    #[arg(long, env = "LLM_BASE_URL")]
    base_url: Option<String>,

    /// API key
    #[arg(long, env = "LLM_API_KEY")]
    api_key: Option<String>,

    /// Model name
    #[arg(short, long, env = "LLM_MODEL")]
    model: Option<String>,

    /// Thinking level
    #[arg(long)]
    thinking: Option<String>,

    /// Session directory
    #[arg(long)]
    session_dir: Option<String>,

    /// Offline mode
    #[arg(long)]
    offline: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// gRPC server address (host:port, e.g., 127.0.0.1:50051)
    #[arg(long, default_value = "127.0.0.1:50051")]
    grpc_addr: String,

    /// Message arguments
    messages: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.offline {
        env::set_var("FUTURE_OFFLINE", "1");
        env::set_var("FUTURE_SKIP_VERSION_CHECK", "1");
    }

    // Always use home directory — the agent should not depend on its launch directory
    let cwd = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .to_string_lossy()
        .to_string();

    // Initialize model registry
    let model_registry = ModelRegistry::new();
    let all_models = model_registry.all_models();

    // Resolve model: CLI arg > env > settings.json default > user models.json > builtin default
    let model_from_user = cli.model.is_some() || env::var("LLM_MODEL").ok().is_some();
    let mut resolved_model = cli
        .model
        .or_else(|| env::var("LLM_MODEL").ok())
        .or_else(future_agent::models::get_default_model)
        .or_else(|| {
            // Try to find a user-configured model from models.json
            all_models
                .iter()
                .find(|m| !m.api_key.is_empty())
                .map(|m| m.id.clone())
        })
        .unwrap_or_else(|| "gpt-4o".to_string());

    // Resolve model config
    let model_config = model_registry.resolve(&resolved_model);

    // Use canonical model ID for API calls (strip provider prefix)
    let engine_model = model_config
        .as_ref()
        .map(|m| m.id.clone())
        .unwrap_or_else(|| resolved_model.clone());

    let base_url = cli
        .base_url
        .or_else(|| env::var("LLM_BASE_URL").ok())
        .or_else(|| model_config.as_ref().map(|m| m.base_url.clone()))
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

    // Load auth store for API keys
    let auth_store = future_agent::AuthStore::load();

    // Resolve API key: CLI arg > env > auth.json > model config
    let api_key = cli
        .api_key
        .or_else(|| {
            let k = env::var("LLM_API_KEY").unwrap_or_default();
            if k.is_empty() {
                None
            } else {
                Some(k)
            }
        })
        .or_else(|| {
            let k = env::var("ANTHROPIC_API_KEY").unwrap_or_default();
            if k.is_empty() {
                None
            } else {
                Some(k)
            }
        })
        .or_else(|| {
            let k = env::var("OPENAI_API_KEY").unwrap_or_default();
            if k.is_empty() {
                None
            } else {
                Some(k)
            }
        })
        .or_else(|| auth_store.get(&resolved_model))
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

    // Load settings for defaults
    let settings_path = std::path::PathBuf::from(future_agent::models::settings_path());
    let settings = future_agent::config::load_settings(&settings_path).unwrap_or_default();

    // If enabled_models is set and model not from CLI, pick from scope
    if !settings.enabled_models.is_empty() && !model_from_user {
        let scoped = model_registry.resolve_scope(&settings.enabled_models, &auth_store);
        if !scoped.is_empty() {
            let default_model = settings.default_model.clone();
            let saved_in_scope = if !default_model.is_empty() {
                scoped.iter().find(|m| *m == &default_model)
            } else {
                None
            };
            if let Some(m) = saved_in_scope {
                resolved_model = m.clone();
            } else {
                // Pick first scoped model
                resolved_model = scoped[0].clone();
            }
        }
    }

    // Resolve thinking level: CLI arg > env > settings.json default
    let thinking = cli
        .thinking
        .or_else(|| env::var("LLM_THINKING").ok())
        .or_else(|| {
            if settings.default_thinking_level.is_empty() {
                None
            } else {
                Some(settings.default_thinking_level.clone())
            }
        });

    // Parse thinking level map from model config (e.g. deepseek: {high: "high"})
    let thinking_level_map: std::collections::HashMap<String, String> = model_config
        .as_ref()
        .map(|m| {
            m.thinking_level_map
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    // Cap max_tokens at 32000 (matching pi-mono)
    let max_tokens = model_config.as_ref().and_then(|m| {
        if m.max_tokens > 0 {
            Some(std::cmp::min(m.max_tokens, 32000))
        } else {
            None
        }
    });

    // Build engine config — apply settings defaults where CLI/env didn't override
    let config = EngineConfig {
        cwd: cwd.clone(),
        max_turns: if settings.max_turns > 0 {
            settings.max_turns
        } else {
            50
        },
        thinking_level: thinking.clone().unwrap_or_default(),
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
            .or_else(|| {
                // Fallback: reasoning models without thinkingFormat need
                // max_completion_tokens (Azure GPT-5/o1/o3 etc. don't set maxTokensField yet)
                let m = model_config.as_ref()?;
                if m.reasoning
                    && !m.compat.contains_key("thinkingFormat")
                {
                    Some("max_completion_tokens".to_string())
                } else {
                    None
                }
            })
            .unwrap_or_default(),
        compaction_reserve_tokens: settings.compaction_reserve_tokens(),
        compaction_keep_recent_tokens: settings.compaction_keep_recent_tokens(),
        ..EngineConfig::with_defaults()
    };

    // Build engine
    let mut engine = Engine::new(
        &base_url,
        &api_key,
        &engine_model,
        config,
        None, // temperature: use model default
        max_tokens,
    )?
    .with_tools(future_agent::coding_tools());

    // Create event bus for server mode
    let event_bus = Arc::new(future_agent::EventBus::new());

    if cli.verbose {
        eprintln!("\x1b[33m[model]\x1b[0m {}", resolved_model);
    }

    // Server mode (gRPC only)
    if cli.messages.is_empty() {
        // No messages → start server mode for TUI
        // Parse grpc_addr (host:port or just port)
        let (grpc_host, grpc_port) = if cli.grpc_addr.starts_with(':') {
            // Just port: :50051
            let port_str = &cli.grpc_addr[1..];
            ("127.0.0.1", port_str.parse().unwrap_or(50051))
        } else if cli.grpc_addr.contains(':') {
            // host:port format
            let parts: Vec<&str> = cli.grpc_addr.split(':').collect();
            let host = parts.first().copied().unwrap_or("127.0.0.1");
            let port: u16 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(50051);
            (host, port)
        } else {
            // Just port number
            match cli.grpc_addr.parse::<u16>() {
                Ok(port) => ("127.0.0.1", port),
                Err(_) => ("127.0.0.1", 50051),
            }
        };
        eprintln!("gRPC server listening on {}:{}", grpc_host, grpc_port);
        // Discover skills (matching Go's paths)
        let skill_dirs = vec![
            USER_SKILLS_DIR.to_string(),
            format!("{}/{}", cwd, PROJECT_SKILLS_DIR),
            AGENTS_SKILLS_DIR.to_string(),
        ];
        let skills = future_agent::discover_skills(&skill_dirs).unwrap_or_default();
        let skill_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();

        // Load project context (CLAUDE.md / AGENTS.md / GEMINI.md)
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

        // Build system prompt (skills + context + identity)
        let today = Local::now().format("%Y-%m-%d").to_string();
        let system_prompt =
            future_agent::prompt::build_prompt(&future_agent::prompt::PromptOptions {
                working_directory: cwd.clone(),
                date: today,
                tools: engine.tools.clone(),
                skills: skills.clone(),
                agent_content,
                ..Default::default()
            });
        engine.agent_loop.system_prompt = system_prompt.clone();
        engine.agent_loop.config.system_prompt = system_prompt;

        let manager = Arc::new(Manager::default_for(&cwd));
        let broadcaster: Arc<future_agent::rpc::SseBroadcaster> =
            Arc::new(future_agent::rpc::SseBroadcaster::new());
        let mut server_session = ServerSession::new(
            future_agent::utils::generate_id(),
            Arc::new(tokio::sync::RwLock::new(engine.agent_loop)),
            manager,
            &cwd,
            event_bus.clone(),
            broadcaster.clone(),
        );
        server_session.model = resolved_model.clone();

        // Apply thinking level from CLI/env/settings (server init defaults to "high")
        if let Some(ref level) = thinking {
            server_session.set_thinking_level(level);
        }

        // Apply settings to session (matching pi's SettingsManager getter defaults)
        server_session.set_steering_mode(&settings.steering_mode_or_default());
        server_session.set_follow_up_mode(&settings.follow_up_mode_or_default());
        server_session.set_auto_compaction(settings.compaction_enabled());
        server_session.set_auto_retry(settings.retry_enabled());

        let session = Arc::new(std::sync::RwLock::new(server_session));

        // Build AppState for gRPC server
        let app_state = future_agent::rpc::AppState {
            session: session.clone(),
            sessions: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            active_session_id: Arc::new(std::sync::RwLock::new(String::new())),
            welcome_version: future_agent::utils::VERSION.to_string(),
            welcome_cwd: cwd.clone(),
            welcome_skills: skill_names.clone(),
            welcome_context: context_lines,
            welcome_exts: vec![],
            explicit_session: false,
            broadcaster: broadcaster.clone(),
            event_bus: event_bus.clone(),
        };

        // Run gRPC server (no HTTP)
        future_agent::grpc::serve(app_state, grpc_host, grpc_port).await?;
        return Ok(());
    }

    // Non-server mode: run prompt
    if cli.messages.is_empty() {
        eprintln!("Usage: future-agent [options] [messages...]");
        std::process::exit(1);
    }

    let msg = cli.messages.join(" ");

    // Build initial message
    let messages = vec![future_agent::AgentMessage::new_user(
        "user",
        serde_json::json!([{"type": "text", "text": msg}]),
    )];

    // Run agent
    let agent_loop = engine.agent_loop;

    let (_result, _final_messages) = agent_loop
        .run_streaming_with_messages(messages, |text| print!("{}", text), |_event| {}, None)
        .await?;

    println!();

    Ok(())
}
