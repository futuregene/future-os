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

/// Default Future platform host (production environment). The production and
/// test environments differ only in the host — see the shared contract below.
const DEFAULT_FUTURE_PLATFORM_URL: &str = "https://future-os.cn";

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

/// Test-only: serialize the single-flight gate's check-and-acquire against
/// concurrent `Registry::new()` threads, which share these process-global
/// statics but do NOT hold `future_models_test_lock`. Without this, a refresh
/// spawned by an unrelated test can stamp `LAST_ATTEMPT` / hold `IN_FLIGHT`
/// inside a future-models test's reset→assert window and flip its assertions.
#[cfg(test)]
static FUTURE_REFRESH_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    // Test builds serialize the single-flight gate's check-and-acquire against
    // concurrent `Registry::new()` threads, which share these process-global
    // statics but do NOT hold `future_models_test_lock`. Without the gate, a
    // refresh spawned by an unrelated test can stamp `LAST_ATTEMPT` or hold
    // `IN_FLIGHT` inside a future-models test's reset→assert window.
    #[cfg(test)]
    let _gate = FUTURE_REFRESH_GATE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _ = spawn_future_models_refresh_unlocked(api_key, base_url);
}

/// Gate already held by the caller (a test that owns the [`FUTURE_REFRESH_GATE`]
/// guard, or the gated wrapper above). The actual backoff + single-flight gate.
/// Returns the refresh thread's handle when a refresh was actually spawned.
fn spawn_future_models_refresh_unlocked(
    api_key: &str,
    base_url: &str,
) -> Option<std::thread::JoinHandle<()>> {
    let now = now_secs();
    if now.saturating_sub(FUTURE_MODELS_LAST_ATTEMPT.load(Ordering::Relaxed))
        < FUTURE_MODELS_REFRESH_BACKOFF
    {
        return None;
    }
    // Single-flight: bail if a refresh is already running.
    if FUTURE_MODELS_REFRESH_IN_FLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return None;
    }
    FUTURE_MODELS_LAST_ATTEMPT.store(now, Ordering::Relaxed);

    let api_key = api_key.to_string();
    let base_url = base_url.to_string();
    Some(std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            #[cfg(test)]
            inject_test_panic();
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
    }))
}

/// Test seam: force the background-refresh closure down its panic-recovery
/// path. 0 = off, 1 = &str payload, 2 = String payload, 3 = other payload.
#[cfg(test)]
static REFRESH_TEST_PANIC: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
fn inject_test_panic() {
    match REFRESH_TEST_PANIC.swap(0, Ordering::Relaxed) {
        1 => panic!("injected refresh panic"),
        2 => std::panic::panic_any(String::from("injected owned panic")),
        3 => std::panic::panic_any(42i32),
        _ => {}
    }
}

fn future_models_cache_path() -> String {
    future_models_cache_path_in(dirs::home_dir())
}

/// `future_models_cache_path` with the home dir injected, so the no-home
/// fallback arm is testable (a real host always resolves one).
fn future_models_cache_path_in(home: Option<std::path::PathBuf>) -> String {
    home.map(|h| h.join(".future/agent/.future-models-cache.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/.future/agent/.future-models-cache.json"))
        .to_string_lossy()
        .to_string()
}

/// Future models cache format
#[derive(Debug, Serialize, Deserialize)]
struct FutureModelsCache {
    fetched_at: u64,
    models: Vec<Model>,
}

/// Resolve the Future **platform root** (no `/api`) from `auth.json`,
/// following the shared contract in `rpc/proto/future.proto` ("Future Platform URL
/// Resolution") — keep aligned with the GUI implementation in
/// `desktop/src-tauri/src/future_platform.rs`:
///   1. `future.base_url`, with a trailing `/api` stripped (the storage format
///      every writer uses: `base_url = {platform}/api`)
///   2. `future.platform_base_url` (legacy field)
///   3. [`DEFAULT_FUTURE_PLATFORM_URL`]
fn platform_url_from_auth(auth: &serde_json::Value) -> Option<String> {
    let future = auth.get("future")?;
    if let Some(base_url) = future
        .get("base_url")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        let trimmed = base_url.trim_end_matches('/');
        let platform = trimmed.strip_suffix("/api").unwrap_or(trimmed);
        return Some(platform.trim_end_matches('/').to_string());
    }
    future
        .get("platform_base_url")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|url| url.trim_end_matches('/').to_string())
}

