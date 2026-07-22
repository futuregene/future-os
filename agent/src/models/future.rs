//! Future platform model catalog: fetch, cache, and conversion.
//!
//! Split out of `models/mod.rs` — everything here concerns the Future
//! platform's `/v1/models` endpoint: a background single-flight refresh that
//! never blocks callers, a two-tier cache (in-process + on-disk JSON), and
//! conversion from the wire format into registry `Model`s.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::{Cost, Model};

/// Default Future API base URL (platform URL + /api)
const DEFAULT_FUTURE_BASE_URL: &str = "https://future-os.cn/api";

/// After a refresh attempt, don't re-hit the network for this long. `Registry::new()`
/// rebuilds on the startup path and on every RPC, so without this backoff each
/// rebuild would re-probe a slow/unreachable Future API.
const FUTURE_MODELS_REFRESH_BACKOFF: u64 = 30;

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
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(models) = fetch_future_models(&api_key, &base_url) {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let cache = FutureModelsCache {
                    fetched_at: now,
                    models,
                };
                save_future_models_cache_inner(&cache);
                *FUTURE_MODELS_MEMORY_CACHE.write() = Some(cache);
            }
        }));
        if let Err(e) = result {
            let msg = e
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| e.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            tracing::warn!("Future models background refresh panicked: {msg}");
        }
        // Always reset the flag — a panic must not permanently block
        // future refreshes.
        FUTURE_MODELS_REFRESH_IN_FLIGHT.store(false, Ordering::Release);
    });
}

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
pub(super) fn resolve_future_base_url() -> String {
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
            .header("Authorization", format!("Bearer {}", api_key))
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
pub(super) fn derive_thinking_compat(
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
    } else if has("thinking")
        || has("reasoning_effort")
        || has("reasoning")
        || has("include_reasoning")
    {
        // DeepSeek / Doubao / Kimi K2.6 / Anthropic Claude:
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
pub(super) fn get_future_models_with_cache(api_key: &str, base_url: &str) -> Vec<Model> {
    // Always kick off a background refresh (backoff + single-flight prevent
    // hammering the server).  This ensures that when the user removes models
    // from the API, the client picks up the change within one backoff window
    // instead of waiting for an hour-long TTL.
    spawn_future_models_refresh(api_key, base_url);

    // Prefer the in-process memory cache — it is updated by completed
    // background refreshes and avoids reading the file from disk.
    if let Some(ref cache) = *FUTURE_MODELS_MEMORY_CACHE.read() {
        return cache.models.clone();
    }

    // Fall back to on-disk cache.
    if let Some(cache) = load_future_models_cache() {
        // Seed the in-process cache so we don't keep hitting disk.
        {
            let mut mem = FUTURE_MODELS_MEMORY_CACHE.write();
            if mem.is_none() {
                *mem = Some(cache);
            }
        }
        // Re-read to return (avoids clone before moving into mem).
        if let Some(ref cache) = *FUTURE_MODELS_MEMORY_CACHE.read() {
            return cache.models.clone();
        }
    }

    // First login on this machine: no cache at all.  The background refresh
    // kicked off above will populate both caches.
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── parse_price_string ────────────────────────────────────────────────

    #[test]
    fn parse_price_valid() {
        let val = Some("0.00025".to_string());
        assert_eq!(parse_price_string(&val, 1.0), 250.0); // 0.00025 * 1M / 1
    }

    #[test]
    fn parse_price_with_unit() {
        let val = Some("0.001".to_string());
        assert_eq!(parse_price_string(&val, 1000.0), 1.0); // 0.001 * 1M / 1000
    }

    #[test]
    fn parse_price_none() {
        assert_eq!(parse_price_string(&None, 1.0), 0.0);
    }

    #[test]
    fn parse_price_invalid_string() {
        let val = Some("not_a_number".to_string());
        assert_eq!(parse_price_string(&val, 1.0), 0.0);
    }

    #[test]
    fn parse_price_empty_string() {
        let val = Some("".to_string());
        assert_eq!(parse_price_string(&val, 1.0), 0.0);
    }

    // ─── derive_thinking_compat ────────────────────────────────────────────

    #[test]
    fn glm_model_gets_zai_format() {
        let params: Vec<String> = vec![];
        let (compat, tlm) = derive_thinking_compat(&params, Some("GLM"));
        assert_eq!(
            compat.get("thinkingFormat").unwrap(),
            &serde_json::json!("zai")
        );
        assert_eq!(
            compat.get("supportsReasoningEffort").unwrap(),
            &serde_json::json!(true)
        );
        assert!(tlm.is_empty());
    }

    #[test]
    fn glm_case_insensitive() {
        let params: Vec<String> = vec![];
        let (compat, _) = derive_thinking_compat(&params, Some("glm"));
        assert_eq!(
            compat.get("thinkingFormat").unwrap(),
            &serde_json::json!("zai")
        );
    }

    #[test]
    fn qwen_model_gets_qwen_format() {
        let params: Vec<String> = vec!["enable_thinking".to_string()];
        let (compat, _) = derive_thinking_compat(&params, None);
        assert_eq!(
            compat.get("thinkingFormat").unwrap(),
            &serde_json::json!("qwen")
        );
        assert_eq!(
            compat.get("supportsReasoningEffort").unwrap(),
            &serde_json::json!(true)
        );
    }

    #[test]
    fn reasoning_split_gets_split_format() {
        let params: Vec<String> = vec!["reasoning_split".to_string()];
        let (compat, tlm) = derive_thinking_compat(&params, None);
        assert_eq!(
            compat.get("thinkingFormat").unwrap(),
            &serde_json::json!("reasoning-split")
        );
        assert!(tlm.is_empty());
    }

    #[test]
    fn deepseek_thinking_params_get_deepseek_format() {
        let params: Vec<String> = vec!["thinking".to_string()];
        let (compat, tlm) = derive_thinking_compat(&params, None);
        assert_eq!(
            compat.get("thinkingFormat").unwrap(),
            &serde_json::json!("deepseek")
        );
        assert_eq!(tlm.get("high").unwrap(), &serde_json::json!("high"));
        assert_eq!(tlm.get("xhigh").unwrap(), &serde_json::json!("max"));
    }

    #[test]
    fn reasoning_effort_alone_gets_deepseek() {
        let params: Vec<String> = vec!["reasoning_effort".to_string()];
        let (compat, _) = derive_thinking_compat(&params, None);
        assert_eq!(
            compat.get("thinkingFormat").unwrap(),
            &serde_json::json!("deepseek")
        );
    }

    #[test]
    fn include_reasoning_gets_deepseek() {
        let params: Vec<String> = vec!["include_reasoning".to_string()];
        let (compat, _) = derive_thinking_compat(&params, None);
        assert_eq!(
            compat.get("thinkingFormat").unwrap(),
            &serde_json::json!("deepseek")
        );
    }

    #[test]
    fn no_thinking_params_empty_compat() {
        let params: Vec<String> = vec!["temperature".to_string()];
        let (compat, tlm) = derive_thinking_compat(&params, None);
        assert!(compat.get("thinkingFormat").is_none());
        assert!(tlm.is_empty());
    }

    #[test]
    fn max_completion_tokens_sets_field() {
        let params: Vec<String> = vec!["max_completion_tokens".to_string()];
        let (compat, _) = derive_thinking_compat(&params, None);
        assert_eq!(
            compat.get("maxTokensField").unwrap(),
            &serde_json::json!("max_completion_tokens")
        );
    }

    #[test]
    fn empty_params_no_max_tokens_field() {
        let params: Vec<String> = vec![];
        let (compat, _) = derive_thinking_compat(&params, None);
        assert!(compat.get("maxTokensField").is_none());
    }

    // ─── resolve_future_base_url ───────────────────────────────────────────

    #[test]
    fn resolve_future_base_url_returns_default() {
        // When auth.json doesn't have future.base_url or future.platform_base_url,
        // should return the default. (In test env, may or may not have auth.json.)
        let url = resolve_future_base_url();
        assert!(!url.is_empty());
        assert!(url.starts_with("https://"));
    }

    // ─── convert_future_model (via public interface) ───────────────────────

    #[test]
    fn convert_model_reasoning_detection() {
        let entry = FutureModelEntry {
            id: "test-model".to_string(),
            name: Some("Test".to_string()),
            context_length: Some(128000),
            architecture: Some(FutureArchitecture {
                modality: Some("text+image->text".to_string()),
                tokenizer: None,
            }),
            pricing: None,
            supported_parameters: Some(vec!["thinking".to_string(), "reasoning_effort".to_string()]),
            knowledge_cutoff: None,
            provider: None,
        };
        let model = convert_future_model(entry, "https://api.example.com/v1");
        assert!(model.reasoning);
        assert_eq!(model.provider, "future");
    }

    #[test]
    fn convert_model_no_reasoning() {
        let entry = FutureModelEntry {
            id: "plain-model".to_string(),
            name: None,
            context_length: Some(64000),
            architecture: None,
            pricing: None,
            supported_parameters: Some(vec!["temperature".to_string()]),
            knowledge_cutoff: None,
            provider: None,
        };
        let model = convert_future_model(entry, "https://api.example.com/v1");
        assert!(!model.reasoning);
        assert_eq!(model.name, "plain-model"); // falls back to id
    }

    #[test]
    fn convert_model_image_input() {
        let entry = FutureModelEntry {
            id: "vision".to_string(),
            name: Some("Vision".to_string()),
            context_length: None,
            architecture: Some(FutureArchitecture {
                modality: Some("text+image->text".to_string()),
                tokenizer: None,
            }),
            pricing: None,
            supported_parameters: None,
            knowledge_cutoff: None,
            provider: None,
        };
        let model = convert_future_model(entry, "https://api.example.com/v1");
        assert!(model.input.iter().any(|i| i == "image"));
        assert_eq!(model.context_window, 128000); // default
    }

    #[test]
    fn convert_model_pricing() {
        let entry = FutureModelEntry {
            id: "priced".to_string(),
            name: None,
            context_length: Some(128000),
            architecture: None,
            pricing: Some(FuturePricing {
                currency: None,
                price_unit: Some(1),
                prices: Some(vec![FuturePriceRule {
                    input: Some("0.001".to_string()),
                    output: Some("0.002".to_string()),
                    input_cache_read: Some("0.0005".to_string()),
                    input_cache_write: None,
                }]),
            }),
            supported_parameters: None,
            knowledge_cutoff: None,
            provider: None,
        };
        let model = convert_future_model(entry, "https://api.example.com/v1");
        assert_eq!(model.cost.input, 1000.0); // 0.001 * 1M / 1
        assert_eq!(model.cost.output, 2000.0);
        assert_eq!(model.cost.cache_read, 500.0);
        assert_eq!(model.cost.cache_write, 0.0); // None → 0
    }
}
