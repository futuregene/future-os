//! Mutating command paths for provider configuration: built-in API key / Base
//! URL overrides and custom-provider upsert/delete. Each performs a strict,
//! per-path-locked models.json read-modify-write, then returns the refreshed
//! view. Also home to the two small models.json helpers the view shares.

use serde_json::{Map, Value};

use crate::auth_store::FUTURE_PROVIDER_ID;
use crate::config_io;

use super::catalog::{builtin_catalog_providers, models_json_path};
use super::validate::{
    is_ascii_no_control, validate_custom_provider, ValidatedCustomProvider, API_KEY_MAX_LEN,
    BASE_URL_MAX_LEN,
};
use super::{
    list_agent_providers, ProvidersView, SetBuiltinProviderBaseUrlInput,
    UpdateBuiltinProviderKeyInput, UpsertCustomProviderInput, BASE_URL_PLACEHOLDER,
    FUTURE_PROVIDER_NAME,
};

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
        return Err(format!("Unknown built-in provider: `{id}`.").into());
    }

    let api_key = input
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(key) = api_key {
        if key.len() > API_KEY_MAX_LEN {
            return Err("API Key exceeds the maximum length.".into());
        }
        if !is_ascii_no_control(key) {
            return Err("API Key contains illegal characters.".into());
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
        return Err("FutureGene's address is managed by the sign-in flow.".into());
    }
    if !builtin_catalog_providers().contains_key(id) {
        return Err(format!("Unknown built-in provider: `{id}`.").into());
    }

    let base_url = input.base_url.trim();
    // Validate the address before touching the file (it doesn't depend on the
    // current contents), so the locked read-modify-write below stays minimal.
    if !base_url.is_empty() {
        if base_url.len() > BASE_URL_MAX_LEN {
            return Err("Base URL is too long.".into());
        }
        match reqwest::Url::parse(base_url) {
            Ok(url) if matches!(url.scheme(), "http" | "https") => {}
            _ => return Err("Base URL must be a valid http/https address.".into()),
        }
        if base_url.contains(BASE_URL_PLACEHOLDER) {
            return Err(format!(
                "Please replace `{BASE_URL_PLACEHOLDER}` in the address with the real value."
            )
            .into());
        }
    }

    let models_path = models_json_path()?;
    config_io::with_config_lock(&models_path, || {
        let mut models_doc = config_io::read_json_object(&models_path)?;
        let root = models_doc
            .as_object_mut()
            .expect("read_json_object always returns an object");

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
        } else {
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
        }
        config_io::write_json_atomic(&models_path, &models_doc, false)
    })?;

    list_agent_providers()
}

pub fn upsert_custom_provider(
    input: UpsertCustomProviderInput,
) -> Result<ProvidersView, crate::AppError> {
    let ValidatedCustomProvider {
        id,
        name,
        api,
        base_url,
        api_key,
        model_values,
        create,
    } = validate_custom_provider(input)?;

    let models_path = models_json_path()?;
    config_io::with_config_lock(&models_path, || {
        let mut models_doc = config_io::read_json_object(&models_path)?;
        let root = models_doc
            .as_object_mut()
            .expect("read_json_object always returns an object");
        let providers = root
            .entry("providers")
            .or_insert_with(|| Value::Object(Map::new()));
        let providers = providers
            .as_object_mut()
            .ok_or_else(|| "models.json `providers` is not an object.".to_string())?;

        // Reject creating a provider whose id already exists (silent overwrite).
        if create && providers.contains_key(&id) {
            return Err(format!("Provider ID `{id}` already exists.").into());
        }
        let builtin_catalog = builtin_catalog_providers();
        if create && builtin_catalog.contains_key(&id) {
            return Err(format!("Provider ID `{id}` is reserved for a built-in provider.").into());
        }
        // Names must be unique (case-insensitive) across the built-in and other
        // custom providers, so the list and model grouping stay unambiguous.
        let normalized_name = name.to_lowercase();
        if normalized_name == FUTURE_PROVIDER_NAME.to_lowercase() {
            return Err(
                format!("Provider name `{name}` conflicts with a built-in provider.").into(),
            );
        }
        let builtin_name_taken = builtin_catalog.iter().any(|(builtin_id, provider)| {
            builtin_id != &id && provider.name.trim().to_lowercase() == normalized_name
        });
        if builtin_name_taken {
            return Err(
                format!("Provider name `{name}` conflicts with a built-in provider.").into(),
            );
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
            return Err(format!("Provider name `{name}` already exists.").into());
        }

        // Preserve any fields the GUI does not manage (e.g. `compat`).
        let mut provider = providers
            .get(&id)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        provider.insert("name".to_string(), Value::String(name.clone()));
        provider.insert("api".to_string(), Value::String(api.clone()));
        provider.insert("baseUrl".to_string(), Value::String(base_url.clone()));
        provider.insert("models".to_string(), Value::Array(model_values.clone()));
        // Write the API key first: if it fails we abort before persisting the
        // provider, avoiding a saved provider with a missing key while returning Err.
        if let Some(key) = api_key.as_deref() {
            crate::auth_store::set_provider_key(&id, key)?;
        }

        providers.insert(id.clone(), Value::Object(provider));
        config_io::write_json_atomic(&models_path, &models_doc, false)
    })?;

    list_agent_providers()
}

pub fn delete_custom_provider(id: String) -> Result<ProvidersView, crate::AppError> {
    let id = id.trim().to_string();
    if id.is_empty() {
        return Err("Provider id is required.".to_string().into());
    }
    // this deletes the provider's models.json entry *and* its auth.json
    // credentials. Only custom providers may be removed — guard the built-in
    // FutureGene (whose key is the user's sign-in) and every catalog provider, so
    // a stray id can't wipe login/override state the UI never offers to delete.
    if id == FUTURE_PROVIDER_ID {
        return Err("FutureGene is a built-in provider and cannot be deleted.".into());
    }
    if builtin_catalog_providers().contains_key(&id) {
        return Err(format!("`{id}` is a built-in provider and cannot be deleted.").into());
    }

    let models_path = models_json_path()?;
    config_io::with_config_lock(&models_path, || {
        let mut models_doc = config_io::read_json_object(&models_path)?;
        if let Some(providers) = models_doc
            .get_mut("providers")
            .and_then(Value::as_object_mut)
        {
            if providers.remove(&id).is_some() {
                config_io::write_json_atomic(&models_path, &models_doc, false)?;
            }
        }
        Ok(())
    })?;

    crate::auth_store::remove_provider_entry(&id)?;

    list_agent_providers()
}

/// True when a models.json provider entry only carries overrides (Base URL) for
/// a built-in provider, i.e. it defines no `name`, `api`, or explicit `models`.
/// Such entries are surfaced through the built-in list rather than as customs.
pub(super) fn is_override_only(config: &Value) -> bool {
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
pub(super) fn provider_base_url_override(models: &Value, id: &str) -> Option<String> {
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
