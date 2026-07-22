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

pub use self::approval::{decide_approval, inject_session_rule};
pub(crate) use self::client::raw_agent_addr;
pub use self::client::{
    connect_agent, delete_session_command, get_session_entries_command, get_state_command,
    set_cwd_command, set_model_command, set_session_name_command, set_thinking_level_command,
    RpcResponseExt,
};
pub use self::headless::{prepare_prompt_persisted, run_prepared_prompt, PreparedPrompt};
pub(crate) use self::import::import_missing_sessions;
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

pub use self::client::AttachmentInput;
use self::client::{base_command, prompt_command};
use self::run_control::{mark_run_failed_if_active, wait_for_agent_idle};
use self::session::{
    ensure_agent_session, is_chat_thread, set_agent_permission_level, set_agent_sandbox_policy,
    workspace_path_for_thread,
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
    attachments: Option<Vec<AttachmentInput>>,
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

    // The prompt guard drops as this function returns; the next Run's
    // before-snapshot then serializes behind the after snapshot via the
    // Workspace lock (§12.1).
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

    let mut event_client = connect_agent().await?;
    let mut event_stream = event_client
        .stream_events(StreamRequest {
            event_types: vec![],
            session_id: session_id.clone(),
        })
        .await
        .map_err(|error| format!("Unable to subscribe to Future Agent events: {error}"))?
        .into_inner();

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

    command_client
        .execute_command(prompt_command(
            message,
            session_id.clone(),
            attachments.unwrap_or_default(),
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
        }
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

// ── Crash-recovery run reanimation ───────────────────────────────────────

/// Called after the agent sidecar is reachable: for every run that was
/// cancelled by startup convergence, check the agent's actual session state.
/// If the agent is still streaming, reanimate the run (back to "running") and
/// spawn a background event collector so the frontend's reattach poll picks up
/// the live preview. If the agent already finished, mark the run completed.
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

async fn check_and_reanimate_run(session_id: &str, run_id: &str) -> Result<(), String> {
    let mut client = connect_agent().await.map_err(|e| format!("connect: {e}"))?;
    let state = client
        .execute_command(get_state_command(session_id.to_string()))
        .await
        .map_err(|e| format!("get_state: {e}"))?
        .into_inner();
    let is_streaming = serde_json::from_str::<serde_json::Value>(&state.data)
        .ok()
        .and_then(|v| v.get("isStreaming").and_then(|s| s.as_bool()))
        .unwrap_or(false);

    if is_streaming {
        crate::store::reanimate_run(run_id).map_err(|e| format!("reanimate: {e}"))?;
        let run_id = run_id.to_string();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = collect_reanimated_run(&session_id, &run_id).await {
                eprintln!("FutureOS reanimated collector for {run_id} failed: {e}");
            }
        });
    } else {
        crate::store::settle_interrupted_run(run_id, "completed")
            .map_err(|e| format!("settle: {e}"))?;
    }
    Ok(())
}

