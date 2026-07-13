//! Model registry — mirrors Go internal/modelregistry/
//!
//! Handles model catalog (built-in + user-provided) and model resolution.

pub mod generated;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Default Future API base URL (platform URL + /api)
const DEFAULT_FUTURE_BASE_URL: &str = "https://future-os.cn/api";

/// After a refresh attempt, don't re-hit the network for this long. `Registry::new()`
/// rebuilds on the startup path and on every RPC, so without this backoff each
/// rebuild would re-probe a slow/unreachable Future API.
const FUTURE_MODELS_REFRESH_BACKOFF: u64 = 30;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::RwLock;

static FUTURE_MODELS_LAST_ATTEMPT: AtomicU64 = AtomicU64::new(0);
static FUTURE_MODELS_REFRESH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// In-process cache so background refreshes take effect immediately on the
/// next `Registry::new()` call (GUI polls every 10s), without waiting for
/// the file cache to be read back from disk.
static FUTURE_MODELS_MEMORY_CACHE: RwLock<Option<FutureModelsCache>> = RwLock::new(None);

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Kick off a one-at-a-time background refresh of the Future model catalog,
/// respecting a backoff window. Never blocks the caller — the fetched models are
/// written to both the file cache and the in-process memory cache, so the next
/// registry rebuild picks up fresh data immediately.
fn spawn_future_models_refresh(api_key: &str, base_url: &str) {
    let now = now_secs();
    if now.saturating_sub(FUTURE_MODELS_LAST_ATTEMPT.load(Ordering::Relaxed))
        < FUTURE_MODELS_REFRESH_BACKOFF
    {
        return;
    }
    // Single-flight: bail if a refresh is already running.
    if FUTURE_MODELS_REFRESH_IN_FLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    FUTURE_MODELS_LAST_ATTEMPT.store(now, Ordering::Relaxed);

    let api_key = api_key.to_string();
    let base_url = base_url.to_string();
    std::thread::spawn(move || {
        if let Some(models) = fetch_future_models(&api_key, &base_url) {
            // Persist to disk AND update in-process cache so the next
            // `Registry::new()` sees fresh models without re-reading the file.
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let cache = FutureModelsCache {
                fetched_at: now,
                models,
            };
            save_future_models_cache_inner(&cache);
            if let Ok(mut mem) = FUTURE_MODELS_MEMORY_CACHE.write() {
                *mem = Some(cache);
            }
        }
        FUTURE_MODELS_REFRESH_IN_FLIGHT.store(false, Ordering::Release);
    });
}

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

/// FutureModelsCachePath returns ~/.future/agent/.future-models-cache.json.
fn future_models_cache_path() -> String {
    let home = dirs::home_dir()
        .map(|h| h.join(".future/agent/.future-models-cache.json"))
        .unwrap_or_else(|| {
            std::path::PathBuf::from("/tmp/.future/agent/.future-models-cache.json")
        });
    home.to_string_lossy().to_string()
}

/// Future models cache format
#[derive(Debug, Serialize, Deserialize)]
struct FutureModelsCache {
    fetched_at: u64,
    models: Vec<Model>,
}

/// Resolve Future provider base URL from auth.json or default
fn resolve_future_base_url() -> String {
    // Try to read base_url or platform_base_url from auth.json
    let auth_path = dirs::home_dir()
        .map(|h| h.join(".future/agent/auth.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/.future/agent/auth.json"));

    if let Ok(contents) = std::fs::read_to_string(&auth_path) {
        if let Ok(auth) = serde_json::from_str::<HashMap<String, serde_json::Value>>(&contents) {
            if let Some(future) = auth.get("future") {
                // legacy: explicit base_url in auth.json
                if let Some(base_url) = future.get("base_url").and_then(|v| v.as_str()) {
                    return base_url.trim_end_matches('/').to_string();
                }
                // new: derive from platform_base_url
                if let Some(platform_url) = future.get("platform_base_url").and_then(|v| v.as_str())
                {
                    return format!("{}/api", platform_url.trim_end_matches('/'));
                }
            }
        }
    }

    DEFAULT_FUTURE_BASE_URL.to_string()
}

/// Response format from Future server /v1/models endpoint
#[derive(Debug, Deserialize)]
struct FutureModelsResponse {
    data: Option<Vec<FutureModelEntry>>,
}

