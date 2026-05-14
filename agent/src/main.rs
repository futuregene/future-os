//! xihu — Rust agent backend (CLI entry point)

use anyhow::Result;
use clap::Parser;
use std::env;
use std::sync::Arc;
use xihu_agent::{
    Engine, EngineConfig, Manager, ModelRegistry, ServerSession,
    USER_SKILLS_DIR, PROJECT_SKILLS_DIR, AGENTS_SKILLS_DIR,
};

#[derive(Parser)]
#[command(name = "xihu")]
#[command(version = xihu_agent::utils::VERSION)]
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
        env::set_var("XIHU_OFFLINE", "1");
        env::set_var("XIHU_SKIP_VERSION_CHECK", "1");
    }

    // Determine workspace - prefer ~/.openclaw/workspace if it exists
    let cwd = if let Some(home) = dirs::home_dir() {
        let workspace = home.join(".openclaw/workspace");
        if workspace.exists() {
            workspace.to_string_lossy().to_string()
        } else {
            env::current_dir()
                .unwrap_or_else(|_| home.join("xihu/agent"))
                .to_string_lossy()
                .to_string()
        }
    } else {
        env::current_dir()?
            .to_string_lossy()
            .to_string()
    };

    // Initialize model registry
    let model_registry = ModelRegistry::new();
    let all_models = model_registry.all_models();
    
    // Resolve model: CLI arg > env > settings.json default > user models.json > builtin default
    let resolved_model = cli
        .model
        .or_else(|| env::var("LLM_MODEL").ok())
        .or_else(|| xihu_agent::models::get_default_model())
        .or_else(|| {
            // Try to find a user-configured model from models.json
            all_models.iter().find(|m| {
                m.api_key.is_empty() == false
            }).map(|m| m.id.clone())
        })
        .unwrap_or_else(|| "gpt-4o".to_string());

    // Resolve model config
    let model_config = model_registry.resolve(&resolved_model);
    
    let base_url = cli
        .base_url
        .or_else(|| env::var("LLM_BASE_URL").ok())
        .or_else(|| {
            model_config.as_ref().map(|m| m.base_url.clone())
        })
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

    // Load auth store for API keys
    let auth_store = xihu_agent::AuthStore::load();
    
    // Resolve API key: CLI arg > env > auth.json > model config
    let api_key = cli
        .api_key
        .or_else(|| {
            let k = env::var("LLM_API_KEY").unwrap_or_default();
            if k.is_empty() { None } else { Some(k) }
        })
        .or_else(|| {
            let k = env::var("ANTHROPIC_API_KEY").unwrap_or_default();
            if k.is_empty() { None } else { Some(k) }
        })
        .or_else(|| {
            let k = env::var("OPENAI_API_KEY").unwrap_or_default();
            if k.is_empty() { None } else { Some(k) }
        })
        .or_else(|| auth_store.get(&resolved_model))
        .or_else(|| model_config.as_ref().and_then(|m| {
            auth_store.get(&m.provider)
        }))
        .or_else(|| auth_store.default_key())
        .or_else(|| {
            model_config.as_ref().and_then(|m| {
                if m.api_key.is_empty() { None } else { Some(m.api_key.clone()) }
            })
        })
        .unwrap_or_default();

    let thinking = cli.thinking.or_else(|| env::var("LLM_THINKING").ok());

    // Build engine config
    let config = EngineConfig {
        cwd: cwd.clone(),
        max_turns: 50,
        thinking_level: thinking.unwrap_or_default(),
        ..Default::default()
    };

    // Build engine
    let engine = Engine::new(&base_url, &api_key, &resolved_model, config)?
        .with_tools(xihu_agent::all_tools());
    
    // Create event bus for server mode
    let event_bus = Arc::new(xihu_agent::EventBus::new());

    if cli.verbose {
        eprintln!("\x1b[33m[model]\x1b[0m {}", resolved_model);
    }

    // Server mode (gRPC only)
    if true {  // Always run in server mode when invoked
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
        let skills = xihu_agent::discover_skills(&skill_dirs).unwrap_or_default();
        let skill_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();

        let manager = Arc::new(Manager::default_for(&cwd));
        let broadcaster: Arc<xihu_agent::rpc::SseBroadcaster> = Arc::new(xihu_agent::rpc::SseBroadcaster::new());
        let mut server_session = ServerSession::new(
            xihu_agent::utils::generate_id(),
            Arc::new(tokio::sync::RwLock::new(engine.agent_loop)),
            manager,
            &cwd,
            event_bus.clone(),
            broadcaster.clone(),
        );
        server_session.model = resolved_model.clone();
        let session = Arc::new(std::sync::RwLock::new(server_session));
        
        // Build AppState for gRPC server
        let app_state = xihu_agent::rpc::AppState {
            session: session.clone(),
            sessions: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            active_session_id: Arc::new(std::sync::RwLock::new(String::new())),
            welcome_version: xihu_agent::utils::VERSION.to_string(),
            welcome_cwd: cwd.clone(),
            welcome_skills: skill_names.clone(),
            welcome_context: vec![],
            welcome_exts: vec![],
            explicit_session: false,
            broadcaster: broadcaster.clone(),
            event_bus: event_bus.clone(),
        };
        
        // Run gRPC server (no HTTP)
        xihu_agent::grpc::serve(app_state, grpc_host, grpc_port).await?;
        return Ok(());
    }

    // Non-server mode: run prompt
    if cli.messages.is_empty() {
        eprintln!("Usage: xihu [options] [messages...]");
        std::process::exit(1);
    }

    let msg = cli.messages.join(" ");

    // Build initial message
    let messages = vec![xihu_agent::AgentMessage::new_user(
        "user",
        serde_json::json!([{"type": "text", "text": msg}]),
    )];

    // Run agent
    let agent_loop = engine.agent_loop;

    let (_result, _final_messages) = agent_loop
        .run_streaming_with_messages(messages, |text| print!("{}", text), |_event| {})
        .await?;

    println!();

    Ok(())
}
