//! Model registry — mirrors Go internal/modelregistry/
//!
//! Handles model catalog (built-in + user-provided) and model resolution.
//! The Future platform catalog (fetch/cache/convert) lives in `future.rs`.

use serde::{Deserialize, Serialize};

pub mod builtin;
mod future;
use future::{
    derive_thinking_compat, get_future_models_with_cache, resolve_future_base_url,
    sync_future_models_cache as sync_future_models_cache_inner,
};
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
    #[serde(default)]
    pub output: Vec<String>,
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
    /// If true, the model is hidden from model lists but still callable.
    #[serde(default)]
    pub hide: bool,
    /// Human-readable description from the Future platform catalog
    /// (e.g. "经济实用版，日常编程和对话任务够用且实惠"). Built-in / user models
    /// leave this as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// English counterpart of `description` from the Future platform catalog
    /// (e.g. "Budget-friendly edition, solid for daily coding and chat"). The GUI
    /// shows this when the UI language is not Chinese. Built-in / user models
    /// leave this as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_en: Option<String>,
    /// Whether the Future platform flags this model as recommended. Defaults to
    /// `false` for built-in / user models and for catalog entries that omit it.
    #[serde(default)]
    pub recommended: bool,
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

/// Catalog summary of one built-in provider: how many models it has and its
/// catalog base URL (no `models.json` overrides applied — clients merge those
/// themselves, and need the raw catalog URL to detect placeholder URLs), plus
/// the provider's human-readable display name. Serialized into the `list_models`
/// response when the client sets `include_builtin_providers`, so clients can
/// fetch the catalog at runtime instead of compiling it in. The agent is the
/// single source of truth for these display names — clients render `name`
/// verbatim rather than maintaining their own id→name maps.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinProviderSummary {
    pub name: String,
    pub model_count: usize,
    pub base_url: String,
}

/// Human-readable display name for a provider id. Curated brand casings for the
/// well-known providers; a title-case fallback for everything else. This is the
/// single source of truth for provider display names (the GUI renders the
/// `name` it receives over RPC rather than hard-coding its own map).
pub fn provider_display_name(id: &str) -> String {
    match id {
        "amazon-bedrock" => "Amazon Bedrock".to_string(),
        "anthropic" => "Anthropic".to_string(),
        "azure-openai-responses" => "Azure OpenAI Responses".to_string(),
        "cerebras" => "Cerebras".to_string(),
        "cloudflare-workers-ai" => "Cloudflare Workers AI".to_string(),
        "deepseek" => "DeepSeek".to_string(),
        "github-copilot" => "GitHub Copilot".to_string(),
        "google" => "Google".to_string(),
        "google-vertex" => "Google Vertex".to_string(),
        "groq" => "Groq".to_string(),
        "huggingface" => "Hugging Face".to_string(),
        "kimi-coding" => "Kimi Coding".to_string(),
        "minimax" => "MiniMax".to_string(),
        "minimax-cn" => "MiniMax CN".to_string(),
        "mistral" => "Mistral".to_string(),
        "moonshotai" => "Moonshot AI".to_string(),
        "moonshotai-cn" => "Moonshot AI CN".to_string(),
        "openai" => "OpenAI".to_string(),
        "openai-codex" => "OpenAI Codex".to_string(),
        "opencode" => "opencode".to_string(),
        "opencode-go" => "opencode Go".to_string(),
        "openrouter" => "OpenRouter".to_string(),
        "vercel-ai-gateway" => "Vercel AI Gateway".to_string(),
        "xai" => "xAI".to_string(),
        "xiaomi" => "Xiaomi".to_string(),
        "xiaomi-token-plan-ams" => "Xiaomi Token Plan AMS".to_string(),
        "xiaomi-token-plan-cn" => "Xiaomi Token Plan CN".to_string(),
        "xiaomi-token-plan-sgp" => "Xiaomi Token Plan SGP".to_string(),
        "zai" => "Z.ai".to_string(),
        "zhipuai" => "ZhipuAI".to_string(),
        _ => id
            .split('-')
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                // The filter above guarantees at least one char.
                let first = chars.next().expect("non-empty part");
                format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Process-wide shared copy of the parsed built-in catalog.  The embedded
/// models.json is ~1.9 MB; parsing it costs ~1.5 MB of fresh heap and
/// several ms of CPU on every call, so parse once and share an `Arc`.
pub fn builtin_models_shared() -> std::sync::Arc<Vec<Model>> {
    static BUILTIN_MODELS: std::sync::OnceLock<std::sync::Arc<Vec<Model>>> =
        std::sync::OnceLock::new();
    BUILTIN_MODELS
        .get_or_init(|| std::sync::Arc::new(builtin_models()))
        .clone()
}

/// BuiltinModels returns the generated model catalog from models_generated.rs.
/// All models are maintained by: make generate-models
pub fn builtin_models() -> Vec<Model> {
    builtin::init_builtin_models()
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
            output: vec!["text".to_string()],
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
            hide: m.hide,
            description: None,
            description_en: None,
            recommended: false,
        })
        .collect()
}

/// Whether the resolved model advertises image input (catalog `input`
/// modalities). Unknown models → false. Shared by the prompt path (deciding
/// image_url vs. a path fallback) and session reload (re-hydrating images).
///
/// Prefer `model_accepts_images_with(registry, model)` to avoid the expensive
/// `Registry::new()` call in hot paths.
pub fn model_accepts_images(model: &str) -> bool {
    model_accepts_images_with(&Registry::new(), model)
}

/// Like `model_accepts_images` but reuses an existing registry to avoid
/// re-deserialising the 906-model built-in catalog on every call.
pub fn model_accepts_images_with(registry: &Registry, model: &str) -> bool {
    registry
        .resolve(model)
        .map(|m| m.input.iter().any(|i| i == "image"))
        .unwrap_or(false)
}

