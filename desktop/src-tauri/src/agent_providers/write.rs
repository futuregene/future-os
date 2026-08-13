//! Mutating command paths for provider configuration: built-in API key / Base
//! URL overrides and custom-provider upsert/delete.
//!
//! Each operation is RPC-first (audit item 2): the validated change is sent to
//! the agent via `set_auth` / `upsert_provider` / `delete_provider`, and the
//! agent writes its own auth.json/models.json and refreshes live sessions. The
//! locked local read-modify-write remains as the fallback for an unreachable
//! or pre-item-2 agent (then followed by the best-effort `reload_auth`). The
//! synchronous `_with_catalog` cores are exactly that local path — validation
//! plus file writes — which keeps the module tests agent-free.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::agent_bridge::config as agent_config;
use crate::auth_store::FUTURE_PROVIDER_ID;
use crate::config_io;
use crate::AppError;

use super::catalog::{builtin_catalog_providers, models_json_path, CatalogProviderSummary};
use super::validate::{
    is_ascii_no_control, model_json_values, validate_custom_provider, ValidatedCustomProvider,
    API_KEY_MAX_LEN, BASE_URL_MAX_LEN,
};
use super::{
    refresh_view_with_catalog, ProvidersView, SetBuiltinProviderBaseUrlInput,
    UpdateBuiltinProviderKeyInput, UpsertCustomProviderInput, BASE_URL_PLACEHOLDER,
    FUTURE_PROVIDER_NAME,
};

/// Best-effort `reload_auth` after a LOCAL config write: the agent caches the
/// resolved credentials per session, so the file change alone leaves live
/// sessions on the old state. Not needed when the RPC write path was used —
/// the agent refreshed itself.
async fn reload_after_local_write() {
    let _ = crate::agent_bridge::reload_agent_credentials().await;
}

/// One-shot injected failure for the paired auth.json write of a transactional
/// local write. models.json and auth.json live in the same directory, so a
/// test cannot make the first write succeed and the second fail organically —
/// this seam lets the rollback path be exercised instead.
#[cfg(test)]
pub(super) static INJECT_AUTH_WRITE_FAILURE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// See [`INJECT_AUTH_WRITE_FAILURE`]; also used by tests to arm the injection.
#[cfg(test)]
fn injected_auth_write_failure() -> Result<(), AppError> {
    INJECT_AUTH_WRITE_FAILURE
        .swap(false, std::sync::atomic::Ordering::Relaxed)
        .then(|| AppError::Message("injected auth write failure".to_string()))
        .map_or(Ok(()), Err)
}

/// The auth half of the transactional upsert. A thin wrapper so the test-only
/// failure injection (see above) shares one line with the real call.
fn paired_key_write(id: &str, key: &str) -> Result<(), AppError> {
    #[cfg(test)]
    injected_auth_write_failure()?;
    crate::auth_store::set_provider_key(id, key)
}

// ── update_builtin_provider_key ─────────────────────────────────────────────

/// Request-local validation for a built-in provider's API key change.
fn validate_builtin_key_update<'a>(
    input: &'a UpdateBuiltinProviderKeyInput,
    catalog: &BTreeMap<String, CatalogProviderSummary>,
) -> Result<(&'a str, Option<&'a str>), AppError> {
    let id = input.id.trim();
    if id.is_empty() {
        return Err("Provider id is required.".to_string().into());
    }
    if id == FUTURE_PROVIDER_ID {
        return Err("FutureGene uses the sign-in flow.".to_string().into());
    }
    if !catalog.contains_key(id) {
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
    }
    Ok((id, api_key))
}

/// Local fallback: write the key straight to auth.json.
fn apply_builtin_key_update_local(id: &str, api_key: Option<&str>) -> Result<(), AppError> {
    if let Some(key) = api_key {
        crate::auth_store::set_provider_key(id, key)?;
    } else {
        crate::auth_store::remove_provider_key(id)?;
    }
    Ok(())
}

pub async fn update_builtin_provider_key(
    input: UpdateBuiltinProviderKeyInput,
) -> Result<ProvidersView, AppError> {
    let catalog = builtin_catalog_providers().await;
    // Validate before the RPC so bad input never reaches the agent; the
    // fallback core below re-validates from the same input.
    let (id, api_key) = validate_builtin_key_update(&input, &catalog)?;
    let applied = match api_key {
        Some(key) => agent_config::set_provider_key(id, key).await?,
        None => agent_config::clear_provider_key(id).await?,
    };
    if applied {
        // The agent wrote auth.json and refreshed live sessions itself.
        return refresh_view_with_catalog(&catalog);
    }
    let view = update_builtin_provider_key_with_catalog(input, &catalog)?;
    reload_after_local_write().await;
    Ok(view)
}

