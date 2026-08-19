mod approval;
mod client;
pub(crate) mod config;
mod config_observer;
mod headless;
mod import;
mod models;
mod observer;
mod persist;
mod replica;
mod review;
mod run_control;
mod session;
mod skills;
mod stream;
#[cfg(test)]
mod test_support;

pub use self::approval::{decide_approval, inject_session_rule, reconcile_pending_approvals};
pub(crate) use self::client::raw_agent_addr;
pub use self::client::{
    connect_agent, delete_session_command, get_available_models_command, get_run_state_command,
    get_session_entries_command, get_state_command, list_streaming_sessions_command, map_rpc_error,
    prune_run_events_command, set_default_model_command, set_model_command,
    set_session_name_command, set_thinking_level_command, RpcResponseExt,
};
pub use self::config_observer::spawn_provider_config_observer;
pub use self::headless::{
    prepare_prompt_persisted_with_trigger, run_prepared_prompt, PreparedPrompt,
};
pub(crate) use self::import::{import_missing_sessions, list_agent_session_ids};
pub use self::models::{list_agent_models, list_builtin_providers, AgentModelOption};
pub use self::observer::{
    drop_observer, ensure_observer_for_thread, seed_observers_from_store, spawn_session_discovery,
};
pub use self::run_control::abort_run;
pub(crate) use self::run_control::{abort_session, wait_for_agent_idle};
pub use self::session::fork_agent_session;
pub use self::skills::{list_installed_skills, refresh_skills, InstalledSkill};
#[cfg(test)]
pub use review::capture_before;
#[cfg(test)]
pub use review::finalize_after;
pub use review::retry as retry_run_review;

use serde::Serialize;

pub use self::client::AttachmentInput;
use self::client::{base_command, prompt_command};
use self::replica::AGENT_REPLICAS;
use self::run_control::{mark_run_completed_if_active, mark_run_failed_if_active};
pub(crate) use self::session::workspace_path_for_thread;
use self::session::{ensure_agent_session, set_agent_permission_level, set_agent_sandbox_policy};

/// Deliver locally tombstoned session deletions after the Agent becomes
/// reachable. A delete is idempotent; `session not found` is success too.
pub async fn reconcile_delete_outbox() {
    let Ok(session_ids) = crate::store::pending_agent_session_deletes() else {
        return;
    };
    for session_id in session_ids {
        let result = async {
            let mut client = connect_agent().await?;
            let response = client
                .execute_command(delete_session_command(session_id.clone()))
                .await
                .map_err(|status| map_rpc_error("Agent delete delivery failed", status))?;
            let response = response.into_inner();
            if response.success || response.error.contains("session not found") {
                Ok(())
            } else {
                Err(crate::AppError::Message(response.error))
            }
        }
        .await;
        match result {
            Ok(()) => {
                let _ = crate::store::acknowledge_agent_session_delete(&session_id);
            }
            Err(error) => {
                let _ = crate::store::note_agent_session_delete_failure(
                    &session_id,
                    &error.to_string(),
                );
            }
        }
    }
}

/// Keep retrying durable deletion intent for as long as this GUI process is
/// alive. Startup-only delivery loses convergence whenever the sidecar starts
/// late or an Agent refuses deletion while a run is draining.
pub fn spawn_delete_outbox_worker() {
    tauri::async_runtime::spawn(async {
        loop {
            reconcile_delete_outbox().await;
            #[cfg(test)]
            if TEST_OUTBOX_STOP.swap(false, std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(delete_outbox_interval()).await;
        }
    });
}

#[cfg(test)]
static TEST_OUTBOX_STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Outbox retry interval; tests shrink it via env (a cfg(test)-only seam).
fn delete_outbox_interval() -> std::time::Duration {
    #[cfg(test)]
    if let Some(ms) = std::env::var("FUTURE_TEST_OUTBOX_INTERVAL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        return std::time::Duration::from_millis(ms);
    }
    std::time::Duration::from_secs(5)
}

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

/// Complete input for one prompt crossing the desktop-to-agent boundary.
/// Keeping the user-authored text and its model-only sidecar together prevents
/// bridge layers from growing parallel positional parameters as prompt metadata
/// evolves.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPromptRequest {
    pub message: String,
    #[serde(default)]
    pub model_context: String,
    pub attachments: Option<Vec<AttachmentInput>>,
    pub thread_id: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub model_id: Option<String>,
    pub thinking_level: Option<String>,
}

/// Events requested per get_events_since page. A long run's journal far
/// exceeds the gRPC message cap when returned whole (every event crosses the
/// wire about three times under the typed dual-write), so full-tail reads
/// page through it. The server additionally bounds a page by a
/// serialized-size budget, keeping pages safe even for runs with multi-MB
/// tool outputs.
const EVENTS_PAGE_SIZE: i64 = 50_000;

/// Cursor advance for the get_events_since page loop: `Some(next)` to keep
/// paging from `next`, `None` when the tail is complete. Terminates on a
/// malformed has_more page (no advancing idx) instead of re-requesting the
/// same cursor forever.
fn next_events_cursor(page: &serde_json::Value, cursor: i64) -> Option<i64> {
    let has_more = page
        .get("hasMore")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !has_more {
        return None;
    }
    let last_idx = page
        .get("events")
        .and_then(|value| value.as_array())
        .and_then(|events| events.last())
        .and_then(|event| event.get("idx"))
        .and_then(|value| value.as_i64());
    last_idx.filter(|idx| *idx > cursor)
}

/// Fetch the agent's buffered events for a session's current run (P1c backfill).
/// `since_idx = -1` returns the requested run's retained prefix. A stale or
/// unknown `run_id` is an explicit error and never realigns to another run.
/// Returns the parsed `data`
/// JSON — shape `{ runId, events: [{ type, data, runId, idx }] }`. Lets a phone /
/// web client that joined an in-flight run mid-stream reconstruct the prefix it
/// missed, keyed by the same `runId`/`idx` the live events carry (so it dedupes).
///
/// Paged under the hood (`max_events`): the returned envelope holds the
/// complete tail regardless of journal size.
pub async fn get_events_since(
    session_id: String,
    run_id: String,
    since_idx: i64,
) -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let mut cursor = since_idx;
    let mut merged: Option<serde_json::Value> = None;
    loop {
        let command = crate::agent_proto::RpcCommand {
            run_id: run_id.clone(),
            since_idx: cursor,
            max_events: EVENTS_PAGE_SIZE,
            ..base_command("get_events_since", session_id.clone())
        };
        let response = client
            .execute_command(command)
            .await
            .map_err(|status| {
                // OutOfRange here is the agent rejecting its own oversized
                // response at the 32 MiB encode cap: it serialized the whole
                // tail even though this client always requests paged reads, so
                // the running agent almost certainly predates the
                // get_events_since paging protocol (proto max_events). Paging
                // is server-side — only an agent restart on a current build
                // fixes it. (A single event over ~10 MiB would trip the same
                // cap even on a current agent.)
                if status.code() == tonic::Code::OutOfRange {
                    format!(
                        "get_events_since failed: {status} — the agent exceeded the 32 MiB gRPC cap on a paged read, so the running agent likely predates get_events_since paging (max_events); rebuild and restart the agent"
                    )
                } else {
                    format!("get_events_since failed: {status}")
                }
            })?
            .into_inner()
            .ok_or_rpc_error("get_events_since returned an error")?;
        let page = if response.data.is_empty() {
            serde_json::json!({ "events": [] })
        } else {
            future_rpc::decode::response_data(&response)
        };
        let next = next_events_cursor(&page, cursor);
        match &mut merged {
            None => merged = Some(page),
            Some(total) => {
                if let (Some(total_events), Some(page_events)) = (
                    total
                        .get_mut("events")
                        .and_then(|value| value.as_array_mut()),
                    page.get("events").and_then(|value| value.as_array()),
                ) {
                    total_events.extend(page_events.iter().cloned());
                }
            }
        }
        match next {
            Some(next_cursor) => cursor = next_cursor,
            None => break,
        }
    }
    let mut result = merged.unwrap_or_else(|| serde_json::json!({ "events": [] }));
    // The merged envelope describes the complete tail, not one page.
    if let Some(object) = result.as_object_mut() {
        object.remove("hasMore");
    }
    Ok(result)
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
        Ok(future_rpc::decode::response_data(&response))
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
        Ok(future_rpc::decode::response_data(&response))
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
        Ok(future_rpc::decode::response_data(&response))
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
        Ok(future_rpc::decode::response_data(&response))
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

