mod approval;
mod client;
mod headless;
mod import;
mod models;
mod persist;
mod replica;
mod review;
mod run_control;
mod session;
mod skills;
mod stream;

pub use self::approval::{decide_approval, inject_session_rule};
pub(crate) use self::client::raw_agent_addr;
pub use self::client::{
    connect_agent, delete_session_command, get_available_models_command, get_run_state_command,
    get_session_entries_command, get_state_command, list_streaming_sessions_command, map_rpc_error,
    set_cwd_command, set_model_command, set_session_name_command, set_thinking_level_command,
    RpcResponseExt,
};
pub use self::headless::{prepare_prompt_persisted, run_prepared_prompt, PreparedPrompt};
pub(crate) use self::import::import_missing_sessions;
pub use self::models::{list_agent_models, AgentModelOption};
pub use self::run_control::abort_run;
pub(crate) use self::run_control::{abort_session, wait_for_agent_idle};
pub use self::session::fork_agent_session;
pub use self::skills::{list_installed_skills, refresh_skills, InstalledSkill};
pub use review::retry as retry_run_review;

use serde::Serialize;
use std::sync::Mutex;

pub use self::client::AttachmentInput;
use self::client::{base_command, prompt_command};
use self::replica::{ReplicaLease, AGENT_REPLICAS};
use self::run_control::mark_run_failed_if_active;
use self::session::{
    ensure_agent_session, is_chat_thread, set_agent_permission_level, set_agent_sandbox_policy,
    workspace_path_for_thread,
};
use crate::agent_proto::StreamRequest;

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
    /// True when the thread already had a session but the agent no longer had
    /// it (or its cwd drifted), so a fresh empty session replaced it. The
    /// frontend must warn the user that prior agent-side context was lost.
    pub session_recreated: bool,
}

/// Fetch the agent's buffered events for a session's current run (P1c backfill).
/// `since_idx = -1` returns the requested run's retained prefix. A stale or
/// unknown `run_id` is an explicit error and never realigns to another run.
/// Returns the parsed `data`
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

