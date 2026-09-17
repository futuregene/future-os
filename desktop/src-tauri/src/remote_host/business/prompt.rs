use crate::remote::protocol::IncomingCmd;
use crate::remote::services::ReplySink;
use serde_json::{json, Value};

use super::{reply, reply_unit};

pub(super) async fn execute(cmd: &IncomingCmd, sink: &dyn ReplySink) {
    match cmd.cmd_type.as_str() {
        "prompt" => {
            match remote_prompt_receipt(&cmd.id) {
                Ok(Some(ack)) => {
                    reply(sink, true, ack, None).await;
                    return;
                }
                Ok(None) => {}
                Err(error) => {
                    reply(sink, false, Value::Null, Some(&error.to_string())).await;
                    return;
                }
            }
            let model_id = qualified_model_id(&cmd.model_id, &cmd.provider_id);
            let thinking_level = (!cmd.level.trim().is_empty()).then(|| cmd.level.clone());
            match prepare_remote_prompt(
                &cmd.session_id,
                cmd.message.clone(),
                RemotePromptOptions {
                    model_id,
                    thinking_level,
                    mode: cmd.mode.clone(),
                    workspace_id: cmd.workspace_id.clone(),
                    upload_references: cmd.attachments.clone(),
                    command_id: cmd.id.clone(),
                },
            )
            .await
            {
                Ok(prepared) => acknowledge_prepared(prepared, sink, "prompt processing").await,
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "get_prompt_receipt" => match remote_prompt_receipt(&cmd.prompt_id) {
            Ok(receipt) => reply(sink, true, receipt.unwrap_or(Value::Null), None).await,
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "abort" => {
            reply_unit(
                sink,
                crate::agent_bridge::abort_session(&cmd.session_id).await,
            )
            .await
        }
        "continue_run" => {
            match remote_prompt_receipt(&cmd.id) {
                Ok(Some(ack)) => {
                    reply(sink, true, ack, None).await;
                    return;
                }
                Ok(None) => {}
                Err(error) => {
                    reply(sink, false, Value::Null, Some(&error.to_string())).await;
                    return;
                }
            }
            if let Err(error) = validate_continue_source(&cmd.session_id, &cmd.run_id) {
                reply(sink, false, Value::Null, Some(&error.to_string())).await;
                return;
            }
            match prepare_remote_prompt(
                &cmd.session_id,
                build_continue_prompt(&cmd.run_id),
                RemotePromptOptions {
                    model_id: None,
                    thinking_level: None,
                    mode: "chat".to_string(),
                    workspace_id: String::new(),
                    upload_references: Vec::new(),
                    command_id: cmd.id.clone(),
                },
            )
            .await
            {
                Ok(prepared) => acknowledge_prepared(prepared, sink, "continue_run").await,
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "approval_decision" => {
            let ownership = (|| -> Result<(), crate::AppError> {
                let approval = crate::store::get_approval_request(&cmd.entry_id)?
                    .ok_or_else(|| "Approval request could not be loaded.".to_string())?;
                let thread = crate::store::get_thread(&approval.thread_id)?
                    .ok_or_else(|| "Approval thread could not be loaded.".to_string())?;
                let owner_session_id = thread.agent_session_id.unwrap_or(thread.id);
                if cmd.session_id != owner_session_id {
                    return Err(crate::AppError::Message(
                        "Approval request does not belong to this session.".to_string(),
                    ));
                }
                Ok(())
            })();
            if let Err(error) = ownership {
                reply(sink, false, Value::Null, Some(&error.to_string())).await;
                return;
            }
            let input = crate::store::DecideApprovalRequestInput {
                approval_request_id: cmd.entry_id.clone(),
                status: cmd.mode.clone(),
                decision_note: None,
            };
            reply_unit(
                sink,
                crate::agent_bridge::decide_approval(input)
                    .await
                    .map(|_| ()),
            )
            .await;
        }
        _ => unreachable!("prompt handler received {}", cmd.cmd_type),
    }
}

async fn acknowledge_prepared(
    prepared: crate::agent_bridge::PreparedPrompt,
    sink: &dyn ReplySink,
    failure_context: &'static str,
) {
    let ack = json!({
        "sessionId": prepared.session_id,
        "threadId": prepared.thread_id,
        "runId": prepared.run_id,
    });
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let thread_id = prepared.thread_id.clone();
        if let Err(error) =
            crate::agent_bridge::run_prepared_prompt_with_acceptance(prepared, accepted_tx).await
        {
            eprintln!("remote: {failure_context} failed: {error}");
        }
        crate::emit_remote_activity(&thread_id);
    });
    match accepted_rx.await {
        Ok(Ok(())) => reply(sink, true, ack, None).await,
        Ok(Err(error)) => reply(sink, false, Value::Null, Some(&error)).await,
        Err(_) => {
            reply(
                sink,
                false,
                Value::Null,
                Some("Future Agent did not acknowledge the prompt."),
            )
            .await
        }
    }
}

pub(crate) fn new_chat_thread_input() -> crate::store::CreateThreadInput {
    crate::store::CreateThreadInput {
        mode: "chat".to_string(),
        title: None,
        workspace_id: None,
        workspace_path: None,
        workspace_name: None,
        agent_session_id: None,
    }
}

/// Model ids from the agent catalogue are only unique inside their provider.
/// The Agent RPC accepts a single qualified `provider/model` value, so normalize
/// new mobile commands and keep legacy already-qualified callers working.
/// Current clients qualify the entire raw catalogue ID, including slashes;
/// a slash alone is not evidence that the selected provider is already present.
pub(crate) fn qualified_model_id(model_id: &str, provider_id: &str) -> Option<String> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return None;
    }
    let provider_id = provider_id.trim();
    if provider_id.is_empty() || model_id.starts_with(&format!("{provider_id}/")) {
        Some(model_id.to_string())
    } else {
        Some(format!("{provider_id}/{model_id}"))
    }
}

/// One-shot injected failure for the prompt-prepare step (tests only): the
/// store write inside `prepare_prompt_persisted` cannot fail deterministically
/// from the outside (pooled WAL connections keep working across chmod/unlink),
/// so the claimed-attachment rollback path is exercised through this seam.
#[cfg(test)]
pub(crate) static INJECT_PREPARE_FAILURE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// See [`INJECT_PREPARE_FAILURE`]; armed by tests, disarmed on use.
#[cfg(test)]
pub(crate) fn injected_prepare_failure() -> Result<(), crate::AppError> {
    INJECT_PREPARE_FAILURE
        .swap(false, std::sync::atomic::Ordering::Relaxed)
        .then(|| crate::AppError::Message("injected prepare failure".to_string()))
        .map_or(Ok(()), Err)
}

/// Build the "continue the previous task" prompt for a failed run. Mirrors the
/// desktop `buildContinuePrompt`/`loadRunResumeSummary` shape, but folds only
/// the run's recent terminal events (tool output detail lives in the GUI-side
/// summary; the store exposes the events, which is enough to resume). Sent to
/// the LLM, so the text is intentionally not localized.
pub(crate) fn build_continue_prompt(run_id: &str) -> String {
    let events = crate::store::list_run_events(run_id).unwrap_or_default();
    let mut lines = vec!["继续上一个任务。".to_string()];
    let terminal: Vec<_> = events
        .iter()
        .filter(|event| {
            matches!(
                event.event_type.as_str(),
                "error" | "agent_error" | "agent_end" | "tool_end" | "tool_result"
            )
        })
        .collect();
    if !terminal.is_empty() {
        lines.push(String::new());
        lines.push("已执行内容摘要:".to_string());
        for event in terminal.iter().rev().take(6).rev() {
            let payload = event.payload.as_deref().unwrap_or("");
            let truncated: String = payload.chars().take(360).collect();
            lines.push(format!("- {}: {}", event.event_type, truncated));
        }
    }
    lines.join("\n")
}

/// A continuation is not a free-form prompt alias: it may only resume the
/// failed run that belongs to the addressed session. This keeps a stale mobile
/// outbox entry (including one from a previous pairing) from creating or
/// steering an unrelated conversation.
pub(crate) fn validate_continue_source(
    session_id: &str,
    run_id: &str,
) -> Result<(), crate::AppError> {
    if session_id.trim().is_empty() || run_id.trim().is_empty() {
        return Err("A continuation requires a session and source run."
            .to_string()
            .into());
    }
    let run = crate::store::get_run(run_id)?
        .ok_or_else(|| "The source run no longer exists.".to_string())?;
    if run.status != "failed" {
        return Err("Only a failed run can be continued.".to_string().into());
    }
    let thread = crate::store::get_thread(&run.thread_id)?
        .ok_or_else(|| "The source run's conversation no longer exists.".to_string())?;
    let run_session_id = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&thread.id);
    if run_session_id != session_id {
        return Err("The source run does not belong to this session."
            .to_string()
            .into());
    }
    Ok(())
}

