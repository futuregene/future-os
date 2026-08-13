//! Agent model listing and prompting Tauri commands.

use crate::agent_bridge;

#[tauri::command]
pub async fn list_agent_models() -> Result<Vec<agent_bridge::AgentModelOption>, crate::AppError> {
    agent_bridge::list_agent_models().await
}

/// Post-login init: synchronously fetch the Future provider's models in the
/// agent (warming its cache + rebuilding its registry) so the model list is
/// complete before the onboarding gate closes. See [`agent_bridge::sync_future_models`].
#[tauri::command]
pub async fn sync_future_models() -> Result<agent_bridge::SyncFutureModelsResult, crate::AppError> {
    agent_bridge::sync_future_models().await
}

/// Persist the onboarding model-picker's choice as the agent's global default
/// model (settings.json `defaultModel`). Sessionless.
#[tauri::command]
pub async fn set_default_model(model_id: String) -> Result<(), crate::AppError> {
    agent_bridge::set_default_model(model_id).await
}

#[tauri::command]
pub async fn agent_prompt(
    message: String,
    attachments: Option<Vec<agent_bridge::AttachmentInput>>,
    thread_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<agent_bridge::AgentPromptResponse, crate::AppError> {
    agent_bridge::agent_prompt(
        message,
        attachments,
        thread_id,
        session_id,
        run_id,
        model_id,
        thinking_level,
    )
    .await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
    use std::collections::HashMap;

    #[tokio::test]
    async fn list_agent_models_parses_the_agent_response() {
        let _lock = mock_agent_lock();
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("list_models".to_string(), "{\"models\":[]}".to_string())]),
            ..Default::default()
        });
        let models = list_agent_models().await.expect("models");
        assert!(models.is_empty());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn set_default_model_succeeds() {
        let _lock = mock_agent_lock();
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("set_default_model".to_string(), "{}".to_string())]),
            ..Default::default()
        });
        set_default_model("future/deepseek".into())
            .await
            .expect("set");
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn sync_future_models_parses_the_agent_result() {
        let _lock = mock_agent_lock();
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "sync_future_models".to_string(),
                "{\"synced\":true,\"modelCount\":3}".to_string(),
            )]),
            ..Default::default()
        });
        let result = sync_future_models().await.expect("sync");
        assert!(result.synced);
        assert_eq!(result.model_count, 3);
        script_mock_agent(MockScript::default());
    }
}
