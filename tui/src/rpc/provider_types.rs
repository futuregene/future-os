//! Provider / model configuration data layer (pure, no I/O).
//!
//! The TUI's `/providers` and `/models` views need a typed surface over two
//! very different representations of the same thing:
//!
//!   - the **read** side — `list_providers` returns a JSON object with two
//!     arrays (`builtin` summaries and `custom` entries) whose entries have
//!     *different* shapes and may be missing fields entirely (the agent fills
//!     defaults and never emits `null` for an object it knows nothing about);
//!   - the **write** side — `upsert_provider` carries the typed proto
//!     `ProviderUpsert` sub-message, whose `models` list is a *full
//!     replacement*, so a silently dropped field is a silent data loss.
//!
//! Everything here is a pure function over these values, so the form/list state
//! machines in `components/provider_dialogs.rs` can be tested without an agent.
//!
//! Validation mirrors the agent's authoritative rules
//! (`agent/src/config/providers.rs` `validate_provider_upsert` /
//! `validate_auth_mutation`) — same limits, and where they overlap the same
//! error strings, so the user sees one message whether the rejection happened
//! client-side or on the agent.

use future_rpc::proto::{ProviderModel, ProviderUpsert};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Maximum number of models per provider (`validate_provider_upsert`).
pub const MAX_PROVIDER_MODELS: usize = 100;
/// Maximum provider id length (`valid_provider_id`).
pub const MAX_PROVIDER_ID_LEN: usize = 64;
/// Maximum provider display-name length.
pub const MAX_PROVIDER_NAME_LEN: usize = 128;
/// Maximum model id length.
pub const MAX_MODEL_ID_LEN: usize = 256;
/// Maximum model display-name length.
pub const MAX_MODEL_NAME_LEN: usize = 128;
/// Maximum API key length.
pub const MAX_API_KEY_LEN: usize = 16_384;
/// Context window assumed when a provider model carries none (the agent's
/// `list_providers` display default).
pub const DEFAULT_CONTEXT_WINDOW: i32 = 128_000;
/// Max output tokens assumed when a provider model carries none. The agent's
/// default, capped by the context window so the write still validates.
pub const DEFAULT_MAX_TOKENS: i32 = 16_384;
/// API types the agent accepts (`validate_provider_upsert`).
pub const API_TYPES: [&str; 3] = ["openai-completions", "openai-responses", "anthropic"];

fn default_context_window() -> i32 {
    DEFAULT_CONTEXT_WINDOW
}

fn default_max_tokens() -> i32 {
    DEFAULT_MAX_TOKENS
}

fn default_true() -> bool {
    true
}

/// One model row of a custom provider.
///
/// `id`/`name`/`context_window`/`supports_images`/`thinking` are the editable
/// fields. The remaining fields are **round-trip only**: `upsert_provider`
/// replaces the whole model list, so a field the form does not edit must still
/// travel back unchanged or the edit would reset it (`max_tokens` would even
/// fail the agent's "token limits must be positive" check).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelInput {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_context_window")]
    pub context_window: i32,
    #[serde(default)]
    pub supports_images: bool,
    /// Whether the model accepts thinking controls (`reasoning` on the wire).
    #[serde(default = "default_true")]
    pub thinking: bool,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: i32,
    #[serde(default)]
    pub cost_input: f64,
    #[serde(default)]
    pub cost_output: f64,
    #[serde(default)]
    pub cost_cache_read: f64,
    #[serde(default)]
    pub cost_cache_write: f64,
}

impl Default for ProviderModelInput {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            context_window: DEFAULT_CONTEXT_WINDOW,
            supports_images: false,
            thinking: true,
            max_tokens: DEFAULT_MAX_TOKENS,
            cost_input: 0.0,
            cost_output: 0.0,
            cost_cache_read: 0.0,
            cost_cache_write: 0.0,
        }
    }
}

