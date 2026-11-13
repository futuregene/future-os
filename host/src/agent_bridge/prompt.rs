use super::reconciliation::reconcile_run_gone;
use super::*;

/// Persist the user entry for a prompt the Agent accepted but queued behind an
/// older run (attach refused with RunGone). The acceptance ordering is
/// durable: session config events, then the user entry, then `run_started` —
/// so the first `run_id`-bound entry is the user entry, and no terminal marker
/// can precede it.
///
/// Durable writes replay on the next import, so on the next launch the entry
/// flows through the same replay path as the original GUI writer; this call
/// only patches the current session.
async fn persist_queued_user_entry(session_id: &str, run_id: &str) -> Result<(), String> {
    let mut client = connect_agent().await.map_err(|e| format!("connect: {e}"))?;
    let entries = fetch_all_session_entries_with_client(&mut client, session_id)
        .await
        .map_err(|e| format!("get_session_entries: {e}"))?;
    let Some(entry) = entries
        .into_iter()
        .find(|entry| entry.run_id.as_deref() == Some(run_id))
    else {
        // The Agent accepted the prompt (RunGone is a response, and queued
        // runs persist their user entry before the ack), so an empty result
        // is a failed session read, not evidence of a missing entry — leave
        // the journal as the source of truth rather than persisting a stub.
        return Err(format!(
            "queued run {run_id} accepted by the Agent but its user entry was not found"
        ));
    };
    // Durable entries live in the Agent journal — the run-event log needs the
    // entry's identity + text at idx 0 so a reload shows the queued prompt.
    persist::persist_run_event(
        Some(run_id),
        "user_message",
        &serde_json::json!({
            "entry_id": entry.id,
            "run_id": run_id,
            "text": entry_text(&entry),
        })
        .to_string(),
        0,
    );
    Ok(())
}