async fn collect_reanimated_run(session_id: &str, run_id: &str) -> Result<(), String> {
    let mut client = connect_agent().await.map_err(|e| format!("connect: {e}"))?;
    let mut stream = client
        .stream_events(StreamRequest {
            event_types: vec![],
            session_id: session_id.to_string(),
        })
        .await
        .map_err(|e| format!("stream_events: {e}"))?
        .into_inner();

    let mut sequence = 0i64;

    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(600), stream.message())
            .await
            .map_err(|_| "Future Agent response timed out.".to_string())?
            .map_err(|e| format!("stream failed: {e}"))?;

        let Some(event) = event else {
            break;
        };

        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run_id.to_string(),
            event_type: event.r#type.clone(),
            payload: Some(event.data.clone()),
            sequence,
        })
        .map_err(|e| format!("append_event: {e}"))?;

        sequence += 1;

        if event.r#type == "agent_end" {
            crate::store::settle_interrupted_run(run_id, "completed")
                .map_err(|e| format!("settle: {e}"))?;
            crate::store::clear_run_event_buffer(run_id);
            break;
        }
    }
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

    // Don't create a duplicate run if one is already collecting for this
    // thread (e.g. the user clicked twice or the frontend poll ticked before
    // the first run appeared in listRuns).
    let existing_runs = crate::store::list_runs(thread_id).unwrap_or_default();
    if existing_runs.iter().any(|r| r.status == "running") {
        return Ok(existing_runs
            .iter()
            .find(|r| r.status == "running")
            .map(|r| r.id.clone())
            .unwrap_or_default());
    }

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
        if let Err(e) = collect_remote_stream(&sid, &run_id).await {
            eprintln!("FutureOS remote-stream collector for {run_id} failed: {e}");
            let _ = crate::store::update_run_status_if_active(
                crate::store::UpdateRunStatusInput {
                    run_id,
                    status: "failed".to_string(),
                    error_message: Some(e),
                    error_type: None,
                },
            );
        }
    });

    Ok(run.id)
}

async fn collect_remote_stream(session_id: &str, run_id: &str) -> Result<(), String> {
    let mut client = connect_agent().await.map_err(|e| format!("connect: {e}"))?;
    let mut stream = client
        .stream_events(StreamRequest {
            event_types: vec![],
            session_id: session_id.to_string(),
        })
        .await
        .map_err(|e| format!("stream_events: {e}"))?
        .into_inner();

    let mut sequence = 0i64;

    loop {
        let event = tokio::time::timeout(
            std::time::Duration::from_secs(600),
            stream.message(),
        )
        .await
        .map_err(|_| "agent response timed out".to_string())?
        .map_err(|e| format!("stream failed: {e}"))?;

        let Some(event) = event else {
            break;
        };

        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run_id.to_string(),
            event_type: event.r#type.clone(),
            payload: Some(event.data.clone()),
            sequence,
        })
        .map_err(|e| format!("append_event: {e}"))?;

        sequence += 1;

        if event.r#type == "agent_end" {
            crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
                run_id: run_id.to_string(),
                status: "completed".to_string(),
                error_message: None,
                error_type: None,
            })
            .map_err(|e| format!("update_status: {e}"))?;
            crate::store::clear_run_event_buffer(run_id);
            break;
        }
    }
    Ok(())
}

// ── Session observer (real-time settings-change events) ───────────────────

use tokio::sync::oneshot;
use tauri::Emitter;

/// Handle to the currently-running session observation task.  When a new
/// observation starts, the old one is cancelled via this channel.
static OBSERVER_CANCEL: Mutex<Option<oneshot::Sender<()>>> = Mutex::new(None);

/// Start observing a session's settings changes in the background.  Subscribes
/// to the agent's StreamEvents and forwards settings-change events to the
/// frontend via Tauri `agent-state-updated` events so the UI reflects model /
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

                        // Forward only settings-change events to the frontend.
                        // Include session_id and type so the frontend can
                        // find the right thread and disambiguate fields.
                        let is_settings_event = matches!(
                            event.r#type.as_str(),
                            "model_changed"
                                | "thinking_level_changed"
                                | "permission_level_changed"
                                | "session_name_changed"
                                | "cwd_changed"
                                | "auto_compaction_changed"
                                | "tools_changed"
                                | "sandbox_policy_changed"
                                | "config_reloaded"
                        );
                        if is_settings_event {
                            if let Ok(mut payload) = serde_json::from_str::<serde_json::Value>(&event.data) {
                                if let serde_json::Value::Object(ref mut map) = payload {
                                    map.insert("sessionId".to_string(),
                                        serde_json::Value::String(session_id.clone()));
                                    map.insert("_eventType".to_string(),
                                        serde_json::Value::String(event.r#type.clone()));
                                }
                                let _ = app_handle.emit("agent-state-updated", &payload);
                            }
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
