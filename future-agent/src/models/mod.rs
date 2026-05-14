//! Model registry — mirrors Go internal/modelregistry/
//!
//! Handles model catalog (built-in + user-provided) and model resolution.

pub mod generated;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Model represents a single model in the catalog.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(rename = "API")]
    pub api: String,
    #[serde(rename = "BaseURL", default)]
    pub base_url: String,
    #[serde(rename = "APIKey", default)]
    pub api_key: String,
    pub reasoning: bool,
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(rename = "ContextWindow")]
    pub context_window: i32,
    #[serde(rename = "MaxTokens", default)]
    pub max_tokens: i32,
    #[serde(rename = "Cost", default)]
    pub cost: Cost,
    #[serde(rename = "Compat", default)]
    pub compat: HashMap<String, serde_json::Value>,
    #[serde(rename = "ThinkingLevelMap", default)]
    pub thinking_level_map: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cost {
    #[serde(rename = "Input", default)]
    pub input: f64,
    #[serde(rename = "Output", default)]
    pub output: f64,
    #[serde(rename = "CacheRead", default)]
    pub cache_read: f64,
    #[serde(rename = "CacheWrite", default)]
    pub cache_write: f64,
}

/// BuiltinModels returns the generated model catalog from models_generated.rs.
/// All models are maintained by: make generate-models
pub fn builtin_models() -> Vec<Model> {
    crate::models::generated::init_builtin_models()
        .into_iter()
        .map(|m| Model {
            id: m.id,
            name: m.name,
            provider: m.provider,
            api: m.api,
            base_url: m.base_url,
            api_key: String::new(),
            reasoning: m.reasoning,
            input: m.input,
            context_window: m.context_window,
            max_tokens: m.max_tokens,
            cost: Cost {
                input: m.cost_input,
                output: m.cost_output,
                cache_read: m.cost_cache_read,
                cache_write: m.cost_cache_write,
            },
            compat: serde_json::from_str(&m.compat_json).unwrap_or_default(),
            thinking_level_map: serde_json::from_str(&m.tlm_json).unwrap_or_default(),
            headers: serde_json::from_str(&m.headers_json).unwrap_or_default(),
        })
        .collect()
}

/// UserModelsPath returns ~/.future_agent/models.json.
pub fn user_models_path() -> String {
    let home = dirs::home_dir()
        .map(|h| h.join(".future_agent/models.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/.future_agent/models.json"));
    home.to_string_lossy().to_string()
}

/// SettingsPath returns ~/.future_agent/settings.json.
pub fn settings_path() -> String {
    let home = dirs::home_dir()
        .map(|h| h.join(".future_agent/settings.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/.future_agent/settings.json"));
    home.to_string_lossy().to_string()
}

/// Settings represents the future_agent settings.json format.
#[derive(Debug, Deserialize)]
struct Settings {
    #[serde(rename = "defaultProvider", default)]
    default_provider: Option<String>,
    #[serde(rename = "defaultModel", default)]
    default_model: Option<String>,
    #[serde(rename = "defaultThinkingLevel", default)]
    default_thinking_level: Option<String>,
    #[serde(rename = "enabledModels", default)]
    enabled_models: Option<Vec<String>>,
}

/// LoadSettings reads ~/.future_agent/settings.json.
pub fn load_settings(path: &str) -> Result<Settings, String> {
    let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
}

/// Get default model from settings or builtin defaults.
pub fn get_default_model() -> Option<String> {
    let path = settings_path();
    if let Ok(settings) = load_settings(&path) {
        settings.default_model
    } else {
        None
    }
}

