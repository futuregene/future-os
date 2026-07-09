//! Agent provider configuration Tauri commands.

use crate::agent_providers;

#[tauri::command]
pub fn list_agent_providers() -> Result<agent_providers::ProvidersView, crate::AppError> {
    agent_providers::list_agent_providers()
}

#[tauri::command]
pub async fn upsert_custom_provider(
    input: agent_providers::UpsertCustomProviderInput,
) -> Result<agent_providers::ProvidersView, crate::AppError> {
    let view = agent_providers::upsert_custom_provider(input)?;
    // A changed key only lands on disk; the agent caches it per-session and
    // never re-reads auth.json on the prompt path. Refresh live sessions so the
    // new key takes effect immediately (best-effort; see reload_agent_credentials).
    let _ = crate::agent_bridge::reload_agent_credentials().await;
    Ok(view)
}

#[tauri::command]
pub async fn update_builtin_provider_key(
    input: agent_providers::UpdateBuiltinProviderKeyInput,
) -> Result<agent_providers::ProvidersView, crate::AppError> {
    let view = agent_providers::update_builtin_provider_key(input)?;
    let _ = crate::agent_bridge::reload_agent_credentials().await;
    Ok(view)
}

#[tauri::command]
pub fn set_builtin_provider_base_url(
    input: agent_providers::SetBuiltinProviderBaseUrlInput,
) -> Result<agent_providers::ProvidersView, crate::AppError> {
    agent_providers::set_builtin_provider_base_url(input)
}

#[tauri::command]
pub async fn delete_custom_provider(
    id: String,
) -> Result<agent_providers::ProvidersView, crate::AppError> {
    let view = agent_providers::delete_custom_provider(id)?;
    // Deleting a provider removes its key from disk; refresh live sessions so a
    // session bound to it stops using the now-removed key.
    let _ = crate::agent_bridge::reload_agent_credentials().await;
    Ok(view)
}
