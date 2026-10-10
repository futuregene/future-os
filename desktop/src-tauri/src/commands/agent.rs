//! Agent model listing and prompting Tauri commands.

use crate::agent_bridge;

#[tauri::command]
pub async fn get_agent_status<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> crate::agent_supervisor::AgentStatus {
    crate::agent_supervisor::agent_status_with_recovery(app).await
}

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
pub async fn probe_sandbox() -> Result<agent_bridge::SandboxProbeResult, crate::AppError> {
    agent_bridge::probe_sandbox().await
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
pub async fn agent_prompt<R: tauri::Runtime>(
    webview: tauri::Webview<R>,
    request: agent_bridge::AgentPromptRequest,
    on_accepted: Option<tauri::ipc::JavaScriptChannelId>,
) -> Result<agent_bridge::AgentPromptResponse, crate::AppError> {
    forward_prompt_acceptance(request, on_accepted.map(|id| id.channel_on(webview))).await
}

pub(crate) async fn forward_prompt_acceptance(
    request: agent_bridge::AgentPromptRequest,
    on_accepted: Option<tauri::ipc::Channel<()>>,
) -> Result<agent_bridge::AgentPromptResponse, crate::AppError> {
    let Some(channel) = on_accepted else {
        return agent_bridge::agent_prompt_with_model_context(request).await;
    };
    let (accepted_tx, mut accepted_rx) = tokio::sync::oneshot::channel();
    let prompt = agent_bridge::agent_prompt_with_acceptance(request, Some(accepted_tx));
    tokio::pin!(prompt);
    tokio::select! {
        biased;
        accepted = &mut accepted_rx => {
            if accepted.is_ok() {
                let _ = channel.send(());
            }
            prompt.await
        }
        result = &mut prompt => {
            // A very short run can finish in the same poll that sends its ACK.
            if accepted_rx.try_recv().is_ok() {
                let _ = channel.send(());
            }
            result
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
    use std::collections::HashMap;

    #[test]
    fn async_command_wrappers_reject_malformed_bodies() {
        // No-argument probe/reset commands legitimately accept an empty body.
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![set_default_model, agent_prompt],
            &["set_default_model", "agent_prompt"],
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

    /// The `get_agent_status` wrapper must forward the agent's own phase (and
    /// not invent a recovery for a healthy agent).
    #[tokio::test]
    async fn get_agent_status_forwards_the_agents_phase() {
        let _lock = mock_agent_lock();
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "get_agent_readiness".to_string(),
                serde_json::json!({
                    "version": crate::build_info::VERSION,
                    "agentInstanceId": "matching-agent",
                })
                .to_string(),
            )]),
            ..Default::default()
        });
        let app = tauri::test::mock_app();
        let status = get_agent_status(app.handle().clone()).await;
        assert_eq!(
            status.phase, "ready",
            "a healthy agent must be reported as ready, not as recovering"
        );
        script_mock_agent(MockScript::default());
    }

    /// With an acceptance channel supplied, the prompt's result still reaches
    /// the caller: the select must fall through to the prompt branch (and must
    /// not push an acceptance the agent never sent).
    #[tokio::test]
    async fn a_prompt_with_an_acceptance_channel_forwards_the_result() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let _lock = mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-agent-accept");
        crate::store::initialize_app_store().expect("init store");
        crate::commands::agent_mock::ensure_mock_agent();

        let accepted = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&accepted);
        let channel = tauri::ipc::Channel::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        let error = forward_prompt_acceptance(
            agent_bridge::AgentPromptRequest {
                message: "hi".to_string(),
                model_context: String::new(),
                attachments: None,
                thread_id: "ghost".to_string(),
                session_id: None,
                run_id: None,
                model_id: None,
                thinking_level: None,
            },
            Some(channel),
        )
        .await
        .expect_err("a prompt for a missing thread must still propagate its error");
        assert!(!error.to_string().is_empty());
        assert_eq!(
            accepted.load(Ordering::SeqCst),
            0,
            "an agent that never acknowledged must not produce an acceptance"
        );
    }

    #[tokio::test]
    async fn agent_status_requires_the_running_agent_version_to_match() {
        let _lock = mock_agent_lock();
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "get_agent_readiness".to_string(),
                serde_json::json!({
                    "version": crate::build_info::VERSION,
                    "agentInstanceId": "matching-agent",
                })
                .to_string(),
            )]),
            ..Default::default()
        });
        assert_eq!(crate::agent_supervisor::agent_status().await.phase, "ready");

        script_mock_agent(MockScript {
            data: HashMap::from([(
                "get_agent_readiness".to_string(),
                r#"{"version":"older","agentInstanceId":"old-agent"}"#.to_string(),
            )]),
            ..Default::default()
        });
        assert_eq!(
            crate::agent_supervisor::agent_status().await.phase,
            "incompatible"
        );
        script_mock_agent(MockScript {
            errors: HashMap::from([(
                "get_agent_readiness".to_string(),
                "unknown command: get_agent_readiness".to_string(),
            )]),
            ..Default::default()
        });
        assert_eq!(
            crate::agent_supervisor::agent_status().await.phase,
            "incompatible"
        );
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
    async fn sandbox_maintenance_commands_parse_agent_results() {
        let _lock = mock_agent_lock();
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([
                (
                    "probe_sandbox".to_string(),
                    r#"{"available":true,"code":"available","backend":"linux_bubblewrap","path":"/usr/bin/bwrap","version":"0.11.1","capabilities":{}}"#.to_string(),
                ),
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
        let product_probe = probe_sandbox().await.expect("product probe");
        assert!(product_probe.available);
        assert_eq!(product_probe.backend, "linux_bubblewrap");
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
        assert_eq!(result.revision, 0);
        script_mock_agent(MockScript::default());
    }

    /// The `agent_prompt` IPC wrapper itself — the body no test ever called.
    ///
    /// In production it is reached only through `tauri::generate_handler!`, so
    /// the wrapper's body (and the `channel_on(webview)` seam) had never
    /// executed. This calls it the way the generated command does: a real
    /// `Webview` handle, no acceptance channel, and a conversation the store does
    /// not know — so the body runs and the handler's error is forwarded verbatim.
    #[tokio::test]
    async fn the_agent_prompt_wrapper_forwards_through_a_real_webview() {
        let _lock = mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-agent-wrapper");
        crate::store::initialize_app_store().expect("init store");
        crate::commands::agent_mock::ensure_mock_agent();

        let app = tauri::test::mock_app();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview");
        let webview: tauri::Webview<tauri::test::MockRuntime> = window.as_ref().clone();

        let error = agent_prompt(
            webview,
            agent_bridge::AgentPromptRequest {
                message: "hello".to_string(),
                model_context: String::new(),
                attachments: None,
                thread_id: "ghost".to_string(),
                session_id: None,
                run_id: None,
                model_id: None,
                thinking_level: None,
            },
            None,
        )
        .await
        .expect_err("a prompt for an unknown conversation must fail");
        assert!(
            !error.to_string().is_empty(),
            "the wrapper must forward the handler's own message"
        );
    }

    /// A prompt the agent *accepts* must reach the acceptance channel — the
    /// whole reason `forward_prompt_acceptance` has a `select!`.
    ///
    /// The sibling test above drives the error path, where no acknowledgement is
    /// ever sent and the channel must stay silent. This one drives the success
    /// path: the agent acknowledges, so the `accepted` branch of the select must
    /// fire and push exactly one message before the run's own result is returned.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_accepted_prompt_notifies_the_channel_once_and_returns_the_run() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let _lock = mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-agent-accept-ok");
        crate::store::initialize_app_store().expect("init store");
        let mock = crate::commands::agent_mock::ensure_mock_agent();

        // A real conversation: a workspace thread with no agent session yet, so
        // the prompt has to create one before the agent can accept it.
        let workspace_dir =
            std::env::temp_dir().join(format!("futureos-agent-accept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workspace_dir);
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let workspace = crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("Accept".into()),
            path: workspace_dir.display().to_string(),
            description: None,
            create_directory: Some(true),
        })
        .expect("workspace");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "workspace".into(),
            title: Some("Accept".into()),
            workspace_id: Some(workspace.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("thread");
        let run = crate::store::create_run(crate::store::CreateRunInput {
            id: Some("run_accept_ok".into()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("run");

        mock.script(
            "new_session",
            true,
            serde_json::json!({ "sessionId": "sess-accept-ok" }),
            "",
        );
        // Let the run finish, so the select's *second* branch also has something
        // to return and the test cannot pass by hanging.
        mock.complete_next_run_stream();

        let accepted = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&accepted);
        let channel = tauri::ipc::Channel::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            forward_prompt_acceptance(
                agent_bridge::AgentPromptRequest {
                    message: "hello from the acceptance test".to_string(),
                    model_context: String::new(),
                    attachments: None,
                    thread_id: thread.id.clone(),
                    session_id: None,
                    run_id: Some(run.id.clone()),
                    model_id: None,
                    thinking_level: None,
                },
                Some(channel),
            ),
        )
        .await
        .expect("the prompt must settle rather than hang");

        assert_eq!(
            accepted.load(Ordering::SeqCst),
            1,
            "the agent's acknowledgement must be pushed to the channel exactly \
             once: {result:?}"
        );
        // The run must also have *completed*: that is what proves the one-shot
        // `complete_next_run_stream` was consumed by this test's stream rather
        // than left armed for a later test to trip over (it lives on the
        // process-global mock, and a leaked flag is what makes a sibling test
        // that expects a still-pending stream fail).
        let response = result.expect("the prompt must return its response");
        assert!(
            response.complete,
            "the scripted stream must have finished the run: {response:?}"
        );
        script_mock_agent(MockScript::default());
        mock.clear_scripts();
    }

    #[tokio::test]
    async fn agent_prompt_wrapper_delegates_and_propagates_errors() {
        let _lock = mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-agent-prompt");
        crate::store::initialize_app_store().expect("init store");
        crate::commands::agent_mock::ensure_mock_agent();
        // A thread the store has never seen fails before any prompt work — the
        // wrapper's job is just to forward the error (and the message).
        let error = forward_prompt_acceptance(
            agent_bridge::AgentPromptRequest {
                message: "hi".to_string(),
                model_context: String::new(),
                attachments: None,
                thread_id: "ghost".to_string(),
                session_id: None,
                run_id: None,
                model_id: None,
                thinking_level: None,
            },
            None,
        )
        .await
        .expect_err("prompt should fail for a missing thread");
        assert!(!error.to_string().is_empty());
    }
}
