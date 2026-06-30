mod approval;
mod client;
mod models;
mod persist;
mod review;
mod run_control;
mod session;
mod skills;
mod stream;

pub use self::approval::decide_approval;
pub(crate) use self::client::raw_agent_addr;
pub use self::models::{list_agent_models, AgentModelOption};
pub use self::run_control::abort_run;
pub use self::skills::{list_installed_skills, InstalledSkill};
pub use review::retry as retry_run_review;

use serde::Serialize;
use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
};

use self::client::{
    base_command, connect_agent, prompt_command, set_model_command, set_thinking_level_command,
    RpcResponseExt,
};
use self::run_control::{mark_run_failed_if_active, wait_for_agent_idle};
use self::session::{
    ensure_agent_session, prior_user_message_count, set_agent_permission_level,
    workspace_path_for_thread,
};
use self::stream::collect_agent_response;
use crate::agent_proto::StreamRequest;

static ACTIVE_AGENT_PROMPTS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPromptResponse {
    content: String,
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
        // §6.1: capture the after snapshot before the guard drops, so the next
        // Run's before-snapshot can't interleave. It forks `git` and does fs IO,
        // so run it on a blocking thread rather than stalling the async runtime.
        let sensitive = {
            let capture_thread = thread_id.clone();
            let capture_run = run_id.clone();
            tokio::task::spawn_blocking(move || {
                review::capture_after(&capture_thread, &capture_run)
            })
            .await
            .unwrap_or_default()
        };
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
    let session_id = session_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| thread_id.clone());
    // The session guard is held by the outer `agent_prompt` so it also covers
    // after-snapshot finalization (§6.1).
    let cwd = workspace_path_for_thread(&thread_id)?;
    let prior_user_message_count = prior_user_message_count(&thread_id)?;
    let force_reset_session = prior_user_message_count == 0;

    let mut command_client = connect_agent().await?;
    ensure_agent_session(&mut command_client, &session_id, &cwd, force_reset_session).await?;
    set_agent_permission_level(&mut command_client, &session_id, "workspace").await?;

    let mut event_client = connect_agent().await?;
    let mut event_stream = event_client
        .stream_events(StreamRequest {
            event_types: vec![],
            session_id: session_id.clone(),
        })
        .await
        .map_err(|error| format!("Unable to subscribe to Future Agent events: {error}"))?
        .into_inner();

    if let Some(model_id) = model_id.filter(|value| !value.trim().is_empty()) {
        command_client
            .execute_command(set_model_command(model_id, session_id.clone()))
            .await
            .map_err(|error| format!("Unable to set Future Agent model: {error}"))?
            .into_inner()
            .ok_or_rpc_error("Future Agent rejected the model selection.")?;
    }

    if let Some(thinking_level) = thinking_level.filter(|value| !value.trim().is_empty()) {
        command_client
            .execute_command(set_thinking_level_command(
                thinking_level,
                session_id.clone(),
            ))
            .await
            .map_err(|error| format!("Unable to set Future Agent thinking level: {error}"))?
            .into_inner()
            .ok_or_rpc_error("Future Agent rejected the thinking level selection.")?;
    }

    // §6.1: before snapshot, after session/model setup but right before the
    // prompt actually reaches the Agent.
    if let Some(run_id) = run_id.as_deref() {
        review::capture_before(&thread_id, run_id);
    }

    command_client
        .execute_command(prompt_command(
            message,
            session_id.clone(),
            image_paths.unwrap_or_default(),
        )?)
        .await
        .map_err(|error| format!("Unable to send prompt to Future Agent: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the prompt.")?;

    match collect_agent_response(&mut event_stream, run_id.as_deref()).await {
        Ok(content) => Ok(AgentPromptResponse { content }),
        Err(error) => {
            // The prompt was already accepted, so the Agent keeps running
            // server-side with no consumer once we drop the stream — and there is
            // no resume path. Tell it to stop so we don't orphan the run (and so
            // the after-snapshot doesn't race a still-writing Agent). Best-effort:
            // if this is itself the result of a user abort, the extra abort is a
            // harmless no-op.
            if let Err(abort_error) = command_client
                .execute_command(base_command("abort", session_id))
                .await
            {
                eprintln!("FutureOS: failed to abort Agent after stream error: {abort_error}");
            }
            Err(error)
        }
    }
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