impl ProviderModelInput {
    /// A new model row with `id`/`name` filled, everything else defaulted.
    pub fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.to_string(),
            name: if name.is_empty() {
                id.to_string()
            } else {
                name.to_string()
            },
            ..Default::default()
        }
    }

    /// Parse one `list_providers` model entry, tolerating missing fields,
    /// wrong types and unknown keys. Returns `None` when the entry is not an
    /// object or has no usable id — the caller skips it instead of failing the
    /// whole list.
    pub fn from_value(value: &Value) -> Option<Self> {
        let map = value.as_object()?;
        let id = map.get("id").and_then(Value::as_str)?.trim().to_string();
        if id.is_empty() {
            return None;
        }
        let name = map
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&id)
            .to_string();
        let context_window =
            positive_i32(map.get("contextWindow")).unwrap_or(DEFAULT_CONTEXT_WINDOW);
        // `supportsImages` is authoritative; older payloads only carry the
        // `modalities` list, so fall back to it rather than silently dropping
        // image support on a round-trip.
        let supports_images = map
            .get("supportsImages")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| {
                map.get("modalities")
                    .and_then(Value::as_array)
                    .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("image")))
            });
        let thinking = map
            .get("reasoning")
            .or_else(|| map.get("thinking"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let max_tokens =
            positive_i32(map.get("maxTokens")).unwrap_or(context_window.min(DEFAULT_MAX_TOKENS));
        Some(Self {
            id,
            name,
            context_window,
            supports_images,
            thinking,
            max_tokens,
            cost_input: number(map.get("inputCost")),
            cost_output: number(map.get("outputCost")),
            cost_cache_read: number(map.get("cacheReadCost")),
            cost_cache_write: number(map.get("cacheWriteCost")),
        })
    }

    /// Convert to the proto sub-message carried by `upsert_provider`.
    pub fn to_proto(&self) -> ProviderModel {
        ProviderModel {
            id: self.id.trim().to_string(),
            name: self.name.trim().to_string(),
            modalities: if self.supports_images {
                vec!["text".to_string(), "image".to_string()]
            } else {
                vec!["text".to_string()]
            },
            context_window: self.context_window,
            max_tokens: self.max_tokens,
            reasoning: Some(self.thinking),
            cost_input: self.cost_input,
            cost_output: self.cost_output,
            cost_cache_read: self.cost_cache_read,
            cost_cache_write: self.cost_cache_write,
        }
    }

    /// Inverse of [`Self::to_proto`].
    pub fn from_proto(model: &ProviderModel) -> Self {
        Self {
            id: model.id.clone(),
            name: model.name.clone(),
            context_window: model.context_window,
            supports_images: model.modalities.iter().any(|entry| entry == "image"),
            thinking: model.reasoning.unwrap_or(true),
            max_tokens: model.max_tokens,
            cost_input: model.cost_input,
            cost_output: model.cost_output,
            cost_cache_read: model.cost_cache_read,
            cost_cache_write: model.cost_cache_write,
        }
    }
}

/// A provider as reported by `list_providers`.
///
/// Built-in catalog providers carry a `modelCount` and no model list; custom
/// providers carry the full list. `model_count` is authoritative for display in
/// both cases (`models.len()` for custom providers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    /// The provider's API dialect ("openai-completions" | …). Empty for
    /// built-in catalog providers, which have no editable `api` field.
    #[serde(default, rename = "api")]
    pub api_type: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub models: Vec<ProviderModelInput>,
    #[serde(default)]
    pub has_api_key: bool,
    /// True when the entry came from the built-in catalog. Built-in providers
    /// cannot be edited or deleted (the agent rejects both); only their API key
    /// or base URL can be changed.
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub model_count: usize,
}

impl ProviderInfo {
    /// A custom-provider input prefilled from this provider. The API key is
    /// intentionally absent (`api_key: None`) — the agent never sends keys back,
    /// and `None` means "leave the stored key unchanged".
    pub fn to_input(&self) -> ProviderInput {
        ProviderInput {
            id: self.id.clone(),
            name: self.name.clone(),
            api_type: self.api_type.clone(),
            base_url: self.base_url.clone(),
            models: self.models.clone(),
            api_key: None,
            clear_api_key: false,
            create_only: false,
        }
    }
}

/// A create/update of one custom provider — the TUI's editable form value,
/// serialisable to the proto `ProviderUpsert` shape.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInput {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Wire field `api`.
    #[serde(default, rename = "api")]
    pub api_type: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub models: Vec<ProviderModelInput>,
    /// `None` (or an empty string) leaves the stored key unchanged. The agent
    /// never sends a key back, so an edit form must not invent one.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Remove the stored key in the same transaction.
    #[serde(default)]
    pub clear_api_key: bool,
    /// Fail when the provider already exists — set for "add", never for "edit".
    #[serde(default)]
    pub create_only: bool,
}