/// Persist the onboarding model-picker's choice as the agent's global default
/// model (sessionless `set_default_model` RPC → settings.json `defaultModel`).
pub async fn set_default_model(model_id: String) -> Result<(), crate::AppError> {
    let mut client = connect_agent().await?;
    client
        .execute_command(set_default_model_command(model_id))
        .await
        .map_err(|status| format!("set_default_model failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("set_default_model returned an error")?;
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
/// session's in-memory API key.
///
/// Since audit item 2 this is the FALLBACK-ONLY refresher: the primary config
/// writes go through `agent_bridge::config` (set_auth / upsert_provider /
/// delete_provider), and the agent refreshes its own live sessions inline. This
/// `reload_auth` round-trip is still sent after a LOCAL file write — used when
/// the agent is unreachable or pre-item-2 — because the agent caches the
/// resolved key inside each session's provider and the prompt path never
/// re-reads `auth.json`, so without it a session keeps serving prompts with a
/// stale key (e.g. still answering after logout) while the model list — which
/// does re-read disk — already shows logged-out.
///
/// Best-effort: if the agent isn't running there's no in-memory state to
/// refresh, so an unavailable agent is treated as success.
#[cfg(test)]
pub async fn reload_agent_credentials() -> Result<(), crate::AppError> {
    let mut client = match connect_agent().await {
        Ok(client) => client,
        // connect_agent's only error kind is AgentUnavailable (every failure
        // it constructs is one), and a down agent has no in-memory state to
        // refresh — treated as success.
        Err(_) => return Ok(()),
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

/// Result of [`sync_future_models`]: whether the platform fetch populated the
/// agent's model cache, and the total model count in the rebuilt registry.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncFutureModelsResult {
    pub synced: bool,
    pub model_count: usize,
}

/// Post-login initialization: ask the agent to synchronously fetch the Future
/// provider's models (warming its cache) and rebuild its model registry, so the
/// next [`list_agent_models`] returns a complete list. Blocks on the platform
/// fetch inside the agent — only called once from the onboarding init flow.
/// Best-effort like [`reload_agent_credentials`]: an unavailable agent yields a
/// zeroed result rather than an error.
pub async fn sync_future_models() -> Result<SyncFutureModelsResult, crate::AppError> {
    let mut client = match connect_agent().await {
        Ok(client) => client,
        // connect_agent's only error kind is AgentUnavailable: an unavailable
        // agent yields a zeroed result rather than an error.
        Err(_) => {
            return Ok(SyncFutureModelsResult {
                model_count: 0,
                synced: false,
            });
        }
    };
    let response = client
        .execute_command(base_command("sync_future_models", String::new()))
        .await
        .map_err(|status| map_rpc_error("Unable to sync Future Agent models", status))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the model sync.")?;
    serde_json::from_value::<SyncFutureModelsResult>(future_rpc::decode::response_data(&response))
        .map_err(|error| format!("Future Agent returned invalid sync result: {error}").into())
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
    agent_prompt_with_model_context(AgentPromptRequest {
        message,
        model_context: String::new(),
        attachments,
        thread_id,
        session_id,
        run_id,
        model_id,
        thinking_level,
    })
    .await
}

pub async fn agent_prompt_with_model_context(
    request: AgentPromptRequest,
) -> Result<AgentPromptResponse, crate::AppError> {
    let effective_session_id = request
        .session_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| request.thread_id.clone());
    let result = agent_prompt_inner(request.clone()).await;

    // Settle the run row HERE, in the backend, not only in the frontend
    // pipeline: the pipeline's status write depends on this invoke response
    // reaching a webview that the OS may have suspended (hidden/occluded
    // window), after which the row would stay `running` forever — the sidebar
    // spinner and the composer's "already running" guard both read this row.
    // Every writer is compare-and-set, so a concurrent user abort (`cancelled`)
    // always wins and is preserved; the frontend's later write becomes a no-op
    // echo. Settling before `capture_after` also means a slow/hung git snapshot
    // can no longer wedge the run's visible state.
    match &result {
        Ok(response) if response.complete => {
            mark_run_completed_if_active(request.run_id.as_deref());
        }
        Ok(_) => {
            mark_run_failed_if_active(
                request.run_id.as_deref(),
                "Future Agent response ended before a clean terminal.",
            );
        }
        Err(error) => mark_run_failed_if_active(request.run_id.as_deref(), &error.to_string()),
    }

    if let Some(run_id) = request.run_id.clone() {
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
            let capture_thread = request.thread_id.clone();
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
            let materialize_thread = request.thread_id.clone();
            let materialize_run = run_id.clone();
            let _ = tokio::task::spawn_blocking(move || {
                review::materialize_changeset(&materialize_thread, &materialize_run, sensitive);
            })
            .await;
            crate::emit_review_updated(&request.thread_id);
        });
    }

    result
}

async fn agent_prompt_inner(
    request: AgentPromptRequest,
) -> Result<AgentPromptResponse, crate::AppError> {
    let AgentPromptRequest {
        message,
        model_context,
        attachments,
        thread_id,
        session_id,
        run_id,
        model_id,
        thinking_level,
    } = request;
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

    // Create (or reuse) the agent session.
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

    // Persist the agent-generated session id for new threads.
    if session_id != stored_session_id {
        let _ = crate::store::update_thread_session_id(&thread_id, &session_id);
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

    // Resolve the run identity and register single-writer ownership BEFORE the
    // prompt reaches the agent. The session observer subscribes first (it is
    // the sole NATS publisher and the fallback event projector, so it must
    // exist before the run starts); with the lease already held it recognizes
    // this run as pipeline-owned from its very first event — closing the ack
    // window where an early event (e.g. `user_message`) could otherwise be
    // persisted by both the observer and this collector.
    let run_id = match run_id.filter(|id| !id.trim().is_empty()) {
        Some(id) => id,
        None => crate::store::create_id("run"),
    };
    let replica_lease = AGENT_REPLICAS
        .acquire(&run_id)
        .map_err(crate::AppError::from)?;
    observer::ensure_observer_for_thread(&session_id, &thread_id).map_err(crate::AppError::from)?;

    let prompt_response = command_client
        .execute_command(prompt_command(
            message,
            model_context,
            session_id.clone(),
            attachments.unwrap_or_default(),
            Some(run_id.clone()),
        ))
        .await
        .map_err(|error| format!("Unable to send prompt to Future Agent: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the prompt.")?;

    let prompt_ack: serde_json::Value = future_rpc::decode::response_data(&prompt_response);
    let canonical_run_id = prompt_ack
        .get("run_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Future Agent prompt acknowledgement omitted run_id.".to_string())?
        .to_string();
    if canonical_run_id != run_id {
        return Err(
            format!("Future Agent adopted run id {canonical_run_id}, expected {run_id}").into(),
        );
    }

    match replica_lease
        .collect(Some(&run_id), &canonical_run_id, &session_id, &thread_id)
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
        Err(stream::CollectError::RunGone(reason)) => {
            // The Agent accepted the prompt (ack) but no longer has the run by
            // attach time — a restart or rollover in the ack→attach window. The
            // run is gone, so an abort would be a no-op; reconcile the local row
            // from the journal instead and surface an error (we have no streamed
            // content to show). local == canonical for a GUI-originated prompt.
            if let Err(reconcile_error) =
                reconcile_run_gone(&canonical_run_id, &canonical_run_id, &session_id, &reason).await
            {
                return Err(format!(
                    "Future Agent run ended before the stream attached: {reason}; \
                     terminal reconciliation failed: {reconcile_error}"
                )
                .into());
            }
            Err(format!("Future Agent run ended before the stream attached: {reason}").into())
        }
        Err(stream::CollectError::App(error)) => {
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
        let thread_id = run.thread_id;
        match check_and_reanimate_run(&session_id, &run_id, &thread_id).await {
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
async fn check_and_reanimate_run(
    session_id: &str,
    run_id: &str,
    thread_id: &str,
) -> Result<(), String> {
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
    let state_value = future_rpc::decode::response_data(&state);
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
        // CAS: only reanimate while still in the interrupted state. If the user
        // already aborted (or another path settled it) between listing and now,
        // the guard matches zero rows and we must NOT reattach against a run
        // whose terminal state would race the projection.
        let reanimated =
            crate::store::reanimate_run(run_id).map_err(|e| format!("reanimate: {e}"))?;
        if !reanimated {
            eprintln!(
                "FutureOS startup reconcile: run {run_id} no longer interrupted; skipping reattach"
            );
            return Ok(());
        }
        // The session observer takes over from the local cursor: it projects
        // the remaining events (this run has no pipeline owner), mirrors them
        // to NATS, and settles the row at agent_end.
        observer::ensure_observer_for_thread(session_id, thread_id)?;
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

/// The Agent returned RunGone (`failed_precondition` / `not_found`) for
/// `canonical_run_id` on attach: it no longer recognizes the run. Settle the
/// local `local_run_id` row from the Agent's authoritative journal instead of
/// leaving it stranded as `running` or guessing `failed`.
///
/// The Agent is reachable (RunGone is a response, not a connect failure), so
/// `get_state` answers with `activeRun` / `interruptedRun` / `requestedRun`:
/// - still active (attach raced a `start_run`): leave running, let the live path
///   converge — do not mark failed;
/// - a durable terminal marker (`requestedRun`): mirror its exact state;
/// - confirmed interrupted-by-restart: settle cancelled/interrupted;
/// - no marker at all: the run is truly gone, settle failed so the UI frees up.
///
/// Every write goes through the compare-and-set writers, so a concurrent user
/// abort (cancelled) always wins.
async fn reconcile_run_gone(
    local_run_id: &str,
    canonical_run_id: &str,
    session_id: &str,
    reason: &str,
) -> Result<(), String> {
    let mut client = connect_agent()
        .await
        .map_err(|e| format!("reconcile connect: {e}"))?;
    let state = client
        .execute_command(get_run_state_command(
            session_id.to_string(),
            canonical_run_id.to_string(),
        ))
        .await
        .map_err(|e| format!("reconcile get_state: {e}"))?
        .into_inner();
    let state_value = if state.success {
        future_rpc::decode::response_data(&state)
    } else {
        serde_json::Value::Null
    };

    let active = state_value
        .get("activeRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str());
    if active == Some(canonical_run_id) {
        eprintln!(
            "FutureOS run {local_run_id} still active on Agent after RunGone ({reason}); leaving running"
        );
        return Ok(());
    }

    if let Some(terminal) = state_value
        .get("requestedRun")
        .filter(|value| value.is_object())
    {
        let agent_state = terminal
            .get("state")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let error = terminal.get("error").and_then(|value| value.as_str());
        settle_from_agent_terminal(local_run_id, agent_state, error)
            .map_err(|error| format!("reconcile terminal status: {error}"))?;
        return Ok(());
    }

    let interrupted = state_value
        .get("interruptedRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str());
    if interrupted == Some(canonical_run_id) {
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: local_run_id.to_string(),
            status: "cancelled".to_string(),
            error_message: Some("Interrupted because Future Agent restarted.".to_string()),
            error_type: Some("interrupted".to_string()),
        })
        .map_err(|error| format!("reconcile interrupted status: {error}"))?;
        return Ok(());
    }

    // No marker at all — the run is genuinely gone. Settle failed (CAS) so the
    // composer can't strand on a permanent "running".
    crate::store::fail_run_if_active(
        local_run_id,
        &format!("Future Agent run no longer active: {reason}"),
        "stream_interrupted",
    )
    .map_err(|error| format!("reconcile missing run: {error}"))?;
    Ok(())
}

/// CAS-mirror the Agent journal's durable terminal marker onto a local run row.
/// Shared by attach-time RunGone reconciliation and the runtime watchdog.
/// Unknown states fall back to a generic failure — a row is never asserted
/// `completed` without positive evidence.
fn settle_from_agent_terminal(
    local_run_id: &str,
    agent_state: &str,
    error: Option<&str>,
) -> Result<(), crate::AppError> {
    let (status, error_type, default_message) = match agent_state {
        "completed" => ("completed", None, None),
        "cancelled" => ("cancelled", Some("cancelled"), Some("Run was cancelled.")),
        "error" => (
            "failed",
            Some("agent_error"),
            Some("Future Agent run failed."),
        ),
        _ => (
            "failed",
            Some("stream_interrupted"),
            Some("Future Agent response ended before a clean terminal."),
        ),
    };
    crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
        run_id: local_run_id.to_string(),
        status: status.to_string(),
        error_message: error
            .map(str::to_string)
            .or_else(|| default_message.map(str::to_string)),
        error_type: error_type.map(str::to_string),
    })
    .map(|_| ())
}

// ── Live run watchdog ─────────────────────────────────────────────────────

/// Seconds between watchdog passes.
const WATCHDOG_INTERVAL_SECS: u64 = 30;
/// A run younger than this is never inspected: the row is created before the
/// Agent acknowledges the prompt (and before the replica lease is acquired),
/// so a young row can legitimately have no Agent marker and no collector yet.
/// Must comfortably exceed the slowest prompt setup (session ensure, model
/// setup, the `capture_before` git fork), or the watchdog could fail a run
/// whose prompt is still being prepared.
const WATCHDOG_GRACE_SECS: u64 = 45;
/// A row the Agent has no marker for at all is only settled as orphaned once
/// it is this old. Below the threshold it is skipped — same startup-window
/// hazard as the grace period, just for rows that survived several passes.
const WATCHDOG_ORPHAN_SECS: u64 = 600;

/// What the watchdog should do with a non-terminal run, given the Agent's
/// authoritative view of it. Pure (no IO) — [`reconcile_active_run_once`]
/// applies it.
#[derive(Debug, PartialEq)]
enum ActiveRunAction {
    /// The Agent is still executing this exact run. Reattach a collector if no
    /// live collector owns the replica lease (a failed acquire means healthy).
    Attach,
    /// The Agent's journal has a durable terminal marker for this run: mirror
    /// its exact state onto the row.
    SettleTerminal {
        agent_state: String,
        error: Option<String>,
    },
    /// The Agent confirms this run began but never committed (restart).
    SettleInterrupted,
    /// The Agent has no marker at all and the row is past the orphan age.
    SettleOrphaned,
    /// Healthy, or not enough evidence yet — leave the row untouched.
    Skip,
}

/// Decide the watchdog action for one non-terminal run from the Agent's
/// `get_run_state` payload. Mirrors the marker precedence of
/// [`reconcile_run_gone`]: live active run first, then the durable terminal
/// marker, then the interrupted-by-restart marker, then age-based orphaning.
fn plan_active_run_reconciliation(
    state: &serde_json::Value,
    canonical_run_id: &str,
    age_secs: u64,
) -> ActiveRunAction {
    let is_streaming = state
        .get("isStreaming")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let active_run_id = state
        .get("activeRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str());
    if is_streaming && active_run_id == Some(canonical_run_id) {
        return ActiveRunAction::Attach;
    }
    if let Some(terminal) = state.get("requestedRun").filter(|value| value.is_object()) {
        return ActiveRunAction::SettleTerminal {
            agent_state: terminal
                .get("state")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            error: terminal
                .get("error")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        };
    }
    let interrupted_run_id = state
        .get("interruptedRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str());
    if interrupted_run_id == Some(canonical_run_id) {
        return ActiveRunAction::SettleInterrupted;
    }
    if age_secs >= WATCHDOG_ORPHAN_SECS {
        return ActiveRunAction::SettleOrphaned;
    }
    ActiveRunAction::Skip
}

/// Reconcile one non-terminal run row against the Agent's authoritative state.
async fn reconcile_active_run_once(
    run: &crate::store::ActiveRun,
    canonical_run_id: &str,
    age_secs: u64,
) -> Result<(), String> {
    let mut client = connect_agent().await.map_err(|e| format!("connect: {e}"))?;
    // Query by the canonical id (not the local row id): remote-attached runs
    // carry a synthetic local id the Agent never saw, while its durable
    // journal marker is keyed by the canonical id it adopted. For GUI runs
    // the two are identical.
    let state = client
        .execute_command(get_run_state_command(
            run.session_id.clone(),
            canonical_run_id.to_string(),
        ))
        .await
        .map_err(|e| format!("get_run_state: {e}"))?
        .into_inner();
    if !state.success {
        // The Agent cannot resolve this session — leave the row as is; startup
        // convergence settles genuinely dead rows on the next launch.
        return Ok(());
    }
    let state_value = future_rpc::decode::response_data(&state);
    match plan_active_run_reconciliation(&state_value, canonical_run_id, age_secs) {
        ActiveRunAction::Skip => Ok(()),
        ActiveRunAction::Attach => {
            // The Agent is still streaming this run but no pipeline collector
            // owns it (a crashed collector, a run started by another client,
            // or one reanimated out from under a dead lease). The session
            // observer projects it from its local cursor — idempotent, and
            // never races a pipeline collector (single-writer rule).
            observer::ensure_observer_for_thread(&run.session_id, &run.thread_id)?;
            Ok(())
        }
        ActiveRunAction::SettleTerminal { agent_state, error } => {
            settle_from_agent_terminal(&run.run_id, &agent_state, error.as_deref())
                .map_err(|e| format!("settle terminal: {e}"))
        }
        ActiveRunAction::SettleInterrupted => {
            crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
                run_id: run.run_id.clone(),
                status: "cancelled".to_string(),
                error_message: Some("Interrupted because Future Agent restarted.".to_string()),
                error_type: Some("interrupted".to_string()),
            })
            .map(|_| ())
            .map_err(|e| format!("settle interrupted: {e}"))
        }
        ActiveRunAction::SettleOrphaned => crate::store::fail_run_if_active(
            &run.run_id,
            "Future Agent run is no longer active on the agent.",
            "stream_interrupted",
        )
        .map(|_| ())
        .map_err(|e| format!("settle orphaned: {e}")),
    }
}

/// Launch the runtime watchdog for active runs: a periodic pass reconciling
/// every non-terminal run row against the Agent's authoritative state. The
/// backstop for rows whose owning pipeline/collector never settled them:
///
/// - The webview was suspended (hidden/occluded window) and never applied the
///   `agent_prompt` invoke response, so the frontend's status write never ran
///   — the backend settles these rows itself now, but this pass repairs rows
///   created by older builds and any future settlement gap.
/// - A collector task died or wedged while the run was still streaming
///   Agent-side: the replica lease probe finds no owner and reattaches one,
///   resuming live projection from the last persisted cursor.
/// - The Agent lost the run (restart/rollover) without the GUI noticing: the
///   durable journal marker (or its absence) settles the row.
///
/// All writes go through the compare-and-set writers, so a healthy collector,
/// a concurrent user abort, and the normal pipeline are never clobbered. The
/// pass self-gates on Agent reachability; Agent downtime is left for startup
/// convergence on the next launch.
pub fn spawn_active_run_watchdog() {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(watchdog_interval()).await;
            #[cfg(test)]
            if TEST_WATCHDOG_STOP.swap(false, std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            if connect_agent().await.is_err() {
                continue;
            }
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            active_run_watchdog_pass(now_ms).await;
        }
    });
}

/// One watchdog pass: reconcile every non-terminal run row against the
/// Agent's authoritative state, then the pending approvals. Extracted from
/// the spawn loop so tests can drive it with a synthetic clock (run rows
/// cannot be backdated through the store API).
async fn active_run_watchdog_pass(now_ms: i64) {
    let Ok(active_runs) = crate::store::list_active_runs() else {
        return;
    };
    for run in active_runs {
        let age_secs = now_ms.saturating_sub(run.created_at).max(0) as u64 / 1000;
        if age_secs < WATCHDOG_GRACE_SECS {
            continue;
        }
        let canonical = AGENT_REPLICAS
            .canonical_for_local(&run.run_id)
            .unwrap_or_else(|| run.run_id.clone());
        if let Err(error) = reconcile_active_run_once(&run, &canonical, age_secs).await {
            eprintln!(
                "FutureOS run watchdog could not reconcile {}: {error}",
                run.run_id
            );
        }
    }
    // Approvals outlive their collectors (the Agent stays parked while
    // the GUI restarts), so reconcile them against the Agent's pending
    // set on every tick — not just at startup.
    approval::reconcile_pending_approvals().await;
}

#[cfg(test)]
static TEST_WATCHDOG_STOP: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Watchdog tick interval; tests shrink it via env (a cfg(test)-only seam).
fn watchdog_interval() -> std::time::Duration {
    #[cfg(test)]
    if let Some(ms) = std::env::var("FUTURE_TEST_WATCHDOG_INTERVAL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        return std::time::Duration::from_millis(ms);
    }
    std::time::Duration::from_secs(WATCHDOG_INTERVAL_SECS)
}

// ── Remote-stream attach (cross-client streaming) ─────────────────────────

/// Called when the GUI opens a thread whose agent session is being driven by
/// another client (TUI, CLI, phone). Ensures the session observer is live (it
/// projects the run's events, mirrors them to NATS, and settles the run),
/// then returns the local run row for the agent's active run so the existing
/// reattach machinery picks up live previews immediately.
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
    // approval decision — the observer is already projecting it.
    fn is_active(status: &str) -> bool {
        status == "running" || status == "waiting_approval"
    }
    let existing_runs = crate::store::list_runs(thread_id).unwrap_or_default();
    if let Some(active) = existing_runs.iter().find(|r| is_active(&r.status)) {
        observer::ensure_observer_for_thread(session_id, thread_id)?;
        return Ok(active.id.clone());
    }
    let mut client = connect_agent().await.map_err(|e| format!("connect: {e}"))?;
    let state = client
        .execute_command(get_state_command(session_id.to_string()))
        .await
        .map_err(|e| format!("get_state: {e}"))?
        .into_inner();
    let state_value = future_rpc::decode::response_data(&state);
    let canonical_run_id = state_value
        .get("activeRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Agent session has no active canonical run".to_string())?
        .to_string();

    observer::ensure_observer_for_thread(session_id, thread_id)?;
    // Get-or-create the local row NOW (id == canonical run id) so the frontend
    // gets a concrete run id back instead of waiting for the observer's first
    // event. The observer reuses this row via the same binding path.
    observer::ensure_run_binding(session_id, &canonical_run_id, thread_id)
        .ok_or_else(|| "Unable to create local run for the agent's active run".to_string())
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

#[cfg(test)]
mod watchdog_tests {
    use super::{plan_active_run_reconciliation, ActiveRunAction, WATCHDOG_ORPHAN_SECS};

    fn state(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn attaches_when_agent_still_running_this_run() {
        let action = plan_active_run_reconciliation(
            &state(r#"{"isStreaming": true, "activeRun": {"runId": "run-1"}}"#),
            "run-1",
            120,
        );
        assert_eq!(action, ActiveRunAction::Attach);
    }

    #[test]
    fn does_not_attach_for_a_different_active_run() {
        let action = plan_active_run_reconciliation(
            &state(r#"{"isStreaming": true, "activeRun": {"runId": "run-2"}}"#),
            "run-1",
            120,
        );
        assert_eq!(action, ActiveRunAction::Skip);
    }

    #[test]
    fn mirrors_durable_completed_marker() {
        let action = plan_active_run_reconciliation(
            &state(r#"{"requestedRun": {"state": "completed"}}"#),
            "run-1",
            120,
        );
        assert_eq!(
            action,
            ActiveRunAction::SettleTerminal {
                agent_state: "completed".to_string(),
                error: None,
            }
        );
    }

    #[test]
    fn mirrors_error_marker_with_its_message() {
        let action = plan_active_run_reconciliation(
            &state(r#"{"requestedRun": {"state": "error", "error": "boom"}}"#),
            "run-1",
            120,
        );
        assert_eq!(
            action,
            ActiveRunAction::SettleTerminal {
                agent_state: "error".to_string(),
                error: Some("boom".to_string()),
            }
        );
    }

    #[test]
    fn settles_interrupted_by_restart() {
        let action = plan_active_run_reconciliation(
            &state(r#"{"interruptedRun": {"runId": "run-1"}}"#),
            "run-1",
            120,
        );
        assert_eq!(action, ActiveRunAction::SettleInterrupted);
    }

    #[test]
    fn skips_young_runs_without_markers() {
        let action =
            plan_active_run_reconciliation(&state(r#"{"isStreaming": false}"#), "run-1", 60);
        assert_eq!(action, ActiveRunAction::Skip);
    }

    #[test]
    fn orphans_old_runs_without_markers() {
        let action = plan_active_run_reconciliation(
            &state(r#"{"isStreaming": false}"#),
            "run-1",
            WATCHDOG_ORPHAN_SECS,
        );
        assert_eq!(action, ActiveRunAction::SettleOrphaned);
    }
}

#[cfg(test)]
mod events_paging_tests {
    use super::next_events_cursor;
    use serde_json::json;

    #[test]
    fn stops_when_the_page_reports_no_tail() {
        // hasMore absent (legacy server) or false ends the loop.
        let page = json!({"events": [{"idx": 3}]});
        assert_eq!(next_events_cursor(&page, -1), None);
        let page = json!({"events": [{"idx": 3}], "hasMore": false});
        assert_eq!(next_events_cursor(&page, -1), None);
    }

    #[test]
    fn advances_to_the_last_event_idx_while_has_more() {
        let page = json!({"events": [{"idx": 3}, {"idx": 7}], "hasMore": true});
        assert_eq!(next_events_cursor(&page, -1), Some(7));
        assert_eq!(next_events_cursor(&page, 7), None); // idx must advance
    }

    #[test]
    fn malformed_has_more_pages_terminate_instead_of_looping() {
        // No events, no idx, or a non-advancing idx would re-request the same
        // cursor forever — the loop must bail.
        assert_eq!(next_events_cursor(&json!({"hasMore": true}), -1), None);
        assert_eq!(
            next_events_cursor(&json!({"events": [], "hasMore": true}), -1),
            None
        );
        assert_eq!(
            next_events_cursor(&json!({"events": [{"idx": 5}], "hasMore": true}), 5),
            None
        );
    }
}

#[cfg(test)]
mod wire_decode_tests {
    use future_rpc::proto;

    fn rpc_response(command: &str, data: &str) -> proto::RpcResponse {
        proto::RpcResponse {
            id: "req".to_string(),
            r#type: "response".to_string(),
            command: command.to_string(),
            success: true,
            data: data.to_string(),
            ..Default::default()
        }
    }

    /// The dual-write guarantee as seen from the GUI: a response carrying BOTH
    /// the typed payload and the JSON string decodes to exactly the JSON, so
    /// deep reads behave identically against old and new agents.
    #[test]
    fn typed_and_data_decode_to_the_same_value() {
        let data = r#"{"models":[{"id":"m","label":"M","provider":"p","supportsImages":false,"thinkingLevel":"off","contextWindow":1,"isDefault":true,"description":null,"descriptionEn":null,"recommended":false}],"defaultModel":"m","isScoped":false}"#;
        let payload = future_rpc::encode::response_payload("list_models", &data_value(data))
            .expect("list_models encodes");
        let mut resp = rpc_response("list_models", data);
        resp.payload = Some(payload);
        assert_eq!(
            future_rpc::decode::response_data(&resp),
            data_value(data),
            "typed decode must match the dual-written JSON"
        );
    }

    /// Old agent (no typed payload) still decodes through the JSON fallback.
    #[test]
    fn data_only_falls_back_to_json() {
        let data = r#"{"sessionId":"s1"}"#;
        let resp = rpc_response("get_state", data);
        assert_eq!(future_rpc::decode::response_data(&resp), data_value(data));
    }

    /// Event byte-stability during the migration window: while the agent
    /// dual-writes `data`, the canonical event payload is the original string
    /// verbatim — persistence and the NATS mirror must not drift.
    #[test]
    fn event_payload_prefers_dual_written_data() {
        let data = r#"{"type":"tool_end","tool_id":"c1","text":"ok"}"#.to_string();
        let payload = future_rpc::encode::event_payload("tool_end", &data);
        let event = proto::StreamEvent {
            r#type: "tool_end".to_string(),
            data: data.clone(),
            payload,
            ..Default::default()
        };
        assert_eq!(future_rpc::decode::event_data_json(&event), data);
    }

    /// Once `data` is retired, the typed payload reconstructs the canonical
    /// shape (the wire JSON minus the redundant injected `type` key).
    #[test]
    fn typed_event_reconstructs_without_data() {
        let data = r#"{"type":"tool_end","tool_id":"c1","text":"ok"}"#;
        let payload = future_rpc::encode::event_payload("tool_end", data).expect("encodes");
        let event = proto::StreamEvent {
            r#type: "tool_end".to_string(),
            data: String::new(),
            payload: Some(payload),
            ..Default::default()
        };
        let reconstructed: serde_json::Value =
            serde_json::from_str(&future_rpc::decode::event_data_json(&event)).unwrap();
        assert_eq!(
            reconstructed,
            serde_json::json!({ "text": "ok", "tool_id": "c1" })
        );
    }

    fn data_value(data: &str) -> serde_json::Value {
        serde_json::from_str(data).unwrap()
    }
}

#[cfg(test)]
mod bridge_tests {
    use super::test_support::{
        break_home, mock_agent, restore_home, seed_run, seed_thread, seed_workspace, Reply,
        TestHome,
    };
    use super::*;

    // ── simple command wrappers ───────────────────────────────────────

    #[tokio::test]
    async fn session_read_wrappers_decode_or_default() {
        let mock = mock_agent();

        mock.push_data(
            "get_messages",
            serde_json::json!({"messages": [{"role": "user"}]}),
        );
        let value = get_session_messages("sess-1".to_string())
            .await
            .expect("messages");
        assert_eq!(value["messages"][0]["role"], "user");

        // Empty data payloads fall back to empty envelopes.
        mock.push("get_messages", Reply::Data(String::new()));
        let value = get_session_messages("sess-1".to_string())
            .await
            .expect("messages");
        assert_eq!(value, serde_json::json!({"messages": []}));

        mock.push_data(
            "get_session_entries",
            serde_json::json!({"entries": [{"id": "e1"}]}),
        );
        let value = get_session_entries("sess-1".to_string())
            .await
            .expect("entries");
        assert_eq!(value["entries"][0]["id"], "e1");
        mock.push("get_session_entries", Reply::Data(String::new()));
        let value = get_session_entries("sess-1".to_string())
            .await
            .expect("entries");
        assert_eq!(value, serde_json::json!({"entries": []}));

        mock.push_data("get_state", serde_json::json!({"isStreaming": true}));
        let value = get_session_state("sess-1".to_string())
            .await
            .expect("state");
        assert_eq!(value["isStreaming"], true);
        mock.push("get_state", Reply::Data(String::new()));
        let value = get_session_state("sess-1".to_string())
            .await
            .expect("state");
        assert_eq!(value, serde_json::json!({}));

        mock.push_data("list_models", serde_json::json!({"models": [{"id": "m"}]}));
        let value = get_available_models().await.expect("models");
        assert_eq!(value["models"][0]["id"], "m");
        mock.push("list_models", Reply::Data(String::new()));
        let value = get_available_models().await.expect("models");
        assert_eq!(value, serde_json::json!({"models": []}));
    }

    #[tokio::test]
    async fn session_read_wrappers_surface_failures() {
        let mock = mock_agent();

        mock.push("get_messages", Reply::Status(tonic::Code::Internal, "boom"));
        let error = get_session_messages("s".to_string())
            .await
            .expect_err("transport");
        assert!(error.to_string().contains("get_messages failed"), "{error}");
        mock.push("get_messages", Reply::Reject("bad".to_string()));
        let error = get_session_messages("s".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad");

        mock.push(
            "get_session_entries",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = get_session_entries("s".to_string())
            .await
            .expect_err("transport");
        assert!(
            error.to_string().contains("get_session_entries failed"),
            "{error}"
        );
        mock.push("get_session_entries", Reply::Reject(String::new()));
        let error = get_session_entries("s".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "get_session_entries returned an error");

        mock.push("get_state", Reply::Status(tonic::Code::Internal, "boom"));
        let error = get_session_state("s".to_string())
            .await
            .expect_err("transport");
        assert!(error.to_string().contains("get_state failed"), "{error}");
        mock.push("get_state", Reply::Reject("bad".to_string()));
        let error = get_session_state("s".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad");

        mock.push("list_models", Reply::Status(tonic::Code::Internal, "boom"));
        let error = get_available_models().await.expect_err("transport");
        assert!(
            error.to_string().contains("get_available_models failed"),
            "{error}"
        );
        mock.push("list_models", Reply::Reject("bad".to_string()));
        let error = get_available_models().await.expect_err("reject");
        assert_eq!(error.to_string(), "bad");
    }

    #[tokio::test]
    async fn session_setter_wrappers() {
        let mock = mock_agent();

        mock.push("set_model", Reply::Data("{}".to_string()));
        set_session_model("sess-1".to_string(), "future/k3".to_string())
            .await
            .expect("set model");
        let request = &mock.requests_of("set_model")[0];
        assert_eq!(request.model_id, "future/k3");
        assert_eq!(request.session_id, "sess-1");
        mock.push("set_model", Reply::Status(tonic::Code::Internal, "boom"));
        let error = set_session_model("s".to_string(), "m".to_string())
            .await
            .expect_err("transport");
        assert!(error.to_string().contains("set_model failed"), "{error}");
        mock.push("set_model", Reply::Reject("bad".to_string()));
        let error = set_session_model("s".to_string(), "m".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad");

        mock.push("set_default_model", Reply::Data("{}".to_string()));
        set_default_model("future/k3".to_string())
            .await
            .expect("default");
        assert_eq!(
            mock.requests_of("set_default_model")[0].model_id,
            "future/k3"
        );
        mock.push(
            "set_default_model",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = set_default_model("m".to_string())
            .await
            .expect_err("transport");
        assert!(
            error.to_string().contains("set_default_model failed"),
            "{error}"
        );
        mock.push("set_default_model", Reply::Reject("bad".to_string()));
        let error = set_default_model("m".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad");

        mock.push("set_thinking_level", Reply::Data("{}".to_string()));
        set_session_thinking_level("sess-1".to_string(), "high".to_string())
            .await
            .expect("thinking");
        assert_eq!(mock.requests_of("set_thinking_level")[0].level, "high");
        mock.push(
            "set_thinking_level",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = set_session_thinking_level("s".to_string(), "l".to_string())
            .await
            .expect_err("transport");
        assert!(
            error.to_string().contains("set_thinking_level failed"),
            "{error}"
        );
        mock.push("set_thinking_level", Reply::Reject("bad".to_string()));
        let error = set_session_thinking_level("s".to_string(), "l".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad");
    }

    #[tokio::test]
    async fn rename_session_mirrors_into_the_store() {
        let home = TestHome::new("bridge-rename");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        mock.push("set_session_name", Reply::Data("{}".to_string()));
        rename_session("sess-1".to_string(), "New Title".to_string())
            .await
            .expect("rename");
        assert_eq!(mock.requests_of("set_session_name")[0].name, "New Title");
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .title,
            "New Title",
            "store mirror keeps the sidebar in sync"
        );

        // Unknown session: the agent call still succeeds, no mirror.
        mock.push("set_session_name", Reply::Data("{}".to_string()));
        rename_session("sess-unknown".to_string(), "T".to_string())
            .await
            .expect("rename");

        mock.push(
            "set_session_name",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = rename_session("sess-1".to_string(), "T".to_string())
            .await
            .expect_err("transport");
        assert!(
            error.to_string().contains("set_session_name failed"),
            "{error}"
        );
        mock.push("set_session_name", Reply::Reject("bad".to_string()));
        let error = rename_session("sess-1".to_string(), "T".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad");
    }

    #[tokio::test]
    async fn reload_agent_credentials_tolerates_a_down_agent() {
        let mock = mock_agent();

        mock.push("reload_auth", Reply::Data("{}".to_string()));
        reload_agent_credentials().await.expect("reload");
        assert_eq!(mock.requests_of("reload_auth").len(), 1);

        // Unreachable agent (unparseable endpoint) → Ok.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        reload_agent_credentials().await.expect("down is ok");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);

        // Transport failure surfaces via map_rpc_error; rejection via the body.
        mock.push("reload_auth", Reply::Status(tonic::Code::Internal, "boom"));
        let error = reload_agent_credentials().await.expect_err("transport");
        assert!(matches!(error, crate::AppError::Message(_)), "{error}");
        mock.push("reload_auth", Reply::Reject("bad".to_string()));
        let error = reload_agent_credentials().await.expect_err("reject");
        assert_eq!(error.to_string(), "bad");
    }

    #[tokio::test]
    async fn sync_future_models_variants() {
        let mock = mock_agent();

        mock.push_data(
            "sync_future_models",
            serde_json::json!({"synced": true, "modelCount": 7}),
        );
        let result = sync_future_models().await.expect("sync");
        assert!(result.synced);
        assert_eq!(result.model_count, 7);

        // Down agent → zeroed result.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let result = sync_future_models().await.expect("down is zeroed");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        assert!(!result.synced);
        assert_eq!(result.model_count, 0);

        mock.push(
            "sync_future_models",
            Reply::Status(tonic::Code::Unavailable, "gone"),
        );
        let error = sync_future_models().await.expect_err("transport");
        assert!(
            matches!(error, crate::AppError::AgentUnavailable(_)),
            "{error}"
        );
        mock.push("sync_future_models", Reply::Reject("bad".to_string()));
        let error = sync_future_models().await.expect_err("reject");
        assert_eq!(error.to_string(), "bad");
        mock.push_data("sync_future_models", serde_json::json!({"synced": "yes"}));
        let error = sync_future_models().await.expect_err("invalid");
        assert!(error.to_string().contains("invalid sync result"), "{error}");
    }

    // ── get_events_since paging ───────────────────────────────────────

    #[tokio::test]
    async fn events_since_merges_pages_and_strips_has_more() {
        let mock = mock_agent();
        mock.push_data(
            "get_events_since",
            serde_json::json!({"events": [{"idx": 1}, {"idx": 2}], "hasMore": true}),
        );
        mock.push_data(
            "get_events_since",
            serde_json::json!({"events": [{"idx": 3}], "hasMore": false}),
        );
        let merged = get_events_since("sess-1".to_string(), "run-1".to_string(), -1)
            .await
            .expect("merged");
        let idxs: Vec<i64> = merged["events"]
            .as_array()
            .expect("array")
            .iter()
            .map(|event| event["idx"].as_i64().unwrap_or_default())
            .collect();
        assert_eq!(idxs, vec![1, 2, 3]);
        assert!(
            merged.get("hasMore").is_none(),
            "merged envelope drops hasMore"
        );

        let requests = mock.requests_of("get_events_since");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].since_idx, -1);
        assert_eq!(requests[0].max_events, EVENTS_PAGE_SIZE);
        assert_eq!(requests[1].since_idx, 2, "paging resumes at the last idx");
        assert_eq!(requests[0].run_id, "run-1");
    }

    #[tokio::test]
    async fn events_since_empty_and_terminating_pages() {
        let mock = mock_agent();

        // Empty data payload → empty envelope.
        mock.push("get_events_since", Reply::Data(String::new()));
        let merged = get_events_since("s".to_string(), "r".to_string(), 0)
            .await
            .expect("empty");
        assert_eq!(merged, serde_json::json!({"events": []}));

        // A hasMore page whose idx does not advance terminates the loop.
        mock.push_data(
            "get_events_since",
            serde_json::json!({"events": [{"idx": 5}], "hasMore": true}),
        );
        let merged = get_events_since("s".to_string(), "r".to_string(), 5)
            .await
            .expect("terminates");
        assert_eq!(merged["events"].as_array().expect("array").len(), 1);

        // Transport failure and rejection.
        mock.push(
            "get_events_since",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = get_events_since("s".to_string(), "r".to_string(), 0)
            .await
            .expect_err("transport");
        assert!(
            error.to_string().contains("get_events_since failed"),
            "{error}"
        );
        mock.push("get_events_since", Reply::Reject("stale run".to_string()));
        let error = get_events_since("s".to_string(), "r".to_string(), 0)
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "stale run");
    }

    // ── provision_agent_session ───────────────────────────────────────

    #[tokio::test]
    async fn provision_creates_and_records_the_agent_session() {
        let home = TestHome::new("bridge-provision");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, None);

        mock.push_data("new_session", serde_json::json!({"sessionId": "sess-prov"}));
        let session_id = provision_agent_session(
            &thread.id,
            Some("future/k3".to_string()),
            Some("high".to_string()),
        )
        .await
        .expect("provision");
        assert_eq!(session_id, "sess-prov");
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .agent_session_id
                .as_deref(),
            Some("sess-prov")
        );
        let created = &mock.requests_of("new_session")[0];
        assert_eq!(created.cwd, workspace.path);
        assert_eq!(created.model_id, "future/k3");
        assert_eq!(created.level, "high");
        assert_eq!(
            mock.requests_of("set_permission_level")[0].level,
            "workspace"
        );
        assert_eq!(mock.requests_of("set_sandbox_policy").len(), 1);

        // Unknown thread → workspace path resolution fails first.
        let error = provision_agent_session("no-such-thread", None, None)
            .await
            .expect_err("missing thread");
        assert_eq!(error.to_string(), "Thread could not be loaded.");
    }

    // ── delete outbox ─────────────────────────────────────────────────

    fn enqueue_delete(home: &TestHome, session_id: &str) {
        // Distinct, increasing requested_at keeps delivery order deterministic.
        static SEQ: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
        let conn =
            rusqlite::Connection::open(home.path().join(".future/app/app.db")).expect("open db");
        conn.execute(
            "INSERT INTO agent_delete_outbox(session_id, requested_at, attempts) VALUES (?1, ?2, 0)",
            rusqlite::params![
                session_id,
                SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ],
        )
        .expect("enqueue");
    }

    #[tokio::test]
    async fn delete_outbox_delivers_acknowledges_and_notes_failures() {
        let home = TestHome::new("bridge-outbox");
        let mock = mock_agent();

        // Nothing pending → no traffic.
        reconcile_delete_outbox().await;
        assert!(mock.requests().is_empty());

        // Successful delivery acknowledges the row.
        enqueue_delete(&home, "sess-del-ok");
        mock.push("delete_session", Reply::Data("{}".to_string()));
        reconcile_delete_outbox().await;
        assert!(
            !crate::store::is_agent_session_tombstoned("sess-del-ok").expect("query"),
            "delivered deletion is acknowledged"
        );

        // "session not found" counts as delivered (idempotent).
        enqueue_delete(&home, "sess-del-gone");
        mock.push(
            "delete_session",
            Reply::Reject("session not found: sess-del-gone".to_string()),
        );
        reconcile_delete_outbox().await;
        assert!(!crate::store::is_agent_session_tombstoned("sess-del-gone").expect("query"));

        // A real rejection is noted, not acknowledged.
        enqueue_delete(&home, "sess-del-busy");
        mock.push(
            "delete_session",
            Reply::Reject("session is running".to_string()),
        );
        reconcile_delete_outbox().await;
        assert!(crate::store::is_agent_session_tombstoned("sess-del-busy").expect("query"));

        // Transport failure is noted too. The still-pending busy row is
        // retried first (FIFO), so it gets a scripted reply as well.
        enqueue_delete(&home, "sess-del-down");
        mock.push(
            "delete_session",
            Reply::Reject("session is running".to_string()),
        );
        mock.push(
            "delete_session",
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        reconcile_delete_outbox().await;
        assert!(crate::store::is_agent_session_tombstoned("sess-del-down").expect("query"));
        assert!(crate::store::is_agent_session_tombstoned("sess-del-busy").expect("query"));

        // Store unreadable → silent return.
        let prev = break_home();
        reconcile_delete_outbox().await;
        restore_home(prev);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_outbox_worker_loops_until_stopped() {
        let home = TestHome::new("bridge-outbox-worker");
        let mock = mock_agent();
        enqueue_delete(&home, "sess-worker");
        mock.push("delete_session", Reply::Data("{}".to_string()));
        std::env::set_var("FUTURE_TEST_OUTBOX_INTERVAL_MS", "20");

        spawn_delete_outbox_worker();
        // Wait for the worker to deliver the pending deletion.
        for _ in 0..100 {
            if !crate::store::is_agent_session_tombstoned("sess-worker").expect("query") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(!crate::store::is_agent_session_tombstoned("sess-worker").expect("query"));
        TEST_OUTBOX_STOP.store(true, std::sync::atomic::Ordering::Relaxed);
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        std::env::remove_var("FUTURE_TEST_OUTBOX_INTERVAL_MS");
        // With the shrink seam removed the default (5s) interval is used.
        assert_eq!(delete_outbox_interval(), std::time::Duration::from_secs(5));
    }

    // ── active run watchdog ───────────────────────────────────────────

    #[tokio::test]
    async fn watchdog_pass_skips_young_runs_and_reconciles_old_ones() {
        let home = TestHome::new("bridge-watchdog-pass");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);

        // A young run (now) is inside the grace window → skipped silently.
        let now = run.created_at;
        active_run_watchdog_pass(now).await;
        assert!(mock.requests().is_empty());

        // Aged past the grace window: the agent has no marker and the run is
        // below the orphan age → Skip (row untouched), but the probe went out.
        mock.push_run_state(&run.id, serde_json::json!({"isStreaming": false}));
        let aged_now = run.created_at + (WATCHDOG_GRACE_SECS as i64 + 1) * 1000;
        active_run_watchdog_pass(aged_now).await;
        assert_eq!(mock.requests_of("get_state").len(), 1);
        assert_eq!(mock.requests_of("get_state")[0].run_id, run.id);
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("run")
                .expect("some")
                .status,
            "running"
        );

        // Past the orphan age with no marker → settled failed.
        mock.push_run_state(&run.id, serde_json::json!({"isStreaming": false}));
        let orphan_now = run.created_at + (WATCHDOG_ORPHAN_SECS as i64) * 1000;
        active_run_watchdog_pass(orphan_now).await;
        let record = crate::store::get_run(&run.id).expect("run").expect("some");
        assert_eq!(record.status, "failed");
        assert_eq!(record.error_type.as_deref(), Some("stream_interrupted"));

        // Reconcile transport error on an aged active run → logged, row
        // untouched (the watchdog continues instead of propagating).
        let run2 = seed_run(&thread.id);
        mock.push(
            &format!("get_state#{}", run2.id),
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        active_run_watchdog_pass(orphan_now).await;
        assert_eq!(
            crate::store::get_run(&run2.id)
                .expect("run")
                .expect("some")
                .status,
            "running",
            "transport error leaves the row untouched"
        );

        // Store unreadable → the pass returns immediately.
        let prev = break_home();
        active_run_watchdog_pass(orphan_now).await;
        restore_home(prev);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn watchdog_loop_ticks_and_stops() {
        let _home = TestHome::new("bridge-watchdog-loop");
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_WATCHDOG_INTERVAL_MS", "20");
        spawn_active_run_watchdog();

        // Agent unreachable (unparseable endpoint): the tick continues quietly.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);

        // Agent reachable: the pass runs (no active runs → no probe traffic,
        // but the tick exercises the full loop).
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        TEST_WATCHDOG_STOP.store(true, std::sync::atomic::Ordering::Relaxed);
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        std::env::remove_var("FUTURE_TEST_WATCHDOG_INTERVAL_MS");
        // With the shrink seam removed the default interval is used.
        assert_eq!(
            watchdog_interval(),
            std::time::Duration::from_secs(WATCHDOG_INTERVAL_SECS)
        );
        let _ = &mock;
    }

    #[tokio::test]
    async fn reconcile_active_run_once_variants() {
        let home = TestHome::new("bridge-reconcile-once");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);
        let active = crate::store::ActiveRun {
            run_id: run.id.clone(),
            thread_id: thread.id.clone(),
            session_id: "sess-1".to_string(),
            created_at: run.created_at,
        };

        // Agent still streaming this run → observer ensured (attach action).
        mock.push_run_state(
            &run.id,
            serde_json::json!({"isStreaming": true, "activeRun": {"runId": run.id}}),
        );
        reconcile_active_run_once(&active, &run.id, 120)
            .await
            .expect("attach");

        // Durable terminal marker → mirrored onto the row.
        mock.push_run_state(
            &run.id,
            serde_json::json!({"requestedRun": {"state": "completed"}}),
        );
        reconcile_active_run_once(&active, &run.id, 120)
            .await
            .expect("settle");
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("run")
                .expect("some")
                .status,
            "completed"
        );

        // get_state rejected → row untouched, Ok.
        mock.push(
            &format!("get_state#{}", run.id),
            Reply::Reject("unknown session".to_string()),
        );
        reconcile_active_run_once(&active, &run.id, 120)
            .await
            .expect("rejected is ok");

        // Transport failure → Err (the watchdog logs it).
        mock.push(
            &format!("get_state#{}", run.id),
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        let error = reconcile_active_run_once(&active, &run.id, 120)
            .await
            .expect_err("transport");
        assert!(error.contains("get_run_state"), "{error}");

        // Interrupted-by-restart marker → the row is cancelled.
        let run_i = seed_run(&thread.id);
        let active_i = crate::store::ActiveRun {
            run_id: run_i.id.clone(),
            thread_id: thread.id.clone(),
            session_id: "sess-1".to_string(),
            created_at: run_i.created_at,
        };
        mock.push_run_state(
            &run_i.id,
            serde_json::json!({"interruptedRun": {"runId": run_i.id}}),
        );
        reconcile_active_run_once(&active_i, &run_i.id, 120)
            .await
            .expect("interrupted");
        assert_eq!(
            crate::store::get_run(&run_i.id)
                .expect("run")
                .expect("some")
                .status,
            "cancelled"
        );
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::test_support::{
        mock_agent, seed_run, seed_thread, seed_workspace, stream_event, MockAgentGuard, Reply,
        StreamScript, TestHome,
    };
    use super::*;

    struct PipelineFixture {
        _home: TestHome,
        mock: MockAgentGuard,
        workspace: crate::store::WorkspaceRecord,
        thread: crate::store::ThreadRecord,
        run: crate::store::RunRecord,
    }

    /// Thread + run with a fresh (unstored) agent session; the observer the
    /// pipeline spawns parks on the default plain Hang stream.
    fn pipeline_fixture(tag: &str, title: &str) -> PipelineFixture {
        let home = TestHome::new(tag);
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let mut thread = seed_thread(&workspace.id, None);
        thread.title = title.to_string();
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: title.to_string(),
        })
        .expect("rename");
        let run = seed_run(&thread.id);
        PipelineFixture {
            _home: home,
            mock,
            workspace,
            thread,
            run,
        }
    }

    fn prompt_args(fixture: &PipelineFixture) -> (String, String, String) {
        (
            "hello from the test".to_string(),
            fixture.thread.id.clone(),
            fixture.run.id.clone(),
        )
    }

    #[tokio::test]
    async fn agent_prompt_new_session_full_pipeline() {
        let fixture = pipeline_fixture("pipe-new", "New Chat");
        let (message, thread_id, run_id) = prompt_args(&fixture);

        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-p1"}));
        fixture.mock.push_stream(StreamScript::Events(
            vec![
                stream_event(&run_id, 0, "text_chunk", r#"{"text":"hi there"}"#),
                stream_event(&run_id, 1, "agent_end", r#"{"reason":"complete"}"#),
            ],
            None,
        ));

        let response = agent_prompt_with_model_context(AgentPromptRequest {
            message: message.clone(),
            model_context: "Referenced FutureOS objects:\n1. file:utils/a.py".to_string(),
            attachments: None,
            thread_id: thread_id.clone(),
            session_id: None,
            run_id: Some(run_id.clone()),
            model_id: Some("future/k3".to_string()),
            thinking_level: Some("high".to_string()),
        })
        .await
        .expect("prompt");

        assert!(response.complete);
        assert_eq!(response.content, "hi there");
        assert_eq!(response.session_id, "sess-p1");
        assert!(!response.session_recreated);

        // Session id persisted; thread auto-named from the first message.
        let thread = crate::store::get_thread(&thread_id)
            .expect("thread")
            .expect("exists");
        assert_eq!(thread.agent_session_id.as_deref(), Some("sess-p1"));
        assert_eq!(thread.title, message);
        assert_eq!(
            crate::store::get_run(&run_id)
                .expect("run")
                .expect("some")
                .status,
            "completed",
            "the backend settles the run row"
        );

        // A freshly created session receives the caller's model + thinking.
        let new_session = &fixture.mock.requests_of("new_session")[0];
        assert_eq!(new_session.cwd, fixture.workspace.path);
        assert_eq!(
            fixture.mock.requests_of("set_model")[0].model_id,
            "future/k3"
        );
        assert_eq!(
            fixture.mock.requests_of("set_thinking_level")[0].level,
            "high"
        );
        let prompt = &fixture.mock.requests_of("prompt")[0];
        assert_eq!(prompt.message, message);
        assert_eq!(
            prompt.model_context,
            "Referenced FutureOS objects:\n1. file:utils/a.py"
        );
        assert_eq!(prompt.requested_run_id, run_id);
        assert_eq!(prompt.session_id, "sess-p1");
        // The observer was registered before the prompt reached the agent.
        assert!(
            observer::OBSERVERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key("sess-p1"),
            "session observer registered"
        );
    }

    #[tokio::test]
    async fn agent_prompt_existing_session_reuses_without_model_reapply() {
        let home = TestHome::new("pipe-existing");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-existing"));
        let run = seed_run(&thread.id);

        mock.push_data(
            "get_state",
            serde_json::json!({"sessionId": "sess-existing", "cwd": workspace.path}),
        );
        mock.push_stream(StreamScript::Events(
            vec![stream_event(
                &run.id,
                0,
                "agent_end",
                r#"{"reason":"incomplete"}"#,
            )],
            None,
        ));

        let response = agent_prompt(
            "follow-up".to_string(),
            Some(vec![AttachmentInput {
                path: "/tmp/a.txt".to_string(),
                kind: "file".to_string(),
                name: "a.txt".to_string(),
                thumbnail: None,
            }]),
            thread.id.clone(),
            None, // falls back to the thread's stored session id
            Some(run.id.clone()),
            Some("future/other".to_string()),
            None,
        )
        .await
        .expect("prompt");

        assert!(!response.complete, "incomplete agent_end is not clean");
        assert_eq!(response.session_id, "sess-existing");
        assert!(!response.session_recreated);
        assert!(
            mock.requests_of("new_session").is_empty(),
            "the stored session was reused"
        );
        assert!(
            mock.requests_of("set_model").is_empty(),
            "an existing session keeps its authoritative model"
        );
        assert_eq!(
            mock.requests_of("prompt")[0].attachments.len(),
            1,
            "attachments forwarded"
        );
        let record = crate::store::get_run(&run.id).expect("run").expect("some");
        assert_eq!(
            record.status, "failed",
            "an interrupted stream fails the run"
        );
    }

    #[tokio::test]
    async fn agent_prompt_recreated_session_reports_context_loss() {
        let home = TestHome::new("pipe-recreated");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-old"));
        let run = seed_run(&thread.id);

        // The agent lost the session's cwd → ensure recreates.
        mock.push_data(
            "get_state",
            serde_json::json!({"sessionId": "sess-old", "cwd": "/moved/elsewhere"}),
        );
        mock.push_data("new_session", serde_json::json!({"sessionId": "sess-p3"}));
        mock.push_stream(StreamScript::Events(
            vec![stream_event(
                &run.id,
                0,
                "agent_end",
                r#"{"reason":"complete"}"#,
            )],
            None,
        ));

        let response = agent_prompt(
            "hi".to_string(),
            None,
            thread.id.clone(),
            Some("sess-old".to_string()),
            Some(run.id.clone()),
            Some("future/k3".to_string()),
            None,
        )
        .await
        .expect("prompt");

        assert!(response.session_recreated);
        assert_eq!(response.session_id, "sess-p3");
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .agent_session_id
                .as_deref(),
            Some("sess-p3")
        );
        // A recreated session is a fresh session: the model is applied.
        assert_eq!(mock.requests_of("set_model").len(), 1);
    }

    #[tokio::test]
    async fn agent_prompt_transport_and_rejection_failures_settle_the_run() {
        let fixture = pipeline_fixture("pipe-prompt-fail", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pf"}));

        fixture.mock.push(
            "prompt",
            Reply::Status(tonic::Code::Internal, "write failed"),
        );
        let error = agent_prompt(
            message.clone(),
            None,
            thread_id.clone(),
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("transport");
        assert!(
            error
                .to_string()
                .contains("Unable to send prompt to Future Agent"),
            "{error}"
        );
        assert_eq!(
            crate::store::get_run(&run_id)
                .expect("run")
                .expect("some")
                .status,
            "failed"
        );

        // Release the first fixture before building the second: each fixture
        // holds the process-global TEST_HOME_LOCK + MOCK_LOCK guards, so two
        // live fixtures on one test thread would self-deadlock.
        drop(fixture);
        let fixture2 = pipeline_fixture("pipe-prompt-reject", "t");
        fixture2
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pr"}));
        fixture2
            .mock
            .push("prompt", Reply::Reject("busy".to_string()));
        let error = agent_prompt(
            message,
            None,
            fixture2.thread.id.clone(),
            None,
            Some(fixture2.run.id.clone()),
            None,
            None,
        )
        .await
        .expect_err("reject");
        assert_eq!(error.to_string(), "busy");
    }

    #[tokio::test]
    async fn agent_prompt_ack_must_carry_the_requested_run_id() {
        let fixture = pipeline_fixture("pipe-ack", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pa"}));

        // Ack without run_id.
        fixture
            .mock
            .push("prompt", Reply::Data(r#"{"ok":true}"#.to_string()));
        let error = agent_prompt(
            message.clone(),
            None,
            thread_id.clone(),
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("missing run id");
        assert_eq!(
            error.to_string(),
            "Future Agent prompt acknowledgement omitted run_id."
        );

        // Ack with a DIFFERENT run id. Release the first fixture first: two
        // live fixtures on one test thread self-deadlock on the
        // process-global TEST_HOME_LOCK + MOCK_LOCK guards.
        drop(fixture);
        let fixture2 = pipeline_fixture("pipe-ack-mismatch", "t");
        fixture2
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pam"}));
        fixture2.mock.push(
            "prompt",
            Reply::Data(r#"{"run_id":"run-other"}"#.to_string()),
        );
        let error = agent_prompt(
            message,
            None,
            fixture2.thread.id.clone(),
            None,
            Some(fixture2.run.id.clone()),
            None,
            None,
        )
        .await
        .expect_err("mismatch");
        assert!(
            error.to_string().contains("adopted run id run-other"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn agent_prompt_generates_a_run_id_when_absent() {
        let fixture = pipeline_fixture("pipe-gen-run", "t");
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pg"}));
        // First attach closes with zero events: the collector treats a stream
        // that ends before a terminal event as a drop and reattaches. The
        // reattach then delivers an unclean agent_end → complete = false.
        fixture.mock.push_stream(StreamScript::Events(vec![], None));
        fixture.mock.push_stream(StreamScript::Events(
            vec![stream_event(
                "@attach",
                0,
                "agent_end",
                r#"{"reason":"incomplete"}"#,
            )],
            None,
        ));
        // No run_id: the pipeline generates one; the mock's default prompt
        // reply echoes the requested_run_id.
        let response = agent_prompt(
            "hi".to_string(),
            None,
            fixture.thread.id.clone(),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("prompt");
        assert!(!response.complete, "an instantly-closed stream is a prefix");
        let prompt = &fixture.mock.requests_of("prompt")[0];
        assert!(
            prompt.requested_run_id.starts_with("run-"),
            "generated id: {}",
            prompt.requested_run_id
        );
    }

    #[tokio::test]
    async fn agent_prompt_rejects_when_the_run_already_has_a_collector() {
        let fixture = pipeline_fixture("pipe-lease", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pl"}));

        let lease = AGENT_REPLICAS.acquire(&run_id).expect("pre-acquire");
        let error = agent_prompt(
            message,
            None,
            thread_id,
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("lease conflict");
        assert!(
            error.to_string().contains("already owns Agent run"),
            "{error}"
        );
        drop(lease);
    }

    #[tokio::test]
    async fn agent_prompt_run_gone_reconciles_from_the_journal() {
        let fixture = pipeline_fixture("pipe-rungone", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-prg"}));
        fixture.mock.push_stream(StreamScript::AttachError(
            tonic::Code::FailedPrecondition,
            "no such run",
        ));
        // The journal holds a durable completed marker for the run.
        fixture.mock.push_run_state(
            &run_id,
            serde_json::json!({"requestedRun": {"state": "completed"}}),
        );
        let error = agent_prompt(
            message,
            None,
            thread_id,
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("run gone");
        assert!(
            error
                .to_string()
                .contains("run ended before the stream attached"),
            "{error}"
        );
        assert_eq!(
            crate::store::get_run(&run_id)
                .expect("run")
                .expect("some")
                .status,
            "completed",
            "the journal marker settles the row"
        );
    }

    #[tokio::test]
    async fn agent_prompt_run_gone_with_a_failed_reconcile_reports_both() {
        let fixture = pipeline_fixture("pipe-rungone-fail", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-prgf"}));
        fixture.mock.push_stream(StreamScript::AttachError(
            tonic::Code::NotFound,
            "unknown run",
        ));
        fixture.mock.push(
            &format!("get_state#{run_id}"),
            Reply::Status(tonic::Code::Unavailable, "agent restarting"),
        );
        let error = agent_prompt(
            message,
            None,
            thread_id,
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("run gone");
        let message = error.to_string();
        assert!(message.contains("unknown run"), "{message}");
        assert!(
            message.contains("terminal reconciliation failed"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn agent_prompt_stream_error_aborts_the_agent_run() {
        let fixture = pipeline_fixture("pipe-stream-err", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pse"}));
        fixture.mock.push_stream(StreamScript::Events(
            vec![stream_event(
                &run_id,
                0,
                "error",
                r#"{"error":"provider down"}"#,
            )],
            None,
        ));
        let error = agent_prompt(
            message,
            None,
            thread_id,
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("stream error");
        assert_eq!(error.to_string(), "provider down");
        // The orphaned agent-side run is aborted best-effort.
        let aborts = fixture.mock.requests_of("abort");
        assert_eq!(aborts.len(), 1);
        assert_eq!(aborts[0].run_id, run_id);
        assert_eq!(aborts[0].session_id, "sess-pse");
        assert_eq!(
            crate::store::get_run(&run_id)
                .expect("run")
                .expect("some")
                .status,
            "failed"
        );
    }

    /// When the stream fails and the best-effort abort is itself rejected, the
    /// abort error is logged and the original stream error still surfaces.
    #[tokio::test]
    async fn agent_prompt_stream_error_logs_a_failed_abort() {
        let fixture = pipeline_fixture("pipe-stream-err-abort", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-psea"}));
        fixture.mock.push_stream(StreamScript::Events(
            vec![stream_event(
                &run_id,
                0,
                "error",
                r#"{"error":"provider down"}"#,
            )],
            None,
        ));
        // The best-effort abort transport-fails — exercise the abort-failure log.
        fixture
            .mock
            .push("abort", Reply::Status(tonic::Code::Internal, "abort down"));

        let error = agent_prompt(
            message,
            None,
            thread_id,
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("stream error");
        assert_eq!(error.to_string(), "provider down");
        assert_eq!(fixture.mock.requests_of("abort").len(), 1);
    }

    #[tokio::test]
    async fn agent_prompt_requires_a_real_thread() {
        let _home = TestHome::new("pipe-no-thread");
        let _mock = mock_agent();
        let error = agent_prompt(
            "hi".to_string(),
            None,
            "no-such-thread".to_string(),
            None,
            None,
            None,
            None,
        )
        .await
        .expect_err("missing thread");
        assert_eq!(error.to_string(), "Thread could not be loaded.");
    }

    // ── auto_name_thread ──────────────────────────────────────────────

    #[tokio::test]
    async fn auto_name_thread_variants() {
        let home = TestHome::new("pipe-autoname");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");

        // Missing thread: silent.
        auto_name_thread("no-such-thread", "hello");

        // Default-titled variants are renamed; the agent is told (fire-and-forget).
        // (rename_thread rejects empty titles, so the empty-titled row is made
        // through create_thread, which takes the caller's title verbatim.)
        let empty_titled = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "workspace".to_string(),
            title: Some(String::new()),
            workspace_id: Some(workspace.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("sess-an-empty".to_string()),
        })
        .expect("empty-titled thread");
        auto_name_thread(&empty_titled.id, "  a fresh question  ");
        assert_eq!(
            crate::store::get_thread(&empty_titled.id)
                .expect("thread")
                .expect("exists")
                .title,
            "a fresh question",
            "an empty title is auto-named"
        );
        for (tag, title) in [("zh", "新对话"), ("newchat", "New Chat")] {
            let thread = seed_thread(&workspace.id, Some(&format!("sess-an-{tag}")));
            crate::store::rename_thread(crate::store::RenameThreadInput {
                thread_id: thread.id.clone(),
                title: title.to_string(),
            })
            .expect("rename");
            auto_name_thread(&thread.id, "  a fresh question  ");
            assert_eq!(
                crate::store::get_thread(&thread.id)
                    .expect("thread")
                    .expect("exists")
                    .title,
                "a fresh question",
                "title {title:?} is auto-named"
            );
        }

        // Long messages truncate to 40 chars + ellipsis.
        let thread = seed_thread(&workspace.id, Some("sess-an-long"));
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "New Chat".to_string(),
        })
        .expect("rename");
        let long = "x".repeat(50);
        auto_name_thread(&thread.id, &long);
        let titled = crate::store::get_thread(&thread.id)
            .expect("thread")
            .expect("exists")
            .title;
        assert!(titled.ends_with('…'), "truncated: {titled}");
        assert_eq!(titled.chars().count(), 41);

        // User-set titles are never overwritten.
        let thread = seed_thread(&workspace.id, Some("sess-an-custom"));
        auto_name_thread(&thread.id, "new message");
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .title,
            "test thread"
        );

        // Blank messages never rename (empty-titled row via create_thread —
        // rename_thread rejects empty titles).
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "workspace".to_string(),
            title: Some(String::new()),
            workspace_id: Some(workspace.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("sess-an-blank".to_string()),
        })
        .expect("empty-titled thread");
        auto_name_thread(&thread.id, "   ");
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .title,
            ""
        );
    }

    /// The auto-name fire-and-forget agent rename silently skips the agent
    /// call when the agent is unreachable (the `if let Ok` else path).
    #[tokio::test]
    async fn auto_name_thread_survives_an_unreachable_agent() {
        let home = TestHome::new("pipe-autoname-down");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-an-down"));
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "New Chat".to_string(),
        })
        .expect("rename");

        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");

        auto_name_thread(&thread.id, "hello");
        // Let the fire-and-forget rename task run against the dead endpoint.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        // The local rename still happened; only the agent propagation was
        // skipped.
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .title,
            "hello"
        );
    }

    // ── crash recovery ────────────────────────────────────────────────

    fn mark_interrupted(run_id: &str) {
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: run_id.to_string(),
            status: "cancelled".to_string(),
            error_message: Some("Interrupted because Future Agent restarted.".to_string()),
            error_type: Some("interrupted".to_string()),
        })
        .expect("mark interrupted");
    }

    #[tokio::test]
    async fn reconcile_interrupted_runs_edge_cases() {
        let _home = TestHome::new("pipe-reconcile-empty");
        let mock = mock_agent();

        // Empty list → no traffic.
        reconcile_interrupted_runs().await;
        assert!(mock.requests().is_empty());

        // Store unreadable → silent return.
        let prev = test_support::break_home();
        reconcile_interrupted_runs().await;
        test_support::restore_home(prev);
    }

    /// A crash-recovery pass over an interrupted run whose reanimation hits an
    /// unreachable agent logs the failure (rather than panicking) and keeps
    /// going.
    #[tokio::test]
    async fn reconcile_interrupted_runs_logs_a_reanimation_error() {
        let home = TestHome::new("pipe-reconcile-err");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-re-err"));
        let run = seed_run(&thread.id);
        mark_interrupted(&run.id);

        // Agent unreachable → check_and_reanimate_run returns Err → logged.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        reconcile_interrupted_runs().await;
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
    }

    #[tokio::test]
    async fn reanimate_still_streaming_run_attaches_an_observer() {
        let home = TestHome::new("pipe-reanimate");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-re"));
        let run = seed_run(&thread.id);
        mark_interrupted(&run.id);

        mock.push_run_state(
            &run.id,
            serde_json::json!({"isStreaming": true, "activeRun": {"runId": run.id}}),
        );
        reconcile_interrupted_runs().await;
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("run")
                .expect("some")
                .status,
            "running",
            "reanimated back to running"
        );
        assert!(
            observer::OBSERVERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key("sess-re"),
            "observer attached for the live run"
        );
    }

    #[tokio::test]
    async fn check_and_reanimate_run_variants() {
        let home = TestHome::new("pipe-check-variants");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-cv"));

        // Agent cannot resolve the session → leave interrupted, Ok.
        let run = seed_run(&thread.id);
        mark_interrupted(&run.id);
        mock.push(
            &format!("get_state#{}", run.id),
            Reply::Reject("unknown session".to_string()),
        );
        check_and_reanimate_run("sess-cv", &run.id, &thread.id)
            .await
            .expect("unresolved is ok");
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("run")
                .expect("some")
                .status,
            "cancelled"
        );

        // Streaming THIS run but the row is no longer interrupted → skip.
        let run2 = seed_run(&thread.id); // still "running" — never interrupted
        mock.push_run_state(
            &run2.id,
            serde_json::json!({"isStreaming": true, "activeRun": {"runId": run2.id}}),
        );
        check_and_reanimate_run("sess-cv", &run2.id, &thread.id)
            .await
            .expect("skip");
        assert!(
            !observer::OBSERVERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key("sess-cv"),
            "no observer for the skipped reanimation"
        );

        // Interrupted-by-restart marker → left cancelled.
        let run3 = seed_run(&thread.id);
        mark_interrupted(&run3.id);
        mock.push_run_state(
            &run3.id,
            serde_json::json!({"interruptedRun": {"runId": run3.id}}),
        );
        check_and_reanimate_run("sess-cv", &run3.id, &thread.id)
            .await
            .expect("interrupted");
        assert_eq!(
            crate::store::get_run(&run3.id)
                .expect("run")
                .expect("some")
                .status,
            "cancelled"
        );

        // Durable terminal marker → mirrored.
        let run4 = seed_run(&thread.id);
        mark_interrupted(&run4.id);
        mock.push_run_state(
            &run4.id,
            serde_json::json!({"requestedRun": {"state": "error", "error": "boom"}}),
        );
        check_and_reanimate_run("sess-cv", &run4.id, &thread.id)
            .await
            .expect("settle");
        let record = crate::store::get_run(&run4.id).expect("run").expect("some");
        assert_eq!(record.status, "failed");
        assert_eq!(record.error_message.as_deref(), Some("boom"));

        // No markers at all → conservatively left interrupted.
        let run5 = seed_run(&thread.id);
        mark_interrupted(&run5.id);
        mock.push_run_state(&run5.id, serde_json::json!({"isStreaming": false}));
        check_and_reanimate_run("sess-cv", &run5.id, &thread.id)
            .await
            .expect("leave");
        assert_eq!(
            crate::store::get_run(&run5.id)
                .expect("run")
                .expect("some")
                .status,
            "cancelled"
        );

        // Transport failure → Err.
        mock.push(
            &format!("get_state#{}", run5.id),
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        let error = check_and_reanimate_run("sess-cv", &run5.id, &thread.id)
            .await
            .expect_err("transport");
        assert!(error.contains("get_state"), "{error}");

        // Connect failure → Err.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let error = check_and_reanimate_run("sess-cv", &run5.id, &thread.id)
            .await
            .expect_err("connect");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        assert!(error.contains("connect"), "{error}");
    }

    #[tokio::test]
    async fn reconcile_run_gone_marker_precedence() {
        let home = TestHome::new("pipe-rungone-precedence");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-rgp"));

        // Still active agent-side (attach raced start_run) → left running.
        let run = seed_run(&thread.id);
        mock.push_run_state(&run.id, serde_json::json!({"activeRun": {"runId": run.id}}));
        reconcile_run_gone(&run.id, &run.id, "sess-rgp", "test")
            .await
            .expect("still active");
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("run")
                .expect("some")
                .status,
            "running"
        );

        // Interrupted marker → cancelled/interrupted.
        let run2 = seed_run(&thread.id);
        mock.push_run_state(
            &run2.id,
            serde_json::json!({"interruptedRun": {"runId": run2.id}}),
        );
        reconcile_run_gone(&run2.id, &run2.id, "sess-rgp", "test")
            .await
            .expect("interrupted");
        let record = crate::store::get_run(&run2.id).expect("run").expect("some");
        assert_eq!(record.status, "cancelled");
        assert_eq!(record.error_type.as_deref(), Some("interrupted"));

        // No marker at all → settled failed.
        let run3 = seed_run(&thread.id);
        mock.push_run_state(&run3.id, serde_json::json!({}));
        reconcile_run_gone(&run3.id, &run3.id, "sess-rgp", "vanished")
            .await
            .expect("failed");
        let record = crate::store::get_run(&run3.id).expect("run").expect("some");
        assert_eq!(record.status, "failed");
        assert!(
            record
                .error_message
                .as_deref()
                .unwrap_or_default()
                .contains("vanished"),
            "message: {:?}",
            record.error_message
        );

        // get_state itself failed → state treated as empty (no marker) → failed.
        let run4 = seed_run(&thread.id);
        mock.push(
            &format!("get_state#{}", run4.id),
            Reply::Reject("gone".to_string()),
        );
        reconcile_run_gone(&run4.id, &run4.id, "sess-rgp", "gone")
            .await
            .expect("failed");
        assert_eq!(
            crate::store::get_run(&run4.id)
                .expect("run")
                .expect("some")
                .status,
            "failed"
        );

        // Connect failure → Err.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let error = reconcile_run_gone(&run4.id, &run4.id, "sess-rgp", "test")
            .await
            .expect_err("connect");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        assert!(error.contains("reconcile connect"), "{error}");
    }

    #[test]
    fn settle_from_agent_terminal_maps_all_states() {
        let home = TestHome::new("pipe-settle");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-s"));

        let cases = [
            ("completed", "completed", None, None),
            (
                "cancelled",
                "cancelled",
                Some("cancelled"),
                Some("Run was cancelled."),
            ),
            (
                "error",
                "failed",
                Some("agent_error"),
                Some("Future Agent run failed."),
            ),
            (
                "mystery",
                "failed",
                Some("stream_interrupted"),
                Some("Future Agent response ended before a clean terminal."),
            ),
        ];
        for (agent_state, status, error_type, default_message) in cases {
            let run = seed_run(&thread.id);
            settle_from_agent_terminal(&run.id, agent_state, None).expect("settle");
            let record = crate::store::get_run(&run.id).expect("run").expect("some");
            assert_eq!(record.status, status, "agent state {agent_state}");
            assert_eq!(record.error_type.as_deref(), error_type);
            assert_eq!(record.error_message.as_deref(), default_message);
        }

        // The agent's own error message wins over the default.
        let run = seed_run(&thread.id);
        settle_from_agent_terminal(&run.id, "error", Some("provider exploded")).expect("settle");
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("run")
                .expect("some")
                .error_message
                .as_deref(),
            Some("provider exploded")
        );
    }

    // ── attach_remote_stream ──────────────────────────────────────────

    #[tokio::test]
    async fn attach_remote_stream_variants() {
        let home = TestHome::new("pipe-attach");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");

        // Missing thread / missing session.
        let error = attach_remote_stream("no-such-thread")
            .await
            .expect_err("missing");
        assert_eq!(error, "Thread not found");
        let no_session = seed_thread(&workspace.id, None);
        let error = attach_remote_stream(&no_session.id)
            .await
            .expect_err("no session");
        assert_eq!(error, "Thread has no agent session");

        // An active local run short-circuits (no get_state round-trip).
        let thread = seed_thread(&workspace.id, Some("sess-at"));
        let active = seed_run(&thread.id);
        let run_id = attach_remote_stream(&thread.id).await.expect("attach");
        assert_eq!(run_id, active.id);
        assert!(mock.requests_of("get_state").is_empty());

        // Same for a run parked on an approval.
        let thread2 = seed_thread(&workspace.id, Some("sess-at2"));
        let parked = seed_run(&thread2.id);
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: parked.id.clone(),
            status: "waiting_approval".to_string(),
            error_message: None,
            error_type: None,
        })
        .expect("park");
        let run_id = attach_remote_stream(&thread2.id).await.expect("attach");
        assert_eq!(run_id, parked.id);

        // The short-circuit paths above spawned observers for `sess-at` /
        // `sess-at2`; their async `get_state` probes would otherwise race the
        // scripted reply below (the mock's get_state queue is per command
        // type, not per session). Cancel them so the reply is deterministic.
        super::observer::cancel_all_observers();

        // No local run: the agent's active run gets a local row + observer.
        let thread3 = seed_thread(&workspace.id, Some("sess-at3"));
        mock.push_state_for_session(
            "sess-at3",
            Reply::Data(r#"{"activeRun": {"runId": "run-remote-1"}}"#.to_string()),
        );
        let run_id = attach_remote_stream(&thread3.id).await.expect("attach");
        assert_eq!(run_id, "run-remote-1");
        let row = crate::store::get_run("run-remote-1")
            .expect("run")
            .expect("some");
        assert_eq!(row.thread_id, thread3.id);

        // No active run agent-side → error.
        let thread4 = seed_thread(&workspace.id, Some("sess-at4"));
        mock.push_state_for_session(
            "sess-at4",
            Reply::Data(r#"{"isStreaming": false}"#.to_string()),
        );
        let error = attach_remote_stream(&thread4.id)
            .await
            .expect_err("no active");
        assert_eq!(error, "Agent session has no active canonical run");

        // Transport failure → error.
        let thread5 = seed_thread(&workspace.id, Some("sess-at5"));
        mock.push_state_for_session("sess-at5", Reply::Status(tonic::Code::Unavailable, "down"));
        let error = attach_remote_stream(&thread5.id)
            .await
            .expect_err("transport");
        assert!(error.contains("get_state"), "{error}");
    }

    // ── reconcile_thread_workspace ────────────────────────────────────

    #[tokio::test]
    async fn reconcile_thread_workspace_variants() {
        let home = TestHome::new("pipe-reconcile-ws");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");

        let error = reconcile_thread_workspace("sess-missing", "/tmp/x").expect_err("no thread");
        assert_eq!(error, "No thread found for this session");

        // Empty cwd is a no-op.
        let thread = seed_thread(&workspace.id, Some("sess-rw"));
        reconcile_thread_workspace("sess-rw", "   ").expect("empty ok");
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .workspace_id,
            workspace.id
        );

        // Chat cwd → rename the chat thread's temporary workspace in place.
        // The rename only applies to threads created in chat mode (their
        // workspace row is the per-thread temporary one).
        let chat_thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: None,
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("sess-rwc".to_string()),
        })
        .expect("chat thread");
        let chat_cwd = format!("{}/.future/workspaces/chat/sess-rwc", home.path().display());
        reconcile_thread_workspace("sess-rwc", &chat_cwd).expect("chat");
        let moved = crate::store::get_thread(&chat_thread.id)
            .expect("thread")
            .expect("exists");
        let moved_ws = crate::store::get_workspace(&moved.workspace_id)
            .expect("ws")
            .expect("exists");
        assert_eq!(moved_ws.path, chat_cwd);

        // Project cwd matching an existing workspace → move there.
        let target = seed_workspace(home.path(), "target");
        let thread2 = seed_thread(&workspace.id, Some("sess-rw2"));
        reconcile_thread_workspace("sess-rw2", &target.path).expect("move");
        assert_eq!(
            crate::store::get_thread(&thread2.id)
                .expect("thread")
                .expect("exists")
                .workspace_id,
            target.id
        );

        // Brand-new project cwd → a workspace is created, then moved to.
        let thread3 = seed_thread(&workspace.id, Some("sess-rw3"));
        let new_dir = home.path().join("brand-new");
        std::fs::create_dir_all(&new_dir).expect("mkdir");
        reconcile_thread_workspace("sess-rw3", &new_dir.display().to_string())
            .expect("create+move");
        let moved = crate::store::get_thread(&thread3.id)
            .expect("thread")
            .expect("exists");
        assert_ne!(moved.workspace_id, workspace.id);
        let created = crate::store::get_workspace(&moved.workspace_id)
            .expect("ws")
            .expect("exists");
        assert_eq!(created.path, new_dir.display().to_string());
        assert_eq!(created.name, "brand-new");
    }
}
