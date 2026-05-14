//! xihu — Rust agent backend (CLI entry point)

use anyhow::Result;
use clap::Parser;
use std::env;
use std::sync::Arc;
use xihu_agent::{
    Engine, EngineConfig, LLMClient, LLMProvider, Manager, ModelRegistry, Server, ServerSession,
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

    /// Run as RPC server (HTTP mode)
    #[arg(long)]
    server: bool,

    /// TCP port for server mode
    #[arg(long)]
    port: Option<String>,

    /// Unix socket path for server mode
    #[arg(long)]
    socket: Option<String>,

    /// gRPC port for server mode (default: disabled)
    #[arg(long)]
    grpc_port: Option<u16>,

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

    let api_key = cli
        .api_key
        .or_else(|| env::var("LLM_API_KEY").ok())
        .or_else(|| env::var("ANTHROPIC_API_KEY").ok())
        .or_else(|| env::var("OPENAI_API_KEY").ok())
        .or_else(|| {
            model_config.as_ref().map(|m| m.api_key.clone())
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

    if cli.verbose {
        eprintln!("\x1b[33m[model]\x1b[0m {}", resolved_model);
    }

    // Server mode
    if cli.server || cli.port.is_some() || cli.socket.is_some() {
        // Discover skills (matching Go's paths)
        let skill_dirs = vec![
            USER_SKILLS_DIR.to_string(),
            format!("{}/{}", cwd, PROJECT_SKILLS_DIR),
            AGENTS_SKILLS_DIR.to_string(),
        ];
        let skills = xihu_agent::discover_skills(&skill_dirs).unwrap_or_default();
        let skill_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();

        let manager = Arc::new(Manager::default_for(&cwd));
        let mut server_session = ServerSession::new(engine.agent_loop, manager, &cwd);
        server_session.model = resolved_model.clone();
        let session = Arc::new(std::sync::RwLock::new(server_session));
        
        if let Some(ref grpc_port) = cli.grpc_port {
            // Combined HTTP + gRPC mode (doesn't use Server HTTP interface)
            let http_port = cli.port
                .as_ref()
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(8080);
            
            // Build AppState directly
            let broadcaster = xihu_agent::rpc::SseBroadcaster::new();
            let app_state = xihu_agent::rpc::AppState {
                session: session.clone(),
                welcome_version: xihu_agent::utils::VERSION.to_string(),
                welcome_cwd: cwd.clone(),
                welcome_skills: skill_names.clone(),
                welcome_context: vec![],
                welcome_exts: vec![],
                explicit_session: false,
                broadcaster,
            };
            
            xihu_agent::grpc::serve_combined(app_state, http_port, *grpc_port).await?;
            return Ok(());
        }
        
        // HTTP server mode
        let mut server = Server::new(session);
        server.set_welcome(
            xihu_agent::utils::VERSION,
            &cwd,
            skill_names,
            vec![],      // context files
            vec![],      // extensions
        );

        if let Some(ref port) = cli.port {
            server.run_tcp(port).await?;
        } else if let Some(ref path) = cli.socket {
            server.run_unix(path).await?;
        } else {
            // Default: run with stdio
            server.run_stdio()?;
        }
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
