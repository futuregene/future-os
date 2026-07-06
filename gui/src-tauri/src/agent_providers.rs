//! Reads and writes the agent's model/provider configuration so the desktop
//! GUI can present a Providers settings page.
//!
//! The agent loads providers from `~/.future/agent/models.json` (merged over
//! its built-in catalog) and API keys from `~/.future/agent/auth.json`. The
//! built-in "FutureGene" provider is dynamic (its base URL comes from
//! `auth.json` `future.platform_base_url` or legacy `future.base_url`,
//! defaulting to the Future API host) and is
//! presented read-only; user-defined providers live under
//! `models.json.providers` and are fully editable here.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::auth_store::{agent_dir, FUTURE_PROVIDER_ID};

#[path = "../../../agent/src/models/generated/mod.rs"]
mod generated_model_catalog;

/// Future platform root (no `/api`); auth/account endpoints hang off this and
/// the model API base is derived as `{platform}/api/v1`.
const DEFAULT_FUTURE_PLATFORM_URL: &str = "https://future-os.cn";
const FUTURE_PROVIDER_NAME: &str = "FutureGene";

/// Magic token in a built-in provider's catalog base URL marking it as a
/// placeholder the user must fill in (e.g. Azure's
/// `https://YOUR_RESOURCE.openai.azure.com/...`). Such providers get an editable
/// Base URL in the Providers page, stored as a `models.json` `baseUrl` override.
const BASE_URL_PLACEHOLDER: &str = "YOUR_RESOURCE";