pub(super) fn update_builtin_provider_key_with_catalog(
    input: UpdateBuiltinProviderKeyInput,
    catalog: &BTreeMap<String, CatalogProviderSummary>,
) -> Result<ProvidersView, AppError> {
    let (id, api_key) = validate_builtin_key_update(&input, catalog)?;
    apply_builtin_key_update_local(id, api_key)?;
    refresh_view_with_catalog(catalog)
}

// ── set_builtin_provider_base_url ───────────────────────────────────────────

/// Request-local validation for a built-in provider's Base URL override.
fn validate_builtin_base_url_update<'a>(
    input: &'a SetBuiltinProviderBaseUrlInput,
    catalog: &BTreeMap<String, CatalogProviderSummary>,
) -> Result<(&'a str, &'a str), AppError> {
    let id = input.id.trim();
    if id.is_empty() {
        return Err("Provider id is required.".to_string().into());
    }
    if id == FUTURE_PROVIDER_ID {
        return Err("FutureGene's address is managed by the sign-in flow."
            .to_string()
            .into());
    }
    if !catalog.contains_key(id) {
        return Err(format!("Unknown built-in provider: `{id}`.").into());
    }

    let base_url = input.base_url.trim();
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
    Ok((id, base_url))
}

/// Local fallback: strict, per-path-locked models.json read-modify-write of
/// the `baseUrl` override.
fn apply_builtin_base_url_local(id: &str, base_url: &str) -> Result<(), AppError> {
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
    })
}

/// Set (or clear) a built-in provider's Base URL override in models.json. Used
/// for catalog providers whose base URL is a placeholder (see
/// [`BASE_URL_PLACEHOLDER`]); the agent applies it to that provider's models.
pub async fn set_builtin_provider_base_url(
    input: SetBuiltinProviderBaseUrlInput,
) -> Result<ProvidersView, AppError> {
    let catalog = builtin_catalog_providers().await;
    let (id, base_url) = validate_builtin_base_url_update(&input, &catalog)?;
    let applied = agent_config::set_builtin_provider_base_url(id, base_url).await?;
    if applied {
        return refresh_view_with_catalog(&catalog);
    }
    let view = set_builtin_provider_base_url_with_catalog(input, &catalog)?;
    reload_after_local_write().await;
    Ok(view)
}

pub(super) fn set_builtin_provider_base_url_with_catalog(
    input: SetBuiltinProviderBaseUrlInput,
    catalog: &BTreeMap<String, CatalogProviderSummary>,
) -> Result<ProvidersView, AppError> {
    let (id, base_url) = validate_builtin_base_url_update(&input, catalog)?;
    apply_builtin_base_url_local(id, base_url)?;
    refresh_view_with_catalog(catalog)
}

// ── upsert_custom_provider ──────────────────────────────────────────────────

/// Catalog-only checks (independent of the file state): reserved ids in create
/// mode and name collisions with built-in providers. File-state checks (id/name
/// uniqueness among existing entries) run inside the locked write — locally in
/// [`apply_upsert_local`], on the agent side in `apply_provider_upsert`.
fn validate_upsert_against_catalog(
    validated: &ValidatedCustomProvider,
    catalog: &BTreeMap<String, CatalogProviderSummary>,
) -> Result<(), AppError> {
    // The built-in catalog is never empty in practice, so an empty catalog
    // means it could not be obtained (agent unreachable on a cold start, no
    // in-process cache). Creating a provider then would validate against
    // nothing — a reserved id like "openai" could slip through and shadow the
    // built-in once the agent returns. Refuse rather than trust an empty set;
    // once the catalog has been fetched once it stays cached for the process,
    // so this only bites on a cold start with the agent already down.
    if validated.create && catalog.is_empty() {
        return Err(
            "Cannot create a custom provider right now: the built-in provider catalog is unavailable. Please make sure the Future Agent is running and try again."
                .into(),
        );
    }
    if validated.create && catalog.contains_key(&validated.id) {
        return Err(format!(
            "Provider ID `{}` is reserved for a built-in provider.",
            validated.id
        )
        .into());
    }
    // Names must be unique (case-insensitive) across the built-in and other
    // custom providers, so the list and model grouping stay unambiguous.
    let normalized_name = validated.name.to_lowercase();
    if normalized_name == FUTURE_PROVIDER_NAME.to_lowercase() {
        return Err(format!(
            "Provider name `{}` conflicts with a built-in provider.",
            validated.name
        )
        .into());
    }
    let builtin_name_taken = catalog.iter().any(|(builtin_id, provider)| {
        builtin_id != &validated.id && provider.name.trim().to_lowercase() == normalized_name
    });
    if builtin_name_taken {
        return Err(format!(
            "Provider name `{}` conflicts with a built-in provider.",
            validated.name
        )
        .into());
    }
    Ok(())
}

