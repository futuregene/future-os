//! Engine — 1:1 compatible with internal/engine/

use crate::agent::{Loop, DEFAULT_MAX_TURNS};
use crate::config::Settings;
use crate::llm::Client as LLMClient;
use crate::session::Manager;
use crate::tools;
use crate::types::LLMProvider;
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

#[derive(Debug, Clone, Default)]
pub struct EngineConfig {
    pub cwd: String,
    pub system_prompt: String,
    pub max_turns: i32,
    pub thinking_level: String,
    pub thinking_level_map: std::collections::HashMap<String, String>,
    pub no_tools: String,
    pub compaction_reserve_tokens: i32,
    pub compaction_keep_recent_tokens: i32,
    pub extension_paths: Vec<String>,
    pub no_extensions: bool,
    pub compat_thinking_format: String,
    pub compat_supports_reasoning_effort: bool,
    pub compat_requires_reasoning_on_assistant: bool,
    pub max_tokens_field: String,
}

impl Engine {
    pub fn new(
        base_url: &str,
        api_key: &str,
        model: &str,
        config: EngineConfig,
        temperature: Option<f32>,
        max_tokens: Option<i32>,
    ) -> Result<Self> {
        let llm_client = LLMClient::new(base_url, api_key, temperature, max_tokens).with_compat(
            &config.compat_thinking_format,
            config.compat_supports_reasoning_effort,
            config.compat_requires_reasoning_on_assistant,
        );

        // Apply optional overrides in a chain via a scoped block — each
        // with_* consumes and returns Self (true builder pattern), so the
        // intermediate reassignments in the old code were always redundant.
        let llm_client = {
            let mut c = llm_client;
            if !config.max_tokens_field.is_empty() {
                c = c.with_max_tokens_field(&config.max_tokens_field);
            }
            if !config.thinking_level.is_empty() {
                c = c.with_thinking_level(&config.thinking_level);
            }
            if !config.thinking_level_map.is_empty() {
                c = c.with_thinking_level_map(config.thinking_level_map.clone());
            }
            c
        };

        let client: Arc<dyn LLMProvider> = Arc::new(llm_client);
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

        // Load default tools (4 core coding tools)
        engine.tools = tools::coding_tools();
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

impl EngineConfig {
    pub fn with_defaults() -> Self {
        Self {
            cwd: ".".to_string(),
            system_prompt: String::new(),
            max_turns: DEFAULT_MAX_TURNS,
            thinking_level: String::new(),
            no_tools: String::new(),
            compaction_reserve_tokens: 16384,
            compaction_keep_recent_tokens: 20000,
            extension_paths: vec![],
            no_extensions: false,
            max_tokens_field: String::new(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── EngineConfig ───────────────────────────────────────────────────────

    #[test]
    fn engine_config_with_defaults() {
        let c = EngineConfig::with_defaults();
        assert_eq!(c.cwd, ".");
        assert!(c.system_prompt.is_empty());
        assert_eq!(c.max_turns, DEFAULT_MAX_TURNS);
        assert!(c.thinking_level.is_empty());
        assert_eq!(c.compaction_reserve_tokens, 16384);
        assert_eq!(c.compaction_keep_recent_tokens, 20000);
        assert!(c.extension_paths.is_empty());
        assert!(!c.no_extensions);
        assert!(c.max_tokens_field.is_empty());
    }

    #[test]
    fn engine_config_default_trait() {
        let c = EngineConfig::default();
        assert!(c.cwd.is_empty());
        assert_eq!(c.max_turns, 0);
        assert!(c.thinking_level_map.is_empty());
    }

    // ─── Engine::new ────────────────────────────────────────────────────────

    #[test]
    fn engine_new_creates_with_tools() {
        let engine = Engine::new(
            "https://api.test.com",
            "test_key",
            "test-model",
            EngineConfig::with_defaults(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(engine.model, "test-model");
        assert_eq!(engine.api_key, "test_key");
        assert!(!engine.tools.is_empty()); // coding_tools loaded
        assert!(!engine.verbose);
    }

    #[test]
    fn engine_new_with_custom_config() {
        let mut config = EngineConfig::with_defaults();
        config.thinking_level = "high".to_string();
        config.max_tokens_field = "max_completion_tokens".to_string();
        let engine = Engine::new(
            "https://api.test.com",
            "key",
            "model",
            config,
            Some(0.7),
            Some(4096),
        )
        .unwrap();
        assert_eq!(engine.config.thinking_level, "high");
    }

    // ─── Builder methods ────────────────────────────────────────────────────

    #[test]
    fn engine_with_settings() {
        let settings = Settings {
            max_turns: 42,
            ..Default::default()
        };
        let engine = Engine::new(
            "https://api.test.com",
            "key",
            "model",
            EngineConfig::with_defaults(),
            None,
            None,
        )
        .unwrap()
        .with_settings(settings);
        assert_eq!(engine.settings.max_turns, 42);
    }

    #[test]
    fn engine_with_system_prompt() {
        let engine = Engine::new(
            "https://api.test.com",
            "key",
            "model",
            EngineConfig::with_defaults(),
            None,
            None,
        )
        .unwrap()
        .with_system_prompt("You are helpful");
        assert_eq!(engine.agent_loop.system_prompt, "You are helpful");
    }

    #[test]
    fn engine_with_config_max_turns() {
        let mut config = EngineConfig::with_defaults();
        config.max_turns = 10;
        let engine = Engine::new(
            "https://api.test.com",
            "key",
            "model",
            config.clone(),
            None,
            None,
        )
        .unwrap()
        .with_config(config);
        assert_eq!(engine.agent_loop.config.max_turns, 10);
    }

    #[test]
    fn engine_builder_chaining() {
        let engine = Engine::new(
            "https://api.test.com",
            "key",
            "model",
            EngineConfig::with_defaults(),
            None,
            None,
        )
        .unwrap()
        .with_system_prompt("Be concise")
        .with_settings(Settings::default());
        assert_eq!(engine.agent_loop.system_prompt, "Be concise");
    }

    // ─── Session management ─────────────────────────────────────────────────

    #[test]
    fn engine_session_has_cwd() {
        let engine = Engine::new(
            "https://api.test.com",
            "key",
            "model",
            EngineConfig::with_defaults(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(engine.session.cwd, ".");
    }

    #[test]
    fn engine_session_manager_exists() {
        let engine = Engine::new(
            "https://api.test.com",
            "key",
            "model",
            EngineConfig::with_defaults(),
            None,
            None,
        )
        .unwrap();
        // Manager should be initialized (Arc, so just verify it exists)
        assert!(Arc::strong_count(&engine.session_manager) >= 1);
    }
}