/// Resolve the output token budget advertised by a model. Catalog values are
/// used as-is; only models without a configured limit receive a fallback.
pub fn effective_max_tokens(model: &Model) -> i32 {
    if model.max_tokens > 0 {
        model.max_tokens
    } else if model.reasoning {
        32000
    } else {
        16384
    }
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

/// Get the first available model, or None.
pub fn get_default_model() -> Option<String> {
    get_default_model_with(&Registry::new())
}

/// Like `get_default_model` but reuses an existing registry to avoid
/// re-deserialising the model catalog on every GUI poll.
pub fn get_default_model_with(registry: &Registry) -> Option<String> {
    let auth = crate::AuthStore::load();
    let all = registry.all_models();

    // A user-chosen global default (set by the onboarding model-picker) wins over
    // every heuristic below. Read fresh from disk so a `set_default_model` change
    // is picked up on the next resolution without restarting the agent. The stored
    // value may be a full `provider/id` or a bare id; accept either, but only when
    // the model still exists in the catalog and is credential-reachable — a stale
    // or now-unconfigured choice must not poison new sessions.
    if let Ok(settings) = crate::config::load_settings(&std::path::PathBuf::from(settings_path())) {
        let chosen = settings.default_model.trim();
        if !chosen.is_empty() {
            let matches = |m: &Model| {
                let credentialed = !m.api_key.is_empty() || auth.get(&m.provider).is_some();
                credentialed && (format!("{}/{}", m.provider, m.id) == chosen || m.id == chosen)
            };
            if let Some(m) = all.iter().find(|m| matches(m)) {
                return Some(format!("{}/{}", m.provider, m.id));
            }
        }
    }

    // Prefer future/deepseek-v4-pro when the future provider is configured,
    // otherwise fall back to the first model with credentials.
    let preferred = if auth.get("future").is_some()
        || all
            .iter()
            .any(|m| m.provider == "future" && !m.api_key.is_empty())
    {
        all.into_iter()
            .find(|m| m.provider == "future" && m.id == "deepseek-v4-pro")
            .map(|m| format!("{}/{}", m.provider, m.id))
    } else {
        None
    };
    preferred.or_else(|| {
        registry
            .all_models()
            .into_iter()
            .find(|m| !m.api_key.is_empty() || auth.get(&m.provider).is_some())
            .map(|m| format!("{}/{}", m.provider, m.id))
    })
}

/// Synchronously pull the Future provider's models from the platform and warm
/// the on-disk + in-memory caches. Used by the post-login `sync_future_models`
/// RPC so a subsequent registry rebuild sees a complete model list. Returns
/// `true` when the caches were populated. See [`future::sync_future_models_cache`].
pub fn sync_future_models_cache() -> bool {
    sync_future_models_cache_inner()
}

/// LoadUserModels reads a models.json file.
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

        // Use provider-level api_key if present, otherwise models will rely on auth.json
        let api_key = provider.api_key.clone().unwrap_or_default();
        let provider_api = provider.api.unwrap_or_else(|| "openai".to_string());
        let provider_base_url = provider.base_url.clone().unwrap_or_default();

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
                output: vec!["text".to_string()],
                context_window: model
                    .context_window
                    .or_else(|| model.limit.as_ref().and_then(|l| l.context))
                    .unwrap_or(128000),
                max_tokens: model
                    .max_tokens
                    .or_else(|| model.limit.as_ref().and_then(|l| l.output))
                    .unwrap_or(0),
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
                hide: model.hide,
                ..Default::default()
            };
            if let Some(ref compat) = provider.compat {
                m.compat = compat.clone();
            }
            // Derive compat from supportedParameters (e.g. max_completion_tokens
            // → maxTokensField).  This mirrors what convert_future_model does for
            // Future platform models.
            if let Some(ref supported_params) = model.supported_parameters {
                let (derived_compat, derived_tlm) = derive_thinking_compat(supported_params, None);
                for (k, v) in derived_compat {
                    m.compat.insert(k, v);
                }
                if m.thinking_level_map.is_empty() {
                    m.thinking_level_map = derived_tlm;
                }
            }
            // Model-level compat overrides provider-level compat on a per-key basis
            if let Some(ref model_compat) = model.compat {
                for (k, v) in model_compat {
                    m.compat.insert(k.clone(), v.clone());
                }
            }
            if let Some(ref tlm) = provider.thinking_level_map {
                m.thinking_level_map = tlm.clone();
            }
            models.push(m);
        }
    }

    Ok((models, overrides))
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
    /// If true, the model is hidden from model lists but still callable.
    #[serde(default)]
    hide: bool,
    /// Model-level compat overrides (e.g. maxTokensField for reasoning models).
    #[serde(rename = "compat", default)]
    compat: Option<HashMap<String, serde_json::Value>>,
    /// Supported parameters, used to auto-derive compat (maxTokensField, thinkingFormat, etc.).
    #[serde(rename = "supportedParameters", default)]
    supported_parameters: Option<Vec<String>>,
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

/// Calculate a simple similarity score between two provider names.
/// Used to pick the best matching builtin when multiple models share the same ID
/// but come from different providers (e.g. "azure" vs "openai" for gpt-* models).
fn provider_similarity(a: &str, b: &str) -> f64 {
    let a = a.to_lowercase();
    let b = b.to_lowercase();

    // Exact match
    if a == b {
        return 1.0;
    }

    // One contains the other (e.g. "azurefo" contains no common substring with "openai")
    if a.contains(&b) || b.contains(&a) {
        return 0.9;
    }

    // Normalize: extract alphanumeric chars only
    let na: String = a.chars().filter(|c| c.is_alphanumeric()).collect();
    let nb: String = b.chars().filter(|c| c.is_alphanumeric()).collect();

    if na == nb {
        return 0.8;
    }
    if na.contains(&nb) || nb.contains(&na) {
        return 0.7;
    }

    // Known provider groups — providers within the same group get a bonus
    let groups: &[&[&str]] = &[
        &["openai", "azure", "azurefo"],
        &["anthropic", "claude"],
        &["deepseek"],
        &["google", "gemini", "vertex"],
        &["qwen", "dashscope", "alibaba"],
    ];
    for group in groups {
        let a_in = group.iter().any(|g| na.contains(g) || g.contains(&na));
        let b_in = group.iter().any(|g| nb.contains(g) || g.contains(&nb));
        if a_in && b_in {
            return 0.5;
        }
    }

    0.0
}