/// The `upsert_provider` RPC payload for a validated custom provider.
fn provider_upsert_message(
    validated: &ValidatedCustomProvider,
) -> crate::agent_proto::ProviderUpsert {
    crate::agent_proto::ProviderUpsert {
        id: validated.id.clone(),
        name: validated.name.clone(),
        api: validated.api.clone(),
        base_url: validated.base_url.clone(),
        models: validated
            .models
            .iter()
            .map(|model| crate::agent_proto::ProviderModel {
                id: model.id.clone(),
                name: model.name.clone(),
                modalities: model.modalities.clone(),
            })
            .collect(),
        create_only: validated.create,
        api_key: validated.api_key.clone().unwrap_or_default(),
        ..Default::default()
    }
}

/// Local fallback: locked models.json read-modify-write (with the file-state
/// uniqueness checks) plus the auth.json key write, mirroring the agent's
/// `upsert_provider_files` ordering and rollback — transactionally, so a failure
/// on either file never leaves a dangling key or a provider whose models and
/// key disagree. The models write comes first; if the key write then fails, the
/// models file is rolled back to its exact pre-call bytes.
fn apply_upsert_local(validated: &ValidatedCustomProvider) -> Result<(), AppError> {
    let models_path = models_json_path()?;
    config_io::with_config_lock(&models_path, || {
        // A snapshot read error must abort BEFORE any write, never be treated
        // as "file did not exist" (which would make rollback delete the file).
        // Only models.json can be rolled back here — auth.json is written last
        // and atomically, so a failed key write leaves it untouched and there
        // is nothing to restore (restoring it could clobber a concurrent
        // writer's just-committed change).
        let models_snapshot = config_io::snapshot_file(&models_path)?;

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
        if validated.create && providers.contains_key(&validated.id) {
            return Err(format!("Provider ID `{}` already exists.", validated.id).into());
        }
        let normalized_name = validated.name.to_lowercase();
        let name_taken = providers.iter().any(|(other_id, config)| {
            other_id != &validated.id
                && config
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(other_id)
                    .trim()
                    .to_lowercase()
                    == normalized_name
        });
        if name_taken {
            return Err(format!("Provider name `{}` already exists.", validated.name).into());
        }

        // Preserve any fields the GUI does not manage (e.g. `compat`).
        let mut provider = providers
            .get(&validated.id)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        provider.insert("name".to_string(), Value::String(validated.name.clone()));
        provider.insert("api".to_string(), Value::String(validated.api.clone()));
        provider.insert(
            "baseUrl".to_string(),
            Value::String(validated.base_url.clone()),
        );
        provider.insert(
            "models".to_string(),
            Value::Array(model_json_values(&validated.models)),
        );
        providers.insert(validated.id.clone(), Value::Object(provider));

        // Persist models.json, then auth.json. A failed models write leaves
        // nothing persisted; a failed key write leaves auth.json untouched, so
        // only models.json is ever rolled back.
        if let Err(error) = config_io::write_json_atomic(&models_path, &models_doc, false) {
            config_io::restore_file(&models_path, models_snapshot.as_deref(), false);
            return Err(error);
        }
        if let Some(key) = validated.api_key.as_deref() {
            if let Err(error) = paired_key_write(&validated.id, key) {
                config_io::restore_file(&models_path, models_snapshot.as_deref(), false);
                return Err(error);
            }
        }
        Ok(())
    })
}

pub async fn upsert_custom_provider(
    input: UpsertCustomProviderInput,
) -> Result<ProvidersView, AppError> {
    let catalog = builtin_catalog_providers().await;
    // Validate before the RPC so bad input never reaches the agent. `input` is
    // cloned because validation consumes it while the fallback core below also
    // needs it.
    let validated = validate_custom_provider(input.clone())?;
    validate_upsert_against_catalog(&validated, &catalog)?;
    let applied = agent_config::upsert_provider(provider_upsert_message(&validated)).await?;
    if applied {
        // The agent wrote models.json/auth.json and refreshed live sessions.
        return refresh_view_with_catalog(&catalog);
    }
    let view = upsert_custom_provider_with_catalog(input, &catalog)?;
    reload_after_local_write().await;
    Ok(view)
}