pub(crate) fn remote_prompt_receipt(command_id: &str) -> Result<Option<Value>, crate::AppError> {
    let Some(run) = crate::store::find_run_by_trigger_message_id(command_id)? else {
        return Ok(None);
    };
    let Some(thread) = crate::store::get_thread(&run.thread_id)? else {
        return Ok(None);
    };
    let session_id = thread
        .agent_session_id
        .clone()
        .unwrap_or_else(|| thread.id.clone());
    Ok(Some(json!({
        "sessionId": session_id,
        "threadId": thread.id,
        "runId": run.id,
    })))
}

pub(crate) struct RemotePromptOptions {
    pub(crate) model_id: Option<String>,
    pub(crate) thinking_level: Option<String>,
    pub(crate) mode: String,
    pub(crate) workspace_id: String,
    pub(crate) upload_references: Vec<crate::remote_host::files::UploadReference>,
    pub(crate) command_id: String,
}

/// Find the thread for `session_id` (create a new chat thread when unknown —
/// remote policy), then persist user message + run via `agent_bridge::headless`.
pub(crate) async fn prepare_remote_prompt(
    session_id: &str,
    message: String,
    options: RemotePromptOptions,
) -> Result<crate::agent_bridge::PreparedPrompt, crate::AppError> {
    let RemotePromptOptions {
        model_id,
        thinking_level,
        mode,
        workspace_id,
        upload_references,
        command_id,
    } = options;
    let thread = match crate::store::find_thread_by_agent_session(session_id)? {
        Some(thread) => thread,
        None => {
            // Lazy creation: the thread is born with the first message, titled
            // from it (mirrors the GUI new-chat draft), and immediately gets a
            // real agent session id so the ack, the event subjects, and history
            // all agree from the start (no empty row, no id drift).
            let mut input = if mode == "workspace" {
                if workspace_id.trim().is_empty() {
                    return Err(crate::AppError::Message(
                        "Select a workspace before starting a workspace conversation.".to_string(),
                    ));
                }
                crate::store::CreateThreadInput {
                    mode: "workspace".to_string(),
                    title: None,
                    workspace_id: Some(workspace_id),
                    workspace_path: None,
                    workspace_name: None,
                    agent_session_id: None,
                }
            } else {
                new_chat_thread_input()
            };
            input.title = Some(derive_thread_title(&message));
            let mut thread = crate::store::create_thread(input)?;
            match crate::agent_bridge::provision_agent_session(
                &thread.id,
                model_id.clone(),
                thinking_level.clone(),
            )
            .await
            {
                Ok(sid) => thread.agent_session_id = Some(sid),
                Err(e) => {
                    // Thread exists but has no agent session → it would show as
                    // an orphan empty row in the GUI list. Remove it best-effort.
                    let _ = crate::store::delete_thread(&thread.id);
                    return Err(e);
                }
            }
            thread
        }
    };
    // Reject a prompt for a session that is already running BEFORE persisting
    // anything (matches GUI semantics: no follow-up/queue). The agent
    // refuses a concurrent prompt too, but only after the ack — checking here
    // keeps a busy session from accumulating a phantom user message, a failed
    // run, and a fake "Future Agent error" assistant reply. Residual race: two
    // clients prompting the same idle session within milliseconds can both pass
    // this check; the agent's is_streaming refusal stays as the backstop.
    let resolved_session_id = thread
        .agent_session_id
        .clone()
        .unwrap_or_else(|| thread.id.clone());
    if crate::store::active_run_sessions()?
        .iter()
        .any(|active| active == &resolved_session_id)
    {
        return Err(crate::AppError::Message(
            "This session is still running; wait for it to finish or abort it first.".to_string(),
        ));
    }
    let attachments = crate::remote_host::files::claim_uploads(&upload_references, &thread.id)?;
    let prepared = crate::agent_bridge::prepare_prompt_persisted_with_trigger(
        &thread,
        message,
        model_id,
        thinking_level,
        attachments.clone(),
        (!command_id.trim().is_empty()).then_some(command_id),
    );
    #[cfg(test)]
    let prepared = prepared.and_then(|prepared| injected_prepare_failure().map(|()| prepared));
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            crate::remote_host::files::rollback_claimed(&attachments);
            return Err(error);
        }
    };
    // Notify frontend: new thread/run appeared (trigger list refresh).
    crate::emit_remote_activity(&thread.id);
    Ok(prepared)
}

/// Derive a thread title from the first message, matching the GUI new-chat
/// draft (`deriveThreadTitle`): collapse whitespace, take 28 chars, ellipsize.
/// Empty input falls back to the default chat title so the row isn't blank.
pub(crate) fn derive_thread_title(content: &str) -> String {
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let compact = compact.trim();
    if compact.is_empty() {
        return "New Chat".to_string();
    }
    let chars: Vec<char> = compact.chars().collect();
    if chars.len() > 28 {
        format!("{}...", chars.into_iter().take(28).collect::<String>())
    } else {
        compact.to_string()
    }
}