impl ProviderInput {
    /// Prefilled input for adding a provider.
    pub fn create() -> Self {
        Self {
            create_only: true,
            ..Default::default()
        }
    }

    /// Prefilled input for editing `info` (see [`ProviderInfo::to_input`]).
    pub fn from_provider(info: &ProviderInfo) -> Self {
        info.to_input()
    }

    /// The key that will actually be written, or `None` when the mutation
    /// leaves the stored key alone. An empty string means "unchanged" on the
    /// proto wire, so it is normalised away here.
    pub fn effective_api_key(&self) -> Option<&str> {
        self.api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
    }

    /// Convert to the proto sub-message for `upsert_provider`.
    ///
    /// `replace_models` is always set: the form owns the complete model list,
    /// and an omitted presence bit would make an emptied list mean "leave
    /// unchanged" instead of "remove every model".
    pub fn to_proto(&self) -> ProviderUpsert {
        ProviderUpsert {
            id: self.id.trim().to_string(),
            name: self.name.trim().to_string(),
            api: self.api_type.trim().to_string(),
            base_url: self.base_url.trim().to_string(),
            clear_base_url: false,
            models: self
                .models
                .iter()
                .map(ProviderModelInput::to_proto)
                .collect(),
            create_only: self.create_only,
            api_key: self.effective_api_key().unwrap_or_default().to_string(),
            replace_models: true,
            clear_api_key: self.clear_api_key,
        }
    }

    /// Inverse of [`Self::to_proto`]. `clear_base_url` / `replace_models` have
    /// no form field (the form always carries a URL and a complete model list)
    /// and are dropped.
    pub fn from_proto(spec: &ProviderUpsert) -> Self {
        Self {
            id: spec.id.clone(),
            name: spec.name.clone(),
            api_type: spec.api.clone(),
            base_url: spec.base_url.clone(),
            models: spec
                .models
                .iter()
                .map(ProviderModelInput::from_proto)
                .collect(),
            api_key: if spec.api_key.is_empty() {
                None
            } else {
                Some(spec.api_key.clone())
            },
            clear_api_key: spec.clear_api_key,
            create_only: spec.create_only,
        }
    }
}

fn number(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0)
}

/// A positive `i32` from a JSON number — integral floats are accepted (some
/// payloads spell a window as `200000.0`), zero/negatives/fractions are not.
fn positive_i32(value: Option<&Value>) -> Option<i32> {
    let raw = value.and_then(Value::as_f64).filter(|n| n.is_finite())?;
    if raw < 1.0 {
        return None;
    }
    Some(raw.min(i32::MAX as f64) as i32)
}

/// A non-negative count from a JSON number, tolerating floats.
fn as_count(value: Option<&Value>) -> Option<usize> {
    let raw = value.and_then(Value::as_f64).filter(|n| n.is_finite())?;
    if raw < 0.0 {
        return None;
    }
    Some(raw.min(usize::MAX as f64) as usize)
}

/// Parse the `list_providers` payload (`{builtin: [...], custom: [...]}`) into
/// providers, built-ins first. Tolerates a bare array (treated as custom), a
/// missing/`null` member, non-object entries and missing fields.
pub fn parse_providers_response(value: &Value) -> Vec<ProviderInfo> {
    let object = value.as_object();
    let builtin = object
        .and_then(|map| map.get("builtin"))
        .and_then(Value::as_array);
    let custom = object
        .and_then(|map| map.get("custom"))
        .and_then(Value::as_array);
    if builtin.is_none() && custom.is_none() {
        // Alternate/older shape: a flat array of provider objects.
        return value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| provider_from_value(item, false))
                    .collect()
            })
            .unwrap_or_default();
    }
    let mut providers = Vec::new();
    for (items, is_builtin) in [(builtin, true), (custom, false)] {
        for item in items.into_iter().flatten() {
            if let Some(provider) = provider_from_value(item, is_builtin) {
                providers.push(provider);
            }
        }
    }
    providers
}

