//! Sessionless provider / auth / model-registry command handlers.

use crate::rpc::{AppState, RpcCommand, RpcResponse};

/// Serializes provider snapshots with config mutations through the registry
/// refresh. The lower config lock protects file RMWs; this command-level lock
/// additionally prevents readers from observing committed files before the
/// matching in-memory registry revision has landed.
static PROVIDER_COMMAND_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn handle_reload_auth(state: &AppState, id: &str) -> String {
    let _provider_guard = PROVIDER_COMMAND_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    // Rebuild the shared model registry FIRST so runtime-added/
    // removed providers and models.json edits become visible to every
    // session — set_model now resolves against this cache instead of
    // constructing a fresh Registry per call.
    refresh_authoritative_provider_state(state);
    let revision = crate::rpc::publish_provider_config_changed("*", "reload", true, true);
    RpcResponse::ok(
        id,
        "reload_auth",
        serde_json::json!({ "revision": revision }),
    )
}

pub(crate) fn handle_sync_future_models(state: &AppState, id: &str) -> String {
    let _provider_guard = PROVIDER_COMMAND_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    // Synchronously fetch the Future provider's models (warming the cache),
    // then rebuild the registry against that warm cache so the very next
    // `list_models` returns a complete list. Unlike `reload_auth`, this blocks
    // on the network fetch; the GUI uses it for onboarding, manual refresh, and
    // its low-frequency maintenance schedule, never on a hot request path.
    let synced = crate::models::sync_future_models_cache();
    refresh_authoritative_provider_state(state);
    // Report only the Future catalogue just synchronized. `all_models()` also
    // includes every built-in and configured third-party provider, which makes
    // the settings success message claim thousands of Future models.
    let model_count = crate::models::cached_model_count();
    let revision =
        crate::rpc::publish_provider_config_changed("future", "models_synced", false, true);
    RpcResponse::ok(
        id,
        "sync_future_models",
        serde_json::json!({ "synced": synced, "modelCount": model_count, "revision": revision }),
    )
}

pub(crate) fn handle_set_default_model(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    // Persist the onboarding model-picker's choice as the global default model
    // (settings.json `defaultModel`). Sessionless: it's a process-wide setting,
    // not tied to any one session. Rebuild the registry afterwards so the next
    // `list_models` reflects the new `isDefault` immediately.
    let model_id = cmd.model_id.trim().to_string();
    if model_id.is_empty() {
        return RpcResponse::build_fail(id, "set_default_model", "model_id is empty");
    }
    let exists = state
        .model_registry
        .read()
        .all_models()
        .iter()
        .any(|m| format!("{}/{}", m.provider, m.id) == model_id || m.id == model_id);
    if !exists {
        return RpcResponse::build_fail(
            id,
            "set_default_model",
            &format!("model `{model_id}` is not in the catalog"),
        );
    }
    let settings_path = std::path::PathBuf::from(crate::models::settings_path());
    let mut settings = match crate::config::load_settings(&settings_path) {
        Ok(s) => s,
        Err(e) => {
            return RpcResponse::build_fail(
                id,
                "set_default_model",
                &format!("failed to load settings: {e}"),
            );
        }
    };
    settings.default_model = model_id.clone();
    if let Err(e) = settings.save(&settings_path) {
        return RpcResponse::build_fail(
            id,
            "set_default_model",
            &format!("failed to save settings: {e}"),
        );
    }
    *state.model_registry.write() = crate::models::Registry::new();
    RpcResponse::ok(
        id,
        "set_default_model",
        serde_json::json!({ "defaultModel": model_id }),
    )
}

pub(crate) fn get_agent_info_response(state: &AppState, id: &str) -> String {
    let skills_count =
        crate::skills::discover_skills_cached(&crate::skills::global_skill_dirs()).len();
    RpcResponse::ok(
        id,
        "get_agent_info",
        serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "agentInstanceId": state.agent_instance_id,
            "skillsCount": skills_count,
        }),
    )
}