/// LoadUserModels reads a pi-compatible models.json file.
/// Returns empty vec if file doesn't exist.
pub fn load_user_models(path: &str) -> Result<Vec<Model>, String> {
    let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let cfg: ModelsConfig = serde_json::from_str(&data).map_err(|e| e.to_string())?;
    
    if cfg.providers.is_none() {
        return Ok(vec![]);
    }
    
    let providers = cfg.providers.unwrap();
    let mut models = vec![];
    
    for (provider_name, provider) in providers {
        if let Some(api_key) = provider.api_key {
            let provider_api = provider.api.unwrap_or_else(|| "openai".to_string());
            let provider_base_url = provider.base_url.unwrap_or_else(|| default_base_url_for_provider(&provider_name));
            for model in provider.models.unwrap_or_default() {
                models.push(Model {
                    id: model.id.clone(),
                    name: model.name.unwrap_or_else(|| model.id.clone()),
                    provider: provider_name.clone(),
                    api: provider_api.clone(),
                    base_url: provider_base_url.clone(),
                    api_key: api_key.clone(),
                    reasoning: model.reasoning.unwrap_or(false),
                    input: model.modalities.unwrap_or_default(),
                    context_window: model.limit.as_ref().and_then(|l| l.context).unwrap_or(128000),
                    max_tokens: model.limit.as_ref().and_then(|l| l.output).unwrap_or(4096),
                    cost: Cost {
                        input: model.cost.as_ref().and_then(|c| c.input).unwrap_or(0.0),
                        output: model.cost.as_ref().and_then(|c| c.output).unwrap_or(0.0),
                        cache_read: model.cost.as_ref().and_then(|c| c.cache_read).unwrap_or(0.0),
                        cache_write: model.cost.as_ref().and_then(|c| c.cache_write).unwrap_or(0.0),
                    },
                    ..Default::default()
                });
            }
        }
    }
    
    Ok(models)
}

fn default_base_url_for_provider(provider: &str) -> String {
    match provider {
        "openai" => "https://api.openai.com/v1".to_string(),
        "anthropic" => "https://api.anthropic.com".to_string(),
        "google" => "https://generativelanguage.googleapis.com/v1beta".to_string(),
        "deepseek" => "https://api.deepseek.com".to_string(),
        "openrouter" => "https://openrouter.ai/api/v1".to_string(),
        _ => "".to_string(),
    }
}

/// ModelsConfig mirrors Go internal/models/models.go
#[derive(Debug, Deserialize)]
struct ModelsConfig {
    #[serde(rename = "providers", default)]
    providers: Option<HashMap<String, ProviderConfig>>,
}

#[derive(Debug, Deserialize)]
struct ProviderConfig {
    #[serde(rename = "api", default)]
    api: Option<String>,
    #[serde(rename = "apiKey", default)]
    api_key: Option<String>,
    #[serde(rename = "baseUrl", default)]
    base_url: Option<String>,
    #[serde(rename = "models", default)]
    models: Option<Vec<ModelConfig>>,
}

#[derive(Debug, Deserialize)]
struct ModelConfig {
    #[serde(rename = "id")]
    id: String,
    #[serde(rename = "name", default)]
    name: Option<String>,
    #[serde(rename = "reasoning", default)]
    reasoning: Option<bool>,
    #[serde(rename = "modalities", default)]
    modalities: Option<Vec<String>>,
    #[serde(rename = "limit", default)]
    limit: Option<ModelLimit>,
    #[serde(rename = "cost", default)]
    cost: Option<ModelCost>,
}

#[derive(Debug, Deserialize)]
struct ModelLimit {
    #[serde(rename = "context", default)]
    context: Option<i32>,
    #[serde(rename = "output", default)]
    output: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct ModelCost {
    #[serde(rename = "input", default)]
    input: Option<f64>,
    #[serde(rename = "output", default)]
    output: Option<f64>,
    #[serde(rename = "cache_read", default)]
    cache_read: Option<f64>,
    #[serde(rename = "cache_write", default)]
    cache_write: Option<f64>,
}

/// Registry provides model resolution.
pub struct Registry {
    builtin: Vec<Model>,
    user: Vec<Model>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            builtin: builtin_models(),
            user: load_user_models(&user_models_path()).unwrap_or_default(),
        }
    }

    /// Get all available models (user models override built-in with same ID)
    pub fn all_models(&self) -> Vec<Model> {
        let mut models = self.builtin.clone();
        for user_model in &self.user {
            if let Some(idx) = models.iter().position(|m| m.id == user_model.id) {
                models[idx] = user_model.clone();
            } else {
                models.push(user_model.clone());
            }
        }
        models
    }

    /// Resolve a model ID to a Model (checks user first, then builtin)
    pub fn resolve(&self, id: &str) -> Option<Model> {
        // Check user models first
        if let Some(m) = self.user.iter().find(|m| m.id == id) {
            return Some(m.clone());
        }
        // Then builtin
        self.builtin.iter().find(|m| m.id == id).cloned()
    }

    /// Get default model for a provider
    pub fn default_for_provider(&self, provider: &str) -> Option<Model> {
        self.user.iter()
            .chain(self.builtin.iter())
            .find(|m| m.provider == provider)
            .cloned()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