fn resolve_future_platform_url() -> String {
    // Try to read base_url or platform_base_url from auth.json
    let auth_path = dirs::home_dir()
        .map(|h| h.join(".future/agent/auth.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/.future/agent/auth.json"));

    std::fs::read_to_string(&auth_path)
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
        .and_then(|auth| platform_url_from_auth(&auth))
        .unwrap_or_else(|| DEFAULT_FUTURE_PLATFORM_URL.to_string())
}

/// Resolve Future provider model-API base URL from auth.json or default:
/// `{platform}/api` (model endpoints hang off it as `{base}/v1/...`).
pub(super) fn resolve_future_base_url() -> String {
    format!("{}/api", resolve_future_platform_url())
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
    #[serde(
        alias = "maxTokens",
        alias = "max_output",
        alias = "maxOutput",
        alias = "max_output_tokens"
    )]
    max_tokens: Option<i64>,
    architecture: Option<FutureArchitecture>,
    pricing: Option<FuturePricing>,
    supported_parameters: Option<Vec<String>>,
    #[allow(dead_code)]
    knowledge_cutoff: Option<String>,
    #[allow(dead_code)]
    provider: Option<String>,
    description: Option<String>,
    description_en: Option<String>,
    recommended: Option<bool>,
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
    let max_tokens = entry
        .max_tokens
        .filter(|value| *value > 0)
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or(0);

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
        max_tokens,
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
        description: entry.description,
        description_en: entry.description_en,
        recommended: entry.recommended.unwrap_or(false),
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
    // `FutureModelsCache` is a flat serializable struct (string-keyed maps,
    // integer/bool/string fields); serde_json cannot fail here, so an
    // `if let Ok` guard would leave an unreachable Err arm.
    let json =
        serde_json::to_string_pretty(cache).expect("FutureModelsCache serialization cannot fail");
    let path = std::path::PathBuf::from(future_models_cache_path());
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Atomic replace: write a sibling temp file and rename it over the
    // target. A plain truncate+write lets a concurrent reader (the GUI
    // polls models every 10s; a test reading right after a background
    // refresh) observe an empty/partial file. Rename swaps the contents
    // atomically within the same directory on all supported platforms.
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
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
        // Re-read to return (avoids clone before moving into mem). The seed
        // above (or a concurrently completed refresh) guarantees the memory
        // cache is populated here.
        return FUTURE_MODELS_MEMORY_CACHE
            .read()
            .as_ref()
            .expect("memory cache seeded above")
            .models
            .clone();
    }

    // First login on this machine: no cache at all.  The background refresh
    // kicked off above will populate both caches.
    Vec::new()
}

/// Synchronously fetch the Future provider's models from the platform and
/// write them to both the on-disk and in-memory caches. Returns `true` when the
/// caches were populated, `false` when there is no Future key or the platform
/// could not be reached.
///
/// This is the dedicated post-login initialization path. Unlike
/// [`get_future_models_with_cache`] — which never blocks and returns an empty
/// list on a cold cache, leaving the just-rebuilt registry model-less until a
/// later refresh — this call blocks (up to the fetch timeout) so the caller can
/// rebuild the registry against a *warm* cache and serve a complete model list
/// immediately. It does not touch the shared registry itself; the caller
/// (`sync_future_models` RPC) rebuilds it afterwards.
pub(super) fn sync_future_models_cache() -> bool {
    let auth_store = crate::AuthStore::load();
    let Some(future_key) = auth_store.get("future") else {
        return false;
    };
    let base_url = resolve_future_base_url();

    let Some(models) = fetch_future_models(&future_key, &base_url) else {
        return false;
    };

    let cache = FutureModelsCache {
        fetched_at: now_secs(),
        models,
    };
    save_future_models_cache_inner(&cache);
    *FUTURE_MODELS_MEMORY_CACHE.write() = Some(cache);
    true
}

/// Process-global future-models caches are shared across test modules
/// (models::mod tests build a Registry that reads them) — serialize every
/// accessor through one lock.
#[cfg(test)]
pub(crate) static FUTURE_MODELS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn future_models_test_lock() -> std::sync::MutexGuard<'static, ()> {
    FUTURE_MODELS_TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

/// Reset every future-models cache static to a clean slate for tests. This
/// force-clears `IN_FLIGHT` rather than waiting out a slow in-flight refresh
/// (which may be hitting a real URL for up to 10s); a leftover thread that later
/// finishes only re-stores `false` and is otherwise harmless to these tests.
#[cfg(test)]
pub(crate) fn reset_future_caches_for_tests() {
    let _gate = FUTURE_REFRESH_GATE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    reset_future_caches_for_tests_unlocked();
}