pub(super) fn upsert_custom_provider_with_catalog(
    input: UpsertCustomProviderInput,
    catalog: &BTreeMap<String, CatalogProviderSummary>,
) -> Result<ProvidersView, AppError> {
    let validated = validate_custom_provider(input)?;
    validate_upsert_against_catalog(&validated, catalog)?;
    apply_upsert_local(&validated)?;
    refresh_view_with_catalog(catalog)
}

// ── delete_custom_provider ──────────────────────────────────────────────────

/// Request-local guards: only custom providers may be removed — guard the
/// built-in FutureGene (whose key is the user's sign-in) and every catalog
/// provider, so a stray id can't wipe login/override state the UI never offers
/// to delete.
fn validate_delete_id(
    id: &str,
    catalog: &BTreeMap<String, CatalogProviderSummary>,
) -> Result<String, AppError> {
    let id = id.trim().to_string();
    if id.is_empty() {
        return Err("Provider id is required.".to_string().into());
    }
    if id == FUTURE_PROVIDER_ID {
        return Err("FutureGene is a built-in provider and cannot be deleted.".into());
    }
    if catalog.contains_key(&id) {
        return Err(format!("`{id}` is a built-in provider and cannot be deleted.").into());
    }
    Ok(id)
}

/// Local fallback: remove the models.json entry AND the auth.json credentials.
fn apply_delete_local(id: &str) -> Result<(), AppError> {
    let models_path = models_json_path()?;
    config_io::with_config_lock(&models_path, || {
        // A snapshot read error must abort BEFORE any write, never be treated
        // as "file did not exist" (which would make rollback delete the file).
        // Only models.json can be rolled back here — auth.json is written last
        // and atomically, so a failed auth write leaves it untouched and there
        // is nothing to restore.
        let models_snapshot = config_io::snapshot_file(&models_path)?;
        let auth_path = crate::auth_store::auth_json_path()?;

        let mut models_doc = config_io::read_json_object(&models_path)?;
        let models_changed = models_doc
            .get_mut("providers")
            .and_then(Value::as_object_mut)
            .map(|providers| providers.remove(id).is_some())
            .unwrap_or(false);

        // Determine the auth half under the auth path lock so a concurrent key
        // write can't interleave; delete always removes the whole entry, so a
        // later re-read under the same lock stays consistent.
        let auth_changed = config_io::with_config_lock(&auth_path, || {
            Ok(crate::auth_store::read()?.contains_key(id))
        })?;

        if !models_changed && !auth_changed {
            // Nothing to remove; leave both files untouched.
            return Ok(());
        }

        // Transactional: if the models write succeeds but the auth write fails,
        // roll back ONLY models.json (the file already written this call) to its
        // exact pre-call bytes — auth.json is untouched, and restoring it could
        // clobber a concurrent writer's just-committed change. Lock order is
        // models → auth everywhere, so nesting is safe. Errors are collected
        // and returned at the end so no block ends on an early-return edge.
        let mut error = None;
        if let Some(e) = models_changed
            .then(|| config_io::write_json_atomic(&models_path, &models_doc, false))
            .and_then(Result::err)
        {
            config_io::restore_file(&models_path, models_snapshot.as_deref(), false);
            error = Some(e);
        }
        if auth_changed && error.is_none() {
            let result = config_io::with_config_lock(&auth_path, || {
                let mut auth = crate::auth_store::read()?;
                #[cfg(test)]
                injected_auth_write_failure()?;
                let _ = auth
                    .remove(id)
                    .map(|_| crate::auth_store::write(&auth))
                    .transpose()?;
                Ok(())
            });
            if let Err(e) = result {
                config_io::restore_file(&models_path, models_snapshot.as_deref(), false);
                error = Some(e);
            }
        }
        if let Some(e) = error {
            return Err(e);
        }
        Ok(())
    })
}

pub async fn delete_custom_provider(id: String) -> Result<ProvidersView, AppError> {
    let catalog = builtin_catalog_providers().await;
    let id = validate_delete_id(&id, &catalog)?;
    let applied = agent_config::delete_provider(&id).await?;
    if applied {
        // The agent removed its models.json + auth.json entries and refreshed.
        return refresh_view_with_catalog(&catalog);
    }
    let view = delete_custom_provider_with_catalog(id, &catalog)?;
    reload_after_local_write().await;
    Ok(view)
}

pub(super) fn delete_custom_provider_with_catalog(
    id: String,
    catalog: &BTreeMap<String, CatalogProviderSummary>,
) -> Result<ProvidersView, AppError> {
    let id = validate_delete_id(&id, catalog)?;
    apply_delete_local(&id)?;
    refresh_view_with_catalog(catalog)
}

// ── shared models.json view helpers ─────────────────────────────────────────

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
