//! xihu — Rust agent backend (CLI entry point)

use anyhow::Result;
use clap::Parser;
use std::env;
use xihu_agent::{Engine, EngineConfig, LLMClient, LLMProvider, Manager, Session};

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

    let cwd = env::current_dir()?.to_string_lossy().to_string();

    let base_url = cli.base_url
        .or_else(|| env::var("LLM_BASE_URL").ok())
        .unwrap_or_else(|| "https://api.openai.com".to_string());

    let api_key = cli.api_key
        .or_else(|| env::var("LLM_API_KEY").ok())
        .or_else(|| env::var("ANTHROPIC_API_KEY").ok())
        .or_else(|| env::var("OPENAI_API_KEY").ok())
        .unwrap_or_default();

    let model = cli.model
        .or_else(|| env::var("LLM_MODEL").ok())
        .unwrap_or_else(|| "gpt-4o".to_string());

    let thinking = cli.thinking.or_else(|| env::var("LLM_THINKING").ok());

    // Build engine
    let config = EngineConfig {
        cwd: cwd.clone(),
        max_turns: 50,
        thinking_level: thinking.unwrap_or_default(),
        ..Default::default()
    };

    let provider: std::sync::Arc<dyn LLMProvider> = std::sync::Arc::new(LLMClient::new(&base_url, &api_key));
    let engine = Engine::new(&base_url, &api_key, &model, config)?
        .with_tools(xihu_agent::all_tools());

    if cli.verbose {
        eprintln!("\x1b[33m[model]\x1b[0m {}", model);
    }

    if cli.messages.is_empty() {
        eprintln!("Usage: xihu [options] [messages...]");
        std::process::exit(1);
    }

    let msg = cli.messages.join(" ");

    // Build initial message
    let messages = vec![xihu_agent::AgentMessage::new_user("user", serde_json::json!([{"type": "text", "text": msg}]))];

    // Run agent
    let agent_loop = engine.agent_loop;

    let (result, _final_messages) = agent_loop.run_streaming(messages, |text| {
        print!("{}", text);
    }, |_event| {}).await?;

    println!();

    Ok(())
}
