//! Agent provider configuration Tauri commands.

use crate::agent_providers;

#[tauri::command]
pub fn list_agent_providers() -> Result<agent_providers::ProvidersView, crate::AppError> {
    agent_providers::list_agent_providers()
}

#[tauri::command]
pub fn upsert_custom_provider(
    input: agent_providers::UpsertCustomProviderInput,
) -> Result<agent_providers::ProvidersView, crate::AppError> {
    agent_providers::upsert_custom_provider(input)
}

#[tauri::command]
pub fn delete_custom_provider(
    id: String,
) -> Result<agent_providers::ProvidersView, crate::AppError> {
    agent_providers::delete_custom_provider(id)
}