fn provider_from_value(value: &Value, builtin: bool) -> Option<ProviderInfo> {
    let map = value.as_object()?;
    let id = map.get("id").and_then(Value::as_str)?.trim().to_string();
    if id.is_empty() {
        return None;
    }
    let name = map
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(&id)
        .to_string();
    let api_type = map
        .get("api")
        .or_else(|| map.get("apiType"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let base_url = map
        .get("baseUrl")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let models: Vec<ProviderModelInput> = map
        .get("models")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(ProviderModelInput::from_value)
                .collect()
        })
        .unwrap_or_default();
    let model_count = as_count(map.get("modelCount")).unwrap_or(models.len());
    Some(ProviderInfo {
        id,
        name,
        api_type,
        base_url,
        models,
        has_api_key: map
            .get("hasApiKey")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        builtin,
        model_count,
    })
}

/// Provider id rule shared by `upsert_provider` / `set_auth` /
/// `delete_provider`: lowercase ASCII letters, digits, `-` and `_`, 1..=64.
pub fn validate_provider_id(id: &str) -> Result<(), String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("provider id is required".to_string());
    }
    if id.len() > MAX_PROVIDER_ID_LEN
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte))
    {
        return Err("provider id must use lowercase letters, digits, '-' or '_'".to_string());
    }
    Ok(())
}

fn ascii_without_control(value: &str) -> bool {
    value.is_ascii() && !value.bytes().any(|byte| byte.is_ascii_control())
}