// Field-validation limits for custom providers (see PLAN.md custom provider field validation).
const PROVIDER_ID_MIN_LEN: usize = 2;
const PROVIDER_ID_MAX_LEN: usize = 40;
const PROVIDER_NAME_MAX_LEN: usize = 40;
const BASE_URL_MAX_LEN: usize = 2048;
const API_KEY_MAX_LEN: usize = 512;
const MODEL_ID_MAX_LEN: usize = 100;
const MODEL_NAME_MAX_LEN: usize = 60;
const MAX_MODELS: usize = 100;
const ALLOWED_APIS: [&str; 3] = ["openai-completions", "openai-responses", "anthropic"];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvidersView {
    pub builtin: Vec<BuiltinProvider>,
    pub custom: Vec<CustomProvider>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinProvider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub has_api_key: bool,
    pub model_count: usize,
    /// True when the catalog base URL is a placeholder (contains
    /// [`BASE_URL_PLACEHOLDER`]) so the user must supply their own — the
    /// Providers page then offers a Base URL field alongside the API key.
    pub requires_base_url: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomProvider {
    pub id: String,
    pub name: String,
    pub api: String,
    pub base_url: String,
    pub has_api_key: bool,
    pub models: Vec<CustomProviderModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomProviderModel {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Whether the model accepts image input. Text is always supported and is
    /// implied — only image is tracked. Persisted in models.json as the
    /// `modalities` array (`["text"]` or `["text","image"]`) that the agent reads.
    #[serde(default)]
    pub supports_images: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertCustomProviderInput {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub api: String,
    #[serde(default)]
    pub base_url: String,
    /// Written to `auth.json` only when present and non-empty; otherwise any
    /// existing key is left untouched.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub models: Vec<CustomProviderModel>,
    /// True when adding a new provider (vs editing an existing one). Used to
    /// reject creating a provider whose id already exists, which would otherwise
    /// silently overwrite it.
    #[serde(default)]
    pub create: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBuiltinProviderKeyInput {
    pub id: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBuiltinProviderBaseUrlInput {
    pub id: String,
    /// The user-supplied Base URL. Empty clears the override (reverting to the
    /// catalog placeholder); otherwise it's validated and stored in models.json.
    #[serde(default)]
    pub base_url: String,
}

#[derive(Debug, Clone)]
struct CatalogProviderSummary {
    name: String,
    base_url: String,
    model_count: usize,
}

pub fn list_agent_providers() -> Result<ProvidersView, crate::AppError> {
    let models = read_json(&models_json_path()?);
    // Display path: a corrupt auth.json shouldn't blank the providers list, so
    // fall back to empty (key badges show "Not Configured"); write paths stay strict.
    let auth = Value::Object(crate::auth_store::read().unwrap_or_default());

    // Entries that fully define a provider (name/api/models) shadow a same-id
    // built-in and show as custom. "Override-only" entries (just a `baseUrl`,
    // e.g. an Azure resource URL) don't shadow — they stay built-in and only
    // supply the override, so the catalog's models remain available.
    let custom_provider_ids = models
        .get("providers")
        .and_then(Value::as_object)
        .map(|providers| {
            providers
                .iter()
                .filter(|(_, config)| !is_override_only(config))
                .map(|(id, _)| id.clone())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();

    let mut builtin = vec![BuiltinProvider {
        id: FUTURE_PROVIDER_ID.to_string(),
        name: FUTURE_PROVIDER_NAME.to_string(),
        base_url: resolve_future_base_url(&auth),
        has_api_key: auth_has_key(&auth, FUTURE_PROVIDER_ID),
        model_count: future_model_count(),
        requires_base_url: false,
    }];
    for (id, summary) in builtin_catalog_providers() {
        if custom_provider_ids.contains(&id) {
            continue;
        }
        // The catalog base URL is the source of truth for whether this provider
        // needs a user-supplied Base URL; a stored override then fills it in.
        let requires_base_url = summary.base_url.contains(BASE_URL_PLACEHOLDER);
        let base_url = provider_base_url_override(&models, &id).unwrap_or(summary.base_url);
        builtin.push(BuiltinProvider {
            has_api_key: auth_has_key(&auth, &id),
            id,
            name: summary.name,
            base_url,
            model_count: summary.model_count,
            requires_base_url,
        });
    }

    let mut custom = Vec::new();
    if let Some(providers) = models.get("providers").and_then(Value::as_object) {
        for (id, config) in providers {
            // FutureGene is shown above as built-in; never echo a stray `future`
            // entry (e.g. a hand-edited models.json) as a custom provider.
            if id == FUTURE_PROVIDER_ID {
                continue;
            }
            // Override-only entries (base-URL fills for a built-in provider) are
            // surfaced through the built-in list, not as standalone customs.
            if is_override_only(config) {
                continue;
            }
            let api = config
                .get("api")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let base_url = config
                .get("baseUrl")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let name = config
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| id.clone());
            let models = config
                .get("models")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|model| {
                            let id = model.get("id").and_then(Value::as_str)?.to_string();
                            let name = model
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or(&id)
                                .to_string();
                            let supports_images = model
                                .get("modalities")
                                .and_then(Value::as_array)
                                .map(|items| {
                                    items.iter().any(|item| item.as_str() == Some("image"))
                                })
                                .unwrap_or(false);
                            Some(CustomProviderModel {
                                id,
                                name,
                                supports_images,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            custom.push(CustomProvider {
                has_api_key: auth_has_key(&auth, id),
                api,
                base_url,
                name,
                models,
                id: id.clone(),
            });
        }
    }
    custom.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(ProvidersView { builtin, custom })
}

pub fn update_builtin_provider_key(
    input: UpdateBuiltinProviderKeyInput,
) -> Result<ProvidersView, crate::AppError> {
    let id = input.id.trim();
    if id.is_empty() {
        return Err("Provider id is required.".to_string().into());
    }
    if id == FUTURE_PROVIDER_ID {
        return Err("FutureGene uses the sign-in flow.".to_string().into());
    }
    if !builtin_catalog_providers().contains_key(id) {
        return Err(format!("未知的内置提供商：`{id}`。").into());
    }

    let api_key = input
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(key) = api_key {
        if key.len() > API_KEY_MAX_LEN {
            return Err("API Key 过长。".into());
        }
        if !is_ascii_no_control(key) {
            return Err("API Key 含非法字符。".into());
        }
        crate::auth_store::set_provider_key(id, key)?;
    } else {
        crate::auth_store::remove_provider_key(id)?;
    }

    list_agent_providers()
}

/// Set (or clear) a built-in provider's Base URL override in models.json. Used
/// for catalog providers whose base URL is a placeholder (see
/// [`BASE_URL_PLACEHOLDER`]); the agent applies it to that provider's models.
pub fn set_builtin_provider_base_url(
    input: SetBuiltinProviderBaseUrlInput,
) -> Result<ProvidersView, crate::AppError> {
    let id = input.id.trim();
    if id.is_empty() {
        return Err("Provider id is required.".to_string().into());
    }
    if id == FUTURE_PROVIDER_ID {
        return Err("FutureGene 的地址由登录流程管理。".into());
    }
    if !builtin_catalog_providers().contains_key(id) {
        return Err(format!("未知的内置提供商：`{id}`。").into());
    }

    let base_url = input.base_url.trim();
    let models_path = models_json_path()?;
    let mut models_doc = read_json(&models_path);
    if !models_doc.is_object() {
        models_doc = json!({});
    }
    let root = models_doc
        .as_object_mut()
        .ok_or_else(|| "models.json is not a JSON object.".to_string())?;

    if base_url.is_empty() {
        // Clear the override; drop the entry entirely if nothing else remains.
        if let Some(providers) = root.get_mut("providers").and_then(Value::as_object_mut) {
            if let Some(entry) = providers.get_mut(id).and_then(Value::as_object_mut) {
                entry.remove("baseUrl");
                if entry.is_empty() {
                    providers.remove(id);
                }
            }
        }
        write_json(&models_path, &models_doc)?;
        return list_agent_providers();
    }

    if base_url.len() > BASE_URL_MAX_LEN {
        return Err("Base URL 过长。".into());
    }
    match reqwest::Url::parse(base_url) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => {}
        _ => return Err("Base URL 必须是合法的 http/https 地址。".into()),
    }
    if base_url.contains(BASE_URL_PLACEHOLDER) {
        return Err(format!("请把地址中的 `{BASE_URL_PLACEHOLDER}` 替换为真实值。").into());
    }

    let providers = root
        .entry("providers")
        .or_insert_with(|| Value::Object(Map::new()));
    let providers = providers
        .as_object_mut()
        .ok_or_else(|| "models.json `providers` is not an object.".to_string())?;
    // Preserve any fields the GUI does not manage on this entry.
    let mut provider = providers
        .get(id)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    provider.insert("baseUrl".to_string(), Value::String(base_url.to_string()));
    providers.insert(id.to_string(), Value::Object(provider));
    write_json(&models_path, &models_doc)?;

    list_agent_providers()
}

pub fn upsert_custom_provider(
    input: UpsertCustomProviderInput,
) -> Result<ProvidersView, crate::AppError> {
    // Provider id: lowercased, [a-z0-9_-], length-bounded, `future` reserved.
    let id = input.id.trim().to_lowercase();
    if id.is_empty() {
        return Err("请填写提供商 ID。".into());
    }
    if id == FUTURE_PROVIDER_ID {
        return Err("`future` 为内置 FutureGene 保留，请换一个 ID。".into());
    }
    if id.len() < PROVIDER_ID_MIN_LEN || id.len() > PROVIDER_ID_MAX_LEN {
        return Err(format!(
            "提供商 ID 长度需在 {PROVIDER_ID_MIN_LEN}–{PROVIDER_ID_MAX_LEN} 个字符之间。"
        )
        .into());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err("提供商 ID 只能包含小写字母、数字、'-' 和 '_'。".into());
    }

    // Base URL: parseable http/https, length-bounded.
    let base_url = input.base_url.trim().to_string();
    if base_url.is_empty() {
        return Err("请填写 Base URL。".into());
    }
    if base_url.len() > BASE_URL_MAX_LEN {
        return Err("Base URL 过长。".into());
    }
    match reqwest::Url::parse(&base_url) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => {}
        _ => return Err("Base URL 必须是合法的 http/https 地址。".into()),
    }

    // API type: must be a supported value.
    let api = {
        let trimmed = input.api.trim();
        if trimmed.is_empty() {
            "openai-completions".to_string()
        } else if ALLOWED_APIS.contains(&trimmed) {
            trimmed.to_string()
        } else {
            return Err(format!("不支持的 API 类型：`{trimmed}`。").into());
        }
    };

    // Name: optional (falls back to id); when given, ASCII only (no CJK / emoji /
    // full-width), restricted punctuation, length-bounded.
    let name = {
        let trimmed = input.name.trim();
        if trimmed.is_empty() {
            id.clone()
        } else {
            if trimmed.chars().count() > PROVIDER_NAME_MAX_LEN {
                return Err(format!("提供商名称不能超过 {PROVIDER_NAME_MAX_LEN} 个字符。").into());
            }
            if !is_provider_name_ok(trimmed) {
                return Err(
                    "提供商名称只能包含字母、数字、空格和 _.()-，不支持中文 / emoji / 全角字符。"
                        .into(),
                );
            }
            trimmed.to_string()
        }
    };

    // API key: validated here, written after models.json (so an invalid key
    // doesn't leave a half-applied change).
    let api_key = input
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    if let Some(key) = api_key {
        if key.len() > API_KEY_MAX_LEN {
            return Err("API Key 过长。".into());
        }
        if !is_ascii_no_control(key) {
            return Err("API Key 含非法字符。".into());
        }
    }

    // Models: validate ids/names, dedupe within the provider, cap the count.
    let mut seen_model_ids = std::collections::HashSet::new();
    let mut model_values: Vec<Value> = Vec::new();
    for model in &input.models {
        let model_id = model.id.trim();
        if model_id.is_empty() {
            continue;
        }
        if model_id.len() > MODEL_ID_MAX_LEN {
            return Err(format!("模型 ID `{model_id}` 过长（上限 {MODEL_ID_MAX_LEN}）。").into());
        }
        if !is_model_id_ok(model_id) {
            return Err(format!("模型 ID `{model_id}` 含非法字符。").into());
        }
        if !seen_model_ids.insert(model_id.to_string()) {
            return Err(format!("模型 ID `{model_id}` 重复。").into());
        }
        let model_name = model.name.trim();
        let model_name = if model_name.is_empty() {
            model_id
        } else {
            if model_name.chars().count() > MODEL_NAME_MAX_LEN {
                return Err(format!("模型名称不能超过 {MODEL_NAME_MAX_LEN} 个字符。").into());
            }
            if !is_ascii_no_control(model_name) {
                return Err(format!("模型名称 `{model_name}` 含非法字符。").into());
            }
            model_name
        };
        // Text is always supported; image is opt-in. Persist as `modalities` so
        // the agent's models.json loader maps it to the model's input modalities.
        let mut modalities = vec![Value::String("text".to_string())];
        if model.supports_images {
            modalities.push(Value::String("image".to_string()));
        }
        model_values.push(json!({
            "id": model_id,
            "name": model_name,
            "modalities": modalities,
        }));
    }
    if model_values.len() > MAX_MODELS {
        return Err(format!("模型数量不能超过 {MAX_MODELS} 个。").into());
    }

    let models_path = models_json_path()?;
    let mut models_doc = read_json(&models_path);
    if !models_doc.is_object() {
        models_doc = json!({});
    }
    let root = models_doc
        .as_object_mut()
        .ok_or_else(|| "models.json is not a JSON object.".to_string())?;
    let providers = root
        .entry("providers")
        .or_insert_with(|| Value::Object(Map::new()));
    let providers = providers
        .as_object_mut()
        .ok_or_else(|| "models.json `providers` is not an object.".to_string())?;

    // Reject creating a provider whose id already exists (silent overwrite).
    if input.create && providers.contains_key(&id) {
        return Err(format!("提供商 ID `{id}` 已存在。").into());
    }
    let builtin_catalog = builtin_catalog_providers();
    if input.create && builtin_catalog.contains_key(&id) {
        return Err(format!("提供商 ID `{id}` 为内置提供商保留。").into());
    }
    // Names must be unique (case-insensitive) across the built-in and other
    // custom providers, so the list and model grouping stay unambiguous.
    let normalized_name = name.to_lowercase();
    if normalized_name == FUTURE_PROVIDER_NAME.to_lowercase() {
        return Err(format!("提供商名称 `{name}` 与内置提供商重复。").into());
    }
    let builtin_name_taken = builtin_catalog.iter().any(|(builtin_id, provider)| {
        builtin_id != &id && provider.name.trim().to_lowercase() == normalized_name
    });
    if builtin_name_taken {
        return Err(format!("提供商名称 `{name}` 与内置提供商重复。").into());
    }
    let name_taken = providers.iter().any(|(other_id, config)| {
        other_id != &id
            && config
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(other_id)
                .trim()
                .to_lowercase()
                == normalized_name
    });
    if name_taken {
        return Err(format!("提供商名称 `{name}` 已存在。").into());
    }

    // Preserve any fields the GUI does not manage (e.g. `compat`).
    let mut provider = providers
        .get(&id)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    provider.insert("name".to_string(), Value::String(name));
    provider.insert("api".to_string(), Value::String(api));
    provider.insert("baseUrl".to_string(), Value::String(base_url));
    provider.insert("models".to_string(), Value::Array(model_values));
    // Write the API key first: if it fails we abort before persisting the
    // provider, avoiding a saved provider with a missing key while returning Err.
    if let Some(key) = api_key {
        crate::auth_store::set_provider_key(&id, key)?;
    }

    providers.insert(id.clone(), Value::Object(provider));
    write_json(&models_path, &models_doc)?;

    list_agent_providers()
}

pub fn delete_custom_provider(id: String) -> Result<ProvidersView, crate::AppError> {
    let id = id.trim().to_string();
    if id.is_empty() {
        return Err("Provider id is required.".to_string().into());
    }

    let models_path = models_json_path()?;
    let mut models_doc = read_json(&models_path);
    if let Some(providers) = models_doc
        .get_mut("providers")
        .and_then(Value::as_object_mut)
    {
        providers.remove(&id);
        write_json(&models_path, &models_doc)?;
    }

    crate::auth_store::remove_provider_entry(&id)?;

    list_agent_providers()
}

/// Resolve the Future **platform** root (no `/api`), mirroring the CLI's
/// `getPlatformUrl()` precedence (see `cli/src/utils/platform.ts`):
///   1. `future.platform_base_url`
///   2. `future.base_url` with a trailing `/api` stripped (the CLI writes
///      `base_url = {platform}/api`)
///   3. [`DEFAULT_FUTURE_PLATFORM_URL`]
///
/// Auth/account endpoints live here (`{platform}/client/v1/...`); the model API
/// base is [`resolve_future_base_url`].
pub(crate) fn resolve_future_platform_url(auth: &Value) -> String {
    let Some(future) = auth.get(FUTURE_PROVIDER_ID) else {
        return DEFAULT_FUTURE_PLATFORM_URL.to_string();
    };
    if let Some(platform_url) = future.get("platform_base_url").and_then(Value::as_str) {
        return platform_url.trim_end_matches('/').to_string();
    }
    if let Some(base_url) = future.get("base_url").and_then(Value::as_str) {
        let trimmed = base_url.trim_end_matches('/');
        let platform = trimmed.strip_suffix("/api").unwrap_or(trimmed);
        return platform.trim_end_matches('/').to_string();
    }
    DEFAULT_FUTURE_PLATFORM_URL.to_string()
}

/// Resolve the FutureGene **model API** base URL: `{platform}/api/v1`. This is
/// what the Providers page shows and what model calls use.
pub(crate) fn resolve_future_base_url(auth: &Value) -> String {
    format!("{}/api/v1", resolve_future_platform_url(auth))
}

/// True when a models.json provider entry only carries overrides (Base URL) for
/// a built-in provider, i.e. it defines no `name`, `api`, or explicit `models`.
/// Such entries are surfaced through the built-in list rather than as customs.
fn is_override_only(config: &Value) -> bool {
    let has_str = |key: &str| {
        config
            .get(key)
            .and_then(Value::as_str)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    };
    let has_models = config
        .get("models")
        .and_then(Value::as_array)
        .map(|items| !items.is_empty())
        .unwrap_or(false);
    !has_str("name") && !has_str("api") && !has_models
}

/// The stored Base URL override for a provider, if any (non-empty).
fn provider_base_url_override(models: &Value, id: &str) -> Option<String> {
    models
        .get("providers")
        .and_then(Value::as_object)
        .and_then(|providers| providers.get(id))
        .and_then(|config| config.get("baseUrl"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn auth_has_key(auth: &Value, id: &str) -> bool {
    auth.get(id)
        .and_then(|entry| entry.get("key"))
        .and_then(Value::as_str)
        .map(|key| !key.trim().is_empty())
        .unwrap_or(false)
}

fn builtin_catalog_providers() -> BTreeMap<String, CatalogProviderSummary> {
    let mut providers = BTreeMap::new();
    for model in generated_model_catalog::init_builtin_models() {
        if model.provider.is_empty() || model.provider == FUTURE_PROVIDER_ID {
            continue;
        }
        let entry =
            providers
                .entry(model.provider.clone())
                .or_insert_with(|| CatalogProviderSummary {
                    name: provider_display_name(&model.provider),
                    base_url: model.base_url.clone(),
                    model_count: 0,
                });
        entry.model_count += 1;
        if entry.base_url.is_empty() && !model.base_url.is_empty() {
            entry.base_url = model.base_url;
        }
    }
    providers
}

fn provider_display_name(id: &str) -> String {
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
                match chars.next() {
                    Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn models_json_path() -> Result<PathBuf, crate::AppError> {
    Ok(agent_dir()?.join("models.json"))
}

fn future_models_cache_path() -> Result<PathBuf, crate::AppError> {
    Ok(agent_dir()?.join(".future-models-cache.json"))
}

fn future_model_count() -> usize {
    future_models_cache_path()
        .ok()
        .and_then(|path| {
            read_json(&path)
                .get("models")
                .and_then(Value::as_array)
                .map(Vec::len)
        })
        .unwrap_or(0)
}

fn read_json(path: &PathBuf) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<Value>(&contents).ok())
        .unwrap_or_else(|| json!({}))
}

fn write_json(path: &PathBuf, value: &Value) -> Result<(), crate::AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let serialized = serde_json::to_string_pretty(value)?;
    // Atomic write (temp + rename) so a crash mid-write can't truncate/corrupt
    // models.json.
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, serialized.as_bytes())?;
    std::fs::rename(&tmp, path).map_err(crate::AppError::from)
}

/// Provider display name: ASCII letters/digits, space, and `_.()-` only —
/// rejects control chars and all non-ASCII (CJK / emoji / full-width).
fn is_provider_name_ok(value: &str) -> bool {
    value.chars().all(|c| {
        c.is_ascii()
            && !c.is_control()
            && (c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '.' | '(' | ')' | '-'))
    })
}

/// Model id: ASCII, no whitespace, plus `._:/-` (covers ids like
/// `anthropic/claude-3.5-sonnet`).
fn is_model_id_ok(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|c| {
            c.is_ascii()
                && !c.is_whitespace()
                && (c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-'))
        })
}

/// Free-form-ish text (model name, API key): ASCII, no control chars.
fn is_ascii_no_control(value: &str) -> bool {
    value.chars().all(|c| c.is_ascii() && !c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::MutexGuard;

    struct HomeGuard {
        previous: Option<String>,
        dir: PathBuf,
        _lock: MutexGuard<'static, ()>,
    }

    impl HomeGuard {
        fn new(label: &str) -> Self {
            let lock = crate::TEST_HOME_LOCK
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let previous = std::env::var("HOME").ok();
            let dir = std::env::temp_dir().join(format!(
                "futureos-prov-test-{}-{}",
                std::process::id(),
                label
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("HOME", &dir);
            HomeGuard {
                previous,
                dir,
                _lock: lock,
            }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn input(id: &str, name: &str, create: bool) -> UpsertCustomProviderInput {
        UpsertCustomProviderInput {
            id: id.to_string(),
            name: name.to_string(),
            api: "openai-completions".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            api_key: None,
            models: vec![],
            create,
        }
    }

    #[test]
    fn create_rejects_existing_id() {
        let _home = HomeGuard::new("dup-id");
        upsert_custom_provider(input("dashscope", "DashScope", true)).unwrap();
        // Re-creating the same id must fail rather than silently overwrite.
        let err = upsert_custom_provider(input("dashscope", "Other", true)).unwrap_err();
        assert!(err.to_string().contains("已存在"));
    }

    #[test]
    fn edit_allows_same_id() {
        let _home = HomeGuard::new("edit-id");
        upsert_custom_provider(input("dashscope", "DashScope", true)).unwrap();
        // Editing (create = false) the same id is fine.
        upsert_custom_provider(input("dashscope", "DashScope 2", false)).unwrap();
        let view = list_agent_providers().unwrap();
        assert_eq!(view.custom.len(), 1);
        assert_eq!(view.custom[0].name, "DashScope 2");
    }

    #[test]
    fn rejects_duplicate_name_case_insensitive() {
        let _home = HomeGuard::new("dup-name");
        upsert_custom_provider(input("p1", "DashScope", true)).unwrap();
        let err = upsert_custom_provider(input("p2", "dashscope", true)).unwrap_err();
        assert!(err.to_string().contains("已存在"));
    }

    #[test]
    fn rejects_builtin_name() {
        let _home = HomeGuard::new("builtin-name");
        let err = upsert_custom_provider(input("mine", "futuregene", true)).unwrap_err();
        assert!(err.to_string().contains("内置"));
    }

    #[test]
    fn reserves_future_id() {
        let _home = HomeGuard::new("future-id");
        let err = upsert_custom_provider(input("future", "Mine", true)).unwrap_err();
        assert!(err.to_string().contains("future") || err.to_string().contains("reserved"));
    }

    #[test]
    fn list_filters_stray_future_entry() {
        let _home = HomeGuard::new("future-filter");
        // Simulate a hand-edited models.json that contains a `future` provider.
        let path = models_json_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"providers":{"future":{"name":"Bogus","baseUrl":"x"},"zai":{"name":"ZAI","baseUrl":"y"}}}"#,
        )
        .unwrap();
        let view = list_agent_providers().unwrap();
        assert!(view.custom.iter().all(|p| p.id != "future"));
        assert!(view.custom.iter().any(|p| p.id == "zai"));
    }

    #[test]
    fn list_includes_catalog_providers_after_future() {
        let _home = HomeGuard::new("catalog-list");
        let view = list_agent_providers().unwrap();
        assert_eq!(view.builtin.first().map(|p| p.id.as_str()), Some("future"));
        let openai = view.builtin.iter().find(|p| p.id == "openai").unwrap();
        assert_eq!(openai.name, "OpenAI");
        assert!(openai.model_count > 0);
    }

    #[test]
    fn future_provider_uses_cached_model_count() {
        let _home = HomeGuard::new("future-count");
        let path = future_models_cache_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"fetched_at":1,"models":[{"id":"m1"},{"id":"m2"}]}"#,
        )
        .unwrap();

        let view = list_agent_providers().unwrap();
        assert_eq!(
            view.builtin
                .iter()
                .find(|provider| provider.id == "future")
                .map(|provider| provider.model_count),
            Some(2)
        );
    }

    #[test]
    fn custom_provider_shadows_builtin_catalog_provider() {
        let _home = HomeGuard::new("catalog-shadow");
        let path = models_json_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"providers":{"openai":{"name":"My OpenAI","api":"openai-completions","baseUrl":"https://proxy.example.com/v1","models":[]}}}"#,
        )
        .unwrap();
        let view = list_agent_providers().unwrap();
        assert!(view.builtin.iter().all(|p| p.id != "openai"));
        assert_eq!(view.custom.len(), 1);
        assert_eq!(view.custom[0].id, "openai");
    }

    #[test]
    fn update_builtin_provider_key_sets_and_clears_auth_entry() {
        let _home = HomeGuard::new("builtin-key");
        let view = update_builtin_provider_key(UpdateBuiltinProviderKeyInput {
            id: "openai".to_string(),
            api_key: Some("sk-test".to_string()),
        })
        .unwrap();
        assert!(
            view.builtin
                .iter()
                .find(|provider| provider.id == "openai")
                .unwrap()
                .has_api_key
        );
        assert_eq!(
            crate::auth_store::read()
                .unwrap()
                .get("openai")
                .and_then(Value::as_object)
                .and_then(|entry| entry.get("key"))
                .and_then(Value::as_str),
            Some("sk-test")
        );

        let view = update_builtin_provider_key(UpdateBuiltinProviderKeyInput {
            id: "openai".to_string(),
            api_key: None,
        })
        .unwrap();
        assert!(
            !view
                .builtin
                .iter()
                .find(|provider| provider.id == "openai")
                .unwrap()
                .has_api_key
        );
        assert!(crate::auth_store::read()
            .unwrap()
            .get("openai")
            .and_then(Value::as_object)
            .and_then(|entry| entry.get("key"))
            .is_none());
    }

    #[test]
    fn create_rejects_builtin_catalog_id_and_name() {
        let _home = HomeGuard::new("builtin-collision");
        let id_err = upsert_custom_provider(input("openai", "OpenAI Proxy", true)).unwrap_err();
        assert!(id_err.to_string().contains("内置"));

        let name_err = upsert_custom_provider(input("p1", "OpenAI", true)).unwrap_err();
        assert!(name_err.to_string().contains("内置"));
    }

    #[test]
    fn id_is_lowercased() {
        let _home = HomeGuard::new("id-lower");
        upsert_custom_provider(input("DashScope", "DashScope", true)).unwrap();
        let view = list_agent_providers().unwrap();
        assert_eq!(view.custom.len(), 1);
        assert_eq!(view.custom[0].id, "dashscope");
    }

    #[test]
    fn rejects_bad_id_charset_and_length() {
        let _home = HomeGuard::new("id-bad");
        // Disallowed punctuation (dot/space).
        assert!(upsert_custom_provider(input("a.b", "A", true)).is_err());
        assert!(upsert_custom_provider(input("a b", "A", true)).is_err());
        // Too short.
        assert!(upsert_custom_provider(input("a", "A", true)).is_err());
    }

    #[test]
    fn rejects_non_ascii_name() {
        let _home = HomeGuard::new("name-cjk");
        assert!(upsert_custom_provider(input("p1", "中文", true)).is_err());
        assert!(upsert_custom_provider(input("p2", "ＦＵＬＬ", true)).is_err());
        assert!(upsert_custom_provider(input("p3", "emoji 🚀", true)).is_err());
    }

    #[test]
    fn rejects_bad_base_url_and_api() {
        let _home = HomeGuard::new("url-api");
        let mut bad_url = input("p1", "P1", true);
        bad_url.base_url = "ftp://example.com".to_string();
        assert!(upsert_custom_provider(bad_url).is_err());

        let mut bad_api = input("p2", "P2", true);
        bad_api.api = "made-up".to_string();
        assert!(upsert_custom_provider(bad_api).is_err());
    }

    #[test]
    fn validates_models() {
        let _home = HomeGuard::new("models");
        // Valid composite model id with `/` and `.`.
        let mut ok = input("p1", "P1", true);
        ok.models = vec![CustomProviderModel {
            id: "anthropic/claude-3.5-sonnet".to_string(),
            name: String::new(),
            supports_images: false,
        }];
        assert!(upsert_custom_provider(ok).is_ok());

        // Whitespace in model id is rejected.
        let mut bad = input("p2", "P2", true);
        bad.models = vec![CustomProviderModel {
            id: "bad id".to_string(),
            name: String::new(),
            supports_images: false,
        }];
        assert!(upsert_custom_provider(bad).is_err());

        // Duplicate model ids are rejected.
        let mut dup = input("p3", "P3", true);
        dup.models = vec![
            CustomProviderModel {
                id: "m".to_string(),
                name: String::new(),
                supports_images: false,
            },
            CustomProviderModel {
                id: "m".to_string(),
                name: String::new(),
                supports_images: false,
            },
        ];
        assert!(upsert_custom_provider(dup).is_err());
    }

    #[test]
    fn model_modalities_round_trip() {
        let _home = HomeGuard::new("modalities");
        let mut in_ = input("p1", "P1", true);
        in_.models = vec![
            CustomProviderModel {
                id: "text-only".to_string(),
                name: String::new(),
                supports_images: false,
            },
            CustomProviderModel {
                id: "vision".to_string(),
                name: String::new(),
                supports_images: true,
            },
        ];
        upsert_custom_provider(in_).unwrap();

        // Persisted as a `modalities` array the agent reads.
        let doc = read_json(&models_json_path().unwrap());
        let models = doc["providers"]["p1"]["models"].as_array().unwrap();
        let vision = models.iter().find(|m| m["id"] == "vision").unwrap();
        assert_eq!(vision["modalities"], json!(["text", "image"]));
        let text_only = models.iter().find(|m| m["id"] == "text-only").unwrap();
        assert_eq!(text_only["modalities"], json!(["text"]));

        // And surfaces back through the view as supports_images.
        let view = list_agent_providers().unwrap();
        let provider = view.custom.iter().find(|p| p.id == "p1").unwrap();
        assert!(
            provider
                .models
                .iter()
                .find(|m| m.id == "vision")
                .unwrap()
                .supports_images
        );
        assert!(
            !provider
                .models
                .iter()
                .find(|m| m.id == "text-only")
                .unwrap()
                .supports_images
        );
    }

    #[test]
    fn azure_provider_requires_base_url_flag() {
        let _home = HomeGuard::new("azure-requires");
        let view = list_agent_providers().unwrap();
        let azure = view
            .builtin
            .iter()
            .find(|p| p.id == "azure-openai-responses")
            .expect("azure provider present in catalog");
        assert!(azure.requires_base_url);
        assert!(azure.base_url.contains("YOUR_RESOURCE"));
    }

    #[test]
    fn set_builtin_base_url_override_keeps_provider_builtin() {
        let _home = HomeGuard::new("azure-override");
        let view = set_builtin_provider_base_url(SetBuiltinProviderBaseUrlInput {
            id: "azure-openai-responses".to_string(),
            base_url: "https://my-res.openai.azure.com/openai/v1".to_string(),
        })
        .unwrap();

        // Still built-in (not moved to custom), with the override applied and the
        // requires-base-url flag intact so it stays editable.
        assert!(view.custom.iter().all(|p| p.id != "azure-openai-responses"));
        let azure = view
            .builtin
            .iter()
            .find(|p| p.id == "azure-openai-responses")
            .unwrap();
        assert_eq!(azure.base_url, "https://my-res.openai.azure.com/openai/v1");
        assert!(azure.requires_base_url);
        assert!(azure.model_count > 0);

        // Persisted as a plain baseUrl override the agent reads.
        let doc = read_json(&models_json_path().unwrap());
        assert_eq!(
            doc["providers"]["azure-openai-responses"]["baseUrl"],
            json!("https://my-res.openai.azure.com/openai/v1")
        );

        // Clearing removes the override entirely.
        set_builtin_provider_base_url(SetBuiltinProviderBaseUrlInput {
            id: "azure-openai-responses".to_string(),
            base_url: String::new(),
        })
        .unwrap();
        let doc = read_json(&models_json_path().unwrap());
        assert!(doc["providers"].get("azure-openai-responses").is_none());
    }

    #[test]
    fn set_builtin_base_url_rejects_placeholder_and_bad_url() {
        let _home = HomeGuard::new("azure-reject");
        let placeholder = set_builtin_provider_base_url(SetBuiltinProviderBaseUrlInput {
            id: "azure-openai-responses".to_string(),
            base_url: "https://YOUR_RESOURCE.openai.azure.com/openai/v1".to_string(),
        });
        assert!(placeholder.is_err());

        let bad = set_builtin_provider_base_url(SetBuiltinProviderBaseUrlInput {
            id: "azure-openai-responses".to_string(),
            base_url: "ftp://example.com".to_string(),
        });
        assert!(bad.is_err());
    }

    #[test]
    fn platform_url_defaults_when_absent() {
        assert_eq!(
            resolve_future_platform_url(&json!({})),
            DEFAULT_FUTURE_PLATFORM_URL
        );
        assert_eq!(
            resolve_future_base_url(&json!({})),
            format!("{DEFAULT_FUTURE_PLATFORM_URL}/api/v1")
        );
    }

    #[test]
    fn platform_url_strips_trailing_api_from_base_url() {
        // The CLI writes `base_url = {platform}/api`; the platform is that minus /api.
        let auth = json!({ "future": { "base_url": "https://future-os.cn/api" } });
        assert_eq!(resolve_future_platform_url(&auth), "https://future-os.cn");
        assert_eq!(
            resolve_future_base_url(&auth),
            "https://future-os.cn/api/v1"
        );

        let trailing = json!({ "future": { "base_url": "https://future-os.cn/api/" } });
        assert_eq!(
            resolve_future_platform_url(&trailing),
            "https://future-os.cn"
        );
    }

    #[test]
    fn platform_url_prefers_platform_base_url() {
        let auth = json!({ "future": { "platform_base_url": "https://staging.example.com/" } });
        assert_eq!(
            resolve_future_platform_url(&auth),
            "https://staging.example.com"
        );
        assert_eq!(
            resolve_future_base_url(&auth),
            "https://staging.example.com/api/v1"
        );
    }

    #[test]
    fn base_url_without_api_suffix_is_used_as_platform() {
        // A bare host (no /api) is treated as the platform root verbatim.
        let auth = json!({ "future": { "base_url": "https://custom.example.com" } });
        assert_eq!(
            resolve_future_platform_url(&auth),
            "https://custom.example.com"
        );
        assert_eq!(
            resolve_future_base_url(&auth),
            "https://custom.example.com/api/v1"
        );
    }
}