/// Fetch a session's full message history from the agent (LLM Message shape:
/// `{role, content, tool_calls?}` where `content` is a string or an array of
/// content blocks). The agent's JSONL is the source of truth for ALL sessions —
/// including TUI/CLI sessions the GUI store only holds as imported thread stubs
/// with no message rows — so the remote bridge serves history from here rather
/// than from the store.
pub async fn get_session_messages(
    session_id: String,
) -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(base_command("get_messages", session_id))
        .await
        .map_err(|status| format!("get_messages failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("get_messages returned an error")?;
    if response.data.is_empty() {
        Ok(serde_json::json!({ "messages": [] }))
    } else {
        Ok(serde_json::from_str(&response.data)?)
    }
}

/// Fetch a session's display entries (user/assistant/tool + session_info) from
/// the agent. Unlike `get_session_messages` (LLM wire shape), entries are
/// display-shaped — plain-text content plus per-entry `meta` (user attachments
/// with cached thumbnails), which is how the GUI rebuilds attachment chips.
pub async fn get_session_entries(session_id: String) -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(get_session_entries_command(session_id))
        .await
        .map_err(|status| format!("get_session_entries failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("get_session_entries returned an error")?;
    if response.data.is_empty() {
        Ok(serde_json::json!({ "entries": [] }))
    } else {
        Ok(serde_json::from_str(&response.data)?)
    }
}

/// Fetch the session's current state (model, thinkingLevel, isStreaming, etc.)
/// from the agent. Used by the remote bridge to populate the web client's
/// model/thinking selectors.
pub async fn get_session_state(session_id: String) -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(get_state_command(session_id))
        .await
        .map_err(|status| format!("get_state failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("get_state returned an error")?;
    if response.data.is_empty() {
        Ok(serde_json::json!({}))
    } else {
        Ok(serde_json::from_str(&response.data)?)
    }
}

/// Fetch the available model list from the agent (for the web client's model selector).
pub async fn get_available_models() -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(get_available_models_command())
        .await
        .map_err(|status| format!("get_available_models failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("get_available_models returned an error")?;
    if response.data.is_empty() {
        Ok(serde_json::json!({ "models": [] }))
    } else {
        Ok(serde_json::from_str(&response.data)?)
    }
}

/// Set the model on a live agent session (remote bridge).
pub async fn set_session_model(
    session_id: String,
    model_id: String,
) -> Result<(), crate::AppError> {
    let mut client = connect_agent().await?;
    client
        .execute_command(set_model_command(model_id, session_id))
        .await
        .map_err(|status| format!("set_model failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("set_model returned an error")?;
    Ok(())
}

/// Set the thinking level on a live agent session (remote bridge).
pub async fn set_session_thinking_level(
    session_id: String,
    level: String,
) -> Result<(), crate::AppError> {
    let mut client = connect_agent().await?;
    client
        .execute_command(set_thinking_level_command(level, session_id))
        .await
        .map_err(|status| format!("set_thinking_level failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("set_thinking_level returned an error")?;
    Ok(())
}

/// Rename a session: update the agent's session name, then mirror to the GUI store.
pub async fn rename_session(session_id: String, name: String) -> Result<(), crate::AppError> {
    let mut client = connect_agent().await?;
    client
        .execute_command(set_session_name_command(name.clone(), session_id.clone()))
        .await
        .map_err(|status| format!("set_session_name failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("set_session_name returned an error")?;
    // Mirror to GUI store so the sidebar title stays in sync.
    if let Ok(Some(thread)) = crate::store::find_thread_by_agent_session(&session_id) {
        let _ = crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id,
            title: name,
        });
    }
    Ok(())
}

/// Create a fresh agent session for a just-created thread and persist the
/// agent-generated session id back onto the thread. Used by the remote
/// `new_session` command so the client receives the *real* agent session id up
/// front. If we instead handed the client the thread id, the agent would run
/// the subsequent prompt under a different (agent-generated) id and every
/// event subject / history lookup on the client would mismatch — events get
/// filtered out and `get_messages` finds nothing.
pub(crate) async fn provision_agent_session(
    thread_id: &str,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<String, crate::AppError> {
    let cwd = workspace_path_for_thread(thread_id)?;
    let mut client = connect_agent().await?;
    // Empty stored id → the agent generates a real session id, seeded with the
    // caller's model / thinking selections (matches the GUI new-chat draft).
    let ensured = ensure_agent_session(
        &mut client,
        "",
        &cwd,
        model_id.as_deref(),
        thinking_level.as_deref(),
    )
    .await?;
    let session_id = ensured.session_id;
    set_agent_permission_level(&mut client, &session_id, "workspace").await?;
    set_agent_sandbox_policy(&mut client, &session_id, thread_id).await?;
    crate::store::update_thread_session_id(thread_id, &session_id)?;
    Ok(session_id)
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
        // Transport-level Unavailable → AgentUnavailable → treated as success
        // above ("no in-memory state to refresh on a down agent").
        .map_err(|status| map_rpc_error("Unable to refresh Future Agent credentials", status))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the credential refresh.")?;
    Ok(())
}

pub async fn agent_prompt(
    message: String,
    attachments: Option<Vec<AttachmentInput>>,
    thread_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<AgentPromptResponse, crate::AppError> {
    let effective_session_id = session_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| thread_id.clone());
    let result = agent_prompt_inner(
        message,
        attachments,
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
        // The run has settled and every event was already persisted to the
        // per-run log (stream.rs awaits each write in order), so drop this run's
        // in-memory events — the Runs panel/inspector read the log from here on.
        // Bounds memory so a long-lived app doesn't hoard every run's events.
        crate::store::clear_run_event_buffer(&run_id);
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

    result
}

async fn agent_prompt_inner(
    message: String,
    attachments: Option<Vec<AttachmentInput>>,
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
    let ensured = ensure_agent_session(
        &mut command_client,
        &stored_session_id,
        &existing_cwd,
        model_id.as_deref(),
        thinking_level.as_deref(),
    )
    .await?;
    let session_id = ensured.session_id;
    if ensured.recreated {
        // The thread's previous agent session was unusable (data gone or cwd
        // drift) and a fresh empty session replaced it. The GUI still shows
        // the old history, so without a visible signal the next reply looks
        // like the agent suddenly "forgot" the conversation.
        eprintln!(
            "FutureOS: thread {thread_id} agent session {stored_session_id} was recreated as {session_id} — prior agent-side context is unavailable"
        );
    }
    set_agent_permission_level(&mut command_client, &session_id, "workspace").await?;
    set_agent_sandbox_policy(&mut command_client, &session_id, &thread_id).await?;

    // For a new chat-thread session, rename the workspace directory to match
    // the agent-generated session id.  Workspace threads already have the
    // correct cwd (the user's project directory).
    if session_id != stored_session_id {
        let _ = crate::store::update_thread_session_id(&thread_id, &session_id);
        if is_chat_thread(&thread_id) {
            let new_cwd =
                crate::store::chat_workspace_path(&session_id).map(|p| p.display().to_string())?;
            if new_cwd != existing_cwd {
                std::fs::create_dir_all(&new_cwd)?;
                let _ = crate::store::update_chat_workspace_path(&thread_id, &new_cwd);
                let _ = command_client
                    .execute_command(set_cwd_command(new_cwd, session_id.clone()))
                    .await;
            }
        }
    }

    // Apply the prompt's model / thinking level ONLY when this call created a
    // fresh session (its generated id differs from the stored one). For an
    // existing session the agent already holds the authoritative model, and an
    // explicit user change is pushed separately by `update_thread_model`'s own
    // `set_model`. Re-applying the caller-supplied value on every prompt let a
    // cold/expired agent-state cache silently switch an existing thread's model
    // to the global last-picked one (the composer's fallback value).
    let session_was_created = session_id != stored_session_id;

    if session_was_created {
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
    }

    // §6.1: before snapshot, after session/model setup but right before the
    // prompt actually reaches the Agent.
    if let Some(run_id) = run_id.as_deref() {
        review::capture_before(&thread_id, run_id);
    }

    // Save the message for auto-naming after the prompt completes.
    let user_message = message.clone();

    let prompt_response = command_client
        .execute_command(prompt_command(
            message,
            session_id.clone(),
            attachments.unwrap_or_default(),
            run_id.clone(),
        )?)
        .await
        .map_err(|error| format!("Unable to send prompt to Future Agent: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the prompt.")?;

    let prompt_ack: serde_json::Value =
        serde_json::from_str(&prompt_response.data).map_err(|error| {
            format!("Future Agent returned an invalid prompt acknowledgement: {error}")
        })?;
    let canonical_run_id = prompt_ack
        .get("run_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Future Agent prompt acknowledgement omitted run_id.".to_string())?
        .to_string();
    if let Some(requested_run_id) = run_id.as_deref() {
        if canonical_run_id != requested_run_id {
            return Err(format!(
                "Future Agent adopted run id {canonical_run_id}, expected {requested_run_id}"
            )
            .into());
        }
    }
    let replica_lease = AGENT_REPLICAS
        .acquire(&canonical_run_id)
        .map_err(crate::AppError::from)?;

    match replica_lease
        .collect(
            run_id.as_deref(),
            &canonical_run_id,
            &session_id,
            &thread_id,
        )
        .await
    {
        Ok(response) => {
            // Auto-name the thread from the first user message if it still has
            // the default title (matching the TUI's first_message fallback).
            auto_name_thread(&thread_id, &user_message);
            Ok(AgentPromptResponse {
                content: response.content,
                complete: response.complete,
                session_id,
                session_recreated: ensured.recreated,
            })
        }
        Err(error) => {
            // The prompt was already accepted, so the Agent keeps running
            // server-side with no consumer once we drop the stream — and there is
            // no resume path. Tell it to stop so we don't orphan the run (and so
            // the after-snapshot doesn't race a still-writing Agent). Best-effort:
            // if this is itself the result of a user abort, the extra abort is a
            // harmless no-op.
            if let Err(abort_error) = command_client
                .execute_command(client::run_control_command(
                    "abort",
                    session_id,
                    Some(canonical_run_id),
                ))
                .await
            {
                eprintln!("FutureOS: failed to abort Agent after stream error: {abort_error}");
            }
            Err(error)
        }
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

// ── Crash-recovery run reanimation ───────────────────────────────────────

/// Called after the agent sidecar is reachable: for every run that was
/// cancelled by startup convergence, check the agent's actual session state.
/// If the agent is still streaming, reanimate the run (back to "running") and
/// spawn a background event collector so the frontend's reattach poll picks up
/// the live preview. If it already finished, mirror its durable journal state.
pub async fn reconcile_interrupted_runs() {
    let Ok(runs) = crate::store::list_interrupted_runs() else {
        return;
    };
    if runs.is_empty() {
        return;
    }
    for run in runs {
        let session_id = run.session_id;
        let run_id = run.run_id;
        match check_and_reanimate_run(&session_id, &run_id).await {
            Ok(()) => {}
            Err(error) => {
                eprintln!("FutureOS run reanimation failed for {run_id}: {error}");
            }
        }
    }
}

/// Reconcile one run that the synchronous startup phase cancelled as
/// interrupted, against the Agent's authoritative view of its session.
///
/// The Agent's `get_state` reports `activeRun` (a live run) and `interruptedRun`
/// (a run that began but never committed — recovered as interrupted-by-restart).
/// The GUI passes its local run id as `requested_run_id` and the Agent adopts it
/// as the canonical id, so canonical == local here. Three cases:
///
/// 1. The Agent is still running THIS exact run (`activeRun.runId == run_id`):
///    reanimate it and spawn a collector so the live preview resumes. We match on
///    the run id, not just "the session is streaming", so a stale local run is
///    never reattached against a different run's stream.
/// 2. The Agent confirms THIS run as interrupted (`interruptedRun.runId ==
///    run_id`): it began but never committed. The sync phase already cancelled it
///    with `error_type='interrupted'`; keep that accurate terminal state rather
///    than falsely settling it as completed.
/// 3. Neither: use `requestedRun` to mirror the exact durable terminal state.
///    If the Agent has no marker for this id, conservatively leave interrupted.
async fn check_and_reanimate_run(session_id: &str, run_id: &str) -> Result<(), String> {
    let mut client = connect_agent().await.map_err(|e| format!("connect: {e}"))?;
    let state = client
        .execute_command(get_run_state_command(
            session_id.to_string(),
            run_id.to_string(),
        ))
        .await
        .map_err(|e| format!("get_state: {e}"))?
        .into_inner();
    if !state.success {
        // The Agent could not resolve this session (its JSONL is gone, or the
        // Agent cannot hydrate it). Treat the run as orphaned: leave it in the
        // interrupted state the synchronous phase set rather than asserting it
        // completed, so it stays visible as interrupted instead of vanishing
        // into a false "completed".
        eprintln!(
            "FutureOS startup reconcile: get_state failed for run {run_id} ({}); leaving interrupted",
            state.error
        );
        return Ok(());
    }
    let state_value = serde_json::from_str::<serde_json::Value>(&state.data).unwrap_or_default();
    let is_streaming = state_value
        .get("isStreaming")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);
    let active_run_id = state_value
        .get("activeRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str())
        .map(str::to_string);
    let interrupted_run_id = state_value
        .get("interruptedRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str())
        .map(str::to_string);
    let requested_terminal = state_value
        .get("requestedRun")
        .filter(|value| value.is_object());

    if is_streaming && active_run_id.as_deref() == Some(run_id) {
        // canonical == local: the Agent adopted this run's requested_run_id.
        let canonical_run_id = run_id.to_string();
        crate::store::reanimate_run(run_id).map_err(|e| format!("reanimate: {e}"))?;
        let run_id = run_id.to_string();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            let replica_lease = match AGENT_REPLICAS.acquire(&canonical_run_id) {
                Ok(lease) => lease,
                Err(error) => {
                    eprintln!("FutureOS skipped duplicate collector for {run_id}: {error}");
                    return;
                }
            };
            if let Err(e) = collect_stored_replica(
                replica_lease,
                &session_id,
                &run_id,
                &canonical_run_id,
                ReplicaSettlement::Interrupted,
            )
            .await
            {
                eprintln!("FutureOS reanimated collector for {run_id} failed: {e}");
            }
        });
    } else if interrupted_run_id.as_deref() == Some(run_id) {
        // The Agent confirms this run began but never committed (interrupted by
        // the restart). The synchronous phase already cancelled it with
        // error_type='interrupted'; leave that accurate state in place.
        eprintln!("FutureOS startup reconcile: run {run_id} confirmed interrupted by restart; leaving cancelled");
    } else if let Some(terminal) = requested_terminal {
        let state = terminal
            .get("state")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "requestedRun omitted terminal state".to_string())?;
        let error = terminal.get("error").and_then(|value| value.as_str());
        crate::store::settle_interrupted_run_from_agent(run_id, state, error)
            .map_err(|e| format!("settle: {e}"))?;
    } else {
        eprintln!(
            "FutureOS startup reconcile: no durable terminal for run {run_id}; leaving interrupted"
        );
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ReplicaSettlement {
    /// A row recovered after process restart.
    Interrupted,
    /// A synthetic row observing a run started by another client.
    Active,
}

async fn collect_stored_replica(
    replica_lease: ReplicaLease,
    session_id: &str,
    local_run_id: &str,
    canonical_run_id: &str,
    settlement: ReplicaSettlement,
) -> Result<(), String> {
    let thread_id = crate::store::get_run(local_run_id)
        .map_err(|e| format!("get_run: {e}"))?
        .ok_or_else(|| format!("local run {local_run_id} not found"))?
        .thread_id;
    let response = replica_lease
        .collect(Some(local_run_id), canonical_run_id, session_id, &thread_id)
        .await
        .map_err(|error| error.to_string())?;
    let terminal = if response.complete {
        "completed"
    } else {
        "failed"
    };
    match settlement {
        ReplicaSettlement::Interrupted => {
            crate::store::settle_interrupted_run(local_run_id, terminal)
                .map_err(|e| format!("settle: {e}"))?;
        }
        ReplicaSettlement::Active => {
            crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
                run_id: local_run_id.to_string(),
                status: terminal.to_string(),
                error_message: (!response.complete)
                    .then(|| "Future Agent response ended before a clean terminal.".to_string()),
                error_type: None,
            })
            .map_err(|e| format!("update_status: {e}"))?;
        }
    }
    crate::emit_thread_runtime_updated(crate::ThreadRuntimeUpdate {
        thread_id,
        run_id: local_run_id.to_string(),
        revision: crate::store::list_run_events(local_run_id)
            .ok()
            .and_then(|events| events.into_iter().map(|event| event.sequence).max())
            .unwrap_or(-1),
        status: terminal.to_string(),
        reset_projection: false,
    });
    crate::store::clear_run_event_buffer(local_run_id);
    Ok(())
}

// ── Remote-stream attach (cross-client streaming) ─────────────────────────

/// Called when the GUI opens a thread whose agent session is being driven by
/// another client (TUI, CLI, phone).  Creates a synthetic run and subscribes
/// to the agent's event stream in the background so the existing reattach
/// machinery picks up live previews and message updates automatically.
pub async fn attach_remote_stream(thread_id: &str) -> Result<String, String> {
    let thread = crate::store::get_thread(thread_id)
        .map_err(|e| format!("get_thread: {e}"))?
        .ok_or_else(|| "Thread not found".to_string())?;
    let session_id = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Thread has no agent session".to_string())?;

    // Don't create a duplicate run if one is already active or waiting on an
    // approval decision.  A run parked at "waiting_approval" is still
    // consuming the agent's event stream; attaching a second synthetic run
    // would spawn a second collector writing duplicate events and misalign
    // run-to-turn pairing in applyRunMetadata.
    //
    // The frontend polls get_state for is_streaming and the run settlement
    // races with that poll — a just-settled run can look like "no active
    // run" for a tick, causing a duplicate creation.
    fn is_active(status: &str) -> bool {
        status == "running" || status == "waiting_approval"
    }
    let existing_runs = crate::store::list_runs(thread_id).unwrap_or_default();
    if let Some(active) = existing_runs.iter().find(|r| is_active(&r.status)) {
        return Ok(active.id.clone());
    }
    let mut client = connect_agent().await.map_err(|e| format!("connect: {e}"))?;
    let state = client
        .execute_command(get_state_command(session_id.to_string()))
        .await
        .map_err(|e| format!("get_state: {e}"))?
        .into_inner();
    let state_value =
        serde_json::from_str::<serde_json::Value>(&state.data).map_err(|e| e.to_string())?;
    let canonical_run_id = state_value
        .get("activeRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Agent session has no active canonical run".to_string())?
        .to_string();
    let replica_lease = AGENT_REPLICAS.acquire(&canonical_run_id)?;

    let run = crate::store::create_run(crate::store::CreateRunInput {
        thread_id: thread_id.to_string(),
        trigger_message_id: None,
        model_provider: None,
        model_id: None,
    })
    .map_err(|e| format!("create_run: {e}"))?;

    let run_id = run.id.clone();
    let sid = session_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = collect_stored_replica(
            replica_lease,
            &sid,
            &run_id,
            &canonical_run_id,
            ReplicaSettlement::Active,
        )
        .await
        {
            eprintln!("FutureOS remote-stream collector for {run_id} failed: {e}");
            let _ = crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
                run_id,
                status: "failed".to_string(),
                error_message: Some(e),
                error_type: None,
            });
        }
    });

    Ok(run.id)
}

// ── Session observer (real-time settings-change events) ───────────────────

use tauri::Emitter;
use tokio::sync::oneshot;

/// Handle to the currently-running session observation task.  When a new
/// observation starts, the old one is cancelled via this channel.
static OBSERVER_CANCEL: Mutex<Option<oneshot::Sender<()>>> = Mutex::new(None);

/// Start observing a session's settings changes in the background.  Subscribes
/// to the agent's StreamEvents and forwards settings-change events to the
/// Event types the session observer forwards to the webview. Whitelist, kept
/// in sync with the frontend consumers: `user_message` (zero-latency user
/// bubble in useThreadMessages) plus the settings-change set applied by
/// agentStateCache (`applySettingsEvent`). Everything else — in particular the
/// per-token `text_chunk`/`thinking_delta`/`tool_*` stream — is dropped here,
/// before the JSON rebuild + Tauri emit, because no frontend listener reads it.
const OBSERVER_FORWARDED_EVENTS: &[&str] = &[
    "agent_start",
    "agent_end",
    "user_message",
    "model_changed",
    "thinking_level_changed",
    "permission_level_changed",
    "session_name_changed",
    "cwd_changed",
    "config_reloaded",
];

/// frontend via Tauri `agent-event` events so the UI reflects model /
/// thinking / name / cwd changes in near real-time (< 1s).
///
/// Cancels any previous observation for this window.  Safe to call on every
/// thread switch — only one observation runs at a time.
pub fn start_observing_session(session_id: String) {
    // Cancel the previous observation.
    if let Ok(mut guard) = OBSERVER_CANCEL.lock() {
        if let Some(tx) = guard.take() {
            let _ = tx.send(());
        }
    }

    let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
    if let Ok(mut guard) = OBSERVER_CANCEL.lock() {
        *guard = Some(cancel_tx);
    }

    tauri::async_runtime::spawn(async move {
        let app_handle = match crate::APP_HANDLE.get() {
            Some(h) => h.clone(),
            None => return,
        };

        // Reconnect loop: if the agent restarts, re-subscribe.
        loop {
            let mut client = match connect_agent().await {
                Ok(c) => c,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };

            let mut stream = match client
                .stream_events(StreamRequest {
                    event_types: vec![],
                    session_id: session_id.clone(),
                    ..Default::default()
                })
                .await
            {
                Ok(s) => s.into_inner(),
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };

            // Process events until cancelled or stream ends.
            loop {
                tokio::select! {
                    _ = &mut cancel_rx => {
                        return;
                    }
                    result = stream.message() => {
                        let event = match result {
                            Ok(Some(e)) => e,
                            _ => break, // stream ended or error — reconnect
                        };

                        // Forward only the events the frontend actually
                        // consumes (see OBSERVER_FORWARDED_EVENTS). Per-token
                        // content events (text_chunk, thinking_delta, tool_*)
                        // used to be JSON-rebuilt and emitted across IPC on
                        // every token only to be discarded by the single
                        // listener — the frontend renders content from the
                        // persisted run-event log instead.
                        if !OBSERVER_FORWARDED_EVENTS.contains(&event.r#type.as_str()) {
                            continue;
                        }
                        if let Ok(mut payload) = serde_json::from_str::<serde_json::Value>(&event.data) {
                            if let serde_json::Value::Object(ref mut map) = payload {
                                map.insert("sessionId".to_string(),
                                    serde_json::Value::String(session_id.clone()));
                                map.insert("_eventType".to_string(),
                                    serde_json::Value::String(event.r#type.clone()));
                            }
                            let _ = app_handle.emit("agent-event", &payload);
                        }
                    }
                }
            }
            // Stream ended — reconnect after a short delay.
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

/// When the agent session's cwd changes (via TUI /cwd or another client),
/// move the thread to the workspace that matches the new cwd.
pub fn reconcile_thread_workspace(session_id: &str, new_cwd: &str) -> Result<(), String> {
    let thread = crate::store::find_thread_by_agent_session(session_id)
        .map_err(|e| format!("find_thread: {e}"))?
        .ok_or_else(|| "No thread found for this session".to_string())?;

    let cwd = new_cwd.trim().trim_end_matches(['/', '\\']);
    if cwd.is_empty() {
        return Ok(());
    }

    // Determine workspace type.
    let is_chat = {
        let cwd_normalized = cwd.replace('\\', "/");
        let chat_dir = format!(
            "{}/.future/workspaces/chat/",
            crate::home_dir().unwrap_or_default()
        );
        cwd_normalized.starts_with(&chat_dir) || cwd_normalized == chat_dir.trim_end_matches('/')
    };

    if is_chat {
        crate::store::update_chat_workspace_path(&thread.id, cwd)
            .map_err(|e| format!("update_workspace: {e}"))?;
        return Ok(());
    }

    // Project workspace: find or create by cwd path.
    let workspace_name = std::path::Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(cwd)
        .to_string();

    let existing = crate::store::list_workspaces()
        .unwrap_or_default()
        .into_iter()
        .find(|w| w.path == cwd);

    let workspace_id = if let Some(ws) = existing {
        ws.id
    } else {
        let ws = crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some(workspace_name),
            path: cwd.to_string(),
            description: None,
            create_directory: Some(false),
        })
        .map_err(|e| format!("create_workspace: {e}"))?;
        ws.id
    };

    // Update the thread's workspace assignment.
    crate::store::move_thread_to_workspace(&thread.id, &workspace_id)
        .map_err(|e| format!("move_thread: {e}"))?;

    Ok(())
}