pub(crate) fn list_models_response(
    id: &str,
    registry: &crate::models::Registry,
    include_builtin_providers: bool,
) -> String {
    // Always return all available models.  Scoping / defaults are client-side.
    // Builtin catalog models are only listed when they are credential-reachable
    // (an API key inline or in auth.json) — otherwise the picker would drown
    // under ~900 unusable entries.  Models the user explicitly configured in
    // models.json are always listed: they are intentional even when keyless
    // (e.g. a local Ollama-style endpoint that needs no key at all).
    let mut models: Vec<crate::models::Model> = registry
        .all_models()
        .into_iter()
        .filter(|model| {
            registry.is_user_defined(model)
                || registry.is_model_available(&format!("{}/{}", model.provider, model.id))
        })
        .filter(|model| model.output.iter().any(|o| o == "text"))
        .collect();

    models.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    models.dedup_by(|left, right| left.id == right.id && left.provider == right.provider);

    // Use the same default-model resolution as cmd_new_session so the list
    // and actual session creation agree on which model is the default.
    let effective_default = crate::models::get_default_model_with(registry)
        .or_else(|| models.first().map(|m| format!("{}/{}", m.provider, m.id)))
        .unwrap_or_default();

    let payload_models: Vec<serde_json::Value> = models
        .into_iter()
        .map(|model| {
            let id = model.id;
            let qualified_id = format!("{}/{}", model.provider, id);
            let label = if model.name.is_empty() {
                id.clone()
            } else {
                model.name.clone()
            };
            let thinking_level = if model.reasoning { "high" } else { "off" };
            serde_json::json!({
                "id": id.clone(),
                "label": label,
                "provider": model.provider.clone(),
                "supportsImages": model.input.iter().any(|input| input == "image"),
                "thinkingLevel": thinking_level.to_string(),
                "contextWindow": model.context_window,
                "isDefault": qualified_id == effective_default,
                "description": model.description,
                "descriptionEn": model.description_en,
                "recommended": model.recommended,
            })
        })
        .collect();

    let mut payload = serde_json::json!({
        "models": payload_models,
        "defaultModel": effective_default,
        "isScoped": false,
    });
    if include_builtin_providers {
        // Catalog summaries so clients (GUI Providers page) can fetch the
        // built-in catalog at runtime instead of compiling agent source in.
        payload["builtinProviders"] = serde_json::to_value(registry.builtin_provider_summaries())
            .unwrap_or_else(|_| serde_json::json!({}));
    }
    RpcResponse::ok(id, "list_models", payload)
}

pub(crate) fn list_providers_response(state: &AppState, id: &str) -> String {
    let _provider_guard = PROVIDER_COMMAND_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    match provider_view(state) {
        Ok(view) => RpcResponse::ok(id, "list_providers", view),
        Err(error) => RpcResponse::build_fail(id, "list_providers", &error),
    }
}

fn provider_view(state: &AppState) -> Result<serde_json::Value, String> {
    let (models, auth) = crate::config::providers::read_provider_documents()?;
    let models = serde_json::Value::Object(models);
    let auth = serde_json::Value::Object(auth);
    let provider_entries = models
        .get("providers")
        .and_then(serde_json::Value::as_object);
    let has_key = |provider: &str| {
        auth.get(provider)
            .and_then(|entry| entry.get("key"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|key| !key.trim().is_empty())
    };
    let is_override_only = |config: &serde_json::Value| {
        let has_text = |field: &str| {
            config
                .get(field)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        };
        let has_models = config
            .get("models")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|items| !items.is_empty());
        !has_text("name") && !has_text("api") && !has_models
    };

    let custom_ids: std::collections::HashSet<&str> = provider_entries
        .into_iter()
        .flat_map(|providers| providers.iter())
        .filter(|(_, config)| !is_override_only(config))
        .map(|(provider_id, _)| provider_id.as_str())
        .collect();

    let registry = state.model_registry.read();
    let mut builtin = vec![serde_json::json!({
        "id": "future",
        "name": "Future",
        "baseUrl": crate::models::display_base_url_from_auth(&auth),
        "hasApiKey": has_key("future"),
        "modelCount": crate::models::cached_model_count(),
        "requiresBaseUrl": false,
    })];
    for (provider_id, summary) in registry.builtin_provider_summaries() {
        if custom_ids.contains(provider_id.as_str()) {
            continue;
        }
        let override_url = provider_entries
            .and_then(|providers| providers.get(&provider_id))
            .and_then(|config| config.get("baseUrl"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        builtin.push(serde_json::json!({
            "id": provider_id,
            "name": summary.name,
            "baseUrl": override_url.unwrap_or(&summary.base_url),
            "hasApiKey": has_key(&provider_id),
            "modelCount": summary.model_count,
            "requiresBaseUrl": summary.base_url.contains("YOUR_RESOURCE"),
        }));
    }

    let mut custom = provider_entries
        .into_iter()
        .flat_map(|providers| providers.iter())
        .filter(|(provider_id, config)| provider_id.as_str() != "future" && !is_override_only(config))
        .map(|(provider_id, config)| {
            let name = config
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(provider_id);
            let provider_models = config
                .get("models")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|model| {
                    let model_id = model.get("id")?.as_str()?;
                    let model_name = model
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .filter(|name| !name.trim().is_empty())
                        .unwrap_or(model_id);
                    let supports_images = model
                        .get("modalities")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("image")));
                    Some(serde_json::json!({
                        "id": model_id,
                        "name": model_name,
                        "supportsImages": supports_images,
                        "contextWindow": model.get("contextWindow").and_then(serde_json::Value::as_i64).unwrap_or(128_000),
                        "maxTokens": model.get("maxTokens").and_then(serde_json::Value::as_i64).unwrap_or(16_384),
                    }))
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "id": provider_id,
                "name": name,
                "api": config.get("api").and_then(serde_json::Value::as_str).unwrap_or_default(),
                "baseUrl": config.get("baseUrl").and_then(serde_json::Value::as_str).unwrap_or_default(),
                "hasApiKey": has_key(provider_id),
                "models": provider_models,
            })
        })
        .collect::<Vec<_>>();
    custom.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    Ok(serde_json::json!({ "builtin": builtin, "custom": custom }))
}

