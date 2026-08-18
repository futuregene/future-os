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
pub async fn update_builtin_provider(
    input: agent_providers::UpdateBuiltinProviderInput,
) -> Result<agent_providers::ProvidersView, crate::AppError> {
    agent_providers::update_builtin_provider(input).await
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

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::remote::test_support::{ensure_mock_agent, mock_agent_lock};

    #[test]
    fn async_command_wrappers_reject_malformed_bodies() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![
                upsert_custom_provider,
                update_builtin_provider_key,
                update_builtin_provider,
                set_builtin_provider_base_url,
                delete_custom_provider
            ],
            &[
                "upsert_custom_provider",
                "update_builtin_provider_key",
                "update_builtin_provider",
                "set_builtin_provider_base_url",
                "delete_custom_provider",
            ],
        );
    }

    #[tokio::test]
    async fn command_wrappers_delegate_to_the_agent() {
        let _lock = mock_agent_lock();
        let _home = HomeGuard::new("cmd-providers");
        let agent = ensure_mock_agent();

        // list
        let view = list_agent_providers().await.expect("list");
        assert!(view.builtin.iter().any(|p| p.id == "deepseek"));

        // upsert (create)
        let view = upsert_custom_provider(agent_providers::UpsertCustomProviderInput {
            id: "acme".to_string(),
            name: "Acme".to_string(),
            api: "openai-completions".to_string(),
            base_url: "https://api.acme.com/v1".to_string(),
            api_key: None,
            models: vec![],
            create: true,
        })
        .await
        .expect("upsert");
        assert!(agent.served("upsert_provider", ""));
        assert!(view.builtin.iter().any(|p| p.id == "deepseek"));

        // update key
        let view = update_builtin_provider_key(agent_providers::UpdateBuiltinProviderKeyInput {
            id: "deepseek".to_string(),
            api_key: Some("sk-test".to_string()),
        })
        .await
        .expect("key");
        assert!(agent.served("set_auth", ""));
        assert!(view.builtin.iter().any(|p| p.id == "deepseek"));

        // set base url
        let view = set_builtin_provider_base_url(agent_providers::SetBuiltinProviderBaseUrlInput {
            id: "deepseek".to_string(),
            base_url: "https://custom.example.com/v1".to_string(),
        })
        .await
        .expect("base url");
        assert!(agent.served("upsert_provider", ""));
        assert!(view.builtin.iter().any(|p| p.id == "deepseek"));

        // delete
        let view = delete_custom_provider("acme".to_string())
            .await
            .expect("delete");
        assert!(agent.served("delete_provider", ""));
        assert!(view.builtin.iter().any(|p| p.id == "deepseek"));
    }
}