/// The user-visible text of a queued prompt's durable entry: first text
/// block, else the whole JSON (attachments-only entries have no text block).
fn entry_text(entry: &future_rpc::payloads::SessionEntryPayload) -> String {
    entry
        .blocks
        .iter()
        .find_map(|block| block.text.as_deref())
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(entry).unwrap_or_default())
}

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
    /// The stable Agent session id for this conversation.
    pub session_id: String,
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
    /// What this send means while the session already has a run in flight: an
    /// Agent `BusyPolicy` value, or empty for the Agent's own default.
    #[serde(default)]
    pub busy_policy: String,
}
#[cfg(any(test, feature = "test-support"))]
pub async fn agent_prompt(
    message: String,
    attachments: Option<Vec<AttachmentInput>>,
    thread_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<AgentPromptResponse, crate::Error> {
    agent_prompt_with_model_context(AgentPromptRequest {
        message,
        model_context: String::new(),
        attachments,
        thread_id,
        session_id,
        run_id,
        model_id,
        thinking_level,
        busy_policy: String::new(),
    })
    .await
}

pub async fn agent_prompt_with_model_context(
    request: AgentPromptRequest,
) -> Result<AgentPromptResponse, crate::Error> {
    agent_prompt_with_acceptance(request, None).await
}

/// Run a prompt while exposing the Agent's durable-acceptance boundary to a
/// headless caller. The signal is sent only after the Agent's prompt command
/// returns the requested canonical run id; the Agent persists the user entry
/// before producing that acknowledgement.
pub async fn agent_prompt_with_acceptance(
    request: AgentPromptRequest,
    accepted: Option<tokio::sync::oneshot::Sender<()>>,
) -> Result<AgentPromptResponse, crate::Error> {
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
        // A queued run is alive on the Agent: `agent_prompt_inner` already
        // settled the row to `running` and the session observer owns its
        // projection from here — do not rewrite the row as failed.
        Ok(response) if response.termination_kind.as_deref() == Some("run_queued") => {}
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
            // Resolve after prompt admission: a previously-unbound thread may
            // have committed its first identity before a later stream error.
            // Never poll a client-supplied stale session.
            let effective_session_id = crate::store::get_thread(&request.thread_id)
                .ok()
                .flatten()
                .and_then(|thread| thread.agent_session_id)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| request.thread_id.clone());
            wait_for_agent_idle(&effective_session_id).await;
        }
        // §6.1: capture the after snapshot before the guard drops, so the next
        // Run's before-snapshot can't interleave. It forks `git` and does fs IO,
        // so run it on a blocking thread rather than stalling the async runtime.
        let sensitive = {
            let capture_thread = request.thread_id.clone();
            let capture_run = run_id.clone();
            crate::runtime::run_blocking(move || {
                review::capture_after(&capture_thread, &capture_run)
            })
            .await
            .unwrap_or_default()
        };
        // C1: the diff materialization is a read-only diff between fixed commits,
        // so defer it off the IPC path. The GUI is notified when it lands.
        crate::runtime::spawn(async move {
            let materialize_thread = request.thread_id.clone();
            let materialize_run = run_id.clone();
            let _ = crate::runtime::run_blocking(move || {
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
) -> Result<AgentPromptResponse, crate::Error> {
    // A prompt with neither text nor an attachment is not a turn, and nothing
    // below this line can tell: the Agent admits it (an empty user entry, a run
    // row, a queued receipt), and a queued send deliberately draws no bubble in
    // the transcript, so the reader gets an invisible turn they cannot see,
    // retry or take back. Measured 2026-11-09: three such sends were admitted on
    // a busy conversation, and the only trace was the host failing to persist
    // their user entry.
    //
    // The composer checks this for itself, but every other caller would have to
    // re-implement that check (the queue panel's edit, the dictation layer, a
    // staged prompt, a phone client), so the boundary is where it belongs.
    let has_attachment = request
        .attachments
        .as_ref()
        .is_some_and(|attachments| !attachments.is_empty());
    if request.message.trim().is_empty() && !has_attachment {
        return Err("A message needs text or an attachment.".into());
    }
    if let Some(task) = future_fusion::fusion_prompt(&request.message) {
        if task.is_empty() {
            return Err(crate::Error::Message(
                "Use /fusion followed by a task".into(),
            ));
        }
        // A new client must never silently send /fusion as plain text to an
        // older Agent, even when the Host itself advertises Fusion.
        super::client::connect_agent()
            .await?
            .execute_command(future_rpc::proto::RpcCommand {
                r#type: "get_fusion_status".into(),
                ..Default::default()
            })
            .await
            .map_err(|e| crate::Error::Message(format!("Fusion requires an upgraded Agent: {e}")))?
            .into_inner()
            .ok_or_rpc_error("Fusion requires an upgraded Agent")?;
    }
    let AgentPromptRequest {
        message,
        model_context,
        attachments,
        thread_id,
        session_id,
        run_id,
        model_id,
        thinking_level,
        busy_policy,
    } = request;
    // The persisted thread binding is the identity authority. The webview's
    // session id is only an optimistic assertion and may be stale for one
    // render during fork/switch reconciliation; it must never reroute a prompt
    // to another conversation.
    let thread = crate::store::get_thread(&thread_id)?
        .ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let stored_session_id = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or_default()
        .to_string();
    let requested_session_id = session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    if stored_session_id.is_empty()
        && (crate::store::legacy_session_id(&thread_id)?.is_some()
            || crate::store::has_legacy_history(&thread_id)?)
    {
        return Err("This conversation has preserved history but no writable Agent session. Restore its matching Agent database or continue from the original conversation; a new empty session was not created.".into());
    }
    if requested_session_id.is_some_and(|requested| requested != stored_session_id) {
        return Err(format!(
            "Conversation session changed before this prompt was sent; expected {stored_session_id:?}, received {requested_session_id:?}. Reload the conversation and retry."
        )
        .into());
    }
    let mut command_client = connect_agent().await?;
    if crate::store::has_legacy_history(&thread_id)? {
        let history = super::queries::get_session_entries_before_with_client(
            &command_client,
            stored_session_id.clone(),
            i64::MAX,
            1,
        )
        .await?;
        if !history["entries"].as_array().is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| matches!(entry["role"].as_str(), Some("user" | "assistant" | "tool")))
        }) {
            return Err("The Agent session has no conversation history, but a GUI recovery archive exists. Restore the matching Agent data before continuing this conversation.".into());
        }
    }

    // Create (or reuse) the agent session.
    let existing_cwd = workspace_path_for_thread(&thread_id)?;
    let setup_started = std::time::Instant::now();
    // run_blocking starts immediately and is tracked through Host shutdown.
    // The snapshot and session setup can overlap; both finish before tools run.
    let before_snapshot = run_id.as_ref().map(|run| {
        let thread = thread_id.clone();
        let run = run.clone();
        crate::runtime::run_blocking(move || {
            let started = std::time::Instant::now();
            review::capture_before(&thread, &run);
            tracing::debug!(
                event = "prompt_start_stage",
                stage = "review_before",
                thread_id = thread,
                run_id = run,
                elapsed_ms = started.elapsed().as_millis() as u64
            );
        })
    });
    let setup = async {
        let ensured = ensure_agent_session(
            &mut command_client,
            &stored_session_id,
            &existing_cwd,
            model_id.as_deref(),
            thinking_level.as_deref(),
        )
        .await?;
        let session_id = ensured.session_id;
        let session_was_created = ensured.created;

        // Identity is assigned exactly once. A bound conversation is never
        // rebound by the prompt path; losing an Agent session is surfaced as an
        // error above instead of silently replacing durable context. Commit the
        // first binding before secondary setup, so a permission/config failure can
        // retry the same session instead of leaking an unbound replacement.
        if session_was_created {
            crate::store::bind_thread_session_id(&thread_id, &session_id)?;
        }
        super::verify_thread_source_with_client(&mut command_client, &thread_id, &session_id)
            .await?;
        set_agent_permission_level(&mut command_client, &session_id, "workspace").await?;
        set_agent_sandbox_policy(&mut command_client, &session_id, &thread_id).await?;

        // new_session already receives model and thinking level atomically. Do
        // not repeat two RPCs and their durable metadata writes before the prompt.
        Ok::<_, crate::Error>(session_id)
    }
    .await;
    tracing::debug!(
        event = "prompt_start_stage",
        stage = "session_setup",
        thread_id,
        run_id = run_id.as_deref().unwrap_or(""),
        elapsed_ms = setup_started.elapsed().as_millis() as u64,
        success = setup.is_ok()
    );
    if let Some(snapshot) = before_snapshot {
        if let Err(error) = snapshot.await {
            tracing::warn!(event = "prompt_review_unavailable", thread_id, %error);
        }
    }
    let session_id = setup?;
    tracing::debug!(
        event = "prompt_start_stage",
        stage = "ready_to_send",
        thread_id,
        run_id = run_id.as_deref().unwrap_or(""),
        elapsed_ms = setup_started.elapsed().as_millis() as u64
    );

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
        None => crate::store::create_run_id(),
    };
    let replica_lease = AGENT_REPLICAS
        .acquire(&run_id)
        .map_err(crate::Error::from)?;
    observer::ensure_observer_for_thread(&session_id, &thread_id).map_err(crate::Error::from)?;

    let prompt_response = command_client
        .execute_command(prompt_command(
            message,
            model_context,
            session_id.clone(),
            attachments.unwrap_or_default(),
            Some(run_id.clone()),
            &busy_policy,
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
            })
        }
        Err(stream::CollectError::RunGone(reason)) => {
            // The Agent accepted the prompt (ack) but the attach was refused —
            // either the run is genuinely gone, or it was accepted QUEUED
            // behind an older run (a queued run has no execution epoch to
            // attach to). Reconcile against the Agent's authoritative state:
            // it settles a truly-gone run from the journal, and for a queued
            // run it leaves the row running and hands the projection to the
            // session observer.
            if let Err(reconcile_error) = reconcile_run_gone(
                &canonical_run_id,
                &canonical_run_id,
                &session_id,
                &thread_id,
                &reason,
            )
            .await
            {
                return Err(format!(
                    "Future Agent run ended before the stream attached: {reason}; \
                     terminal reconciliation failed: {reconcile_error}"
                )
                .into());
            }
            let still_running = crate::store::get_run(&canonical_run_id)
                .ok()
                .flatten()
                .map(|run| !matches!(run.status.as_str(), "completed" | "failed" | "cancelled"))
                .unwrap_or(false);
            if still_running {
                // Queued: persist its user entry (durable on the Agent since
                // acceptance) so the reload path sees the prompt, then let the
                // observer resume the stream when the run starts. The prompt's
                // own stream ends here — NOT as a failure of the run.
                if let Err(error) = persist_queued_user_entry(&session_id, &canonical_run_id).await
                {
                    tracing::error!(event = "agent_prompt_inner", error = %error,"FutureOS queued user-entry persistence failed: {error}");
                }
                // Mark the row for what it is. Until it is `queued` it stays
                // `running`, and being the newest row it wins every "latest run"
                // read — so the open conversation anchors its live preview on
                // this run, which has no events and may never run at all (a
                // coalescing burst is folded into the run ahead). The reply then
                // arrives in one lump when the turn ends, and the preview keeps
                // failing against a run id the Agent does not know.
                crate::store::mark_run_queued(&canonical_run_id);
                // The collector is about to drop its lease, and the observer
                // must be free to project this run the instant the Agent starts
                // it. Without this the released-at stamp parks the run behind
                // the single-writer grace window, and its whole reply streams
                // with nobody persisting it.
                super::replica::AGENT_REPLICAS.hand_off_to_observer(&canonical_run_id);
                return Ok(AgentPromptResponse {
                    content: String::new(),
                    complete: false,
                    termination_kind: Some("run_queued".to_string()),
                    session_id,
                });
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
                tracing::error!(event = "agent_prompt_inner", error = %abort_error,
                    "FutureOS: failed to abort Agent after stream error: {abort_error}"
                );
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
    // Truncate to ~40 chars visible width (same as the TUI's truncate_visible),
    // from the message's plain text — a title is not a place to draw markdown,
    // and the Agent names an unnamed session the same way (future_rpc::title).
    let plain = future_rpc::title::plain_title_text(trimmed);
    let title: String = plain.chars().take(40).collect();
    let title = if title.len() < plain.len() {
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
    crate::runtime::spawn(async move {
        if let Ok(mut client) = crate::agent_bridge::connect_agent().await {
            let cmd = crate::agent_bridge::set_session_name_command(title, session_id);
            let _ = client.execute_command(cmd).await;
        }
    });
}