/// Find the best matching builtin model for a user-provided model.
/// Matches by `id` first; if multiple builtins share the same ID (different
/// providers), picks the one whose provider name is most similar.
fn find_best_builtin_match<'a>(user_model: &Model, builtins: &'a [Model]) -> Option<&'a Model> {
    let candidates: Vec<&'a Model> = builtins.iter().filter(|b| b.id == user_model.id).collect();

    match candidates.len() {
        0 => None,
        1 => Some(candidates[0]),
        _ => {
            // Multiple matches — pick by provider similarity
            candidates.into_iter().max_by(|a, b| {
                let sa = provider_similarity(&user_model.provider, &a.provider);
                let sb = provider_similarity(&user_model.provider, &b.provider);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
        }
    }
}

/// Enrich user-provided models with missing parameters from the builtin catalog.
///
/// When a user defines a custom model in models.json they often omit
/// provider-specific parameters (thinking format, compat flags, thinking level
/// map, cost, etc.).  This function matches each user model against the builtin
/// catalog by model ID (with provider-similarity tie-breaking) and back-fills
/// any fields the user didn't explicitly set.
///
/// Rules:
/// - `compat` — empty map → take all from builtin; non-empty → merge key-by-key
/// - `thinking_level_map` — same as compat
/// - `reasoning` — if builtin has it true, adopt it
/// - `input` — fill from builtin when empty
/// - `context_window` — fill when == 128000 (default)
/// - `max_tokens` — fill when == 0
/// - `cost` — fill each field when == 0.0
/// - `headers` — merge from builtin
fn enrich_user_models(user_models: &mut [Model], builtins: &[Model]) {
    for user_model in user_models.iter_mut() {
        let best = match find_best_builtin_match(user_model, builtins) {
            Some(b) => b,
            None => continue,
        };

        // reasoning: adopt from builtin if builtin has it
        if best.reasoning {
            user_model.reasoning = true;
        }

        // input: fill from builtin if user didn't specify
        if user_model.input.is_empty() && !best.input.is_empty() {
            user_model.input = best.input.clone();
        }

        // context_window: fill if still at default (0 from Default, or
        // 128000 from load_user_models_with_overrides fallback).
        if user_model.context_window == 0 || user_model.context_window == 128000 {
            user_model.context_window = best.context_window;
        }

        // max_tokens: fill if user didn't specify
        if user_model.max_tokens == 0 {
            user_model.max_tokens = best.max_tokens;
        }

        // cost: per-field fill
        if user_model.cost.input == 0.0 {
            user_model.cost.input = best.cost.input;
        }
        if user_model.cost.output == 0.0 {
            user_model.cost.output = best.cost.output;
        }
        if user_model.cost.cache_read == 0.0 {
            user_model.cost.cache_read = best.cost.cache_read;
        }
        if user_model.cost.cache_write == 0.0 {
            user_model.cost.cache_write = best.cost.cache_write;
        }

        // compat: empty → full takeover; non-empty → merge missing keys
        if user_model.compat.is_empty() {
            user_model.compat = best.compat.clone();
        } else {
            for (k, v) in &best.compat {
                user_model
                    .compat
                    .entry(k.clone())
                    .or_insert_with(|| v.clone());
            }
        }

        // thinking_level_map: same merge strategy
        if user_model.thinking_level_map.is_empty() {
            user_model.thinking_level_map = best.thinking_level_map.clone();
        } else {
            for (k, v) in &best.thinking_level_map {
                user_model
                    .thinking_level_map
                    .entry(k.clone())
                    .or_insert_with(|| v.clone());
            }
        }

        // headers: merge from builtin
        for (k, v) in &best.headers {
            user_model
                .headers
                .entry(k.clone())
                .or_insert_with(|| v.clone());
        }

        // Fallback: reasoning models on OpenAI-compatible APIs need
        // max_completion_tokens instead of max_tokens. The builtin catalog
        // often has empty compat_json for these models, and users may not
        // provide supportedParameters either.
        if !user_model.compat.contains_key("maxTokensField")
            && user_model.reasoning
            && is_openai_compatible_api(&user_model.api)
        {
            user_model.compat.insert(
                "maxTokensField".to_string(),
                serde_json::json!("max_completion_tokens"),
            );
        }
    }
}

/// Check whether an API identifier refers to an OpenAI-compatible
/// completions/chat endpoint (where reasoning models use max_completion_tokens).
fn is_openai_compatible_api(api: &str) -> bool {
    matches!(
        api,
        "openai-completions" | "chat" | "openai" | "azure-openai-responses"
    )
}

/// Registry provides model resolution.
pub struct Registry {
    // Shared, parsed-once built-in catalog (see `builtin_models_shared`).
    // Copy-on-write: registries that inject future-provider models clone the
    // Vec via Arc::make_mut; everyone else just shares the Arc.
    builtin: std::sync::Arc<Vec<Model>>,
    user: Vec<Model>,
    provider_overrides: HashMap<String, ProviderOverride>,
}

impl Registry {
    pub fn new() -> Self {
        let (mut user_models, overrides) =
            load_user_models_with_overrides(&user_models_path()).unwrap_or_default();

        let mut builtin = builtin_models_shared();

        // Load Future provider models dynamically if auth is available
        let auth_store = crate::AuthStore::load();
        let future_provider_override = if let Some(future_key) = auth_store.get("future") {
            let base_url = resolve_future_base_url();
            let future_models = get_future_models_with_cache(&future_key, &base_url);

            // Add future models to builtin (they override same-ID builtin
            // models).  Clones the shared Vec only on this path.
            let builtin_mut = std::sync::Arc::make_mut(&mut builtin);
            for fm in future_models {
                if let Some(idx) = builtin_mut.iter().position(|m| m.id == fm.id) {
                    builtin_mut[idx] = fm;
                } else {
                    builtin_mut.push(fm);
                }
            }

            // Return provider override for "future" provider
            Some((
                "future".to_string(),
                ProviderOverride {
                    base_url: Some(format!("{}/v1", base_url)),
                    api_key: Some(future_key),
                },
            ))
        } else {
            None
        };

        // Enrich user models with missing parameters from builtin catalog
        // (e.g. compat, thinking_level_map, cost) based on model ID match.
        enrich_user_models(&mut user_models, &builtin);

        let mut final_overrides = overrides;
        if let Some((name, ov)) = future_provider_override {
            final_overrides.insert(name, ov);
        }

        Self {
            builtin,
            user: user_models,
            provider_overrides: final_overrides,
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

    /// Get all available models (user models override built-in with same ID).
    /// Models with `hide: true` are excluded from the listing but remain callable via `resolve()`.
    pub fn all_models(&self) -> Vec<Model> {
        let mut models = self.builtin.as_ref().clone();
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
        // Filter out hidden models from the list
        models.retain(|m| !m.hide);
        models
    }

    /// Summaries of the built-in provider catalog, keyed by provider id: model
    /// count and the catalog base URL. The dynamic Future platform catalog is
    /// excluded — clients present that provider separately. Serves the
    /// `list_models` `include_builtin_providers` flag so clients can fetch the
    /// catalog at runtime instead of compiling it in.
    pub fn builtin_provider_summaries(
        &self,
    ) -> std::collections::BTreeMap<String, BuiltinProviderSummary> {
        let mut summaries = std::collections::BTreeMap::new();
        for model in self.builtin.iter() {
            if model.hide || model.provider.is_empty() || model.provider == "future" {
                continue;
            }
            let entry =
                summaries
                    .entry(model.provider.clone())
                    .or_insert_with(|| BuiltinProviderSummary {
                        name: provider_display_name(&model.provider),
                        model_count: 0,
                        base_url: String::new(),
                    });
            entry.model_count += 1;
            if entry.base_url.is_empty() && !model.base_url.is_empty() {
                entry.base_url = model.base_url.clone();
            }
        }
        summaries
    }

    /// The set of built-in provider ids (plus the reserved Future platform id).
    /// Used to reject custom-provider writes that would collide with a built-in —
    /// the agent is the authority on its own catalog, so this guard stays
    /// correct regardless of which client issues the write and whether any
    /// client-side catalog is stale or unavailable.
    pub fn builtin_provider_ids(&self) -> std::collections::HashSet<String> {
        let mut ids: std::collections::HashSet<String> = self
            .builtin
            .iter()
            .filter(|model| !model.provider.is_empty())
            .map(|model| model.provider.clone())
            .collect();
        ids.insert("future".to_string());
        ids
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

#[cfg(test)]
mod tests {
    use super::{
        derive_thinking_compat, enrich_user_models, find_best_builtin_match, provider_similarity,
    };
    use super::{Cost, Model, ProviderOverride, Registry};
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn derives_max_completion_tokens_field_from_supported_parameters() {
        let supported = vec![
            "max_tokens".to_string(),
            "max_completion_tokens".to_string(),
        ];

        let (compat, _) = derive_thinking_compat(&supported, Some("GPT"));

        assert_eq!(
            compat
                .get("maxTokensField")
                .and_then(|value| value.as_str()),
            Some("max_completion_tokens")
        );
    }

    #[test]
    fn keeps_default_max_tokens_field_when_not_advertised() {
        let supported = vec!["max_tokens".to_string()];

        let (compat, _) = derive_thinking_compat(&supported, None);

        assert!(!compat.contains_key("maxTokensField"));
    }

    // ── provider_similarity ──

    #[test]
    fn provider_similarity_exact_match() {
        assert_eq!(provider_similarity("openai", "openai"), 1.0);
    }

    #[test]
    fn provider_similarity_contains() {
        // "azurefo" contains neither "openai" nor vice versa, but falls
        // through to group matching.
        assert!(provider_similarity("azurefo", "openai") > 0.0);
    }

    #[test]
    fn provider_similarity_different_groups() {
        assert_eq!(provider_similarity("openai", "deepseek"), 0.0);
    }

    // ── find_best_builtin_match ──

    fn make_model(id: &str, provider: &str) -> Model {
        Model {
            id: id.to_string(),
            provider: provider.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn effective_max_tokens_preserves_catalog_limit() {
        let model = Model {
            max_tokens: 384000,
            ..Default::default()
        };
        assert_eq!(super::effective_max_tokens(&model), 384000);
    }

    #[test]
    fn effective_max_tokens_uses_fallback_only_when_missing() {
        let reasoning = Model {
            reasoning: true,
            ..Default::default()
        };
        let plain = Model::default();
        assert_eq!(super::effective_max_tokens(&reasoning), 32000);
        assert_eq!(super::effective_max_tokens(&plain), 16384);
    }

    #[test]
    fn find_best_builtin_match_no_candidates() {
        let builtins = vec![make_model("gpt-4", "openai")];
        let user = make_model("claude-opus-4-8", "realapi");
        assert!(find_best_builtin_match(&user, &builtins).is_none());
    }

    #[test]
    fn find_best_builtin_match_single_candidate() {
        let builtins = vec![make_model("gpt-4", "openai")];
        let user = make_model("gpt-4", "azurefo");
        let result = find_best_builtin_match(&user, &builtins);
        assert!(result.is_some());
        assert_eq!(result.unwrap().provider, "openai");
    }

    #[test]
    fn find_best_builtin_match_picks_by_provider_similarity() {
        let builtins = vec![
            make_model("gpt-5.6-sol", "amazon-bedrock"),
            make_model("gpt-5.6-sol", "openai"),
        ];
        let user = make_model("gpt-5.6-sol", "azurefo");
        let result = find_best_builtin_match(&user, &builtins);
        assert!(result.is_some());
        // azurefo should be closer to openai than amazon-bedrock
        assert_eq!(result.unwrap().provider, "openai");
    }

    // ── enrich_user_models ──

    fn make_builtin_deepseek_v4() -> Model {
        let mut compat = HashMap::new();
        compat.insert("thinkingFormat".to_string(), json!("deepseek"));
        compat.insert(
            "requiresReasoningContentOnAssistantMessages".to_string(),
            json!(true),
        );

        let mut tlm = HashMap::new();
        tlm.insert("high".to_string(), json!("high"));
        tlm.insert("xhigh".to_string(), json!("max"));

        Model {
            id: "deepseek-v4-pro".to_string(),
            name: "DeepSeek V4 Pro".to_string(),
            provider: "deepseek".to_string(),
            reasoning: true,
            input: vec!["text".to_string(), "image".to_string()],
            context_window: 1000000,
            max_tokens: 384000,
            cost: Cost {
                input: 1.74,
                output: 3.48,
                cache_read: 0.14,
                cache_write: 0.0,
            },
            compat,
            thinking_level_map: tlm,
            ..Default::default()
        }
    }

    #[test]
    fn enrich_fills_empty_compat_from_builtin() {
        let builtins = vec![make_builtin_deepseek_v4()];
        let mut models = vec![Model {
            id: "deepseek-v4-pro".to_string(),
            name: "DSv4".to_string(),
            provider: "custom-provider".to_string(),
            ..Default::default()
        }];

        enrich_user_models(&mut models, &builtins);

        let enriched = &models[0];
        assert_eq!(
            enriched
                .compat
                .get("thinkingFormat")
                .and_then(|v| v.as_str()),
            Some("deepseek")
        );
        assert_eq!(
            enriched
                .thinking_level_map
                .get("xhigh")
                .and_then(|v| v.as_str()),
            Some("max")
        );
    }

    #[test]
    fn enrich_merges_compat_key_by_key() {
        let builtins = vec![make_builtin_deepseek_v4()];

        // User already set maxTokensField but not thinkingFormat
        let mut models = vec![Model {
            id: "deepseek-v4-pro".to_string(),
            name: "DSv4".to_string(),
            provider: "custom-provider".to_string(),
            compat: {
                let mut c = HashMap::new();
                c.insert("maxTokensField".to_string(), json!("max_completion_tokens"));
                c
            },
            ..Default::default()
        }];

        enrich_user_models(&mut models, &builtins);

        let user = &models[0];
        // User's key preserved
        assert_eq!(
            user.compat.get("maxTokensField").and_then(|v| v.as_str()),
            Some("max_completion_tokens")
        );
        // Missing keys filled from builtin
        assert_eq!(
            user.compat.get("thinkingFormat").and_then(|v| v.as_str()),
            Some("deepseek")
        );
        // thinking_level_map was empty → full takeover from builtin
        assert_eq!(
            user.thinking_level_map
                .get("xhigh")
                .and_then(|v| v.as_str()),
            Some("max")
        );
    }

    #[test]
    fn enrich_fills_scalar_fields_from_builtin() {
        let builtins = vec![make_builtin_deepseek_v4()];
        let mut models = vec![Model {
            id: "deepseek-v4-pro".to_string(),
            name: "DSv4".to_string(),
            provider: "custom-provider".to_string(),
            ..Default::default()
        }];

        enrich_user_models(&mut models, &builtins);

        let user = &models[0];
        assert!(user.reasoning);
        assert_eq!(user.input, vec!["text", "image"]);
        assert_eq!(user.context_window, 1000000);
        assert_eq!(user.max_tokens, 384000);
        assert_eq!(user.cost.input, 1.74);
        assert_eq!(user.cost.output, 3.48);
        assert_eq!(user.cost.cache_read, 0.14);
    }

    #[test]
    fn enrich_respects_user_scalar_values() {
        let builtins = vec![make_builtin_deepseek_v4()];
        let mut models = vec![Model {
            id: "deepseek-v4-pro".to_string(),
            name: "DSv4".to_string(),
            provider: "custom-provider".to_string(),
            // reasoning left as default false — builtin has true, so it gets
            // enriched. This is intentional: reasoning-capable models should
            // be marked as such.
            input: vec!["text".to_string()],
            context_window: 64000,
            max_tokens: 8192,
            cost: Cost {
                input: 0.5,
                output: 1.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            ..Default::default()
        }];

        enrich_user_models(&mut models, &builtins);

        let user = &models[0];
        // reasoning adopted from builtin (builtin says true)
        assert!(user.reasoning);
        // User-provided values preserved
        assert_eq!(user.input, vec!["text"]);
        assert_eq!(user.context_window, 64000);
        assert_eq!(user.max_tokens, 8192);
        assert_eq!(user.cost.input, 0.5);
        assert_eq!(user.cost.output, 1.0);
    }

    #[test]
    fn derive_max_tokens_field_from_only_max_completion_tokens() {
        // Exact gpt-5.5 scenario: only "max_completion_tokens" in supportedParameters
        let supported = vec!["max_completion_tokens".to_string()];
        let (compat, _) = derive_thinking_compat(&supported, None);
        assert_eq!(
            compat.get("maxTokensField").and_then(|v| v.as_str()),
            Some("max_completion_tokens"),
            "maxTokensField should be set when supportedParameters has max_completion_tokens"
        );
    }

    #[test]
    fn enrich_gpt55_from_builtin_preserves_derived_max_tokens_field() {
        // Simulate the builtin gpt-5.5 entries (all have empty compat_json)
        let builtins = vec![
            Model {
                id: "gpt-5.5".to_string(),
                provider: "openai".to_string(),
                reasoning: true,
                context_window: 272000,
                max_tokens: 128000,
                thinking_level_map: {
                    let mut m = HashMap::new();
                    m.insert("off".to_string(), json!(null));
                    m.insert("xhigh".to_string(), json!("xhigh"));
                    m
                },
                ..Default::default()
            },
            Model {
                id: "gpt-5.5".to_string(),
                provider: "github-copilot".to_string(),
                reasoning: true,
                context_window: 400000,
                max_tokens: 128000,
                ..Default::default()
            },
        ];

        // Simulate user model from models.json: azurefo/gpt-5.5 with
        // supportedParameters: ["max_completion_tokens"]
        // After load_user_models_with_overrides processing:
        let mut compat = HashMap::new();
        // derive_thinking_compat would have set this from supportedParameters
        let (derived_compat, _) =
            derive_thinking_compat(&["max_completion_tokens".to_string()], None);
        for (k, v) in derived_compat {
            compat.insert(k, v);
        }

        let user = Model {
            id: "gpt-5.5".to_string(),
            provider: "azurefo".to_string(),
            reasoning: false, // user didn't set it
            max_tokens: 0,    // user didn't set it
            compat,
            ..Default::default()
        };

        let mut models = vec![user];
        enrich_user_models(&mut models, &builtins);
        let user = &models[0];

        // After enrichment:
        // - maxTokensField from supportedParameters MUST be preserved
        assert_eq!(
            user.compat.get("maxTokensField").and_then(|v| v.as_str()),
            Some("max_completion_tokens"),
            "maxTokensField from supportedParameters should survive enrichment"
        );
        // - reasoning adopted from builtin
        assert!(user.reasoning, "reasoning should be adopted from builtin");
        // - thinking_level_map filled from builtin
        assert_eq!(
            user.thinking_level_map
                .get("xhigh")
                .and_then(|v| v.as_str()),
            Some("xhigh"),
            "thinking_level_map should be filled from builtin"
        );
    }

    #[test]
    fn enrich_no_match_does_nothing() {
        let builtins = vec![make_builtin_deepseek_v4()];
        let mut models = vec![Model {
            id: "nonexistent-model".to_string(),
            provider: "custom-provider".to_string(),
            ..Default::default()
        }];

        let original = models[0].clone();
        enrich_user_models(&mut models, &builtins);

        // No match → model unchanged
        assert_eq!(models[0].compat, original.compat);
        assert_eq!(models[0].max_tokens, original.max_tokens);
    }

    #[test]
    fn enrich_infers_max_tokens_field_for_reasoning_models() {
        // gpt-5.5 with no supportedParameters, no compat at all.
        // Builtin also has empty compat. Fallback should infer maxTokensField
        // from reasoning + openai-compatible API.
        let builtins = vec![Model {
            id: "gpt-5.5".to_string(),
            provider: "openai".to_string(),
            reasoning: true,
            ..Default::default()
        }];

        let mut models = vec![Model {
            id: "gpt-5.5".to_string(),
            provider: "azurefo".to_string(),
            api: "openai-completions".to_string(),
            reasoning: false,
            ..Default::default()
        }];

        enrich_user_models(&mut models, &builtins);

        let user = &models[0];
        assert!(user.reasoning, "reasoning should be true from builtin");
        assert_eq!(
            user.compat.get("maxTokensField").and_then(|v| v.as_str()),
            Some("max_completion_tokens"),
            "maxTokensField should be inferred for reasoning models on openai-compatible API"
        );
    }

    // ─── glob_match ────────────────────────────────────────────────────────

    #[test]
    fn glob_match_exact() {
        assert!(super::glob_match("gpt-4o", "gpt-4o"));
        assert!(!super::glob_match("gpt-4o", "gpt-4"));
    }

    #[test]
    fn glob_match_star_prefix() {
        assert!(super::glob_match("*", "anything"));
        assert!(super::glob_match("gpt-*", "gpt-4o"));
        assert!(super::glob_match("gpt-*", "gpt-3.5"));
        assert!(!super::glob_match("gpt-*", "claude-3"));
    }

    #[test]
    fn glob_match_star_suffix() {
        assert!(super::glob_match("*.txt", "file.txt"));
        assert!(!super::glob_match("*.txt", "file.rs"));
    }

    #[test]
    fn glob_match_star_middle() {
        assert!(super::glob_match("gpt*4o", "gpt-4o"));
        assert!(super::glob_match("gpt*4o", "gpt4o"));
        assert!(!super::glob_match("gpt*4o", "claude-4o"));
    }

    #[test]
    fn glob_match_multiple_stars() {
        assert!(super::glob_match("*-*-*", "a-b-c"));
        assert!(super::glob_match("*-*-*", "gpt-4o-turbo"));
        assert!(!super::glob_match("*-*-*", "a-b"));
    }

    // ─── is_openai_compatible_api ──────────────────────────────────────────

    #[test]
    fn is_openai_compatible_api_true() {
        assert!(super::is_openai_compatible_api("openai"));
        assert!(super::is_openai_compatible_api("openai-completions"));
        assert!(super::is_openai_compatible_api("chat"));
        assert!(super::is_openai_compatible_api("azure-openai-responses"));
    }

    #[test]
    fn is_openai_compatible_api_false() {
        assert!(!super::is_openai_compatible_api("anthropic"));
        assert!(!super::is_openai_compatible_api("gemini"));
        assert!(!super::is_openai_compatible_api(""));
    }

    // ─── model_accepts_images ──────────────────────────────────────────────

    #[test]
    fn model_accepts_images_returns_bool() {
        // The function depends on the global Registry — just verify it doesn't panic
        let result = super::model_accepts_images("gpt-4o");
        // Result depends on the builtin model catalog
        let _ = result;
    }

    #[test]
    fn model_accepts_images_unknown_returns_false() {
        assert!(!super::model_accepts_images(
            "definitely-not-a-real-model-xyz"
        ));
    }

    // ─── builtin_models / user_models_path / settings_path / get_default_model ──

    #[test]
    fn builtin_models_returns_nonempty() {
        let models = super::builtin_models();
        assert!(!models.is_empty(), "builtin models should not be empty");
    }

    #[test]
    fn user_models_path_contains_models_json() {
        let path = super::user_models_path();
        assert!(path.contains("models.json"));
    }

    #[test]
    fn settings_path_contains_settings_json() {
        let path = super::settings_path();
        assert!(path.contains("settings.json"));
    }

    #[test]
    fn get_default_model_returns_something() {
        let model = super::get_default_model();
        // In CI there may be no auth.json configured, so None is acceptable
        let _ = model;
    }

    // ─── provider_similarity additional ────────────────────────────────────

    #[test]
    fn provider_similarity_same_string_always_one() {
        assert_eq!(provider_similarity("any", "any"), 1.0);
    }

    // ─── Registry ──────────────────────────────────────────────────────────

    #[test]
    fn registry_new_creates_instance() {
        let reg = super::Registry::new();
        let models = reg.all_models();
        assert!(!models.is_empty(), "registry should have models");
    }

    #[test]
    fn registry_resolve_existing_model() {
        let reg = super::Registry::new();
        let models = reg.all_models();
        let first = models.first().expect("builtin catalog is non-empty");
        let resolved = reg.resolve(&first.id);
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().id, first.id);
    }

    #[test]
    fn registry_resolve_nonexistent_returns_none() {
        let reg = super::Registry::new();
        assert!(reg.resolve("definitely-not-real-model-xyz").is_none());
    }

    #[test]
    fn registry_resolve_provider_slash_format() {
        let reg = super::Registry::new();
        let models = reg.all_models();
        let first = models.first().expect("builtin catalog is non-empty");
        let full_id = format!("{}/{}", first.provider, first.id);
        let resolved = reg.resolve(&full_id);
        assert!(resolved.is_some());
    }

    #[test]
    fn registry_default_for_provider() {
        let reg = super::Registry::new();
        let models = reg.all_models();
        let first = models.first().expect("builtin catalog is non-empty");
        let resolved = reg.default_for_provider(&first.provider);
        assert!(resolved.is_some());
    }

    #[test]
    fn registry_resolve_scope_with_star() {
        let reg = super::Registry::new();
        let auth = crate::AuthStore::load();
        let scope = reg.resolve_scope(&["*".to_string()], &auth);
        // Star should match all models (if auth is available)
        let _ = scope;
    }

    // ─── derive_thinking_compat ────────────────────────────────────────────

    #[test]
    fn derive_thinking_compat_none_reasoning() {
        let (compat, _) = derive_thinking_compat(&[], None);
        assert!(compat.is_empty());
    }

    #[test]
    fn derive_thinking_compat_glm() {
        let (compat, _) = derive_thinking_compat(&[], Some("GLM"));
        assert!(!compat.is_empty());
        assert_eq!(
            compat.get("thinkingFormat").and_then(|v| v.as_str()),
            Some("zai")
        );
    }

    #[test]
    fn derive_thinking_compat_with_reasoning_params() {
        let supported = vec!["reasoning_effort".to_string()];
        let (compat, tlm) = derive_thinking_compat(&supported, None);
        assert!(!compat.is_empty());
        assert_eq!(
            compat.get("thinkingFormat").and_then(|v| v.as_str()),
            Some("deepseek")
        );
        assert!(tlm.contains_key("high"));
    }

    #[test]
    fn derive_thinking_compat_with_enable_thinking() {
        let supported = vec!["enable_thinking".to_string()];
        let (compat, _) = derive_thinking_compat(&supported, None);
        assert!(!compat.is_empty());
        assert_eq!(
            compat.get("thinkingFormat").and_then(|v| v.as_str()),
            Some("qwen")
        );
    }

    #[test]
    fn derive_thinking_compat_with_reasoning_split() {
        let supported = vec!["reasoning_split".to_string()];
        let (compat, _) = derive_thinking_compat(&supported, None);
        assert!(!compat.is_empty());
        assert_eq!(
            compat.get("thinkingFormat").and_then(|v| v.as_str()),
            Some("reasoning-split")
        );
    }

    // ─── load_user_models_with_overrides ────────────────────────────────────

    #[test]
    fn load_user_models_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.json");
        let content = r#"{
            "providers": {
                "custom": {
                    "baseUrl": "https://custom.api.com/v1",
                    "apiKey": "sk-custom",
                    "api": "openai",
                    "models": [{
                        "id": "custom-model",
                        "name": "Custom Model",
                        "reasoning": true,
                        "contextWindow": 64000,
                        "maxTokens": 4096,
                        "modalities": ["text", "image"],
                        "cost": {"input": 1.0, "output": 2.0, "cacheRead": 0.5, "cacheWrite": 0.3}
                    }]
                }
            }
        }"#;
        std::fs::write(&path, content).unwrap();

        let (models, overrides) =
            super::load_user_models_with_overrides(path.to_str().unwrap()).unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "custom-model");
        assert_eq!(models[0].provider, "custom");
        assert_eq!(models[0].api_key, "sk-custom");
        assert!(models[0].reasoning);
        assert_eq!(models[0].context_window, 64000);
        assert_eq!(models[0].max_tokens, 4096);
        assert_eq!(models[0].cost.input, 1.0);
        assert_eq!(models[0].cost.output, 2.0);
        assert_eq!(overrides.len(), 1);
        assert_eq!(
            overrides["custom"].base_url.as_deref(),
            Some("https://custom.api.com/v1")
        );
    }

    #[test]
    fn load_user_models_missing_file_errors() {
        let result = super::load_user_models_with_overrides("/no/such/file.json");
        assert!(result.is_err());
    }

    #[test]
    fn load_user_models_no_providers_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.json");
        std::fs::write(&path, r#"{}"#).unwrap();

        let (models, overrides) =
            super::load_user_models_with_overrides(path.to_str().unwrap()).unwrap();

        assert!(models.is_empty());
        assert!(overrides.is_empty());
    }

    #[test]
    fn load_user_models_provider_without_models() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("override-only.json");
        std::fs::write(
            &path,
            r#"{
            "providers": {
                "test": {
                    "baseUrl": "https://test.api.com/v1",
                    "apiKey": "key123"
                }
            }
        }"#,
        )
        .unwrap();

        let (models, overrides) =
            super::load_user_models_with_overrides(path.to_str().unwrap()).unwrap();

        assert!(models.is_empty());
        assert_eq!(overrides.len(), 1);
        assert_eq!(
            overrides["test"].base_url.as_deref(),
            Some("https://test.api.com/v1")
        );
    }

    // ── coverage batch: merge/override/scope arms ───────────────────────────

    #[test]
    fn provider_similarity_scores() {
        assert_eq!(provider_similarity("openai", "OPENAI"), 1.0);
        assert_eq!(provider_similarity("openai", "openai-azure"), 0.9);
        assert_eq!(provider_similarity("gpt-4", "gpt4"), 0.8);
        // Normalized containment without raw containment.
        assert_eq!(provider_similarity("gpt4o", "gpt-4o-mini"), 0.7);
        // Same known group → bonus over unrelated.
        let grouped = provider_similarity("openai", "azure");
        let unrelated = provider_similarity("openai", "zzz");
        assert!(grouped > unrelated, "{grouped} vs {unrelated}");
    }

    #[test]
    fn registry_merges_user_provider_models_and_overrides() {
        let home = crate::test_support::TestHome::new();
        let models_path = home.models_path();
        std::fs::create_dir_all(models_path.parent().unwrap()).unwrap();
        std::fs::write(
            &models_path,
            r#"{
              "providers": {
                "custom": {
                  "api": "openai-completions",
                  "baseUrl": "https://custom.example.com/v1",
                  "apiKey": "sk-custom",
                  "compat": {"providerFlag": true},
                  "thinkingLevelMap": {"high": {"budget": 1234}},
                  "models": [
                    {
                      "id": "custom-model",
                      "name": "Custom Model",
                      "supportedParameters": ["max_completion_tokens"],
                      "compat": {"modelFlag": "on"},
                      "limit": {"context": 99999, "output": 4096}
                    }
                  ]
                },
                "deepseek": {"baseUrl": "https://proxy.example.com/v1", "apiKey": "sk-override"}
              }
            }"#,
        )
        .unwrap();

        let reg = super::Registry::new();
        // User model via exact-id resolve.
        let m = reg.resolve("custom-model").expect("user model resolves");
        assert_eq!(m.provider, "custom");
        assert_eq!(m.api, "openai-completions");
        assert_eq!(m.api_key, "sk-custom");
        assert_eq!(m.context_window, 99999);
        assert_eq!(m.max_tokens, 4096);
        // supportedParameters derived the maxTokensField compat…
        assert_eq!(
            m.compat.get("maxTokensField").and_then(|v| v.as_str()),
            Some("max_completion_tokens")
        );
        // …model-level compat overrides per-key…
        assert_eq!(
            m.compat.get("modelFlag").and_then(|v| v.as_str()),
            Some("on")
        );
        // …and provider-level compat / thinking map filled the rest.
        assert_eq!(m.compat.get("providerFlag"), Some(&json!(true)));
        assert_eq!(
            m.thinking_level_map.get("high"),
            Some(&json!({"budget": 1234}))
        );

        // Provider override rewrites the builtin model's endpoint + key.
        let gpt = reg
            .resolve("deepseek/deepseek-chat")
            .expect("builtin resolves");
        assert_eq!(gpt.base_url, "https://proxy.example.com/v1");
        assert_eq!(gpt.api_key, "sk-override");
    }

    #[test]
    fn registry_resolve_scope_matches_globs_and_providers() {
        let home = crate::test_support::TestHome::new();
        let auth_path = home.auth_path();
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        std::fs::write(
            &auth_path,
            r#"{"openai": {"type": "api_key", "key": "sk-x"},
                "deepseek": {"type": "api_key", "key": "sk-y"}}"#,
        )
        .unwrap();
        let reg = super::Registry::new();
        let auth = crate::AuthStore::load();

        let scoped = reg.resolve_scope(&["deepseek/*".to_string()], &auth);
        assert!(!scoped.is_empty());
        assert!(scoped.iter().all(|id| !id.is_empty()));

        let by_provider = reg.resolve_scope(&["deepseek".to_string()], &auth);
        assert!(!by_provider.is_empty());

        let exact = reg.resolve_scope(&["deepseek-chat".to_string()], &auth);
        assert!(exact.contains(&"deepseek-chat".to_string()));

        assert!(reg
            .resolve_scope(&["no-such-provider/*".to_string()], &auth)
            .is_empty());
    }

    #[test]
    fn default_model_prefers_user_choice_when_credentialed() {
        let home = crate::test_support::TestHome::new();
        let auth_path = home.auth_path();
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        std::fs::write(
            &auth_path,
            r#"{"deepseek": {"type": "api_key", "key": "sk-x"}}"#,
        )
        .unwrap();
        std::fs::write(
            home.settings_path(),
            r#"{"defaultModel": "deepseek/deepseek-chat"}"#,
        )
        .unwrap();
        let reg = super::Registry::new();
        let chosen = super::get_default_model_with(&reg).expect("a default resolves");
        assert_eq!(chosen, "deepseek/deepseek-chat");
    }

    #[test]
    fn enrich_user_models_fills_from_builtin_match() {
        let builtins = vec![Model {
            reasoning: true,
            headers: [("X-Custom".to_string(), "v".to_string())]
                .into_iter()
                .collect(),
            ..make_model("gpt-4", "openai")
        }];
        let mut user = vec![Model {
            thinking_level_map: [("high".to_string(), json!({"budget": 5}))]
                .into_iter()
                .collect(),
            ..make_model("gpt-4", "custom")
        }];
        enrich_user_models(&mut user, &builtins);
        assert!(user[0].reasoning, "adopted from builtin");
        assert_eq!(
            user[0].headers.get("X-Custom").map(String::as_str),
            Some("v")
        );
        // Existing thinking_level_map entries win; missing keys are merged.
        assert_eq!(
            user[0].thinking_level_map.get("high"),
            Some(&json!({"budget": 5}))
        );
    }

    #[test]
    fn enrich_merges_thinking_level_map_key_by_key() {
        let builtins = vec![make_builtin_deepseek_v4()];
        // User already carries one thinking_level_map entry → merge (not
        // takeover): builtin keys are added, user keys preserved.
        let mut models = vec![Model {
            id: "deepseek-v4-pro".to_string(),
            provider: "custom-provider".to_string(),
            thinking_level_map: {
                let mut m = HashMap::new();
                m.insert("high".to_string(), json!("user-high"));
                m
            },
            ..Default::default()
        }];

        enrich_user_models(&mut models, &builtins);

        let user = &models[0];
        assert_eq!(
            user.thinking_level_map.get("high").and_then(|v| v.as_str()),
            Some("user-high"),
            "user entry wins over builtin"
        );
        assert_eq!(
            user.thinking_level_map.get("xhigh").and_then(|v| v.as_str()),
            Some("max"),
            "builtin-only key is merged in"
        );
    }

    #[test]
    fn apply_override_with_only_api_key_leaves_base_url() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "testprov".to_string(),
            ProviderOverride {
                base_url: None,
                api_key: Some("override-key".to_string()),
            },
        );
        let reg = Registry {
            builtin: std::sync::Arc::new(vec![]),
            user: vec![],
            provider_overrides: overrides,
        };
        let mut model = Model {
            provider: "testprov".to_string(),
            base_url: "https://original.example".to_string(),
            api_key: "original-key".to_string(),
            ..Default::default()
        };
        reg.apply_override(&mut model);
        assert_eq!(model.base_url, "https://original.example");
        assert_eq!(model.api_key, "override-key");
    }

    #[test]
    fn all_models_user_model_replaces_builtin_with_same_id() {
        let builtin = make_model("shared-id", "testprov");
        let mut user = make_model("shared-id", "testprov");
        user.name = "User Override".to_string();
        let reg = Registry {
            builtin: std::sync::Arc::new(vec![builtin]),
            user: vec![user],
            provider_overrides: HashMap::new(),
        };
        let models = reg.all_models();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "User Override");
    }

    #[test]
    fn builtin_provider_summaries_skips_hidden_future_and_providerless() {
        let mut hidden = make_model("hidden-model", "testprov");
        hidden.hide = true;
        let future = make_model("future-model", "future");
        let no_provider = make_model("orphan-model", "");
        let reg = Registry {
            builtin: std::sync::Arc::new(vec![hidden, future, no_provider, make_model("visible", "testprov")]),
            user: vec![],
            provider_overrides: HashMap::new(),
        };
        let summaries = reg.builtin_provider_summaries();
        assert_eq!(summaries.len(), 1);
        assert!(summaries.contains_key("testprov"));
    }

    #[test]
    fn registry_default_matches_new() {
        let reg = Registry::default();
        assert!(!reg.all_models().is_empty());
    }

    #[test]
    fn registry_injects_future_models_from_disk_cache() {
        // Serialize against future.rs tests and start from cold caches so the
        // disk seed below is the only catalog source.
        let _cache_guard = super::future::future_models_test_lock();
        super::future::reset_future_caches_for_tests();
        let home = crate::test_support::TestHome::new();
        // Auth present (dead base URL — no network) + a disk-cached catalog
        // entry whose ID matches no builtin → pushed into the registry.
        let auth_path = home.auth_path();
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        std::fs::write(
            &auth_path,
            r#"{"future": {"type": "api_key", "key": "k", "base_url": "http://127.0.0.1:1/api"}}"#,
        )
        .unwrap();
        let future_model = Model {
            id: "future-cov-model".to_string(),
            provider: "future".to_string(),
            ..Default::default()
        };
        let cache = serde_json::json!({
            "fetched_at": 1,
            "models": [serde_json::to_value(&future_model).unwrap()],
        });
        std::fs::write(
            home.path().join(".future/agent/.future-models-cache.json"),
            serde_json::to_string(&cache).unwrap(),
        )
        .unwrap();

        let reg = Registry::new();
        assert!(
            reg.all_models()
                .iter()
                .any(|m| m.id == "future-cov-model" && m.provider == "future"),
            "cached future model must be injected into the registry"
        );
    }
}
