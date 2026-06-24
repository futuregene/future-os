//! Reads and writes the agent's model/provider configuration so the desktop
//! GUI can present a Providers settings page.
//!
//! The agent loads providers from `~/.future/agent/models.json` (merged over
//! its built-in catalog) and API keys from `~/.future/agent/auth.json`. The
//! built-in "FutureGene" provider is dynamic (its base URL comes from
//! `auth.json` `future.base_url`, defaulting to the Future API host) and is
//! presented read-only; user-defined providers live under
//! `models.json.providers` and are fully editable here.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

const DEFAULT_FUTURE_BASE_URL: &str = "http://api.westlakefuturegene.com";
const FUTURE_PROVIDER_ID: &str = "future";
const FUTURE_PROVIDER_NAME: &str = "FutureGene";

// Field-validation limits for custom providers (see PLAN.md「自定义 Provider 字段校验」).
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

pub fn list_agent_providers() -> Result<ProvidersView, crate::AppError> {
    let models = read_json(&models_json_path()?);
    // Display path: a corrupt auth.json shouldn't blank the providers list, so
    // fall back to empty (key badges show "未配置"); write paths stay strict.
    let auth = Value::Object(crate::auth_store::read().unwrap_or_default());

    let builtin = vec![BuiltinProvider {
        id: FUTURE_PROVIDER_ID.to_string(),
        name: FUTURE_PROVIDER_NAME.to_string(),
        base_url: resolve_future_base_url(&auth),
        has_api_key: auth_has_key(&auth, FUTURE_PROVIDER_ID),
    }];

    let mut custom = Vec::new();
    if let Some(providers) = models.get("providers").and_then(Value::as_object) {
        for (id, config) in providers {
            // FutureGene is shown above as built-in; never echo a stray `future`
            // entry (e.g. a hand-edited models.json) as a custom provider.
            if id == FUTURE_PROVIDER_ID {
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
                            Some(CustomProviderModel { id, name })
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
        model_values.push(json!({ "id": model_id, "name": model_name }));
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
    // Names must be unique (case-insensitive) across the built-in and other
    // custom providers, so the list and model grouping stay unambiguous.
    let normalized_name = name.to_lowercase();
    if normalized_name == FUTURE_PROVIDER_NAME.to_lowercase() {
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
    providers.insert(id.clone(), Value::Object(provider));
    write_json(&models_path, &models_doc)?;

    if let Some(key) = api_key {
        crate::auth_store::set_provider_key(&id, key)?;
    }

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

pub(crate) fn resolve_future_base_url(auth: &Value) -> String {
    auth.get(FUTURE_PROVIDER_ID)
        .and_then(|future| future.get("base_url"))
        .and_then(Value::as_str)
        .map(|value| value.trim_end_matches('/').to_string())
        .unwrap_or_else(|| DEFAULT_FUTURE_BASE_URL.to_string())
}

fn auth_has_key(auth: &Value, id: &str) -> bool {
    auth.get(id)
        .and_then(|entry| entry.get("key"))
        .and_then(Value::as_str)
        .map(|key| !key.trim().is_empty())
        .unwrap_or(false)
}

fn agent_dir() -> Result<PathBuf, crate::AppError> {
    let home = std::env::var("HOME").map_err(|_| "HOME environment variable is not set.")?;
    Ok(PathBuf::from(home).join(".future").join("agent"))
}

fn models_json_path() -> Result<PathBuf, crate::AppError> {
    Ok(agent_dir()?.join("models.json"))
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
    std::fs::write(path, serialized).map_err(crate::AppError::from)
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
        }];
        assert!(upsert_custom_provider(ok).is_ok());

        // Whitespace in model id is rejected.
        let mut bad = input("p2", "P2", true);
        bad.models = vec![CustomProviderModel {
            id: "bad id".to_string(),
            name: String::new(),
        }];
        assert!(upsert_custom_provider(bad).is_err());

        // Duplicate model ids are rejected.
        let mut dup = input("p3", "P3", true);
        dup.models = vec![
            CustomProviderModel {
                id: "m".to_string(),
                name: String::new(),
            },
            CustomProviderModel {
                id: "m".to_string(),
                name: String::new(),
            },
        ];
        assert!(upsert_custom_provider(dup).is_err());
    }
}
