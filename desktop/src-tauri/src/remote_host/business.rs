//! Desktop business adapter. No socket, subscription or Tauri AppHandle enters this API.
use crate::remote::protocol::IncomingCmd;
use crate::remote::services::ReplySink;
use serde_json::{json, Value};
async fn product_sandbox_available() -> Result<bool, crate::AppError> {
    #[cfg(target_os = "macos")]
    {
        Ok(true)
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        crate::agent_bridge::probe_sandbox()
            .await
            .map(|result| result.available)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Ok(false)
    }
}

pub(crate) async fn execute(cmd: IncomingCmd, sink: &dyn ReplySink) {
    match cmd.cmd_type.as_str() {
        "list_sessions" => {
            let snapshot = crate::remote_host::catalog::sessions("");
            match snapshot {
                Some((payload, _)) => reply(sink, true, payload, None).await,
                None => reply(sink, false, Value::Null, Some("catalog_unavailable")).await,
            }
        }
        "list_workspaces" => {
            let snapshot = crate::remote_host::catalog::workspaces();
            match snapshot {
                Some((payload, _)) => reply(sink, true, payload, None).await,
                None => reply(sink, false, Value::Null, Some("catalog_unavailable")).await,
            }
        }
        "get_messages" => {
            // Serve history from the agent (source of truth for all sessions).
            // The GUI store only has message rows for GUI-native threads —
            // TUI/CLI sessions imported as thread stubs would show empty history.
            // Fall back to the store when the agent is unreachable.
            //
            // The whole history is fetched locally (gRPC/store have no payload
            // limit) then paged here, because NATS rejects any single reply over
            // the 1MB user-JWT payload cap — a long session's full history would
            // otherwise fail silently (client times out with no response).
            let offset = cmd.offset.max(0) as usize;
            let limit = if cmd.limit > 0 {
                cmd.limit as usize
            } else {
                DEFAULT_MESSAGE_PAGE_LIMIT
            };
            let messages = match crate::agent_bridge::get_session_messages(cmd.session_id.clone())
                .await
            {
                Ok(data) => messages_vec(data),
                Err(agent_err) => {
                    reply(
                            sink,
                            false,
                            Value::Null,
                            Some(&format!(
                                "{agent_err}; conversation history is unavailable while the Agent is offline"
                            )),
                        )
                        .await;
                    return;
                }
            };
            reply(sink, true, paginate_messages(messages, offset, limit), None).await;
        }
        "get_session_entries" => {
            // Display-shaped history (plain-text content + per-entry meta with
            // user attachments) for clients that render attachment chips.
            // Paged for the same NATS payload cap as get_messages.
            let offset = cmd.offset.max(0) as usize;
            let limit = if cmd.limit > 0 {
                cmd.limit as usize
            } else {
                DEFAULT_MESSAGE_PAGE_LIMIT
            };
            if let Some(before) = cmd.before {
                match crate::agent_bridge::get_session_entries_before(
                    cmd.session_id.clone(),
                    before,
                    limit as i64,
                )
                .await
                {
                    Ok(data) => {
                        let page = prepare_backward_entries_page(&cmd.session_id, data);
                        reply(sink, true, page, None).await;
                    }
                    Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
                }
                return;
            }
            match crate::agent_bridge::get_session_entries(cmd.session_id.clone()).await {
                Ok(data) => {
                    let entries = entries_vec(data);
                    reply(
                        sink,
                        true,
                        paginate_items(entries, offset, limit, "entries"),
                        None,
                    )
                    .await;
                }
                Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "get_events_since" => {
            // P1c: replay buffered events for the current in-progress run, so late-joining clients can catch up on missed prefix events.
            let offset = cmd.offset.max(0) as usize;
            let limit = if cmd.limit > 0 {
                cmd.limit as usize
            } else {
                DEFAULT_MESSAGE_PAGE_LIMIT
            };
            match crate::agent_bridge::get_events_since(
                cmd.session_id.clone(),
                cmd.run_id.clone(),
                cmd.since_idx,
            )
            .await
            {
                Ok(mut data) => {
                    let source_watermark = data["events"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|event| event["idx"].as_i64())
                        .max()
                        .unwrap_or(cmd.since_idx)
                        .max(data["projection"]["cursor"].as_i64().unwrap_or(-1));
                    let watermark = cmd.replay_until_idx.unwrap_or(source_watermark);
                    if data["projection"]["cursor"]
                        .as_i64()
                        .is_some_and(|cursor| cursor > watermark)
                    {
                        reply(sink, false, Value::Null, Some("replay_window_changed")).await;
                        return;
                    }
                    if let Some(events) = data["events"].as_array_mut() {
                        events.retain(|event| {
                            event["idx"].as_i64().is_none_or(|idx| idx <= watermark)
                        });
                    }
                    let mut page = paginate_events(data, offset, limit);
                    page["watermark"] = json!(watermark);
                    page["nextSinceIdx"] = page["events"]
                        .as_array()
                        .and_then(|events| events.last())
                        .and_then(|event| event.get("idx"))
                        .cloned()
                        .unwrap_or(json!(cmd.since_idx));
                    reply(sink, true, page, None).await;
                }
                Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "upload_init" => {
            match super::files::init_upload(
                &cmd.name,
                &cmd.transfer_name,
                &cmd.mime_type,
                &cmd.kind,
                cmd.original_size,
                cmd.transfer_size,
            ) {
                Ok(data) => {
                    reply(
                        sink,
                        true,
                        serde_json::to_value(data).unwrap_or(Value::Null),
                        None,
                    )
                    .await
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "upload_complete" => match super::files::complete_upload(&cmd.transfer_id) {
            Ok(data) => {
                reply(
                    sink,
                    true,
                    serde_json::to_value(data).unwrap_or(Value::Null),
                    None,
                )
                .await
            }
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "upload_cancel" => reply_unit(sink, super::files::cancel_upload(&cmd.transfer_id)).await,
        "download_prepare" => {
            match super::files::prepare_download_variant(&cmd.session_id, &cmd.file_path, &cmd.mode)
                .await
            {
                Ok(data) => {
                    reply(
                        sink,
                        true,
                        serde_json::to_value(data).unwrap_or(Value::Null),
                        None,
                    )
                    .await
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "download_cancel" => {
            super::files::cancel_download(&cmd.transfer_id);
            reply(sink, true, json!({}), None).await;
        }
        "prompt" => {
            // The command id is persisted on the run. This lookup survives the
            // in-memory reply cache, mobile process death, and desktop restart.
            // A retry therefore returns the original receipt without executing
            // the user's prompt twice.
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
            // Lazy creation (matches the GUI new-chat flow): the web client's
            // "new" button only stages a local draft and sends the first message
            // with an empty `session_id`. Here an empty/unknown id creates the
            // thread + a real agent session on the fly, so the accept-ack can
            // carry the identifiers the events will be published under and the
            // client can latch onto the real session id. Model / thinking level
            // travel with the first prompt so the freshly-created session is
            // seeded with the user's draft selections.
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
                Ok(prepared) => {
                    let ack = json!({
                        "sessionId": prepared.session_id,
                        "threadId": prepared.thread_id,
                        "runId": prepared.run_id,
                    });
                    // Actual execution runs in the background (completion visible via event stream agent_end).
                    tokio::spawn(async move {
                        let thread_id = prepared.thread_id.clone();
                        if let Err(e) = crate::agent_bridge::run_prepared_prompt(prepared).await {
                            eprintln!("remote: prompt processing failed: {e}");
                        }
                        crate::emit_remote_activity(&thread_id);
                    });
                    reply(sink, true, ack, None).await;
                }
                Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
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
            // Continuations create a normal persisted run with this command id
            // as its trigger. Consult that durable receipt before doing any
            // work so retries remain idempotent after reply-cache expiry,
            // mobile process death, or a desktop restart.
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
            // Resume a failed run: synthesize a continue prompt from the run's
            // recent terminal events and push it through the normal prompt
            // pipeline (model/thinking default to the session's current values).
            let prompt = build_continue_prompt(&cmd.run_id);
            match prepare_remote_prompt(
                &cmd.session_id,
                prompt,
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
                Ok(prepared) => {
                    let ack = json!({
                        "sessionId": prepared.session_id,
                        "threadId": prepared.thread_id,
                        "runId": prepared.run_id,
                    });
                    tokio::spawn(async move {
                        let thread_id = prepared.thread_id.clone();
                        if let Err(e) = crate::agent_bridge::run_prepared_prompt(prepared).await {
                            eprintln!("remote: continue_run failed: {e}");
                        }
                        crate::emit_remote_activity(&thread_id);
                    });
                    reply(sink, true, ack, None).await;
                }
                Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
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
        "get_state" => match crate::agent_bridge::get_session_state(cmd.session_id.clone()).await {
            Ok(data) => reply(sink, true, data, None).await,
            Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
        },
        "list_models" | "get_available_models" => {
            match crate::agent_bridge::get_available_models().await {
                Ok(data) => reply(sink, true, data, None).await,
                Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "set_model" => {
            reply_unit(
                sink,
                crate::agent_bridge::set_session_model(
                    cmd.session_id.clone(),
                    qualified_model_id(&cmd.model_id, &cmd.provider_id).unwrap_or_default(),
                )
                .await,
            )
            .await;
        }
        "set_thinking_level" => {
            reply_unit(
                sink,
                crate::agent_bridge::set_session_thinking_level(
                    cmd.session_id.clone(),
                    cmd.level.clone(),
                )
                .await,
            )
            .await;
        }
        "get_settings" => {
            match crate::store::get_app_settings() {
                Ok(settings) => {
                    let sandbox_available = match product_sandbox_available().await {
                        Ok(available) => available,
                        Err(error) => {
                            reply(sink, false, Value::Null, Some(&error.to_string())).await;
                            return;
                        }
                    };
                    reply(
                        sink,
                        true,
                        json!({
                            "approvalTier": settings.approval_tier,
                            // Windows is exposed only when the real host probe
                            // passes. The phone never receives paths or native
                            // diagnostics.
                            "sandboxAvailable": sandbox_available,
                        }),
                        None,
                    )
                    .await
                }
                Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "set_approval_tier" => {
            // The tier is a global app preference (not per-session): writing it
            // here is the same as flipping it in the desktop Settings. It takes
            // effect on the next session establishment, where the bridge pushes
            // it to the agent via `set_agent_sandbox_policy`.
            let tier = if cmd.tier == "sandbox" {
                match product_sandbox_available().await {
                    Ok(true) => cmd.tier.clone(),
                    Ok(false) => "manual".to_string(),
                    Err(error) => {
                        reply(sink, false, Value::Null, Some(&error.to_string())).await;
                        return;
                    }
                }
            } else {
                cmd.tier.clone()
            };
            match crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
                approval_tier: Some(tier),
                ..Default::default()
            }) {
                Ok(settings) => {
                    reply(
                        sink,
                        true,
                        json!({ "approvalTier": settings.approval_tier }),
                        None,
                    )
                    .await
                }
                Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "set_session_name" => {
            match crate::agent_bridge::rename_session(cmd.session_id.clone(), cmd.name.clone())
                .await
            {
                Ok(()) => {
                    if let Ok(Some(thread)) =
                        crate::store::find_thread_by_agent_session(&cmd.session_id)
                    {
                        crate::emit_remote_activity(&thread.id);
                    }
                    reply(sink, true, json!({}), None).await
                }
                Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "set_session_pinned" => {
            match crate::store::pin_thread(crate::store::PinThreadInput {
                thread_id: cmd.thread_id.clone(),
                pinned: cmd.pinned,
            }) {
                Ok(_) => {
                    crate::emit_remote_activity(&cmd.thread_id);
                    reply(sink, true, json!({}), None).await
                }
                Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "delete_session" => {
            // Matches the desktop single-thread delete: the session record is
            // removed (and, when it is the only owner, the agent session too),
            // but the temporary chat workspace files are kept (delete_files =
            // false). Only reachable with a non-empty thread id from the remote
            // client; a missing id is a malformed request, not a deletion.
            if cmd.thread_id.is_empty() {
                reply(sink, false, Value::Null, Some("missing thread_id")).await;
            } else {
                match crate::store::delete_thread_with_files(&cmd.thread_id, false) {
                    Ok(_) => {
                        crate::emit_remote_activity(&cmd.thread_id);
                        reply(sink, true, json!({}), None).await
                    }
                    Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
                }
            }
        }
        "delete_workspace" => {
            // Phone parity with the desktop sidebar's workspace delete: reuse the
            // exact command the GUI calls, so the store cascade, the agent-session
            // delete outbox and the orphaned scratch/review dirs are handled
            // identically. The user's own files at `workspace.path` are never
            // touched. A missing id is a malformed request, not a deletion.
            if cmd.workspace_id.is_empty() {
                reply(sink, false, Value::Null, Some("missing workspace_id")).await;
            } else {
                match crate::commands::delete_workspace(cmd.workspace_id.clone()).await {
                    Ok(_) => {
                        // The workspace and every thread in it are gone; the GUI
                        // sidebar re-lists from this bare invalidation. The phone's
                        // catalogue converges through the dirty-flag workspace push.
                        crate::emit_threads_updated();
                        reply(sink, true, json!({}), None).await
                    }
                    Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
                }
            }
        }
        other => {
            reply(
                sink,
                false,
                Value::Null,
                Some(&format!("Unsupported command: {other}")),
            )
            .await;
        }
    }
}

async fn reply(sink: &dyn ReplySink, success: bool, data: Value, error: Option<&str>) {
    sink.send(success, data, error.map(str::to_owned)).await;
}
async fn reply_unit(sink: &dyn ReplySink, result: Result<(), crate::AppError>) {
    match result {
        Ok(()) => reply(sink, true, json!({}), None).await,
        Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
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
    pub(crate) upload_references: Vec<super::files::UploadReference>,
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
    let attachments = super::files::claim_uploads(&upload_references, &thread.id)?;
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
            super::files::rollback_claimed(&attachments);
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

/// Reply budget for a `get_messages` page: comfortably under NATS's 1MB
/// user-JWT payload limit, leaving headroom for the reply envelope.
pub(crate) const MESSAGES_PAGE_BYTES: usize = 512 * 1024;
/// Backward mobile history keeps the requested ten-exchange semantic maximum,
/// with the same 512 KiB wire budget as other remote history pages. Complete
/// oldest exchanges are deferred only when an unusually content-heavy ten-turn
/// page would exceed that budget; a page never splits an exchange.
pub(crate) const BACKWARD_HISTORY_PAGE_BYTES: usize = 512 * 1024;
/// A single persisted message can embed a huge tool result; cap its content so
/// one oversized message can't push a page past the payload limit on its own.
pub(crate) const MESSAGE_CONTENT_CAP_BYTES: usize = 256 * 1024;
/// Default page size when the client doesn't ask for one.
pub(crate) const DEFAULT_MESSAGE_PAGE_LIMIT: usize = 100;

/// Extract the `messages` array from an agent `get_messages` reply.
pub(crate) fn messages_vec(data: Value) -> Vec<Value> {
    data.get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Extract the `entries` array from an agent `get_session_entries` reply.
pub(crate) fn entries_vec(data: Value) -> Vec<Value> {
    data.get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn paginate_messages(messages: Vec<Value>, offset: usize, limit: usize) -> Value {
    paginate_items(messages, offset, limit, "messages")
}

/// Apply the remote wire caps to a backward Agent page without losing its
/// cursor. If ten unusually large exchanges exceed the NATS page budget, drop
/// complete oldest exchanges until the page fits and advance the returned
/// cursor past those omitted rows; they remain reachable on the next pull.
pub(crate) fn prepare_backward_entries_page(_session_id: &str, data: Value) -> Value {
    let mut entries = entries_vec(data.clone());
    for entry in &mut entries {
        cap_remote_item(entry, MESSAGE_CONTENT_CAP_BYTES);
    }
    let agent_start = data
        .get("nextOffset")
        .or_else(|| data.get("next_offset"))
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize;
    let agent_has_more = data
        .get("hasMore")
        .or_else(|| data.get("has_more"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut removed = 0usize;
    while serde_json::to_vec(&entries).map_or(0, |bytes| bytes.len()) > BACKWARD_HISTORY_PAGE_BYTES
    {
        let Some(next_user) = entries
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(index, entry)| {
                (entry.get("role").and_then(Value::as_str) == Some("user")).then_some(index)
            })
        else {
            break;
        };
        entries.drain(..next_user);
        removed += next_user;
    }
    let next_offset = agent_start.saturating_add(removed);
    json!({
        "offset": next_offset,
        "nextOffset": next_offset,
        "hasMore": agent_has_more || removed > 0,
        "entries": entries,
    })
}

/// Page a full item list into a reply that fits the NATS payload cap.
///
/// Each item is content-capped first (so no single item is huge), then items
/// are accumulated from `offset` until the serialized page would exceed
/// [`MESSAGES_PAGE_BYTES`] or `limit` is reached (always at least one item —
/// it's already capped). Returns the page (under `key`) plus cursor fields the
/// client uses to fetch the remainder.
pub(crate) fn paginate_items(
    mut items: Vec<Value>,
    offset: usize,
    limit: usize,
    key: &str,
) -> Value {
    for item in items.iter_mut() {
        cap_remote_item(item, MESSAGE_CONTENT_CAP_BYTES);
    }
    let total = items.len();
    let start = offset.min(total);
    let mut end = start;
    let mut bytes = 0usize;
    for (index, item) in items.iter().skip(start).enumerate() {
        let size = serde_json::to_vec(item)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        if index > 0 && (index >= limit || bytes + size > MESSAGES_PAGE_BYTES) {
            break;
        }
        bytes += size;
        end += 1;
    }
    let page: Vec<Value> = items.drain(start..end).collect();
    let mut value = json!({
        "offset": start,
        "nextOffset": end,
        "total": total,
        "hasMore": end < total,
    });
    value[key] = json!(page);
    value
}

/// Page a session's replay event tail into a reply that fits the NATS payload
/// cap, mirroring `paginate_items` (each event's `data` is capped, then events
/// accumulate until the page would exceed [`MESSAGES_PAGE_BYTES`]). The reply
/// keeps the envelope's non-event fields (`runId`, `projection`, `truncated`)
/// on every page so the client can distinguish a ring-overflow projection from
/// a plain tail replay regardless of which page it lands on.
pub(crate) fn paginate_events(mut data: Value, offset: usize, limit: usize) -> Value {
    let run_id = data.get("runId").cloned().unwrap_or(Value::Null);
    let projection = data.get("projection").cloned().unwrap_or(Value::Null);
    let truncated = data.get("truncated").cloned().unwrap_or(Value::Null);
    let events = data
        .get_mut("events")
        .and_then(Value::as_array_mut)
        .map(std::mem::take)
        .unwrap_or_default();
    let mut page = paginate_items(events, offset, limit, "events");
    if !run_id.is_null() {
        page["runId"] = run_id;
    }
    if !projection.is_null() {
        page["projection"] = projection;
    }
    if !truncated.is_null() {
        page["truncated"] = truncated;
    }
    page
}

/// Bound presentation payloads without changing the stored record.
pub(crate) fn truncate_message_content(message: &mut Value, cap: usize) {
    if serialized_len(message) <= cap {
        return;
    }
    if let Some(blocks) = message.get_mut("blocks").and_then(Value::as_array_mut) {
        let mut remaining = cap;
        for block in blocks {
            if let Some(Value::String(text)) = block.get_mut("text") {
                let (end, truncated) = byte_cut(text, remaining);
                if truncated {
                    let mut cut = text[..end].to_owned();
                    cut.push('…');
                    *text = cut;
                }
                remaining = remaining.saturating_sub(text.len());
            }
        }
    } else if let Some(Value::String(data)) = message.get_mut("data") {
        if data.len() > cap {
            *data = json!({"_truncated":true,"bytes":data.len()}).to_string();
        }
    }
}

pub(crate) fn cap_remote_item(item: &mut Value, cap: usize) {
    truncate_message_content(item, cap.saturating_sub(16 * 1024));
    if serialized_len(item) <= cap {
        return;
    }
    if let Some(blocks) = item.get_mut("blocks").and_then(Value::as_array_mut) {
        for block in blocks {
            if let Some(arguments) = block.get_mut("arguments") {
                let bytes = serialized_len(arguments);
                if bytes > 8 * 1024 {
                    *arguments = json!({"truncated":true,"bytes":bytes});
                }
            }
        }
    }
    if serialized_len(item) <= cap {
        return;
    }
    let original_bytes = serialized_len(item);
    if item.get("blocks").is_some() {
        let mut replacement = serde_json::Map::new();
        for key in ["id", "role", "kind", "runId", "createdAtMs", "usage", "run"] {
            if let Some(value) = item.get(key) {
                replacement.insert(key.into(), value.clone());
            }
        }
        if let Some(checkpoint) = item.get("checkpoint").and_then(Value::as_object) {
            let minimal: serde_json::Map<String, Value> =
                ["checkpointId", "tokensBefore", "tokensAfter", "trigger"]
                    .into_iter()
                    .filter_map(|key| checkpoint.get(key).map(|value| (key.into(), value.clone())))
                    .collect();
            replacement.insert("checkpoint".into(), Value::Object(minimal));
        }
        replacement.insert(
            "metadata".into(),
            json!({"remoteTruncated":true,"originalBytes":original_bytes}),
        );
        replacement.insert(
            "blocks".into(),
            json!([{"kind":"text","text":"[…远程条目过大，已截断；完整内容见本机会话…]"}]),
        );
        *item = Value::Object(replacement);
    } else if let Some(object) = item.as_object_mut() {
        object.insert(
            "data".into(),
            Value::String(json!({"_truncated":true,"bytes":original_bytes}).to_string()),
        );
    }
}

pub(crate) fn serialized_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(0, |bytes| bytes.len())
}

/// Return a byte index at a char boundary, not exceeding `max_bytes`, and
/// whether the string had to be cut.
pub(crate) fn byte_cut(text: &str, max_bytes: usize) -> (usize, bool) {
    if text.len() <= max_bytes {
        return (text.len(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (end, true)
}
