mod client;
mod persist;
mod review;
mod stream;

pub use review::retry as retry_run_review;

use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
};

use self::client::{
    agent_endpoint, approval_decision_command, base_command, get_state_command,
    new_session_command, prompt_command, set_model_command, set_permission_level_command,
    set_thinking_level_command,
};
use self::stream::collect_agent_response;
use crate::{
    agent_proto::{FutureAgentClient, StreamRequest},
    store,
};

static ACTIVE_AGENT_PROMPTS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPromptResponse {
    content: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelOption {
    id: String,
    label: String,
    provider: String,
    #[serde(default)]
    supports_images: bool,
    #[serde(default)]
    thinking_level: Option<String>,
    #[serde(default)]
    context_window: Option<i32>,
    #[serde(default)]
    is_default: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentModelsResponse {
    models: Vec<AgentModelOption>,
}

pub async fn list_agent_models() -> Result<Vec<AgentModelOption>, crate::AppError> {
    let endpoint = agent_endpoint();
    let mut client = FutureAgentClient::connect(endpoint.clone())
        .await
        .map_err(|error| format!("Unable to connect to Future Agent at {endpoint}: {error}"))?;
    let response = client
        .execute_command(base_command("list_models", String::new()))
        .await
        .map_err(|error| format!("Unable to load Future Agent models: {error}"))?
        .into_inner();

    if !response.success {
        return Err(if response.error.is_empty() {
            "Future Agent rejected the model list request.".to_string()
        } else {
            response.error
        }
        .into());
    }

    let parsed = serde_json::from_str::<AgentModelsResponse>(&response.data)
        .map_err(|error| format!("Future Agent returned invalid model data: {error}"))?;
    Ok(parsed.models)
}

pub async fn agent_prompt(
    message: String,
    image_paths: Option<Vec<String>>,
    thread_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<AgentPromptResponse, crate::AppError> {
    // The session guard spans the whole prompt *and* the synchronous after
    // snapshot capture (§6.1), so the next prompt for this session can't start
    // writing before this Run's after snapshot lands. The deferred diff
    // materialization (C1) needs no guard — it's a read-only diff of fixed commits.
    let effective_session_id = session_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| thread_id.clone());
    let _prompt_guard = match PromptSessionGuard::acquire(&effective_session_id) {
        Ok(guard) => guard,
        Err(error) => {
            mark_run_failed_if_active(run_id.as_deref(), &error.to_string());
            return Err(error);
        }
    };

    let result = agent_prompt_inner(
        message,
        image_paths,
        thread_id.clone(),
        session_id,
        run_id.clone(),
        model_id,
        thinking_level,
    )
    .await;

    // Project the failure status immediately so the Run row is correct on return.
    if let Err(error) = &result {
        mark_run_failed_if_active(run_id.as_deref(), &error.to_string());
    }

    if let Some(run_id) = run_id.clone() {
        // §6.2: a normal `agent_end` means the Agent has stopped writing. On an
        // abnormal return wait for the Agent to confirm idle before snapshotting.
        if result.is_err() {
            wait_for_agent_idle(&effective_session_id).await;
        }
        // §6.1: capture the after snapshot synchronously — the prompt guard is
        // still held here, so the next Run's before-snapshot can't interleave.
        let sensitive = review::capture_after(&thread_id, &run_id);
        // C1: the diff materialization is a read-only diff between fixed commits,
        // so defer it off the IPC path. The GUI is notified when it lands.
        tokio::spawn(async move {
            let materialize_thread = thread_id.clone();
            let materialize_run = run_id.clone();
            let _ = tokio::task::spawn_blocking(move || {
                review::materialize_changeset(&materialize_thread, &materialize_run, sensitive);
            })
            .await;
            crate::emit_review_updated(&thread_id);
        });
    }

    // The prompt guard drops as this function returns; the next Run's
    // before-snapshot then serializes behind the after snapshot via the
    // Workspace lock (§12.1).
    result
}

async fn agent_prompt_inner(
    message: String,
    image_paths: Option<Vec<String>>,
    thread_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<AgentPromptResponse, crate::AppError> {
    let endpoint = agent_endpoint();
    let session_id = session_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| thread_id.clone());
    // The session guard is held by the outer `agent_prompt` so it also covers
    // after-snapshot finalization (§6.1).
    let cwd = workspace_path_for_thread(&thread_id)?;
    let prior_user_message_count = prior_user_message_count(&thread_id)?;
    let force_reset_session = prior_user_message_count == 0;

    let mut command_client = FutureAgentClient::connect(endpoint.clone())
        .await
        .map_err(|error| format!("Unable to connect to Future Agent at {endpoint}: {error}"))?;
    ensure_agent_session(&mut command_client, &session_id, &cwd, force_reset_session).await?;
    set_agent_permission_level(&mut command_client, &session_id, "workspace").await?;

    let mut event_client = FutureAgentClient::connect(endpoint.clone())
        .await
        .map_err(|error| format!("Unable to connect to Future Agent at {endpoint}: {error}"))?;
    let mut event_stream = event_client
        .stream_events(StreamRequest {
            event_types: vec![],
            session_id: session_id.clone(),
        })
        .await
        .map_err(|error| format!("Unable to subscribe to Future Agent events: {error}"))?
        .into_inner();

    if let Some(model_id) = model_id.filter(|value| !value.trim().is_empty()) {
        let response = command_client
            .execute_command(set_model_command(model_id, session_id.clone()))
            .await
            .map_err(|error| format!("Unable to set Future Agent model: {error}"))?
            .into_inner();

        if !response.success {
            return Err(if response.error.is_empty() {
                "Future Agent rejected the model selection.".to_string()
            } else {
                response.error
            }
            .into());
        }
    }

    if let Some(thinking_level) = thinking_level.filter(|value| !value.trim().is_empty()) {
        let response = command_client
            .execute_command(set_thinking_level_command(
                thinking_level,
                session_id.clone(),
            ))
            .await
            .map_err(|error| format!("Unable to set Future Agent thinking level: {error}"))?
            .into_inner();

        if !response.success {
            return Err(if response.error.is_empty() {
                "Future Agent rejected the thinking level selection.".to_string()
            } else {
                response.error
            }
            .into());
        }
    }

    // §6.1: before snapshot, after session/model setup but right before the
    // prompt actually reaches the Agent.
    if let Some(run_id) = run_id.as_deref() {
        review::capture_before(&thread_id, run_id);
    }

    let response = command_client
        .execute_command(prompt_command(
            message,
            session_id,
            image_paths.unwrap_or_default(),
        )?)
        .await
        .map_err(|error| format!("Unable to send prompt to Future Agent: {error}"))?
        .into_inner();

    if !response.success {
        return Err(if response.error.is_empty() {
            "Future Agent rejected the prompt.".to_string()
        } else {
            response.error
        }
        .into());
    }

    let content = collect_agent_response(&mut event_stream, run_id.as_deref()).await?;
    Ok(AgentPromptResponse { content })
}

struct PromptSessionGuard {
    session_id: String,
}

impl PromptSessionGuard {
    fn acquire(session_id: &str) -> Result<Self, crate::AppError> {
        let active = ACTIVE_AGENT_PROMPTS.get_or_init(|| Mutex::new(HashSet::new()));
        let mut guard = active
            .lock()
            .map_err(|_| "Unable to lock active Agent prompt registry.".to_string())?;
        if !guard.insert(session_id.to_string()) {
            return Err("Future Agent is already running for this session."
                .to_string()
                .into());
        }
        Ok(Self {
            session_id: session_id.to_string(),
        })
    }
}

impl Drop for PromptSessionGuard {
    fn drop(&mut self) {
        if let Some(active) = ACTIVE_AGENT_PROMPTS.get() {
            if let Ok(mut guard) = active.lock() {
                guard.remove(&self.session_id);
            }
        }
    }
}

/// Poll the Agent's `get_state.isStreaming` until it reports idle (or a short
/// timeout / the agent disappears). Best-effort confirmation that the Agent has
/// stopped writing files before the after snapshot (§6.2).
async fn wait_for_agent_idle(session_id: &str) {
    let endpoint = agent_endpoint();
    let Ok(mut client) = FutureAgentClient::connect(endpoint).await else {
        return;
    };
    // ~5s budget at 200ms intervals.
    for _ in 0..25 {
        match client
            .execute_command(get_state_command(session_id.to_string()))
            .await
        {
            Ok(response) => {
                let data = response.into_inner().data;
                let streaming = serde_json::from_str::<serde_json::Value>(&data)
                    .ok()
                    .and_then(|value| value.get("isStreaming").and_then(|s| s.as_bool()))
                    .unwrap_or(false);
                if !streaming {
                    return;
                }
            }
            // Agent unreachable → treat as idle; nothing more we can confirm.
            Err(_) => return,
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

fn mark_run_failed_if_active(run_id: Option<&str>, error: &str) {
    let Some(run_id) = run_id else {
        return;
    };
    let Ok(Some(run)) = store::get_run(run_id) else {
        return;
    };
    if matches!(run.status.as_str(), "completed" | "failed" | "cancelled") {
        return;
    }
    let error_type = crate::run_error::classify_run_error(error);
    if let Err(update_error) = store::update_run_status(store::UpdateRunStatusInput {
        run_id: run_id.to_string(),
        status: "failed".to_string(),
        error_message: Some(error.to_string()),
        error_type: Some(error_type.to_string()),
    }) {
        eprintln!("FutureOS run failure status update failed: {update_error}");
    }
}

pub async fn notify_agent_approval_decision(
    approval: &store::ApprovalRequestRecord,
    input: &store::DecideApprovalRequestInput,
) -> Result<(), crate::AppError> {
    let thread = store::get_thread(&approval.thread_id)?
        .ok_or_else(|| "Approval thread could not be loaded.".to_string())?;
    let endpoint = agent_endpoint();
    let mut client = FutureAgentClient::connect(endpoint.clone())
        .await
        .map_err(|error| format!("Unable to connect to Future Agent at {endpoint}: {error}"))?;
    let response = client
        .execute_command(approval_decision_command(
            approval.id.clone(),
            input.status.clone(),
            input.decision_note.clone().unwrap_or_default(),
            thread.agent_session_id.unwrap_or(thread.id),
        ))
        .await
        .map_err(|error| format!("Unable to send approval decision to Future Agent: {error}"))?
        .into_inner();

    if response.success {
        Ok(())
    } else if response.error.is_empty() {
        Err("Future Agent rejected the approval decision."
            .to_string()
            .into())
    } else {
        Err(response.error.into())
    }
}

pub async fn abort_agent_thread(thread_id: &str) -> Result<(), crate::AppError> {
    let thread =
        store::get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let endpoint = agent_endpoint();
    let mut client = FutureAgentClient::connect(endpoint.clone())
        .await
        .map_err(|error| format!("Unable to connect to Future Agent at {endpoint}: {error}"))?;
    let response = client
        .execute_command(base_command(
            "abort",
            thread.agent_session_id.unwrap_or(thread.id),
        ))
        .await
        .map_err(|error| format!("Unable to abort Future Agent run: {error}"))?
        .into_inner();

    if response.success {
        Ok(())
    } else if response.error.is_empty() {
        Err("Future Agent rejected the abort request."
            .to_string()
            .into())
    } else {
        Err(response.error.into())
    }
}

/// Abort an in-flight agent run, then mark its store run cancelled. A missing
/// agent (e.g. the backend is down) is tolerated — the run is still cancelled
/// locally so the UI doesn't strand on a "running" row.
pub async fn abort_run(
    thread_id: String,
    run_id: String,
) -> Result<store::RunRecord, crate::AppError> {
    if let Err(error) = abort_agent_thread(&thread_id).await {
        if !is_agent_unavailable_error(&error.to_string()) {
            return Err(error);
        }
        eprintln!("FutureOS agent abort skipped because agent is unavailable: {error}");
    }
    store::update_run_status(store::UpdateRunStatusInput {
        run_id,
        status: "cancelled".to_string(),
        error_message: Some("Terminated by user.".to_string()),
        error_type: Some("abort_requested".to_string()),
    })
}

/// Record an approval decision: notify the agent while the request is still
/// pending, persist the decision, and resume the owning run. A request the
/// agent already dropped is reconciled by cancelling it locally.
pub async fn decide_approval(
    input: store::DecideApprovalRequestInput,
) -> Result<store::ApprovalRequestRecord, crate::AppError> {
    let current = store::get_approval_request(&input.approval_request_id)?
        .ok_or_else(|| "Approval request could not be loaded.".to_string())?;
    if current.status == "pending" {
        if let Err(error) = notify_agent_approval_decision(&current, &input).await {
            if is_stale_approval_error(&error.to_string()) {
                return store::decide_approval_request(store::DecideApprovalRequestInput {
                    approval_request_id: input.approval_request_id,
                    status: "cancelled".to_string(),
                    decision_note: Some("Cancelled because the approval request is no longer active in Future Agent.".to_string()),
                });
            }
            return Err(error);
        }
    }
    let updated = store::decide_approval_request(input)?;
    if let Some(run_id) = &updated.run_id {
        if let Ok(Some(run)) = store::get_run(run_id) {
            if !matches!(run.status.as_str(), "completed" | "failed" | "cancelled") {
                let _ = store::update_run_status(store::UpdateRunStatusInput {
                    run_id: run_id.clone(),
                    status: "running".to_string(),
                    error_message: None,
                    error_type: None,
                });
            }
        }
    }
    Ok(updated)
}

fn is_stale_approval_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("approval request") && normalized.contains("not pending")
}

fn is_agent_unavailable_error(error: &str) -> bool {
    error.starts_with("Unable to connect to Future Agent")
}

async fn ensure_agent_session(
    client: &mut FutureAgentClient<tonic::transport::Channel>,
    session_id: &str,
    cwd: &str,
    force_reset: bool,
) -> Result<(), crate::AppError> {
    if force_reset {
        return create_agent_session(client, session_id, cwd).await;
    }

    let response = client
        .execute_command(get_state_command(session_id.to_string()))
        .await
        .map_err(|error| format!("Unable to inspect Future Agent session: {error}"))?
        .into_inner();

    if response.success {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.data) {
            let active_session_id = value
                .get("sessionId")
                .and_then(|session_id| session_id.as_str())
                .unwrap_or_default();
            let active_cwd = value
                .get("cwd")
                .and_then(|cwd| cwd.as_str())
                .unwrap_or_default();

            if active_session_id == session_id && active_cwd == cwd {
                return Ok(());
            }
        }
    }

    create_agent_session(client, session_id, cwd).await
}

async fn create_agent_session(
    client: &mut FutureAgentClient<tonic::transport::Channel>,
    session_id: &str,
    cwd: &str,
) -> Result<(), crate::AppError> {
    let response = client
        .execute_command(new_session_command(session_id.to_string(), cwd.to_string()))
        .await
        .map_err(|error| format!("Unable to create Future Agent session: {error}"))?
        .into_inner();

    if response.success {
        Ok(())
    } else if response.error.is_empty() {
        Err("Future Agent rejected the session initialization."
            .to_string()
            .into())
    } else {
        Err(response.error.into())
    }
}

async fn set_agent_permission_level(
    client: &mut FutureAgentClient<tonic::transport::Channel>,
    session_id: &str,
    level: &str,
) -> Result<(), crate::AppError> {
    let response = client
        .execute_command(set_permission_level_command(
            level.to_string(),
            session_id.to_string(),
        ))
        .await
        .map_err(|error| format!("Unable to set Future Agent permission level: {error}"))?
        .into_inner();

    if response.success {
        Ok(())
    } else if response.error.is_empty() {
        Err("Future Agent rejected the permission level selection."
            .to_string()
            .into())
    } else {
        Err(response.error.into())
    }
}

fn prior_user_message_count(thread_id: &str) -> Result<usize, crate::AppError> {
    let messages = store::list_messages(thread_id)?;
    let user_message_count = messages
        .iter()
        .filter(|message| message.role == "user")
        .count();
    Ok(user_message_count.saturating_sub(1))
}

fn workspace_path_for_thread(thread_id: &str) -> Result<String, crate::AppError> {
    let thread =
        store::get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let workspace = store::get_workspace(&thread.workspace_id)?
        .ok_or_else(|| "Thread workspace could not be loaded.".to_string())?;
    Ok(workspace.path)
}
