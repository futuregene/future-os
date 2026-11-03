use future_remote_protocol::IncomingCmd;
use future_remote_server::services::ReplySink;
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
            let busy_policy = match parse_busy_policy(&cmd.busy_policy) {
                Ok(policy) => policy,
                Err(error) => {
                    reply(sink, false, Value::Null, Some(&error.to_string())).await;
                    return;
                }
            };
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
                    busy_policy,
                },
            )
            .await
            {
                Ok(prepared) => acknowledge_prepared(prepared, sink, "prompt processing").await,
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "get_queue" => match crate::agent_bridge::session_queue_snapshot(&cmd.session_id).await {
            Ok(snapshot) => reply(sink, true, snapshot, None).await,
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "cancel_queued_run" => {
            reply_unit(
                sink,
                crate::agent_bridge::cancel_queued_run(&cmd.session_id, &cmd.run_id)
                    .await
                    .map(|_| ()),
            )
            .await
        }
        "release_queued_run" => {
            reply_unit(
                sink,
                crate::agent_bridge::release_queued_run(&cmd.session_id, &cmd.run_id)
                    .await
                    .map(|_| ()),
            )
            .await
        }
        "get_prompt_receipt" => match remote_prompt_receipt(&cmd.prompt_id) {
            Ok(receipt) => reply(sink, true, receipt.unwrap_or(Value::Null), None).await,
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "abort" => {
            reply_unit(
                sink,
                crate::agent_bridge::abort_session(&cmd.session_id)
                    .await
                    .map(|_| ()),
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
                    // Continuing a run is a fresh turn of its own; it never
                    // overtakes a queue the reader already built.
                    busy_policy: String::new(),
                },
            )
            .await
            {
                Ok(prepared) => acknowledge_prepared(prepared, sink, "continue_run").await,
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "approval_decision" => {
            let ownership = (|| -> Result<(), crate::Error> {
                let approval = crate::store::get_approval_request(&cmd.entry_id)?
                    .ok_or_else(|| "Approval request could not be loaded.".to_string())?;
                let thread = crate::store::get_thread(&approval.thread_id)?
                    .ok_or_else(|| "Approval thread could not be loaded.".to_string())?;
                let owner_session_id = thread.agent_session_id.unwrap_or(thread.id);
                if cmd.session_id != owner_session_id {
                    return Err(crate::Error::Message(
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
    crate::runtime::spawn(async move {
        let thread_id = prepared.thread_id.clone();
        if let Err(error) =
            crate::agent_bridge::run_prepared_prompt_with_acceptance(prepared, accepted_tx).await
        {
            tracing::error!(event = "acknowledge_prepared", failure_context = %failure_context, error = %error,"remote: {failure_context} failed: {error}");
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
pub(crate) fn injected_prepare_failure() -> Result<(), crate::Error> {
    INJECT_PREPARE_FAILURE
        .swap(false, std::sync::atomic::Ordering::Relaxed)
        .then(|| crate::Error::Message("injected prepare failure".to_string()))
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

/// The stable token a refused continuation carries, so a client can say *why*
/// instead of reporting a generic send failure (the same pattern as
/// [`UNKNOWN_SESSION_CODE`]). Prefixed to the human sentence: the phone's toast
/// maps the token, the text stays readable in the desktop's own logs.
pub(crate) const CONTINUE_REFUSED_CODE: &str = "continue_unavailable";

fn continue_refused(reason: &str) -> crate::Error {
    format!("{CONTINUE_REFUSED_CODE}: {reason}").into()
}

/// A continuation is not a free-form prompt alias: it may only resume a run
/// that belongs to the addressed session. This keeps a stale mobile outbox entry
/// (including one from a previous pairing) from creating or steering an
/// unrelated conversation.
///
/// **Identity only — never the reason the run stopped.** Any run of this
/// conversation can be picked up from: a failure, a user stop, a stream the
/// Agent or the app cut off, a restart that interrupted it. The rule used to
/// read "only a `failed` run can be continued", which turned the one thing a
/// client can offer after a restart (Continue — startup convergence settles
/// that run `cancelled`/`interrupted`, not `failed`) into a permanent dead end,
/// and asked the server to judge a decision the user had already made by tapping
/// the button. The desktop's own Continue never consulted the run at all: it
/// re-sends a prompt either way.
pub(crate) fn validate_continue_source(session_id: &str, run_id: &str) -> Result<(), crate::Error> {
    if session_id.trim().is_empty() || run_id.trim().is_empty() {
        return Err(continue_refused(
            "A continuation requires a session and source run.",
        ));
    }
    let run = crate::store::get_run(run_id)?
        .ok_or_else(|| continue_refused("The source run no longer exists."))?;
    let thread = crate::store::get_thread(&run.thread_id)?
        .ok_or_else(|| continue_refused("The source run's conversation no longer exists."))?;
    let run_session_id = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&thread.id);
    if run_session_id != session_id {
        return Err(continue_refused(
            "The source run does not belong to this session.",
        ));
    }
    Ok(())
}

pub(crate) fn remote_prompt_receipt(command_id: &str) -> Result<Option<Value>, crate::Error> {
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
    /// What this send means while the session still runs something, as an Agent
    /// `BusyPolicy` value. Empty keeps the refusal below.
    pub(crate) busy_policy: String,
}

/// The policy values a Remote client may ask for.
///
/// `enqueue_if_busy` and `enqueue_coalescing` both mean "hold it, do not drop
/// it": the Agent keeps a durable receipt and runs the prompt at the turn
/// boundary (`coalescing` folds the consecutive ones into a single run, which is
/// how a queue leaves as one message). `supersede_session` means "interrupt the
/// run and do this instead". Anything else is a client bug, and answering it
/// with the old refusal would hide it.
pub(crate) fn parse_busy_policy(raw: &str) -> Result<String, crate::Error> {
    let policy = raw.trim();
    if policy.is_empty() {
        return Ok(String::new());
    }
    if matches!(
        policy,
        "enqueue_if_busy" | "enqueue_coalescing" | "supersede_session"
    ) {
        return Ok(policy.to_string());
    }
    Err(crate::Error::Message(format!(
        "unknown busy policy `{policy}`; expected one of: enqueue_if_busy, enqueue_coalescing, supersede_session"
    )))
}

/// Whether this policy has the Agent hold the prompt instead of refusing it.
pub(crate) fn queues_while_busy(policy: &str) -> bool {
    matches!(policy, "enqueue_if_busy" | "enqueue_coalescing")
}

/// The stable token a refusal of an unknown session carries, so a client can
/// recognize *why* the prompt failed instead of showing a generic send error.
/// Prefixed to the human sentence: the phone's toast maps the token, the text
/// stays readable in the desktop's own logs.
pub(crate) const UNKNOWN_SESSION_CODE: &str = "session_not_on_desktop";

/// The same token for a conversation whose run is still in flight. The client
/// no longer guesses at this locally (its own "still streaming" flag outlives
/// the run it described), so the refusal has to arrive with a reason it can
/// turn into a sentence instead of "message failed to send".
pub(crate) const SESSION_BUSY_CODE: &str = "session_busy";

/// Find the thread for `session_id`, then persist user message + run via
/// `agent_bridge::headless`.
///
/// An **empty** `session_id` is a client's new-conversation draft, and the
/// thread is born with the first message (lazy creation): titled from the
/// message, and given a real agent session id up front so the ack, the event
/// subjects and history all agree from the start (no empty row, no id drift).
///
/// A **non-empty** id no thread here owns is refused. Creating a thread from it
/// would answer a message addressed to conversation A with a brand new
/// conversation B — on whichever machine happens to be connected. The user's
/// text lands somewhere they were not looking, under a title derived from it,
/// while the conversation they meant to continue never sees it; and because
/// nothing fails, nothing tells them. Naming a conversation this desktop does
/// not have means the client is reading a list that is not this machine's (a
/// merged row from another desktop) or that the conversation has since been
/// deleted — both want an error at the sender.
pub(crate) async fn prepare_remote_prompt(
    session_id: &str,
    message: String,
    options: RemotePromptOptions,
) -> Result<crate::agent_bridge::PreparedPrompt, crate::Error> {
    let RemotePromptOptions {
        model_id,
        thinking_level,
        mode,
        workspace_id,
        upload_references,
        command_id,
        busy_policy,
    } = options;
    let thread = match crate::store::find_thread_by_agent_session(session_id)? {
        Some(thread) => thread,
        None if !session_id.trim().is_empty() => {
            return Err(crate::Error::Message(format!(
                "{UNKNOWN_SESSION_CODE}: this conversation is not on this desktop. \
                 Open it from the machine that owns it, or start a new conversation."
            )));
        }
        None => {
            // Lazy creation: the thread is born with the first message, titled
            // from it (mirrors the GUI new-chat draft), and immediately gets a
            // real agent session id so the ack, the event subjects, and history
            // all agree from the start (no empty row, no id drift).
            let mut input = if mode == "workspace" {
                if workspace_id.trim().is_empty() {
                    return Err(crate::Error::Message(
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
    // A session that is already running takes a prompt only from a client that
    // said what that means. Two answers, and the difference is who decided:
    //
    //  * nothing said — the legacy refusal ("wait or abort first"). A client
    //    that knows nothing about queueing must not have its message land behind
    //    a run it never saw, so it is told to come back.
    //  * `enqueue_*` — the Agent holds it and runs it at the turn boundary, on a
    //    durable receipt, so this is a delivery and not a phantom message. The
    //    check below is skipped on purpose.
    //
    // Residual race: two clients prompting the same idle session within
    // milliseconds can both pass this check; the Agent's own admission decides.
    let resolved_session_id = thread
        .agent_session_id
        .clone()
        .unwrap_or_else(|| thread.id.clone());
    if !queues_while_busy(&busy_policy)
        && crate::store::active_run_sessions()?
            .iter()
            .any(|active| active == &resolved_session_id)
    {
        return Err(crate::Error::Message(format!(
            "{SESSION_BUSY_CODE}: this session is still running; wait for it to finish or abort it first."
        )));
    }
    let attachments = crate::remote_host::files::claim_uploads(&upload_references, &thread.id)?;
    let prepared = crate::agent_bridge::prepare_prompt_persisted_busy(
        &thread,
        message,
        model_id,
        thinking_level,
        attachments.clone(),
        (!command_id.trim().is_empty()).then_some(command_id),
        busy_policy,
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
/// draft (`deriveThreadTitle`): plain text, whitespace collapsed, 28 chars,
/// ellipsized. Empty input falls back to the default chat title so the row
/// isn't blank.
///
/// The plain text matters here: a phone message may open with a reference to
/// another conversation (`[标题](futureos://session/<id>)`), and a title made of
/// that link names this conversation after the one it points at.
pub(crate) fn derive_thread_title(content: &str) -> String {
    let compact = future_rpc::title::plain_title_text(content);
    if compact.is_empty() {
        return "New Chat".to_string();
    }
    let chars: Vec<char> = compact.chars().collect();
    if chars.len() > 28 {
        format!("{}...", chars.into_iter().take(28).collect::<String>())
    } else {
        compact
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A policy a client names is a decision the Agent acts on, so the three
    /// values are passed through and nothing else is guessed at.
    #[test]
    fn only_the_three_agent_policies_are_accepted_from_a_client() {
        for policy in ["enqueue_if_busy", "enqueue_coalescing", "supersede_session"] {
            assert_eq!(parse_busy_policy(policy).expect("a known policy"), policy);
        }
        // Absent is the legacy client, and means "ask me again when it is free".
        assert_eq!(parse_busy_policy("").expect("absent is allowed"), "");
        assert_eq!(parse_busy_policy("  ").expect("blank is absent"), "");
        let error = parse_busy_policy("queue_it")
            .expect_err("an unknown policy is a bug")
            .to_string();
        assert!(error.contains("queue_it"), "{error}");
        // Two of them have the Agent hold the prompt; superseding does not.
        assert!(queues_while_busy("enqueue_if_busy"));
        assert!(queues_while_busy("enqueue_coalescing"));
        assert!(!queues_while_busy("supersede_session"));
        assert!(!queues_while_busy(""));
    }

    /// The same busy session, two answers: a client that said nothing is told to
    /// come back, and one that named a queueing policy has its message held. The
    /// second is the whole point — the refusal is about *who decided*, not about
    /// the session being busy.
    #[tokio::test]
    async fn a_busy_session_holds_a_named_send_and_refuses_an_unnamed_one() {
        use crate::remote::test_support::{raw_store_connection, HomeGuard};

        let _home = HomeGuard::new("business-prompt-busy-policy");
        crate::remote::test_support::init_store();
        {
            let conn = raw_store_connection();
            conn.execute_batch(
                "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                     VALUES ('ws_busy', 'WS', 'user', '/tmp/ws', 1, 1);
                 INSERT INTO threads (id, workspace_id, mode, title, agent_session_id,
                                      created_at, updated_at)
                     VALUES ('th_busy', 'ws_busy', 'chat', 'T', 'sess_busy', 1, 1);
                 INSERT INTO runs (id, thread_id, status, created_at, updated_at)
                     VALUES ('run_busy', 'th_busy', 'running', 1, 1);",
            )
            .expect("seed a running conversation");
        }

        let options = |policy: &str| RemotePromptOptions {
            model_id: None,
            thinking_level: None,
            mode: "chat".to_string(),
            workspace_id: String::new(),
            upload_references: Vec::new(),
            command_id: "cmd-busy".to_string(),
            busy_policy: policy.to_string(),
        };

        // `expect_err` would need `PreparedPrompt: Debug`; the refusal is what
        // this asserts, so it is matched directly.
        let refused =
            match prepare_remote_prompt("sess_busy", "next".to_string(), options("")).await {
                Ok(_) => panic!("an unnamed send must be refused while the run is live"),
                Err(error) => error.to_string(),
            };
        assert!(
            refused.starts_with(SESSION_BUSY_CODE),
            "the client maps this code to its own sentence: {refused}"
        );

        for policy in ["enqueue_if_busy", "enqueue_coalescing"] {
            let prepared =
                match prepare_remote_prompt("sess_busy", "held".to_string(), options(policy)).await
                {
                    Ok(prepared) => prepared,
                    Err(error) => panic!("{policy} must be accepted: {error}"),
                };
            assert_eq!(prepared.session_id, "sess_busy");
        }
    }

    /// A continuation without a session and source run would resume "whatever
    /// was last used", which is not something the user asked for. Both ids are
    /// required, and the guard runs before any store lookup so a stale outbox
    /// entry cannot reach the database at all.
    #[test]
    fn a_continuation_needs_both_a_session_and_a_source_run() {
        for (session, run) in [("", "run"), ("  ", "run"), ("sess", ""), ("sess", "\t")] {
            let error = validate_continue_source(session, run)
                .expect_err("a missing id must be refused")
                .to_string();
            assert!(
                error.contains("requires a session and source run"),
                "session {session:?} run {run:?} -> {error}"
            );
        }
    }

    /// Every reason a run can stop is resumable — the guard is identity, not
    /// outcome. The old reading ("only a `failed` run can be continued") made
    /// the phone's Continue a dead end for exactly the run a restart leaves
    /// behind (`cancelled`/`interrupted`), while the desktop's own Continue
    /// re-sent a prompt without ever consulting the run.
    #[test]
    fn every_terminal_run_of_the_session_is_a_continuation_source() {
        use crate::remote::test_support::{raw_store_connection, HomeGuard};

        let _home = HomeGuard::new("business-prompt-continue-any-reason");
        crate::remote::test_support::init_store();
        {
            let conn = raw_store_connection();
            conn.execute_batch(
                "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                     VALUES ('ws_rst', 'WS', 'user', '/tmp/ws', 1, 1);
                 INSERT INTO threads (id, workspace_id, mode, title, agent_session_id,
                                      created_at, updated_at)
                     VALUES ('th_rst', 'ws_rst', 'chat', 'T', 'sess_rst', 1, 1);
                 INSERT INTO runs (id, thread_id, status, error_message, error_type,
                                   created_at, updated_at) VALUES
                     ('run_failed', 'th_rst', 'failed', 'boom', 'model_failed', 1, 1),
                     ('run_interrupted', 'th_rst', 'cancelled',
                      'Interrupted by application restart.', 'interrupted', 1, 1),
                     ('run_completed', 'th_rst', 'completed', NULL, NULL, 1, 1),
                     ('run_stopped', 'th_rst', 'cancelled', 'Run was cancelled.', 'cancelled', 1, 1);",
            )
            .expect("seed the source runs");
        }

        for run_id in [
            "run_failed",
            "run_interrupted",
            "run_completed",
            "run_stopped",
        ] {
            validate_continue_source("sess_rst", run_id)
                .unwrap_or_else(|error| panic!("{run_id} must be continuable: {error}"));
        }

        // The one thing still refused: a run that belongs to another
        // conversation, which would steer text into a chat the user is not in.
        let error = validate_continue_source("another-session", "run_failed")
            .expect_err("a foreign run is not a continuation source")
            .to_string();
        assert!(
            error.starts_with(CONTINUE_REFUSED_CODE),
            "the phone maps this token to its own text: {error}"
        );
        assert!(error.contains("does not belong"), "{error}");
    }

    /// A prompt addressed to a conversation this desktop does not have must be
    /// refused, not answered with a conversation of its own. The old lazy
    /// creation made "message for conversation A" and "message for a
    /// conversation I have never heard of" the same thing, so a client reading
    /// another machine's merged list (or a list whose conversation was since
    /// deleted) had its text quietly re-homed into a brand new chat under a
    /// title derived from it — with nothing reported, so nothing told the user.
    ///
    /// The refusal must also leave the store untouched: a phantom thread/run
    /// would show up in the sidebar as the very conversation the user was
    /// trying to avoid.
    #[tokio::test]
    async fn a_prompt_for_an_unknown_conversation_is_refused_before_it_persists() {
        use crate::remote::test_support::{HomeGuard, RecordingSink};

        let _home = HomeGuard::new("business-prompt-unknown-session");
        crate::remote::test_support::init_store();
        let threads_before = crate::store::list_threads().expect("thread list").len();
        let command_id = "cmd-unknown-session";

        let sink = RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "prompt".to_string(),
            id: command_id.to_string(),
            // The shape a phone sends when it opens a row belonging to another
            // desktop: a real session id, just not one of ours.
            session_id: "20260101-000000-session-from-another-desktop".to_string(),
            message: "继续".to_string(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;
        let (success, data, error) = sink.last();
        assert!(!success, "an unknown conversation must not be accepted");
        assert!(data.is_null(), "a refusal carries no ack: {data}");
        let error = error.expect("a failed reply carries the reason");
        assert!(
            error.starts_with(UNKNOWN_SESSION_CODE),
            "the reply must carry the code a client maps to a message, got {error:?}"
        );
        assert_eq!(
            crate::store::list_threads().expect("thread list").len(),
            threads_before,
            "the refusal must not create the conversation it refused"
        );
        assert!(
            crate::store::find_run_by_trigger_message_id(command_id)
                .expect("run lookup")
                .is_none(),
            "a refused prompt must leave no run behind"
        );
    }

    /// The refusal is scoped to a *named* conversation. An empty session id is a
    /// client's new-conversation draft and still takes the lazy-creation path —
    /// asserted through the workspace branch, which refuses a missing workspace
    /// before any Agent call, so the test needs no Agent to tell the two apart.
    #[tokio::test]
    async fn an_empty_session_id_is_still_a_draft_not_an_unknown_conversation() {
        use crate::remote::test_support::HomeGuard;

        let _home = HomeGuard::new("business-prompt-draft-session");
        crate::remote::test_support::init_store();

        let error = prepare_remote_prompt(
            "",
            "hi".to_string(),
            RemotePromptOptions {
                model_id: None,
                thinking_level: None,
                mode: "workspace".to_string(),
                workspace_id: String::new(),
                upload_references: Vec::new(),
                command_id: "cmd-draft-session".to_string(),
                busy_policy: String::new(),
            },
        )
        .await
        .err()
        .expect("a workspace draft without a workspace cannot be prepared")
        .to_string();
        assert!(
            !error.contains(UNKNOWN_SESSION_CODE),
            "a draft must not be read as an unknown conversation: {error:?}"
        );
        assert!(error.contains("Select a workspace"), "{error:?}");
    }

    /// A receipt lookup is the first thing `prompt`, `get_prompt_receipt` and
    /// `continue_run` do, because a retried command id must be answered with the
    /// run it already started. A store that cannot answer must fail the command
    /// loudly: "I could not check" is not "there is no receipt", and treating it
    /// as one would let a lost reply become a second run.
    #[tokio::test]
    async fn a_prompt_whose_receipt_cannot_be_checked_is_refused() {
        use crate::remote::test_support::{HomeGuard, RecordingSink};

        // A fresh HOME with no store: every receipt lookup fails.
        let _home = HomeGuard::new("business-prompt-unreadable");
        for cmd_type in ["prompt", "get_prompt_receipt", "continue_run"] {
            let sink = RecordingSink::default();
            let command_id = format!("cmd-{cmd_type}");
            let cmd = IncomingCmd {
                cmd_type: cmd_type.to_string(),
                id: command_id.clone(),
                // `get_prompt_receipt` keys off `prompt_id`; the other two key
                // off the command id. All three must reach the store.
                prompt_id: command_id,
                ..Default::default()
            };
            execute(&cmd, &sink).await;
            let (success, _data, error) = sink.last();
            assert!(!success, "{cmd_type} must not answer without its receipt");
            let error = error.expect("a failed reply carries the reason");
            assert!(
                error.contains("no such table"),
                "{cmd_type} must report the store fault, got {error:?}"
            );
        }
    }

    /// A receipt is only meaningful for a conversation that still exists. A run
    /// row whose thread was removed out-of-band (the TUI/CLI can delete a
    /// thread; the GUI's own delete cascades) must read as "no receipt", never
    /// as a receipt naming a thread the phone cannot open.
    #[tokio::test]
    async fn a_receipt_for_a_thread_that_no_longer_exists_reads_as_no_receipt() {
        use crate::remote::test_support::{raw_store_connection, HomeGuard, RecordingSink};
        use future_remote_protocol::IncomingCmd;

        let _home = HomeGuard::new("business-prompt-dangling-run");
        crate::remote::test_support::init_store();
        let command_id = "cmd-dangling-run";
        {
            let conn = raw_store_connection();
            conn.execute(
                "INSERT INTO runs (
                     id, thread_id, trigger_message_id, status, remote_accepted_at,
                     created_at, updated_at
                 ) VALUES ('run-dangling', 'thread-that-is-gone', ?1, 'completed', 1, 1, 1)",
                rusqlite::params![command_id],
            )
            .expect("write a run whose thread is gone");
        }
        assert!(
            crate::store::get_thread("thread-that-is-gone")
                .expect("thread lookup")
                .is_none(),
            "the fixture must describe a dangling run"
        );

        let sink = RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "get_prompt_receipt".to_string(),
            prompt_id: command_id.to_string(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;
        let (success, data, error) = sink.last();
        assert!(
            success,
            "a receipt question is answered, not refused: {error:?}"
        );
        assert!(
            data.is_null(),
            "a run whose conversation is gone has no receipt to hand back: {data}"
        );
    }

    /// A retried `continue_run` is answered with the run it already started.
    /// The stored receipt is what makes that survive the reply-slot window: the
    /// phone's outbox can retry long after the in-memory cache expired, and the
    /// second delivery must not start a second continuation of the same failed
    /// run.
    #[tokio::test]
    async fn a_retried_continuation_is_answered_from_its_stored_receipt() {
        use crate::remote::test_support::{raw_store_connection, HomeGuard, RecordingSink};

        let _home = HomeGuard::new("business-prompt-continue-receipt");
        crate::remote::test_support::init_store();
        let command_id = "cmd-continue-replay";
        {
            let conn = raw_store_connection();
            conn.execute_batch(&format!(
                "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                     VALUES ('ws_cont', 'WS', 'user', '/tmp/ws', 1, 1);
                 INSERT INTO threads (id, workspace_id, mode, title, agent_session_id,
                                      created_at, updated_at)
                     VALUES ('th_cont', 'ws_cont', 'chat', 'T', 'sess_cont', 1, 1);
                 INSERT INTO runs (id, thread_id, trigger_message_id, status,
                                   remote_accepted_at, created_at, updated_at)
                     VALUES ('run_cont', 'th_cont', '{command_id}', 'failed', 1, 1, 1);"
            ))
            .expect("write the receipt the retry must find");
        }

        let sink = RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "continue_run".to_string(),
            id: command_id.to_string(),
            session_id: "sess_cont".to_string(),
            run_id: "run_cont".to_string(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;
        let (success, data, error) = sink.last();
        assert!(success, "a retry is answered, not refused: {error:?}");
        assert_eq!(
            data["runId"],
            json!("run_cont"),
            "the receipt names the run"
        );
        assert_eq!(data["threadId"], json!("th_cont"));
        assert_eq!(
            data["sessionId"],
            json!("sess_cont"),
            "and the session the phone asked about"
        );
    }

    /// The prompt family is a closed set of five commands; a name from another
    /// family must not be answered with a plausible-looking reply.
    #[tokio::test]
    #[should_panic(expected = "handler received")]
    async fn a_command_from_another_family_is_not_answered() {
        let sink = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "get_settings".into(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;
    }
}
