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

/// UserModelsPath returns ~/.future/agent/models.json.
pub fn user_models_path() -> String {
    let home = dirs::home_dir()
        .map(|h| h.join(".future/agent/models.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/.future/agent/models.json"));
    home.to_string_lossy().to_string()
}

/// SettingsPath returns ~/.future/agent/settings.json.
pub fn settings_path() -> String {
    let home = dirs::home_dir()
        .map(|h| h.join(".future/agent/settings.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/.future/agent/settings.json"));
    home.to_string_lossy().to_string()
}

/// Settings represents the FutureAgent settings.json format.
#[derive(Debug, Deserialize)]
pub(crate) struct Settings {
    #[serde(rename = "defaultProvider", default)]
    #[allow(dead_code)]
    default_provider: Option<String>,
    #[serde(rename = "defaultModel", default)]
    default_model: Option<String>,
    #[serde(rename = "defaultThinkingLevel", default)]
    #[allow(dead_code)]
    default_thinking_level: Option<String>,
    #[serde(rename = "enabledModels", default)]
    #[allow(dead_code)]
    enabled_models: Option<Vec<String>>,
}

/// LoadSettings reads ~/.future/agent/settings.json.
pub(crate) fn load_settings(path: &str) -> Result<Settings, String> {
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
/// Load user models + provider-level overrides from models.json.
/// Providers without models still contribute baseUrl/compat overrides.
fn load_user_models_with_overrides(
    path: &str,
) -> Result<(Vec<Model>, HashMap<String, ProviderOverride>), String> {
    let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let cfg: ModelsConfig = serde_json::from_str(&data).map_err(|e| e.to_string())?;

    let providers = match cfg.providers {
        Some(p) => p,
        None => return Ok((vec![], HashMap::new())),
    };

    let mut models = vec![];
    let mut overrides = HashMap::new();

    for (provider_name, provider) in providers {
        // Store provider-level override (even if no models listed)
        if provider.base_url.is_some() || provider.api_key.is_some() {
            overrides.insert(
                provider_name.clone(),
                ProviderOverride {
                    base_url: provider.base_url.clone(),
                    api_key: provider.api_key.clone(),
                },
            );
        }

        // Skip model loading if no api_key
        let api_key = match provider.api_key.clone() {
            Some(k) => k,
            None => continue,
        };
        let provider_api = provider.api.unwrap_or_else(|| "openai".to_string());
        let provider_base_url = provider
            .base_url
            .clone()
            .unwrap_or_else(|| default_base_url_for_provider(&provider_name));

        // Load explicit models
        for model in provider.models.unwrap_or_default() {
            let mut m = Model {
                id: model.id.clone(),
                name: model.name.unwrap_or_else(|| model.id.clone()),
                provider: provider_name.clone(),
                api: provider_api.clone(),
                base_url: provider_base_url.clone(),
                api_key: api_key.clone(),
                reasoning: model.reasoning.unwrap_or(false),
                input: model.modalities.unwrap_or_default(),
                context_window: model
                    .context_window
                    .or_else(|| model.limit.as_ref().and_then(|l| l.context))
                    .unwrap_or(128000),
                max_tokens: model
                    .max_tokens
                    .or_else(|| model.limit.as_ref().and_then(|l| l.output))
                    .unwrap_or(4096),
                cost: Cost {
                    input: model.cost.as_ref().and_then(|c| c.input).unwrap_or(0.0),
                    output: model.cost.as_ref().and_then(|c| c.output).unwrap_or(0.0),
                    cache_read: model
                        .cost
                        .as_ref()
                        .and_then(|c| c.cache_read)
                        .unwrap_or(0.0),
                    cache_write: model
                        .cost
                        .as_ref()
                        .and_then(|c| c.cache_write)
                        .unwrap_or(0.0),
                },
                ..Default::default()
            };
            if let Some(ref compat) = provider.compat {
                m.compat = compat.clone();
            }
            if let Some(ref tlm) = provider.thinking_level_map {
                m.thinking_level_map = tlm.clone();
            }
            models.push(m);
        }
    }

    Ok((models, overrides))
}

fn default_base_url_for_provider(provider: &str) -> String {
    match provider {
        "openai" => "https://api.openai.com/v1".to_string(),
        "anthropic" => "https://api.anthropic.com".to_string(),
        "google" => "https://generativelanguage.googleapis.com/v1beta".to_string(),
        "deepseek" => "https://api.deepseek.com".to_string(),
        "openrouter" => "https://openrouter.ai/api/v1".to_string(),
        "dashscope" | "dashscope-coding" => {
            "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string()
        }
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
    #[serde(rename = "compat", default)]
    compat: Option<HashMap<String, serde_json::Value>>,
    #[serde(rename = "thinkingLevelMap", default)]
    thinking_level_map: Option<HashMap<String, serde_json::Value>>,
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
    #[serde(rename = "contextWindow", default)]
    context_window: Option<i32>,
    #[serde(rename = "maxTokens", default)]
    max_tokens: Option<i32>,
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

/// Provider-level override from user models.json (no models needed — just baseUrl/apiKey/compat etc.)
#[derive(Debug, Clone, Default)]
struct ProviderOverride {
    base_url: Option<String>,
    api_key: Option<String>,
}

/// Registry provides model resolution.
pub struct Registry {
    builtin: Vec<Model>,
    user: Vec<Model>,
    provider_overrides: HashMap<String, ProviderOverride>,
}

impl Registry {
    pub fn new() -> Self {
        let (user_models, overrides) =
            load_user_models_with_overrides(&user_models_path()).unwrap_or_default();
        Self {
            builtin: builtin_models(),
            user: user_models,
            provider_overrides: overrides,
        }
    }

    fn apply_override(&self, model: &mut Model) {
        if let Some(ov) = self.provider_overrides.get(&model.provider) {
            if let Some(ref url) = ov.base_url {
                if !url.is_empty() {
                    model.base_url = url.clone();
                }
            }
            if let Some(ref key) = ov.api_key {
                if !key.is_empty() {
                    model.api_key = key.clone();
                }
            }
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
        for m in &mut models {
            self.apply_override(m);
        }
        models
    }

    /// Resolve a model ID to a Model (checks user first, then builtin)
    pub fn resolve(&self, id: &str) -> Option<Model> {
        // Handle "provider/model" format
        if let Some((_provider, _model_id)) = id.split_once('/') {
            let full_id = id.to_string();
            return self
                .user
                .iter()
                .chain(self.builtin.iter())
                .find(|m| format!("{}/{}", m.provider, m.id) == full_id)
                .cloned()
                .map(|mut m| {
                    self.apply_override(&mut m);
                    m
                });
        }
        // Check user models first by exact ID
        if let Some(mut m) = self.user.iter().find(|m| m.id == id).cloned() {
            self.apply_override(&mut m);
            return Some(m);
        }
        // Then builtin
        self.builtin
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .map(|mut m| {
                self.apply_override(&mut m);
                m
            })
    }

    /// Get default model for a provider
    pub fn default_for_provider(&self, provider: &str) -> Option<Model> {
        self.user
            .iter()
            .chain(self.builtin.iter())
            .find(|m| m.provider == provider)
            .cloned()
            .map(|mut m| {
                self.apply_override(&mut m);
                m
            })
    }

    /// Resolve enabled_models patterns to available model IDs.
    /// Filters by auth (only models with configured API keys) and supports glob patterns.
    /// Returns ordered list of model IDs matching the patterns.
    pub fn resolve_scope(&self, patterns: &[String], auth: &crate::AuthStore) -> Vec<String> {
        let mut all = self.all_models();

        // Filter to only auth-configured models
        all.retain(|m| !m.api_key.is_empty() || auth.get(&m.provider).is_some());

        // Filter to only models matching patterns
        let mut matched: Vec<String> = vec![];
        for pattern in patterns {
            let pattern_lower = pattern.to_lowercase();
            for m in &all {
                let id_lower = m.id.to_lowercase();
                let prov_lower = m.provider.to_lowercase();
                let full = format!("{}/{}", prov_lower, id_lower);

                // Match: exact ID, glob *, or provider/model format
                let is_match = glob_match(&pattern_lower, &id_lower)
                    || glob_match(&pattern_lower, &full)
                    || glob_match(&pattern_lower, &prov_lower);

                if is_match && !matched.contains(&m.id) {
                    matched.push(m.id.clone());
                }
            }
        }

        matched
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple glob matching: supports * wildcard and literal matching.
fn glob_match(pattern: &str, target: &str) -> bool {
    if !pattern.contains('*') && !pattern.contains('?') {
        return pattern == target;
    }

    let parts: Vec<&str> = pattern.split('*').collect();
    let mut pos = 0;

    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }

        if i == 0 {
            // First part must match at beginning
            if !target.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if i == parts.len() - 1 {
            // Last part must match at end
            let remaining = &target[pos..];
            if !remaining.ends_with(part) {
                return false;
            }
        } else {
            // Middle parts must match somewhere
            if let Some(idx) = target[pos..].find(part) {
                pos += idx + part.len();
            } else {
                return false;
            }
        }
    }

    true
}
