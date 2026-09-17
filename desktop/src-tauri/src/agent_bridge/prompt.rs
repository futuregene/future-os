use super::reconciliation::reconcile_run_gone;
use super::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPromptResponse {
    pub content: String,
    /// Whether the agent stream reached a clean `agent_end`. When false, the
    /// content is a truncated prefix (stream closed mid-reply) and the caller
    /// should finalize the run as failed rather than completed.
    pub complete: bool,
    /// Stable user-facing failure category when `complete` is false.
    pub termination_kind: Option<String>,
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
#[cfg(test)]
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
    agent_prompt_with_acceptance(request, None).await
}

/// Run a prompt while exposing the Agent's durable-acceptance boundary to a
/// headless caller. The signal is sent only after the Agent's prompt command
/// returns the requested canonical run id; the Agent persists the user entry
/// before producing that acknowledgement.
pub(crate) async fn agent_prompt_with_acceptance(
    request: AgentPromptRequest,
    accepted: Option<tokio::sync::oneshot::Sender<()>>,
) -> Result<AgentPromptResponse, crate::AppError> {
    let effective_session_id = request
        .session_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| request.thread_id.clone());
    let result = agent_prompt_inner(request.clone(), accepted).await;

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
        Ok(response) => {
            let error = stream::termination_error(response.termination_kind.as_deref());
            mark_run_failed_if_active(request.run_id.as_deref(), error);
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

pub(super) async fn agent_prompt_inner(
    request: AgentPromptRequest,
    mut accepted: Option<tokio::sync::oneshot::Sender<()>>,
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
    if let Some(accepted) = accepted.take() {
        let _ = accepted.send(());
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
                termination_kind: response.termination_kind,
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
pub(super) fn auto_name_thread(thread_id: &str, first_message: &str) {
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
