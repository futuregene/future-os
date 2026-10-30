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

/// Detect which thinking levels the provider in the editor accepts. One level
/// per call: the GUI probes the implicit `default` first (which proves the
/// endpoint, key and model id), then each candidate level.
#[tauri::command]
pub async fn probe_thinking_level(
    input: agent_providers::ThinkingLevelProbeInput,
) -> Result<agent_providers::ThinkingLevelProbeOutcome, crate::AppError> {
    agent_providers::probe_thinking_level(input).await
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
                delete_custom_provider,
                probe_thinking_level
            ],
            &[
                "upsert_custom_provider",
                "update_builtin_provider_key",
                "update_builtin_provider",
                "set_builtin_provider_base_url",
                "delete_custom_provider",
                "probe_thinking_level",
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

        // atomic builtin update (base URL + key in one form submission)
        let view = update_builtin_provider(agent_providers::UpdateBuiltinProviderInput {
            id: "deepseek".to_string(),
            base_url: Some("https://atomic.example.com/v1".to_string()),
            api_key: Some("sk-atomic".to_string()),
            update_api_key: true,
        })
        .await
        .expect("atomic update");
        assert!(agent.served("upsert_provider", ""));
        assert!(view.builtin.iter().any(|p| p.id == "deepseek"));

        // thinking-level detection: one level per call, against the form the
        // editor holds (nothing is saved before a probe).
        let outcome = probe_thinking_level(agent_providers::ThinkingLevelProbeInput {
            api: "openai-completions".to_string(),
            base_url: "https://api.acme.com/v1".to_string(),
            api_key: None,
            provider_id: Some("acme".to_string()),
            model_id: "qwen".to_string(),
            level: Some("high".to_string()),
        })
        .await
        .expect("probe");
        assert!(agent.served("probe_thinking_level", ""));
        assert_eq!(outcome.level, "high");
        assert!(outcome.ok);

        // delete
        let view = delete_custom_provider("acme".to_string())
            .await
            .expect("delete");
        assert!(agent.served("delete_provider", ""));
        assert!(view.builtin.iter().any(|p| p.id == "deepseek"));
    }
}