/// Client-side mirror of the agent's `validate_provider_upsert`, so the form
/// can reject a bad provider before a round-trip. The agent stays
/// authoritative — this only shortens the feedback loop.
pub fn validate_provider_input(input: &ProviderInput) -> Result<(), String> {
    validate_provider_id(&input.id)?;
    let name = input.name.trim();
    if name.len() > MAX_PROVIDER_NAME_LEN || !ascii_without_control(name) {
        return Err("provider name is invalid or too long".to_string());
    }
    let api_type = input.api_type.trim();
    if api_type.is_empty() {
        return Err("provider API type is required".to_string());
    }
    if !API_TYPES.contains(&api_type) {
        return Err("unsupported provider API type".to_string());
    }
    let base_url = input.base_url.trim();
    if base_url.is_empty() {
        return Err("base URL is required".to_string());
    }
    let parsed = reqwest::Url::parse(base_url)
        .map_err(|_| "base URL must be a valid http/https address".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("base URL must be a valid http/https address".to_string());
    }
    if let Some(key) = input.effective_api_key() {
        if key.len() > MAX_API_KEY_LEN || !ascii_without_control(key) {
            return Err("API key is invalid or too long".to_string());
        }
    }
    if input.effective_api_key().is_some() && input.clear_api_key {
        return Err("cannot set and clear an API key in the same mutation".to_string());
    }
    if input.models.len() > MAX_PROVIDER_MODELS {
        return Err("provider has too many models".to_string());
    }
    let mut seen: Vec<&str> = Vec::with_capacity(input.models.len());
    for model in &input.models {
        let id = model.id.trim();
        if id.is_empty()
            || id.len() > MAX_MODEL_ID_LEN
            || !ascii_without_control(id)
            || seen.contains(&id)
        {
            return Err("provider model id is invalid or duplicated".to_string());
        }
        seen.push(id);
        let name = model.name.trim();
        if name.len() > MAX_MODEL_NAME_LEN || !ascii_without_control(name) {
            return Err("provider model name is invalid or too long".to_string());
        }
        if model.context_window <= 0 || model.max_tokens <= 0 {
            return Err("provider model token limits must be positive".to_string());
        }
        if model.max_tokens > model.context_window {
            return Err("provider model max tokens cannot exceed its context window".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn provider() -> ProviderInput {
        ProviderInput {
            id: "acme".to_string(),
            name: "Acme".to_string(),
            api_type: "openai-completions".to_string(),
            base_url: "https://api.acme.test/v1".to_string(),
            models: vec![ProviderModelInput::new("acme-large", "Acme Large")],
            api_key: None,
            clear_api_key: false,
            create_only: true,
        }
    }

    // ─── parsing ───────────────────────────────────────────────────────

    #[test]
    fn parse_full_payload_keeps_builtin_before_custom() {
        let payload = json!({
            "builtin": [
                {"id": "future", "name": "Future", "baseUrl": "https://api.future.test",
                 "hasApiKey": true, "modelCount": 900, "requiresBaseUrl": false},
                {"id": "openai", "name": "OpenAI", "baseUrl": "", "hasApiKey": false,
                 "modelCount": 3}
            ],
            "custom": [
                {"id": "acme", "name": "Acme", "api": "openai-completions",
                 "baseUrl": "https://api.acme.test/v1", "hasApiKey": true,
                 "models": [{"id": "acme-large", "name": "Acme Large", "supportsImages": true,
                             "reasoning": false, "contextWindow": 200000, "maxTokens": 8192,
                             "inputCost": 1.5, "outputCost": 2.5, "cacheReadCost": 0.25,
                             "cacheWriteCost": 0.5}]}
            ]
        });
        let providers = parse_providers_response(&payload);
        assert_eq!(providers.len(), 3);
        assert_eq!(providers[0].id, "future");
        assert!(providers[0].builtin);
        assert_eq!(providers[0].model_count, 900);
        assert!(providers[0].models.is_empty());
        assert_eq!(providers[0].api_type, "");
        assert_eq!(providers[1].id, "openai");
        assert_eq!(providers[1].model_count, 3);
        let acme = &providers[2];
        assert!(!acme.builtin);
        assert_eq!(acme.api_type, "openai-completions");
        assert_eq!(acme.model_count, 1);
        let model = &acme.models[0];
        assert_eq!(model.id, "acme-large");
        assert_eq!(model.context_window, 200_000);
        assert_eq!(model.max_tokens, 8_192);
        assert!(model.supports_images);
        assert!(!model.thinking);
        assert_eq!(model.cost_input, 1.5);
        assert_eq!(model.cost_cache_write, 0.5);
    }

    #[test]
    fn parse_tolerates_missing_wrong_typed_and_malformed_entries() {
        let payload = json!({
            "builtin": [
                {"id": "only-id"},
                {"id": "   "},
                {"name": "no id"},
                "not an object",
                {"id": "typed", "name": 7, "baseUrl": 5, "hasApiKey": "yes", "modelCount": -4}
            ],
            "custom": [
                {"id": "half", "models": [
                    {"id": "m1", "contextWindow": "big", "maxTokens": -1,
                     "supportsImages": "no", "inputCost": "free"},
                    {"id": ""},
                    {"no": "id"},
                    "nope"
                ]}
            ]
        });
        let providers = parse_providers_response(&payload);
        // The malformed built-in entries are skipped, not fatal.
        assert_eq!(
            providers.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["only-id", "typed", "half"]
        );
        let only_id = &providers[0];
        assert_eq!(only_id.name, "only-id"); // falls back to the id
        assert_eq!(only_id.base_url, "");
        assert!(!only_id.has_api_key);
        assert_eq!(only_id.model_count, 0);
        // A non-string name/baseUrl and a bogus bool fall back to defaults.
        assert_eq!(providers[1].name, "typed");
        assert_eq!(providers[1].model_count, 0);
        // Only the single well-formed model of `half` survives, with clamped
        // defaults; a model without a usable id is dropped.
        assert_eq!(providers[2].models.len(), 1);
        let model = &providers[2].models[0];
        assert_eq!(model.id, "m1");
        assert_eq!(model.context_window, DEFAULT_CONTEXT_WINDOW);
        assert_eq!(model.max_tokens, DEFAULT_MAX_TOKENS);
        assert!(!model.supports_images);
        assert!(model.thinking);
        assert_eq!(model.cost_input, 0.0);
    }

    #[test]
    fn parse_accepts_bare_array_and_empty_shapes() {
        let array = json!([{"id": "a"}, {"id": "b"}]);
        let providers = parse_providers_response(&array);
        assert_eq!(providers.len(), 2);
        assert!(providers.iter().all(|p| !p.builtin));
        for payload in [
            json!({}),
            json!({"builtin": null, "custom": null}),
            json!(null),
        ] {
            assert!(parse_providers_response(&payload).is_empty());
        }
        assert!(parse_providers_response(&json!({"builtin": [], "custom": []})).is_empty());
    }

    #[test]
    fn model_parse_falls_back_to_modalities_and_clamps_max_tokens() {
        let model = ProviderModelInput::from_value(&json!({
            "id": "m", "modalities": ["text", "image"], "contextWindow": 4096
        }))
        .expect("model");
        assert!(model.supports_images);
        // maxTokens absent → capped by the context window, never above it.
        assert_eq!(model.max_tokens, 4_096);
        let tiny =
            ProviderModelInput::from_value(&json!({"id": "m", "contextWindow": 1})).expect("model");
        assert_eq!(tiny.max_tokens, 1);
        assert!(
            !ProviderModelInput::from_value(&json!({"id": "m", "thinking": false}))
                .expect("model")
                .thinking
        );
        // Integral floats are accepted for the numeric windows.
        let float_window =
            ProviderModelInput::from_value(&json!({"id": "m", "contextWindow": 200000.0}))
                .expect("model");
        assert_eq!(float_window.context_window, 200_000);
        assert!(ProviderModelInput::from_value(&json!("x")).is_none());
    }

    // ─── proto interop ─────────────────────────────────────────────────

    #[test]
    fn proto_round_trip_preserves_editable_and_round_trip_fields() {
        let mut input = provider();
        input.api_key = Some("sk-test".to_string());
        input.models[0].supports_images = true;
        input.models[0].thinking = false;
        input.models[0].context_window = 64_000;
        input.models[0].max_tokens = 4_000;
        input.models[0].cost_output = 3.5;
        let spec = input.to_proto();
        assert_eq!(spec.id, "acme");
        assert_eq!(spec.api, "openai-completions");
        assert_eq!(spec.base_url, "https://api.acme.test/v1");
        assert!(spec.replace_models);
        assert!(spec.create_only);
        assert_eq!(spec.api_key, "sk-test");
        assert_eq!(spec.models.len(), 1);
        assert_eq!(spec.models[0].modalities, vec!["text", "image"]);
        assert_eq!(spec.models[0].reasoning, Some(false));
        assert_eq!(spec.models[0].max_tokens, 4_000);
        assert_eq!(spec.models[0].cost_output, 3.5);
        assert_eq!(ProviderInput::from_proto(&spec), input);
    }

    #[test]
    fn proto_conversion_normalises_empty_key_and_trims() {
        let input = ProviderInput {
            id: "  acme  ".to_string(),
            api_key: Some("   ".to_string()),
            ..provider()
        };
        let spec = input.to_proto();
        assert_eq!(spec.id, "acme");
        // Whitespace-only key means "unchanged", not "write a blank key".
        assert_eq!(spec.api_key, "");
        assert_eq!(ProviderInput::from_proto(&spec).api_key, None);
        // A clearing mutation round-trips `clear_api_key`.
        let clearing = ProviderInput {
            clear_api_key: true,
            ..provider()
        };
        assert!(clearing.to_proto().clear_api_key);
        assert!(ProviderInput::from_proto(&clearing.to_proto()).clear_api_key);
    }

    #[test]
    fn info_to_input_leaves_the_key_unset_and_starts_add_in_create_mode() {
        let info = ProviderInfo {
            id: "acme".to_string(),
            name: "Acme".to_string(),
            api_type: "openai-completions".to_string(),
            base_url: "https://api.acme.test/v1".to_string(),
            models: vec![ProviderModelInput::new("m", "M")],
            has_api_key: true,
            builtin: false,
            model_count: 1,
        };
        let input = info.to_input();
        assert_eq!(input.id, "acme");
        assert_eq!(input.models, info.models);
        assert_eq!(input.api_key, None);
        assert!(!input.create_only);
        assert!(!input.clear_api_key);
        assert_eq!(ProviderInput::from_provider(&info), input);
        let add = ProviderInput::create();
        assert!(add.create_only);
        assert!(add.id.is_empty());
    }

    #[test]
    fn serde_uses_camel_case_wire_names() {
        let input = provider();
        let value = serde_json::to_value(&input).expect("serialise");
        assert_eq!(value["api"], "openai-completions");
        assert_eq!(value["baseUrl"], "https://api.acme.test/v1");
        assert_eq!(value["createOnly"], true);
        assert_eq!(value["models"][0]["contextWindow"], DEFAULT_CONTEXT_WINDOW);
        assert_eq!(value["models"][0]["supportsImages"], false);
        let back: ProviderInput = serde_json::from_value(value).expect("deserialise");
        assert_eq!(back, input);
    }

    #[test]
    fn serde_fills_the_documented_defaults_for_omitted_model_fields() {
        // `upsert_provider` replaces the whole model list, so a payload that
        // omits `contextWindow`/`maxTokens`/`thinking` must come back with the
        // documented defaults — a zeroed `maxTokens` would fail the agent's own
        // "token limits must be positive" check on the next write.
        let model: ProviderModelInput =
            serde_json::from_value(json!({"id": "m"})).expect("deserialise model");
        assert_eq!(model.context_window, DEFAULT_CONTEXT_WINDOW);
        assert_eq!(model.max_tokens, DEFAULT_MAX_TOKENS);
        assert!(model.thinking);
        assert!(!model.supports_images);
        assert_eq!(model.name, "");

        // The same defaults apply when the model travels inside a provider.
        let input: ProviderInput = serde_json::from_value(json!({
            "id": "acme",
            "api": "anthropic",
            "models": [{"id": "m"}],
        }))
        .expect("deserialise provider");
        assert_eq!(input.models[0].context_window, DEFAULT_CONTEXT_WINDOW);
        assert_eq!(input.models[0].max_tokens, DEFAULT_MAX_TOKENS);
        assert!(input.models[0].thinking);
    }

    // ─── validation ────────────────────────────────────────────────────

    #[test]
    fn validate_accepts_a_complete_provider() {
        validate_provider_input(&provider()).expect("valid");
        // A name is optional (the agent then displays the id) and may be blank.
        let unnamed = ProviderInput {
            name: String::new(),
            ..provider()
        };
        validate_provider_input(&unnamed).expect("valid");
        // No models at all is fine.
        let model_less = ProviderInput {
            models: Vec::new(),
            ..provider()
        };
        validate_provider_input(&model_less).expect("valid");
    }

    #[test]
    fn validate_rejects_bad_ids() {
        for id in ["", "   "] {
            assert_eq!(
                validate_provider_input(&ProviderInput {
                    id: id.to_string(),
                    ..provider()
                })
                .unwrap_err(),
                "provider id is required"
            );
        }
        for id in ["Acme", "acme api", "acme!", "acmé"] {
            assert_eq!(
                validate_provider_input(&ProviderInput {
                    id: id.to_string(),
                    ..provider()
                })
                .unwrap_err(),
                "provider id must use lowercase letters, digits, '-' or '_'",
                "id {id:?}"
            );
        }
        let long = "a".repeat(MAX_PROVIDER_ID_LEN + 1);
        assert!(validate_provider_id(&long).is_err());
        assert!(validate_provider_id(&"a".repeat(MAX_PROVIDER_ID_LEN)).is_ok());
        assert!(validate_provider_id("local_1-x").is_ok());
    }

    #[test]
    fn validate_rejects_bad_names_api_types_and_urls() {
        let input = ProviderInput {
            name: "x".repeat(MAX_PROVIDER_NAME_LEN + 1),
            ..provider()
        };
        assert_eq!(
            validate_provider_input(&input).unwrap_err(),
            "provider name is invalid or too long"
        );
        let input = ProviderInput {
            name: "Acme\u{7}".to_string(),
            ..provider()
        };
        assert!(validate_provider_input(&input).is_err());

        let input = ProviderInput {
            api_type: String::new(),
            ..provider()
        };
        assert_eq!(
            validate_provider_input(&input).unwrap_err(),
            "provider API type is required"
        );
        let input = ProviderInput {
            api_type: "gemini".to_string(),
            ..provider()
        };
        assert_eq!(
            validate_provider_input(&input).unwrap_err(),
            "unsupported provider API type"
        );

        let input = ProviderInput {
            base_url: String::new(),
            ..provider()
        };
        assert_eq!(
            validate_provider_input(&input).unwrap_err(),
            "base URL is required"
        );
        for url in ["not a url", "ftp://acme.test", "file:///tmp/x"] {
            let input = ProviderInput {
                base_url: url.to_string(),
                ..provider()
            };
            assert_eq!(
                validate_provider_input(&input).unwrap_err(),
                "base URL must be a valid http/https address",
                "url {url:?}"
            );
        }
        for url in ["http://acme.test", "https://acme.test/v1/"] {
            let input = ProviderInput {
                base_url: url.to_string(),
                ..provider()
            };
            validate_provider_input(&input).expect("valid url");
        }
    }

    #[test]
    fn validate_rejects_bad_api_keys() {
        let input = ProviderInput {
            api_key: Some("k".repeat(MAX_API_KEY_LEN + 1)),
            ..provider()
        };
        assert_eq!(
            validate_provider_input(&input).unwrap_err(),
            "API key is invalid or too long"
        );
        let input = ProviderInput {
            api_key: Some("sk-\u{1b}".to_string()),
            ..provider()
        };
        assert_eq!(
            validate_provider_input(&input).unwrap_err(),
            "API key is invalid or too long"
        );
        let input = ProviderInput {
            api_key: Some("sk-test".to_string()),
            clear_api_key: true,
            ..provider()
        };
        assert_eq!(
            validate_provider_input(&input).unwrap_err(),
            "cannot set and clear an API key in the same mutation"
        );
        // A clear-only mutation (no key) is legal.
        let input = ProviderInput {
            clear_api_key: true,
            ..provider()
        };
        validate_provider_input(&input).expect("clear-only is valid");
        // A blank key is "unchanged" and never collides with clear_api_key.
        let input = ProviderInput {
            api_key: Some("  ".to_string()),
            clear_api_key: true,
            ..provider()
        };
        validate_provider_input(&input).expect("blank key is unchanged");
    }

    #[test]
    fn validate_rejects_bad_model_rows() {
        let too_many = ProviderInput {
            models: (0..=MAX_PROVIDER_MODELS)
                .map(|index| ProviderModelInput::new(&format!("m{index}"), ""))
                .collect(),
            ..provider()
        };
        assert_eq!(
            validate_provider_input(&too_many).unwrap_err(),
            "provider has too many models"
        );
        let exactly_max = ProviderInput {
            models: (0..MAX_PROVIDER_MODELS)
                .map(|index| ProviderModelInput::new(&format!("m{index}"), ""))
                .collect(),
            ..provider()
        };
        validate_provider_input(&exactly_max).expect("boundary is allowed");

        let duplicate = ProviderInput {
            models: vec![
                ProviderModelInput::new("m", "M"),
                ProviderModelInput::new("m", "M2"),
            ],
            ..provider()
        };
        assert_eq!(
            validate_provider_input(&duplicate).unwrap_err(),
            "provider model id is invalid or duplicated"
        );

        for id in ["", "  "] {
            let input = ProviderInput {
                models: vec![ProviderModelInput::new(id, "")],
                ..provider()
            };
            assert_eq!(
                validate_provider_input(&input).unwrap_err(),
                "provider model id is invalid or duplicated",
                "id {id:?}"
            );
        }
        let long_id = ProviderInput {
            models: vec![ProviderModelInput::new(
                &"m".repeat(MAX_MODEL_ID_LEN + 1),
                "",
            )],
            ..provider()
        };
        assert!(validate_provider_input(&long_id).is_err());

        let long_name = ProviderInput {
            models: vec![ProviderModelInput {
                name: "n".repeat(MAX_MODEL_NAME_LEN + 1),
                ..ProviderModelInput::new("m", "M")
            }],
            ..provider()
        };
        assert_eq!(
            validate_provider_input(&long_name).unwrap_err(),
            "provider model name is invalid or too long"
        );
    }

    #[test]
    fn validate_rejects_bad_model_token_limits() {
        let zero = ProviderInput {
            models: vec![ProviderModelInput {
                max_tokens: 0,
                ..ProviderModelInput::new("m", "M")
            }],
            ..provider()
        };
        assert_eq!(
            validate_provider_input(&zero).unwrap_err(),
            "provider model token limits must be positive"
        );
        let no_window = ProviderInput {
            models: vec![ProviderModelInput {
                context_window: 0,
                ..ProviderModelInput::new("m", "M")
            }],
            ..provider()
        };
        assert_eq!(
            validate_provider_input(&no_window).unwrap_err(),
            "provider model token limits must be positive"
        );
        let inverted = ProviderInput {
            models: vec![ProviderModelInput {
                context_window: 1_000,
                max_tokens: 2_000,
                ..ProviderModelInput::new("m", "M")
            }],
            ..provider()
        };
        assert_eq!(
            validate_provider_input(&inverted).unwrap_err(),
            "provider model max tokens cannot exceed its context window"
        );
        let exact = ProviderInput {
            models: vec![ProviderModelInput {
                context_window: 1_000,
                max_tokens: 1_000,
                ..ProviderModelInput::new("m", "M")
            }],
            ..provider()
        };
        validate_provider_input(&exact).expect("equal limits are allowed");
    }
}
