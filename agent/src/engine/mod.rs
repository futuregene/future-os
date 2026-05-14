//! Engine — 1:1 compatible with Go internal/engine/

use crate::agent::{Loop, DEFAULT_MAX_TURNS};
use crate::compaction;
use crate::config::Settings;
use crate::llm::Client as LLMClient;
use crate::session::Manager;
use crate::tools;
use crate::types::{LLMProvider, Message};
use anyhow::Result;
use std::sync::Arc;

pub struct Engine {
    pub provider: Arc<dyn LLMProvider>,
    pub model: String,
    pub api_key: String,
    pub config: EngineConfig,
    pub tools: Vec<crate::types::AgentTool>,
    pub session: crate::session::Session,
    pub session_manager: Arc<Manager>,
    pub settings: Arc<Settings>,
    pub agent_loop: Loop,
    pub verbose: bool,
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub cwd: String,
    pub system_prompt: String,
    pub max_turns: i32,
    pub thinking_level: String,
    pub no_tools: String,
    pub compaction_reserve_tokens: i32,
    pub compaction_keep_recent_tokens: i32,
    pub extension_paths: Vec<String>,
    pub no_extensions: bool,
}

impl Engine {
    pub fn new(
        base_url: &str,
        api_key: &str,
        model: &str,
        config: EngineConfig,
    ) -> Result<Self> {
        let client: Arc<dyn LLMProvider> = Arc::new(LLMClient::new(base_url, api_key));
        let cwd = config.cwd.clone();
        let _max_turns = config.max_turns;
        let agent_loop = Loop::new(client.clone(), model);

        let mut engine = Self {
            provider: client,
            model: model.to_string(),
            api_key: api_key.to_string(),
            config,
            tools: vec![],
            session: crate::session::Session::new(&cwd, model, ""),
            session_manager: Arc::new(Manager::default_for(&cwd)),
            settings: Arc::new(Settings::default()),
            agent_loop,
            verbose: false,
        };

        // Load default tools
        engine.tools = tools::all_tools();
        engine.agent_loop = engine.agent_loop.with_tools(engine.tools.clone());

        Ok(engine)
    }

    pub fn with_settings(mut self, settings: Settings) -> Self {
        self.settings = Arc::new(settings);
        self
    }

    pub fn with_tools(mut self, tools: Vec<crate::types::AgentTool>) -> Self {
        self.tools = tools.clone();
        self.agent_loop = self.agent_loop.with_tools(tools);
        self
    }

    pub fn with_system_prompt(mut self, prompt: &str) -> Self {
        self.agent_loop = self.agent_loop.with_system_prompt(prompt);
        self
    }

    pub fn with_config(mut self, config: EngineConfig) -> Self {
        if config.max_turns > 0 {
            self.agent_loop.config.max_turns = config.max_turns;
        }
        self.config = config.clone();
        self
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            cwd: ".".to_string(),
            system_prompt: String::new(),
            max_turns: DEFAULT_MAX_TURNS,
            thinking_level: String::new(),
            no_tools: String::new(),
            compaction_reserve_tokens: 160000,
            compaction_keep_recent_tokens: 80000,
            extension_paths: vec![],
            no_extensions: false,
        }
    }
}
