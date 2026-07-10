mod approval;
mod client;
mod headless;
mod import;
mod models;
mod persist;
mod review;
mod run_control;
mod session;
mod skills;
mod stream;

pub(crate) use self::import::import_missing_sessions;
pub use self::approval::{decide_approval, inject_session_rule};
pub use self::client::{
    connect_agent, delete_session_command, get_session_entries_command, get_state_command,
    set_cwd_command, set_model_command, set_session_name_command, set_thinking_level_command,
    RpcResponseExt,
};
pub(crate) use self::client::raw_agent_addr;
pub use self::headless::{prepare_prompt_persisted, run_prepared_prompt, PreparedPrompt};
pub use self::models::{list_agent_models, AgentModelOption};
pub use self::run_control::abort_run;
pub(crate) use self::run_control::abort_session;
pub use self::session::fork_agent_session;
pub use self::skills::{list_installed_skills, InstalledSkill};
pub use review::retry as retry_run_review;

use serde::Serialize;
use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
};

use self::client::{
    base_command, prompt_command,
};
use self::run_control::{mark_run_failed_if_active, wait_for_agent_idle};
use self::session::{
    ensure_agent_session, is_chat_thread, set_agent_permission_level,
    set_agent_sandbox_policy, workspace_path_for_thread,
};
use self::stream::collect_agent_response;
use crate::agent_proto::StreamRequest;

static ACTIVE_AGENT_PROMPTS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPromptResponse {
    pub content: String,
    /// Whether the agent stream reached a clean `agent_end`. When false, the
    /// content is a truncated prefix (stream closed mid-reply) and the caller
    /// should finalize the run as failed rather than completed.
    pub complete: bool,
    /// The agent session id (newly-created or existing). The frontend persists
    /// this on the thread so subsequent prompts reuse the same session.
    pub session_id: String,
}

/// Fetch the agent's buffered events for a session's current run (P1c backfill).
/// `since_idx = -1` returns the whole current run; a stale/empty `run_id` also
/// returns the whole current run (the agent realigns). Returns the parsed `data`
/// JSON — shape `{ runId, events: [{ type, data, runId, idx }] }`. Lets a phone /
/// web client that joined an in-flight run mid-stream reconstruct the prefix it
/// missed, keyed by the same `runId`/`idx` the live events carry (so it dedupes).
pub async fn get_events_since(
    session_id: String,
    run_id: String,
    since_idx: i64,
) -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let command = crate::agent_proto::RpcCommand {
        run_id,
        since_idx,
        ..base_command("get_events_since", session_id)
    };
    let response = client
        .execute_command(command)
        .await
        .map_err(|status| format!("get_events_since failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("get_events_since returned an error")?;
    if response.data.is_empty() {
        Ok(serde_json::json!({ "events": [] }))
    } else {
        Ok(serde_json::from_str(&response.data)?)
    }
}

/// Tell the running agent to re-read `auth.json` and refresh every live
/// session's in-memory API key. Call after the GUI mutates credentials
/// (FutureGene login/logout, custom-provider key edits): the agent caches the
/// resolved key inside each session's provider and the prompt path never
/// re-reads `auth.json`, so without this a session keeps serving prompts with a
/// stale key (e.g. still answering after logout) while the model list — which
/// does re-read disk — already shows logged-out.
///
/// Best-effort: if the agent isn't running there's no in-memory state to
/// refresh, so an unavailable agent is treated as success.
pub async fn reload_agent_credentials() -> Result<(), crate::AppError> {
    let mut client = match connect_agent().await {
        Ok(client) => client,
        Err(crate::AppError::AgentUnavailable(_)) => return Ok(()),
        Err(error) => return Err(error),
    };
    client
        .execute_command(base_command("reload_auth", String::new()))
        .await
        .map_err(|error| format!("Unable to refresh Future Agent credentials: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the credential refresh.")?;
    Ok(())
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
    // The frontend may pass None when it doesn't know the session id yet
    // (e.g. first prompt after the thread was created).  Fall back to the
    // thread's persisted agent_session_id so we don't create a new session
    // on every prompt.
    let stored_session_id = session_id
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            crate::store::get_thread(&thread_id)
                .ok()
                .flatten()
                .and_then(|t| t.agent_session_id)
                .filter(|id| !id.trim().is_empty())
        })
        .unwrap_or_default();
    let mut command_client = connect_agent().await?;

    // Create (or reuse) the agent session.  For brand-new threads the session
    // is created with whatever cwd the workspace already has; we'll fix it up
    // once we know the agent-generated session id so the directory can be named
    // after it.
    let existing_cwd = workspace_path_for_thread(&thread_id)?;
    let session_id = ensure_agent_session(
        &mut command_client,
        &stored_session_id,
        &existing_cwd,
        model_id.as_deref(),
        thinking_level.as_deref(),
    )
    .await?;
    set_agent_permission_level(&mut command_client, &session_id, "workspace").await?;
    set_agent_sandbox_policy(&mut command_client, &session_id, &thread_id).await?;

    // For a new chat-thread session, rename the workspace directory to match
    // the agent-generated session id.  Workspace threads already have the
    // correct cwd (the user's project directory).
    if session_id != stored_session_id {
        let _ = crate::store::update_thread_session_id(&thread_id, &session_id);
        if is_chat_thread(&thread_id) {
            let new_cwd = crate::store::chat_workspace_path(&session_id)
                .map(|p| p.display().to_string())?;
            if new_cwd != existing_cwd {
                std::fs::create_dir_all(&new_cwd)?;
                let _ = crate::store::update_chat_workspace_path(&thread_id, &new_cwd);
                let _ = command_client
                    .execute_command(set_cwd_command(new_cwd, session_id.clone()))
                    .await;
            }
        }
    }

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

    // Save the message for auto-naming after the prompt completes.
    let user_message = message.clone();

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

    match collect_agent_response(&mut event_stream, run_id.as_deref(), &session_id).await {
        Ok(response) => {
            // Auto-name the thread from the first user message if it still has
            // the default title (matching the TUI's first_message fallback).
            auto_name_thread(&thread_id, &user_message);
            Ok(AgentPromptResponse {
                content: response.content,
                complete: response.complete,
                session_id,
            })
        },
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

/// Derive a thread title from the first user message, matching the TUI's
/// `first_message` behavior. Only updates the title when it's still a default
/// ("New Chat" or empty), so user-set names are never overwritten.
fn auto_name_thread(thread_id: &str, first_message: &str) {
    let Ok(Some(thread)) = crate::store::get_thread(thread_id) else {
        return;
    };
    // Only auto-name default-titled threads.
    if !thread.title.is_empty() && thread.title != "New Chat" && thread.title != "新对话" {
        return;
    }
    let trimmed = first_message.trim();
    if trimmed.is_empty() {
        return;
    }
    // Truncate to ~40 chars visible width (same as the TUI's truncate_visible).
    let title: String = trimmed.chars().take(40).collect();
    let title = if title.len() < trimmed.len() {
        format!("{}…", title)
    } else {
        title
    };
    let input = crate::store::RenameThreadInput {
        thread_id: thread_id.to_string(),
        title: title.clone(),
    };
    let _ = crate::store::rename_thread(input);

    // Propagate to the agent as well (best-effort, fire-and-forget).
    let session_id = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(&thread.id)
        .to_string();
    tokio::spawn(async move {
        if let Ok(mut client) = crate::agent_bridge::connect_agent().await {
            let cmd = crate::agent_bridge::set_session_name_command(title, session_id);
            let _ = client.execute_command(cmd).await;
        }
    });
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
