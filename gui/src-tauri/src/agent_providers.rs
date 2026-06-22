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
}

pub fn list_agent_providers() -> Result<ProvidersView, crate::AppError> {
    let models = read_json(&models_json_path()?);
    let auth = read_json(&auth_json_path()?);

    let builtin = vec![BuiltinProvider {
        id: FUTURE_PROVIDER_ID.to_string(),
        name: "FutureGene".to_string(),
        base_url: resolve_future_base_url(&auth),
        has_api_key: auth_has_key(&auth, FUTURE_PROVIDER_ID),
    }];

    let mut custom = Vec::new();
    if let Some(providers) = models.get("providers").and_then(Value::as_object) {
        for (id, config) in providers {
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
    let id = input.id.trim().to_string();
    if id.is_empty() {
        return Err("Provider id is required.".to_string().into());
    }
    if id == FUTURE_PROVIDER_ID {
        return Err("`future` is reserved for the built-in FutureGene provider."
            .to_string()
            .into());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(
            "Provider id may only contain letters, numbers, '-', '_' and '.'."
                .to_string()
                .into(),
        );
    }
    let base_url = input.base_url.trim();
    if base_url.is_empty() {
        return Err("Base URL is required.".to_string().into());
    }

    let api = {
        let trimmed = input.api.trim();
        if trimmed.is_empty() {
            "openai-completions".to_string()
        } else {
            trimmed.to_string()
        }
    };
    let name = {
        let trimmed = input.name.trim();
        if trimmed.is_empty() {
            id.clone()
        } else {
            trimmed.to_string()
        }
    };

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

    // Preserve any fields the GUI does not manage (e.g. `compat`).
    let mut provider = providers
        .get(&id)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    provider.insert("name".to_string(), Value::String(name));
    provider.insert("api".to_string(), Value::String(api));
    provider.insert("baseUrl".to_string(), Value::String(base_url.to_string()));
    provider.insert(
        "models".to_string(),
        Value::Array(
            input
                .models
                .iter()
                .filter(|model| !model.id.trim().is_empty())
                .map(|model| {
                    let model_id = model.id.trim();
                    let model_name = if model.name.trim().is_empty() {
                        model_id
                    } else {
                        model.name.trim()
                    };
                    json!({ "id": model_id, "name": model_name })
                })
                .collect(),
        ),
    );
    providers.insert(id.clone(), Value::Object(provider));
    write_json(&models_path, &models_doc)?;

    if let Some(key) = input
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        let auth_path = auth_json_path()?;
        let mut auth_doc = read_json(&auth_path);
        if !auth_doc.is_object() {
            auth_doc = json!({});
        }
        if let Some(auth) = auth_doc.as_object_mut() {
            auth.insert(id.clone(), json!({ "type": "api_key", "key": key }));
        }
        write_json(&auth_path, &auth_doc)?;
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

    let auth_path = auth_json_path()?;
    let mut auth_doc = read_json(&auth_path);
    if let Some(auth) = auth_doc.as_object_mut() {
        if auth.remove(&id).is_some() {
            write_json(&auth_path, &auth_doc)?;
        }
    }

    list_agent_providers()
}

fn resolve_future_base_url(auth: &Value) -> String {
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

fn auth_json_path() -> Result<PathBuf, crate::AppError> {
    Ok(agent_dir()?.join("auth.json"))
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
