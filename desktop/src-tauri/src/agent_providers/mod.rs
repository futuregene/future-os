//! Reads and writes the agent's model/provider configuration so the desktop
//! GUI can present a Providers settings page.
//!
//! The agent loads providers from `~/.future/agent/models.json` (merged over
//! its built-in catalog) and API keys from `~/.future/agent/auth.json`. The
//! built-in "FutureGene" provider is dynamic (its base URL is resolved from
//! the `future` entry in `auth.json` — legacy `base_url` first, then
//! `platform_base_url`, per the shared contract in `packages/rpc/proto/future.proto` —
//! defaulting to the Future API host) and is
//! presented read-only; user-defined providers live under
//! `models.json.providers` and are fully editable here.
//!
//! Split across submodules: `catalog` (built-in catalog + config paths),
//! `validate` (custom-provider field validation), `write` (the mutating command
//! paths). This module owns the public DTOs and the read-only `list` view.

#[cfg(test)]
use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Value;

#[cfg(test)]
use crate::auth_store::FUTURE_PROVIDER_ID;
#[cfg(test)]
use crate::config_io;

mod catalog;
#[cfg(test)]
mod tests;
mod validate;
mod write;

pub use write::{
    delete_custom_provider, set_builtin_provider_base_url, update_builtin_provider,
    update_builtin_provider_key, upsert_custom_provider,
};

#[cfg(test)]
use catalog::{future_model_count, models_json_path, CatalogProviderSummary};
#[cfg(test)]
use write::{is_override_only, provider_base_url_override};

/// Display name of the built-in Future provider. Shared with `write` so the
/// custom-provider name check can reject collisions with it.
pub(super) const FUTURE_PROVIDER_NAME: &str = "Future";

/// Magic token in a built-in provider's catalog base URL marking it as a
/// placeholder the user must fill in (e.g. Azure's
/// `https://YOUR_RESOURCE.openai.azure.com/...`). Such providers get an editable
/// Base URL in the Providers page, stored as a `models.json` `baseUrl` override.
pub(super) const BASE_URL_PLACEHOLDER: &str = "YOUR_RESOURCE";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvidersView {
    pub builtin: Vec<BuiltinProvider>,
    pub custom: Vec<CustomProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Maximum total context window, in tokens.
    #[serde(default = "default_context_window")]
    pub context_window: i32,
    /// Maximum tokens generated in one response.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: i32,
}

const fn default_context_window() -> i32 { 128_000 }
const fn default_max_tokens() -> i32 { 16_384 }

#[derive(Debug, Clone, Deserialize)]
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

/// Atomic built-in provider form submission. Presence is explicit so a single
/// Agent transaction can update Base URL and set/clear/keep the key.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBuiltinProviderInput {
    pub id: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub update_api_key: bool,
}

pub async fn list_agent_providers() -> Result<ProvidersView, crate::AppError> {
    crate::agent_bridge::config::list_providers().await
}

/// Re-read the config files and rebuild the view with an already-fetched
/// catalog. Used by the write paths so each command fetches the catalog once
/// (for validation and for the view) instead of twice.
#[cfg(test)]
pub(super) fn refresh_view_with_catalog(
    catalog: &BTreeMap<String, CatalogProviderSummary>,
) -> Result<ProvidersView, crate::AppError> {
    // Same lenient display-path reads as `list_agent_providers`.
    let models = config_io::read_json_lenient(&models_json_path()?);
    let auth = Value::Object(crate::auth_store::read().unwrap_or_default());
    Ok(build_providers_view(&models, &auth, catalog))
}

/// Pure view assembly over the current config files and the agent-provided
/// built-in catalog. Split out so the write paths can rebuild the view with
/// the catalog they already fetched, and so tests can inject a fixture catalog.
#[cfg(test)]
fn build_providers_view(
    models: &Value,
    auth: &Value,
    catalog: &BTreeMap<String, CatalogProviderSummary>,
) -> ProvidersView {
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
        base_url: crate::future_platform::resolve_future_base_url(auth),
        has_api_key: auth_has_key(auth, FUTURE_PROVIDER_ID),
        model_count: future_model_count(),
        requires_base_url: false,
    }];
    for (id, summary) in catalog {
        if custom_provider_ids.contains(id) {
            continue;
        }
        // The catalog base URL is the source of truth for whether this provider
        // needs a user-supplied Base URL; a stored override then fills it in.
        let requires_base_url = summary.base_url.contains(BASE_URL_PLACEHOLDER);
        let base_url =
            provider_base_url_override(models, id).unwrap_or_else(|| summary.base_url.clone());
        builtin.push(BuiltinProvider {
            has_api_key: auth_has_key(auth, id),
            id: id.clone(),
            name: summary.name.clone(),
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
                .filter(|name| !name.trim().is_empty())
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
                                .filter(|name| !name.trim().is_empty())
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
                                context_window: model
                                    .get("contextWindow")
                                    .and_then(Value::as_i64)
                                    .and_then(|value| i32::try_from(value).ok())
                                    .unwrap_or_else(default_context_window),
                                max_tokens: model
                                    .get("maxTokens")
                                    .and_then(Value::as_i64)
                                    .and_then(|value| i32::try_from(value).ok())
                                    .unwrap_or_else(default_max_tokens),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            custom.push(CustomProvider {
                has_api_key: auth_has_key(auth, id),
                api,
                base_url,
                name,
                models,
                id: id.clone(),
            });
        }
    }
    custom.sort_by(|left, right| left.id.cmp(&right.id));

    ProvidersView { builtin, custom }
}

#[cfg(test)]
fn auth_has_key(auth: &Value, id: &str) -> bool {
    auth.get(id)
        .and_then(|entry| entry.get("key"))
        .and_then(Value::as_str)
        .map(|key| !key.trim().is_empty())
        .unwrap_or(false)
}