/// Gate already held by the caller.
#[cfg(test)]
fn reset_future_caches_for_tests_unlocked() {
    FUTURE_MODELS_LAST_ATTEMPT.store(0, Ordering::Relaxed);
    FUTURE_MODELS_REFRESH_IN_FLIGHT.store(false, Ordering::Relaxed);
    *FUTURE_MODELS_MEMORY_CACHE.write() = None;
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
        assert!(!compat.contains_key("thinkingFormat"));
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
        assert!(!compat.contains_key("maxTokensField"));
    }

    // ─── platform_url_from_auth (shared URL contract) ─────────────────────

    fn auth_with(future: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "future": future })
    }

    #[test]
    fn platform_url_defaults_when_absent() {
        assert_eq!(platform_url_from_auth(&serde_json::json!({})), None);
        assert_eq!(
            platform_url_from_auth(&auth_with(serde_json::json!({}))),
            None
        );
    }

    #[test]
    fn platform_url_strips_trailing_api_from_base_url() {
        // Writers store `base_url = {platform}/api`; the platform is that minus /api.
        let auth = auth_with(serde_json::json!({ "base_url": "https://future-os.cn/api" }));
        assert_eq!(
            platform_url_from_auth(&auth).as_deref(),
            Some("https://future-os.cn")
        );

        let trailing = auth_with(serde_json::json!({ "base_url": "https://future-os.cn/api/" }));
        assert_eq!(
            platform_url_from_auth(&trailing).as_deref(),
            Some("https://future-os.cn")
        );
    }

    #[test]
    fn base_url_wins_over_platform_base_url() {
        // Same precedence as the desktop (rpc/proto/future.proto contract): the stored
        // `base_url` beats a stale `platform_base_url`.
        let auth = auth_with(serde_json::json!({
            "base_url": "https://future-os.cn/api",
            "platform_base_url": "https://stale.example.com",
        }));
        assert_eq!(
            platform_url_from_auth(&auth).as_deref(),
            Some("https://future-os.cn")
        );
    }

    #[test]
    fn platform_base_url_is_the_legacy_fallback() {
        let auth =
            auth_with(serde_json::json!({ "platform_base_url": "https://staging.example.com/" }));
        assert_eq!(
            platform_url_from_auth(&auth).as_deref(),
            Some("https://staging.example.com")
        );
    }

    #[test]
    fn base_url_without_api_suffix_is_used_as_platform() {
        // A bare host (no /api) is treated as the platform root verbatim.
        let auth = auth_with(serde_json::json!({ "base_url": "https://custom.example.com" }));
        assert_eq!(
            platform_url_from_auth(&auth).as_deref(),
            Some("https://custom.example.com")
        );
    }

    #[test]
    fn resolve_future_base_url_returns_default() {
        // Hermetic: an empty isolated $HOME (no auth.json) always resolves the
        // default. TestHome also holds the process-global HOME lock for the
        // test's duration, so a parallel fixture's auth.json (e.g. an http://
        // mock base_url) cannot leak in and flip the scheme assertion.
        let _home = crate::test_support::TestHome::new();
        let url = resolve_future_base_url();
        assert!(!url.is_empty());
        assert!(url.starts_with("https://"));
        assert!(url.ends_with("/api"));
    }

    // ─── convert_future_model (via public interface) ───────────────────────

    #[test]
    fn convert_model_reasoning_detection() {
        let entry = FutureModelEntry {
            id: "test-model".to_string(),
            name: Some("Test".to_string()),
            context_length: Some(128000),
            max_tokens: Some(384000),
            architecture: Some(FutureArchitecture {
                modality: Some("text+image->text".to_string()),
                tokenizer: None,
            }),
            pricing: None,
            supported_parameters: Some(vec![
                "thinking".to_string(),
                "reasoning_effort".to_string(),
            ]),
            knowledge_cutoff: None,
            provider: None,
            description: None,
            description_en: None,
            recommended: None,
        };
        let model = convert_future_model(entry, "https://api.example.com/v1");
        assert!(model.reasoning);
        assert_eq!(model.provider, "future");
        assert_eq!(model.max_tokens, 384000);
    }

    #[test]
    fn convert_model_no_reasoning() {
        let entry = FutureModelEntry {
            id: "plain-model".to_string(),
            name: None,
            context_length: Some(64000),
            max_tokens: Some(65536),
            architecture: None,
            pricing: None,
            supported_parameters: Some(vec!["temperature".to_string()]),
            knowledge_cutoff: None,
            provider: None,
            description: None,
            description_en: None,
            recommended: None,
        };
        let model = convert_future_model(entry, "https://api.example.com/v1");
        assert!(!model.reasoning);
        assert_eq!(model.name, "plain-model"); // falls back to id
        assert_eq!(model.max_tokens, 65536);
    }

    #[test]
    fn convert_model_image_input() {
        let entry = FutureModelEntry {
            id: "vision".to_string(),
            name: Some("Vision".to_string()),
            context_length: None,
            max_tokens: None,
            architecture: Some(FutureArchitecture {
                modality: Some("text+image->text".to_string()),
                tokenizer: None,
            }),
            pricing: None,
            supported_parameters: None,
            knowledge_cutoff: None,
            provider: None,
            description: None,
            description_en: None,
            recommended: None,
        };
        let model = convert_future_model(entry, "https://api.example.com/v1");
        assert!(model.input.iter().any(|i| i == "image"));
        assert_eq!(model.context_window, 128000); // default
        assert_eq!(model.max_tokens, 0); // runtime fallback applies when omitted
    }

    #[test]
    fn convert_model_pricing() {
        let entry = FutureModelEntry {
            id: "priced".to_string(),
            name: None,
            context_length: Some(128000),
            max_tokens: Some(8192),
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
            description: None,
            description_en: None,
            recommended: None,
        };
        let model = convert_future_model(entry, "https://api.example.com/v1");
        assert_eq!(model.cost.input, 1000.0); // 0.001 * 1M / 1
        assert_eq!(model.cost.output, 2000.0);
        assert_eq!(model.cost.cache_read, 500.0);
        assert_eq!(model.cost.cache_write, 0.0); // None → 0
    }

    // ─── fetch/cache/refresh against a mock server ─────────────────────────

    /// HTTP server: answers up to 8 requests with a canned (status, body).
    fn mock_json_server(status: u16, body: String) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for _ in 0..8 {
                // Blocking accept on a live listener does not error; a parked
                // surplus iteration is harmless (test process exit reaps it).
                let (mut stream, _) = listener.accept().expect("mock server accept");
                let mut sink = [0u8; 8192];
                let _ = stream.read(&mut sink);
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                if stream.write_all(response.as_bytes()).is_err() {
                    return;
                }
                let _ = stream.flush();
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    /// Serializes tests that touch the process-global future-models statics.
    fn future_test_lock() -> std::sync::MutexGuard<'static, ()> {
        super::future_models_test_lock()
    }

    /// Serializes the single-flight gate against concurrent `Registry::new()`
    /// threads (which don't hold `future_test_lock`), for tests that reset the
    /// shared statics and then assert on them exclusively.
    fn future_refresh_gate() -> std::sync::MutexGuard<'static, ()> {
        FUTURE_REFRESH_GATE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn reset_future_cache_state() {
        super::reset_future_caches_for_tests();
    }

    #[test]
    fn fetch_future_models_parses_data_and_array_forms() {
        let _guard = future_test_lock();
        let entry = r#"{"id":"future-model-1","name":"Future One","context_length":128000}"#;
        let base = mock_json_server(200, format!(r#"{{"data":[{entry}]}}"#));
        let models = fetch_future_models("k", &base).expect("data form parses");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "future-model-1");

        let base = mock_json_server(200, format!("[{entry}]"));
        let models = fetch_future_models("k", &base).expect("array form parses");
        assert_eq!(models.len(), 1);

        // HTTP error → None.
        let base = mock_json_server(500, "{}".to_string());
        assert!(fetch_future_models("k", &base).is_none());

        // Unparseable body → None.
        let base = mock_json_server(200, "not json".to_string());
        assert!(fetch_future_models("k", &base).is_none());

        // Connection refused → None (no server listening here).
        assert!(fetch_future_models("k", "http://127.0.0.1:1").is_none());
    }

    #[test]
    fn sync_future_models_cache_warms_disk_and_memory() {
        let _guard = future_test_lock();
        let home = crate::test_support::TestHome::new();
        reset_future_cache_state();
        let entry = r#"{"id":"sync-model","name":"Synced","context_length":64000}"#;
        let base = mock_json_server(200, format!(r#"{{"data":[{entry}]}}"#));
        // Point the future provider at the mock via auth.json baseUrl.
        let auth_path = home.auth_path();
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        std::fs::write(
            &auth_path,
            format!(r#"{{"future": {{"type": "api_key", "key": "k", "base_url": "{base}/api"}}}}"#),
        )
        .unwrap();

        let synced = sync_future_models_cache();
        assert!(synced);
        // The in-process cache now serves the model…
        let cached_ids: Vec<String> = FUTURE_MODELS_MEMORY_CACHE
            .read()
            .as_ref()
            .unwrap()
            .models
            .iter()
            .map(|m| m.id.clone())
            .collect();
        assert_eq!(cached_ids, ["sync-model"]);
        // …and the disk cache was written.
        let disk = load_future_models_cache().unwrap();
        assert_eq!(disk.models[0].id, "sync-model");
    }

    #[test]
    fn cache_save_and_concurrent_load_never_torn() {
        let _guard = future_test_lock();
        let home = crate::test_support::TestHome::new();
        reset_future_cache_state();
        std::fs::create_dir_all(home.path().join(".future/agent")).unwrap();

        // A large payload stretches the truncate→write window of a non-atomic
        // save, so a reader racing the writer observes torn (empty/partial)
        // files unless the save goes through tmp+rename.
        let models: Vec<Model> = (0..800)
            .map(|i| {
                convert_future_model(
                    serde_json::from_str(&format!(
                        r#"{{"id":"big-model-{i}","name":"Big {i}","context_length":64000}}"#
                    ))
                    .unwrap(),
                    "http://x/v1",
                )
            })
            .collect();
        let expected_len = models.len();

        use std::sync::atomic::{AtomicBool, Ordering};
        let first_write_done = std::sync::Arc::new(AtomicBool::new(false));
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let writer = std::thread::spawn({
            let first_write_done = first_write_done.clone();
            let stop = stop.clone();
            move || {
                let mut i = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    save_future_models_cache_inner(&FutureModelsCache {
                        fetched_at: i,
                        models: models.clone(),
                    });
                    first_write_done.store(true, Ordering::Relaxed);
                    i += 1;
                }
            }
        });

        while !first_write_done.load(Ordering::Relaxed) {
            std::hint::spin_loop();
        }
        let mut torn = 0u64;
        for _ in 0..500 {
            // After the first completed save, the file must always exist and
            // parse to complete content — None/short content = torn read.
            torn += u64::from(
                load_future_models_cache().is_none_or(|c| c.models.len() != expected_len),
            );
        }
        stop.store(true, Ordering::Relaxed);
        writer.join().unwrap();
        assert_eq!(torn, 0, "torn/partial cache reads observed");
    }

    #[test]
    fn get_future_models_with_cache_prefers_memory_then_disk() {
        let _guard = future_test_lock();
        let home = crate::test_support::TestHome::new();
        reset_future_cache_state();

        // Seed only the memory cache → returned directly.
        let mem_model = convert_future_model(
            serde_json::from_str(r#"{"id":"mem-model"}"#).unwrap(),
            "http://x/v1",
        );
        *FUTURE_MODELS_MEMORY_CACHE.write() = Some(FutureModelsCache {
            fetched_at: 1,
            models: vec![mem_model],
        });
        let models = get_future_models_with_cache("k", "http://127.0.0.1:1");
        assert_eq!(models[0].id, "mem-model");

        // Clear memory; seed only the disk cache → disk arm re-seeds memory.
        reset_future_cache_state();
        std::fs::create_dir_all(home.path().join(".future/agent")).unwrap();
        let disk_model = convert_future_model(
            serde_json::from_str(r#"{"id":"disk-model"}"#).unwrap(),
            "http://x/v1",
        );
        save_future_models_cache_inner(&FutureModelsCache {
            fetched_at: 1,
            models: vec![disk_model],
        });
        let models = get_future_models_with_cache("k", "http://127.0.0.1:1");
        assert_eq!(models[0].id, "disk-model");
        assert!(FUTURE_MODELS_MEMORY_CACHE.read().is_some());
        drop(home);
    }

    #[test]
    fn spawn_refresh_respects_backoff_window() {
        let _guard = future_test_lock();
        let _gate = future_refresh_gate();
        reset_future_caches_for_tests_unlocked();
        // First call spawns a refresh thread (fetch to a dead port fails fast).
        let handle = spawn_future_models_refresh_unlocked("k", "http://127.0.0.1:1")
            .expect("first refresh spawns");
        // The attempt timestamp is now fresh — an immediate second call hits
        // the backoff window and returns without spawning.
        assert!(
            spawn_future_models_refresh_unlocked("k", "http://127.0.0.1:1").is_none(),
            "second call must be blocked by the backoff window"
        );
        assert!(FUTURE_MODELS_LAST_ATTEMPT.load(Ordering::Relaxed) > 0);
        // Join the refresh thread deterministically, then assert the flag cleared.
        handle.join().unwrap();
        assert!(!FUTURE_MODELS_REFRESH_IN_FLIGHT.load(Ordering::Relaxed));
    }

    #[test]
    fn refresh_bails_when_already_in_flight() {
        let _guard = future_test_lock();
        let _gate = future_refresh_gate();
        reset_future_caches_for_tests_unlocked();
        FUTURE_MODELS_REFRESH_IN_FLIGHT.store(true, Ordering::Relaxed);
        assert!(
            spawn_future_models_refresh_unlocked("k", "http://127.0.0.1:1").is_none(),
            "already-in-flight must bail"
        );
        // Bailed at the single-flight gate: no new attempt was stamped.
        assert_eq!(FUTURE_MODELS_LAST_ATTEMPT.load(Ordering::Relaxed), 0);
        FUTURE_MODELS_REFRESH_IN_FLIGHT.store(false, Ordering::Relaxed);
    }

    #[test]
    fn refresh_recovers_from_closure_panics() {
        let _guard = future_test_lock();
        let _gate = future_refresh_gate();
        let _subscriber = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .finish(),
        );
        for payload_kind in [1usize, 2, 3] {
            reset_future_caches_for_tests_unlocked();
            REFRESH_TEST_PANIC.store(payload_kind, Ordering::Relaxed);
            let handle = spawn_future_models_refresh_unlocked("k", "http://127.0.0.1:1")
                .expect("refresh spawns");
            // The panic is caught and the single-flight flag released.
            handle.join().unwrap();
            assert!(
                !FUTURE_MODELS_REFRESH_IN_FLIGHT.load(Ordering::Relaxed),
                "panic must not wedge the in-flight flag"
            );
        }
    }

    #[test]
    fn cache_path_falls_back_to_tmp_without_home() {
        assert_eq!(
            future_models_cache_path_in(None),
            "/tmp/.future/agent/.future-models-cache.json"
        );
        assert_eq!(
            future_models_cache_path_in(Some(std::path::PathBuf::from("/home/x"))),
            "/home/x/.future/agent/.future-models-cache.json"
        );
    }

    #[test]
    fn fetch_returns_none_for_unexpected_json_shape() {
        let _guard = future_test_lock();
        // Valid JSON, but neither the {data:[...]} form nor the array form.
        let base = mock_json_server(200, "42".to_string());
        assert!(fetch_future_models("k", &base).is_none());
    }

    #[test]
    fn get_models_with_cache_cold_start_returns_empty() {
        let _guard = future_test_lock();
        let _home = crate::test_support::TestHome::new();
        reset_future_cache_state();
        // No memory cache, no disk cache, refresh target unreachable → the
        // first-login path returns an empty catalog immediately.
        let models = get_future_models_with_cache("k", "http://127.0.0.1:1");
        assert!(models.is_empty());
    }

    #[test]
    fn sync_cache_returns_false_when_fetch_fails() {
        let _guard = future_test_lock();
        let home = crate::test_support::TestHome::new();
        reset_future_cache_state();
        let auth_path = home.auth_path();
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        std::fs::write(
            &auth_path,
            r#"{"future": {"type": "api_key", "key": "k", "base_url": "http://127.0.0.1:1/api"}}"#,
        )
        .unwrap();
        assert!(!sync_future_models_cache());
    }

    #[test]
    fn save_cache_inner_ignores_write_failure() {
        let _guard = future_test_lock();
        let home = crate::test_support::TestHome::new();
        reset_future_cache_state();
        // Make the cache's parent directory a plain file so both the
        // create_dir_all and the temp write fail (no panic; rename skipped).
        let agent_dir = home.path().join(".future/agent");
        std::fs::create_dir_all(home.path().join(".future")).unwrap();
        std::fs::write(&agent_dir, "not a dir").unwrap();
        save_future_models_cache_inner(&FutureModelsCache {
            fetched_at: 1,
            models: vec![],
        });
    }
}
