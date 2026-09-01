//! Agent model listing and prompting Tauri commands.

use crate::agent_bridge;

#[tauri::command]
pub async fn list_agent_models() -> Result<Vec<agent_bridge::AgentModelOption>, crate::AppError> {
    agent_bridge::list_agent_models().await
}

/// Explicitly fetch the Future provider's models in the agent (warming its
/// cache + rebuilding its registry). The scheduler and this manual/onboarding
/// command share the same single-flight execution path.
#[tauri::command]
pub async fn sync_future_models() -> Result<agent_bridge::SyncFutureModelsResult, crate::AppError> {
    crate::scheduler::refresh_future_models_now().await
}

/// Persist the onboarding model-picker's choice as the agent's global default
/// model (settings.json `defaultModel`). Sessionless.
#[tauri::command]
pub async fn set_default_model(model_id: String) -> Result<(), crate::AppError> {
    agent_bridge::set_default_model(model_id).await
}

#[tauri::command]
pub async fn probe_windows_sandbox(
) -> Result<agent_bridge::WindowsSandboxProbeResult, crate::AppError> {
    agent_bridge::probe_windows_sandbox().await
}

#[tauri::command]
pub async fn reset_windows_sandbox() -> Result<usize, crate::AppError> {
    agent_bridge::reset_windows_sandbox().await
}

#[tauri::command]
pub async fn agent_prompt(
    request: agent_bridge::AgentPromptRequest,
) -> Result<agent_bridge::AgentPromptResponse, crate::AppError> {
    agent_bridge::agent_prompt_with_model_context(request).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
    use std::collections::HashMap;

    #[test]
    fn async_command_wrappers_reject_malformed_bodies() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![
                set_default_model,
                probe_windows_sandbox,
                reset_windows_sandbox,
                agent_prompt
            ],
            &[
                "set_default_model",
                "probe_windows_sandbox",
                "reset_windows_sandbox",
                "agent_prompt",
            ],
        );
        // Feed the request argument a scalar so its `CommandArg` conversion
        // reaches the wrapper's error arm.
        crate::commands::ipc_harness::assert_all_reject_bodies(
            tauri::generate_handler![agent_prompt],
            &[("agent_prompt", serde_json::json!({ "request": 123 }))],
        );
    }

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
    async fn windows_sandbox_maintenance_commands_parse_agent_results() {
        let _lock = mock_agent_lock();
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([
                (
                    "probe_windows_sandbox".to_string(),
                    r#"{"available":true,"code":"available"}"#.to_string(),
                ),
                (
                    "reset_windows_sandbox".to_string(),
                    r#"{"removedCapabilities":3}"#.to_string(),
                ),
            ]),
            ..Default::default()
        });
        let probe = probe_windows_sandbox().await.expect("probe");
        assert!(probe.available);
        assert_eq!(probe.code, "available");
        assert_eq!(reset_windows_sandbox().await.expect("reset"), 3);
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

    #[tokio::test]
    async fn agent_prompt_wrapper_delegates_and_propagates_errors() {
        let _lock = mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-agent-prompt");
        crate::store::initialize_app_store().expect("init store");
        crate::commands::agent_mock::ensure_mock_agent();
        // A thread the store has never seen fails before any prompt work — the
        // wrapper's job is just to forward the error (and the message).
        let error = agent_prompt(agent_bridge::AgentPromptRequest {
            message: "hi".to_string(),
            model_context: String::new(),
            attachments: None,
            thread_id: "ghost".to_string(),
            session_id: None,
            run_id: None,
            model_id: None,
            thinking_level: None,
        })
        .await
        .expect_err("prompt should fail for a missing thread");
        assert!(!error.to_string().is_empty());
    }
}