/// Rebuild the shared model registry so provider/models.json changes become
/// visible to every request, then reconcile only invalid session model
/// references. Provider settings themselves are request-time authority and are
/// never copied into sessions. Shared by `reload_auth` and config-write commands.
/// Callers hold `PROVIDER_COMMAND_LOCK` and invoke this only after the durable
/// mutation succeeds; they publish `provider_config_changed` only after this
/// complete Registry revision is installed.
fn refresh_authoritative_provider_state(state: &AppState) {
    *state.model_registry.write() = crate::models::Registry::new();
    state.reconcile_provider_references();
}

/// Apply one auth.json mutation and refresh live state (see dispatch comment).
pub(crate) fn cmd_set_auth(state: &AppState, id: &str, cmd: &RpcCommand) -> String {
    let _provider_guard = PROVIDER_COMMAND_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let Some(mutation) = cmd.auth_update.as_ref() else {
        return RpcResponse::build_fail(id, "set_auth", "missing auth_update payload");
    };
    if mutation.provider.trim().is_empty() {
        return RpcResponse::build_fail(id, "set_auth", "auth_update.provider is empty");
    }
    let carries_change = mutation.key.is_some()
        || mutation.base_url.is_some()
        || mutation.clear_key
        || mutation.clear_base_url
        || mutation.remove_entry
        || mutation.remove_platform_base_url;
    if !carries_change {
        return RpcResponse::build_fail(id, "set_auth", "auth_update carries no change");
    }
    if let Err(error) = crate::config::providers::mutate_auth(mutation) {
        return RpcResponse::build_fail(id, "set_auth", &error);
    }
    refresh_authoritative_provider_state(state);
    let revision = crate::rpc::publish_provider_config_changed(
        &mutation.provider,
        "auth_updated",
        true,
        false,
    );
    RpcResponse::ok(
        id,
        "set_auth",
        serde_json::json!({ "provider": mutation.provider, "revision": revision }),
    )
}

/// Create/update a models.json provider (plus optional auth.json key) and
/// refresh live state (see dispatch comment).
pub(crate) fn cmd_upsert_provider(state: &AppState, id: &str, cmd: &RpcCommand) -> String {
    let _provider_guard = PROVIDER_COMMAND_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let Some(spec) = cmd.provider_config.as_ref() else {
        return RpcResponse::build_fail(id, "upsert_provider", "missing provider_config payload");
    };
    if spec.id.trim().is_empty() {
        return RpcResponse::build_fail(id, "upsert_provider", "provider_config.id is empty");
    }
    let carries_change = spec.name.is_some()
        || spec.api.is_some()
        || spec.base_url.is_some()
        || spec.clear_base_url
        || spec.replace_models
        || !spec.models.is_empty()
        || spec.api_key.is_some()
        || spec.clear_api_key;
    if !carries_change {
        return RpcResponse::build_fail(id, "upsert_provider", "provider_config carries no change");
    }
    // The agent is the authority on its own built-in catalog: reject any write
    // that would *define* a custom provider (name/api/models/key) under an id
    // that belongs to a built-in provider or the Future platform. Pure base-URL
    // overrides (no name/api/models/key) are still allowed — that is how clients
    // legitimately point a built-in provider at a different endpoint. Guarding
    // here (not only in the GUI) keeps the invariant correct no matter which
    // client issues the write and whether any client-side catalog is stale or
    // temporarily unavailable.
    let defines_custom_provider =
        spec.name.is_some() || spec.api.is_some() || spec.replace_models || !spec.models.is_empty();
    if defines_custom_provider
        && state
            .model_registry
            .read()
            .builtin_provider_ids()
            .contains(spec.id.trim())
    {
        return RpcResponse::build_fail(
            id,
            "upsert_provider",
            &format!(
                "Provider ID `{}` is reserved for a built-in provider.",
                spec.id.trim()
            ),
        );
    }
    if let Err(error) = crate::config::providers::upsert_provider(spec) {
        return RpcResponse::build_fail(id, "upsert_provider", &error);
    }
    refresh_authoritative_provider_state(state);
    let revision = crate::rpc::publish_provider_config_changed(
        &spec.id,
        if spec.create_only {
            "created"
        } else {
            "updated"
        },
        spec.api_key.is_some() || spec.clear_api_key,
        true,
    );
    RpcResponse::ok(
        id,
        "upsert_provider",
        serde_json::json!({ "id": spec.id, "revision": revision }),
    )
}

