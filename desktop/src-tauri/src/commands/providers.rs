//! Agent provider configuration Tauri commands.
//!
//! The write commands are RPC-first (audit item 2): the agent applies the
//! change to its own config files and refreshes live sessions internally. The
//! legacy local-write + reload_auth fallback lives inside the write layer
//! (`agent_providers::write`) for unreachable / pre-item-2 agents, so no
//! follow-up refresh is needed at this layer.

use crate::agent_providers;

#[tauri::command]
pub async fn list_agent_providers() -> Result<agent_providers::ProvidersView, crate::AppError> {
    agent_providers::list_agent_providers().await
}

#[tauri::command]
pub async fn upsert_custom_provider(
    input: agent_providers::UpsertCustomProviderInput,
) -> Result<agent_providers::ProvidersView, crate::AppError> {
    agent_providers::upsert_custom_provider(input).await
}

#[tauri::command]
pub async fn update_builtin_provider_key(
    input: agent_providers::UpdateBuiltinProviderKeyInput,
) -> Result<agent_providers::ProvidersView, crate::AppError> {
    agent_providers::update_builtin_provider_key(input).await
}

#[tauri::command]
pub async fn set_builtin_provider_base_url(
    input: agent_providers::SetBuiltinProviderBaseUrlInput,
) -> Result<agent_providers::ProvidersView, crate::AppError> {
    agent_providers::set_builtin_provider_base_url(input).await
}

#[tauri::command]
pub async fn delete_custom_provider(
    id: String,
) -> Result<agent_providers::ProvidersView, crate::AppError> {
    agent_providers::delete_custom_provider(id).await
}