#[derive(Debug, Deserialize)]
struct FutureModelEntry {
    id: String,
    name: Option<String>,
    #[serde(alias = "ContextWindow", alias = "contextWindow")]
    context_length: Option<i64>,
    architecture: Option<FutureArchitecture>,
    pricing: Option<FuturePricing>,
    supported_parameters: Option<Vec<String>>,
    #[allow(dead_code)]
    knowledge_cutoff: Option<String>,
    #[allow(dead_code)]
    provider: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FutureArchitecture {
    modality: Option<String>,
    #[allow(dead_code)]
    tokenizer: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FuturePricing {
    #[allow(dead_code)]
    currency: Option<String>,
    price_unit: Option<i64>,
    prices: Option<Vec<FuturePriceRule>>,
}

#[derive(Debug, Deserialize)]
struct FuturePriceRule {
    input: Option<String>,
    output: Option<String>,
    input_cache_read: Option<String>,
    input_cache_write: Option<String>,
}

/// Fetch models from Future server.
/// Runs in a dedicated thread to isolate reqwest::blocking's internal runtime.
fn fetch_future_models(api_key: &str, base_url: &str) -> Option<Vec<Model>> {
    let api_key = api_key.to_string();
    let base_url = base_url.to_string();

    std::thread::spawn(move || {
        let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
        let response = reqwest::blocking::Client::new()
            .get(&url)
            .header("Authorization", format!("Bearer {}", &api_key))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .ok()?;

        if !response.status().is_success() {
            return None;
        }

        let body: serde_json::Value = response.json().ok()?;

        // Handle both array response and {data: [...]} response
        let entries: Vec<FutureModelEntry> =
            if let Ok(resp) = serde_json::from_value::<FutureModelsResponse>(body.clone()) {
                resp.data.unwrap_or_default()
            } else if let Ok(arr) = serde_json::from_value::<Vec<FutureModelEntry>>(body) {
                arr
            } else {
                return None;
            };

        let models_url = format!("{}/v1", base_url.trim_end_matches('/'));

        let models: Vec<Model> = entries
            .into_iter()
            .map(|entry| convert_future_model(entry, &models_url))
            .collect();

        Some(models)
    })
    .join()
    .ok()?
}

/// Derive compat and thinking_level_map for a Future platform model from its
/// supported_parameters list and tokenizer. This mirrors the manual compat_json /
/// tlm_json entries in generated/mod.rs for the direct-provider case.
fn derive_thinking_compat(
    supported_params: &[String],
    tokenizer: Option<&str>,
) -> (
    HashMap<String, serde_json::Value>,
    HashMap<String, serde_json::Value>,
) {
    use std::collections::HashMap;

    let mut compat: HashMap<String, serde_json::Value> = HashMap::new();
    let mut tlm: HashMap<String, serde_json::Value> = HashMap::new();

    let has = |s: &str| supported_params.iter().any(|p| p == s);
    let is_glm = tokenizer
        .map(|t| t.eq_ignore_ascii_case("GLM"))
        .unwrap_or(false);

    if is_glm {
        // GLM / Z.AI models: enable_thinking toggle.
        compat.insert("thinkingFormat".into(), serde_json::json!("zai"));
        // GLM supports reasoning_effort alongside enable_thinking
        compat.insert("supportsReasoningEffort".into(), serde_json::json!(true));
    } else if has("enable_thinking") {
        // Qwen family: enable_thinking + thinking_budget
        compat.insert("thinkingFormat".into(), serde_json::json!("qwen"));
        // Qwen supports reasoning_effort alongside enable_thinking
        compat.insert("supportsReasoningEffort".into(), serde_json::json!(true));
    } else if has("reasoning_split") {
        // MiniMax M3: reasoning_split only, no depth control
        compat.insert(
            "thinkingFormat".into(),
            serde_json::json!("reasoning-split"),
        );
    } else if has("thinking") || has("reasoning_effort") || has("reasoning") {
        // DeepSeek / Doubao / Kimi K2.6:
        // thinking toggle + reasoning_effort for depth
        compat.insert("thinkingFormat".into(), serde_json::json!("deepseek"));
        tlm.insert("high".into(), serde_json::json!("high"));
        tlm.insert("xhigh".into(), serde_json::json!("max"));
    }
    // else: no thinking parameters → empty compat (model doesn't support thinking)

    // Models that declare max_completion_tokens (e.g. o1/o3/gpt-5 reasoning models)
    // must use it instead of max_tokens
    if has("max_completion_tokens") {
        compat.insert(
            "maxTokensField".into(),
            serde_json::json!("max_completion_tokens"),
        );
    }

    (compat, tlm)
}

/// Convert Future server model entry to agent Model
fn convert_future_model(entry: FutureModelEntry, base_url: &str) -> Model {
    let supported_params = entry.supported_parameters.unwrap_or_default();
    // A model supports reasoning if it has ANY thinking-related parameter.
    let reasoning = supported_params.iter().any(|p| {
        matches!(
            p.as_str(),
            "reasoning"
                | "reasoning_effort"
                | "include_reasoning"
                | "thinking"
                | "enable_thinking"
                | "reasoning_split"
                | "thinking_budget"
        )
    });

    // Derive compat and thinking_level_map from supported_parameters.
    let tokenizer = entry
        .architecture
        .as_ref()
        .and_then(|a| a.tokenizer.as_deref());
    let (compat, thinking_level_map) = derive_thinking_compat(&supported_params, tokenizer);

    let (input, output) = entry
        .architecture
        .as_ref()
        .and_then(|a| a.modality.as_ref())
        .map(|m| {
            let parts: Vec<&str> = m.split("->").collect();
            let input_str = parts.first().unwrap_or(&"text");
            let output_str = parts.get(1).unwrap_or(&"text");

            let input: Vec<String> = input_str
                .split('+')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let output: Vec<String> = output_str
                .split('+')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            (input, output)
        })
        .unwrap_or_else(|| (vec!["text".to_string()], vec!["text".to_string()]));

    let context_window = entry.context_length.map(|v| v as i32).unwrap_or(128000);

    // Parse pricing
    let (cost_input, cost_output, cost_cache_read, cost_cache_write) = entry
        .pricing
        .as_ref()
        .and_then(|p| p.prices.as_ref())
        .and_then(|prices| prices.first())
        .map(|rule| {
            let price_unit = entry
                .pricing
                .as_ref()
                .and_then(|p| p.price_unit)
                .unwrap_or(1)
                .max(1) as f64;
            (
                parse_price_string(&rule.input, price_unit),
                parse_price_string(&rule.output, price_unit),
                parse_price_string(&rule.input_cache_read, price_unit),
                parse_price_string(&rule.input_cache_write, price_unit),
            )
        })
        .unwrap_or((0.0, 0.0, 0.0, 0.0));

    let name = entry.name.unwrap_or_else(|| entry.id.clone());

    Model {
        id: entry.id,
        name: name.clone(),
        provider: "future".to_string(),
        api: "openai-completions".to_string(),
        base_url: base_url.to_string(),
        api_key: String::new(), // Will be resolved from auth_store at runtime
        reasoning,
        input,
        output,
        context_window,
        max_tokens: 16384,
        cost: Cost {
            input: cost_input,
            output: cost_output,
            cache_read: cost_cache_read,
            cache_write: cost_cache_write,
        },
        compat,
        thinking_level_map,
        headers: HashMap::new(),
        hide: false,
    }
}

/// Parse price string to per-million-tokens cost
fn parse_price_string(value: &Option<String>, price_unit: f64) -> f64 {
    value
        .as_ref()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|v| v * 1_000_000.0 / price_unit)
        .unwrap_or(0.0)
}

/// Load cached future models
fn load_future_models_cache() -> Option<FutureModelsCache> {
    let path = future_models_cache_path();
    let contents = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Save future models cache to disk (internal helper).
fn save_future_models_cache_inner(cache: &FutureModelsCache) {
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        let path = future_models_cache_path();
        let _ = std::fs::write(&path, json);
    }
}

/// Get future models with caching logic.
///
/// Never blocks the caller — always returns whatever cache is available
/// immediately (in-memory first, then on-disk) and triggers a background
/// refresh.  When the background refresh completes, it writes fresh data
/// into the in-process memory cache so the very next `Registry::new()`
/// call (GUI polls models every 10s) picks up the updated catalog without
/// re-reading the file.
fn get_future_models_with_cache(api_key: &str, base_url: &str) -> Vec<Model> {
    // Always kick off a background refresh (backoff + single-flight prevent
    // hammering the server).  This ensures that when the user removes models
    // from the API, the client picks up the change within one backoff window
    // instead of waiting for an hour-long TTL.
    spawn_future_models_refresh(api_key, base_url);

    // Prefer the in-process memory cache — it is updated by completed
    // background refreshes and avoids reading the file from disk.
    if let Ok(mem) = FUTURE_MODELS_MEMORY_CACHE.read() {
        if let Some(ref cache) = *mem {
            return cache.models.clone();
        }
    }

    // Fall back to on-disk cache.
    if let Some(cache) = load_future_models_cache() {
        // Seed the in-process cache so we don't keep hitting disk.
        if let Ok(mut mem) = FUTURE_MODELS_MEMORY_CACHE.write() {
            if mem.is_none() {
                *mem = Some(cache);
            }
        }
        // Re-read to return (avoids clone before moving into mem).
        if let Ok(mem) = FUTURE_MODELS_MEMORY_CACHE.read() {
            if let Some(ref cache) = *mem {
                return cache.models.clone();
            }
        }
    }

    // First login on this machine: no cache at all.  The background refresh
    // kicked off above will populate both caches.
    Vec::new()
}

/// Get the first available model, or None.
pub fn get_default_model() -> Option<String> {
    let registry = Registry::new();
    let auth = crate::AuthStore::load();
    registry
        .all_models()
        .into_iter()
        .find(|m| !m.api_key.is_empty() || auth.get(&m.provider).is_some())
        .map(|m| format!("{}/{}", m.provider, m.id))
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
                let (derived_compat, derived_tlm) =
                    derive_thinking_compat(supported_params, None);
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

#[cfg(test)]
mod tests {
    use super::derive_thinking_compat;

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

        let mut builtin = builtin_models();

        // Load Future provider models dynamically if auth is available
        let auth_store = crate::AuthStore::load();
        let future_provider_override = if let Some(future_key) = auth_store.get("future") {
            let base_url = resolve_future_base_url();
            let future_models = get_future_models_with_cache(&future_key, &base_url);

            // Add future models to builtin (they override same-ID builtin models)
            for fm in future_models {
                if let Some(idx) = builtin.iter().position(|m| m.id == fm.id) {
                    builtin[idx] = fm;
                } else {
                    builtin.push(fm);
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
        // Filter out hidden models from the list
        models.retain(|m| !m.hide);
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