/// Remove a provider's models.json entry AND auth.json entry, then refresh
/// live state (see dispatch comment).
pub(crate) fn cmd_delete_provider(state: &AppState, id: &str, cmd: &RpcCommand) -> String {
    let _provider_guard = PROVIDER_COMMAND_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let Some(spec) = cmd.provider_config.as_ref() else {
        return RpcResponse::build_fail(id, "delete_provider", "missing provider_config payload");
    };
    let provider_id = spec.id.trim();
    if provider_id.is_empty() {
        return RpcResponse::build_fail(id, "delete_provider", "provider_config.id is empty");
    }
    // The agent is the authority on its own catalog: refuse to delete a
    // built-in provider or the Future platform entry via this command. Clients
    // legitimately remove built-in overrides / the Future login through the
    // dedicated set_auth / upsert paths — a direct `delete_provider` must never
    // be able to wipe the Future sign-in credentials or a built-in's key/URL
    // override. The GUI already guards this; guarding here closes the bypass
    // for any other gRPC client.
    if state
        .model_registry
        .read()
        .builtin_provider_ids()
        .contains(provider_id)
    {
        return RpcResponse::build_fail(
            id,
            "delete_provider",
            &format!(
                "Provider ID `{provider_id}` is reserved for a built-in provider and cannot be deleted."
            ),
        );
    }
    if let Err(error) = crate::config::providers::delete_provider(provider_id) {
        return RpcResponse::build_fail(id, "delete_provider", &error);
    }
    refresh_authoritative_provider_state(state);
    let revision = crate::rpc::publish_provider_config_changed(provider_id, "deleted", true, true);
    RpcResponse::ok(
        id,
        "delete_provider",
        serde_json::json!({ "id": provider_id, "revision": revision }),
    )
}

pub(crate) fn cmd_refresh_skills(state: &AppState, id: &str) -> String {
    // Always invalidate the cache. install/uninstall write to disk *after* the
    // previous scan, so the cache is stale for them no matter how recently it
    // was refreshed; invalidation is O(1) and the rescan below repopulates it
    // (and warms the cache so the GUI's follow-up get_commands hits the fast
    // path with the new state).
    //
    // This used to be gated behind a 5 s minimum-interval rate limit, but that
    // limit was process-global and kept getting consumed by the harmless scan
    // the GUI fires on startup / page open / agent (re)connect. The invalidation
    // that actually matters — the one right after a write — then landed inside
    // the window and was silently skipped, so get_commands kept returning the
    // pre-install cache and the Skills view showed the old installed/uninstalled
    // state until the app was restarted (which resets the limit and forces a
    // scan). Burst protection is not needed here: invalidation has no I/O cost,
    // and a rescan is a cheap local walk of two directories.
    crate::skills::invalidate_skills_cache();
    let skills = crate::skills::discover_skills_cached(&crate::skills::global_skill_dirs());
    let skill_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
    // Keep the get_state snapshot in step with the discovery cache — reload_config
    // updates it too, but that path needs a session, and refresh_skills is the
    // sessionless post-install/uninstall entry point. Without this, get_state
    // kept reporting the pre-install skill list until the next reload_config.
    *state.welcome_skills.write() = skill_names.clone();
    RpcResponse::ok(
        id,
        "refresh_skills",
        serde_json::json!({
            "skills_count": skill_names.len(),
            "skills": skill_names,
            "refreshed": true,
        }),
    )
}
