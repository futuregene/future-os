//! Prompt pipeline for headless callers (no webview driving the store): create
//! the local run, drive [`super::agent_prompt_with_acceptance`], and finalize run metadata with
//! the SAME semantics the frontend
//! `handleSend` applies (CAS-guarded status writes; a stream that closes before
//! `agent_end` persists the partial text but fails the run). Keeping this in
//! `agent_bridge` means the finalization contract lives in one backend place —
//! remote (phone) prompts and any future host path must not re-implement it.

use super::AttachmentInput;
use crate::store;
use futures::FutureExt;

/// A prompt whose user message + run row are already persisted. Created by
/// [`prepare_prompt_persisted_with_trigger`]. Remote callers acknowledge these
/// identifiers only after the Agent accepts the prompt and the receipt is
/// durably committed locally.
pub struct PreparedPrompt {
    pub thread_id: String,
    /// The session the agent (and event mirror) will actually use — resolved
    /// from the thread, never assumed from caller input.
    pub session_id: String,
    pub run_id: String,
    remote_command_id: Option<String>,
    message: String,
    model_id: Option<String>,
    thinking_level: Option<String>,
    attachments: Vec<AttachmentInput>,
    /// What this send means while the session already runs something: an Agent
    /// `BusyPolicy` value, empty for the Agent's own default. Only the remote
    /// path sets it today (a phone or another computer asking to queue).
    busy_policy: String,
    /// The answer a `futureos-choice` card gave this message, when it came from
    /// one. The message already reads `[question → answer]`; this rides along so
    /// the Agent can keep it with the accepted run and let a waiting row offer a
    /// different pick (design §5.8).
    choice: Option<future_rpc::payloads::QueuedRunChoice>,
}

/// Create the run for `thread` and return its stable local identifiers.
#[cfg(test)]
pub fn prepare_prompt_persisted(
    thread: &store::ThreadRecord,
    message: String,
    model_id: Option<String>,
    thinking_level: Option<String>,
    attachments: Vec<AttachmentInput>,
) -> Result<PreparedPrompt, crate::Error> {
    prepare_prompt_persisted_with_trigger(
        thread,
        message,
        model_id,
        thinking_level,
        attachments,
        None,
    )
}

/// Remote prompts use their persisted command id as the run trigger. Keeping
/// the regular headless entrypoint above unchanged avoids assigning transport
/// identities to local GUI prompts.
pub fn prepare_prompt_persisted_with_trigger(
    thread: &store::ThreadRecord,
    message: String,
    model_id: Option<String>,
    thinking_level: Option<String>,
    attachments: Vec<AttachmentInput>,
    trigger_message_id: Option<String>,
) -> Result<PreparedPrompt, crate::Error> {
    prepare_prompt_persisted_busy(
        thread,
        message,
        model_id,
        thinking_level,
        attachments,
        trigger_message_id,
        String::new(),
    )
}

/// The same, for a caller that says what a send means while the session still
/// runs one (the remote prompt path). An empty policy keeps the Agent default.
pub fn prepare_prompt_persisted_busy(
    thread: &store::ThreadRecord,
    message: String,
    model_id: Option<String>,
    thinking_level: Option<String>,
    attachments: Vec<AttachmentInput>,
    trigger_message_id: Option<String>,
    busy_policy: String,
) -> Result<PreparedPrompt, crate::Error> {
    prepare_prompt_persisted_busy_with_choice(
        thread,
        message,
        model_id,
        thinking_level,
        attachments,
        trigger_message_id,
        busy_policy,
        None,
    )
}

/// The same, carrying the answer a `futureos-choice` card gave the message.
///
/// `None` is "this message answered no card", which is every local send and
/// every remote prompt that did not start from a card.
#[allow(clippy::too_many_arguments)]
pub fn prepare_prompt_persisted_busy_with_choice(
    thread: &store::ThreadRecord,
    message: String,
    model_id: Option<String>,
    thinking_level: Option<String>,
    attachments: Vec<AttachmentInput>,
    trigger_message_id: Option<String>,
    busy_policy: String,
    choice: Option<future_rpc::payloads::QueuedRunChoice>,
) -> Result<PreparedPrompt, crate::Error> {
    let session_id = thread
        .agent_session_id
        .clone()
        .unwrap_or_else(|| thread.id.clone());

    let remote_command_id = trigger_message_id.clone();
    let run = store::create_run(store::CreateRunInput {
        id: None,
        thread_id: thread.id.clone(),
        trigger_message_id,
        model_provider: None,
        model_id: None,
    })?;

    Ok(PreparedPrompt {
        thread_id: thread.id.clone(),
        session_id,
        run_id: run.id,
        remote_command_id,
        message,
        model_id,
        thinking_level,
        attachments,
        busy_policy,
        choice,
    })
}

/// Drive the agent for a [`PreparedPrompt`] and finalize local run metadata.
/// Conversation messages are persisted once, by the Agent JSONL writer.
#[cfg(test)]
pub async fn run_prepared_prompt(prepared: PreparedPrompt) -> Result<(), crate::Error> {
    run_prepared_prompt_inner(prepared, None).await
}

/// Execute a remote prompt and report when its acceptance receipt is durable
/// in both stores. Admission failures are sent back to the caller, so a local
/// run row cannot be mistaken for an Agent-accepted message.
pub async fn run_prepared_prompt_with_acceptance(
    prepared: PreparedPrompt,
    receipt: tokio::sync::oneshot::Sender<Result<(), String>>,
) -> Result<(), crate::Error> {
    run_prepared_prompt_inner(prepared, Some(receipt)).await
}

async fn run_prepared_prompt_inner(
    prepared: PreparedPrompt,
    receipt: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
) -> Result<(), crate::Error> {
    let PreparedPrompt {
        thread_id,
        session_id,
        run_id,
        remote_command_id,
        message,
        model_id,
        thinking_level,
        attachments,
        busy_policy,
        choice,
    } = prepared;

    let request = super::AgentPromptRequest {
        message,
        model_context: String::new(),
        attachments: Some(attachments),
        thread_id: thread_id.clone(),
        session_id: Some(session_id),
        run_id: Some(run_id.clone()),
        model_id,
        thinking_level,
        busy_policy,
        choice,
    };
    let (agent_accepted_tx, agent_accepted_rx) = tokio::sync::oneshot::channel();
    // Poll acceptance alongside the prompt in this task. A spawned JoinHandle
    // detaches on drop: cancelling a host task would then leave its collector
    // and run lease alive after the host cleanup barrier had returned.
    let prompt = std::panic::AssertUnwindSafe(super::agent_prompt_with_acceptance(
        request,
        Some(agent_accepted_tx),
    ))
    .catch_unwind();
    let acceptance = async {
        if agent_accepted_rx.await.is_ok() {
            let mut retry_receipt_commit = false;
            if let Some(receipt) = receipt {
                let accepted = match remote_command_id.as_deref() {
                    Some(command_id) => store::mark_remote_prompt_accepted(&run_id, command_id)
                        .map_err(|error| error.to_string()),
                    None => Err("Remote prompt is missing its command id.".to_string()),
                };
                retry_receipt_commit = accepted.is_err() && remote_command_id.is_some();
                let _ = receipt.send(accepted);
            }
            (retry_receipt_commit, None)
        } else {
            (false, receipt)
        }
    };
    let (result, (retry_receipt_commit, receipt)) = tokio::join!(prompt, acceptance);
    // Preserve the previous spawned-task panic boundary without giving the
    // child an independent lifetime. Ordinary Agent errors pass through.
    let result = result.map_err(|panic| {
        let reason = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("task panicked");
        crate::Error::Message(format!("Agent prompt task failed: {reason}"))
    })?;
    if retry_receipt_commit {
        if let Some(command_id) = remote_command_id.as_deref() {
            let _ = store::mark_remote_prompt_accepted(&run_id, command_id);
        }
    }
    if let Some(receipt) = receipt {
        let error = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_else(|| "Future Agent did not acknowledge the prompt.".to_string());
        let _ = receipt.send(Err(error));
    }

    match result {
        // The Agent accepted this turn and is holding it behind the run ahead of
        // it. The prompt path already settled the row to `queued` — the one
        // status the rest of the app reads as "alive, but not the run this
        // conversation is watching stream" — and the session observer owns it
        // from here.
        //
        // This wrapper used to know only complete/incomplete, so it restamped
        // that row `failed` a moment later. The settled-run repair then read the
        // Agent's ledger, saw the run alive, and revived it to `running`
        // (`store::revive_unverified_run` had no other shape to restore) — which
        // made the queued row win every "latest run" read and the open
        // conversation draw a second, empty "thinking" bubble for a turn that had
        // not started (2026-11-26: two 未来助手 bubbles under one prompt).
        //
        // Still no `Ok`: the turn is owed, so a caller that only saw success must
        // not record it as a finished answer — but nothing failed, so the row and
        // its failure markers stay exactly as the prompt path left them.
        Ok(response) if response.termination_kind.as_deref() == Some("run_queued") => Err(
            crate::Error::Message(super::stream::termination_error(Some("run_queued")).to_string()),
        ),
        // Stream closed before `agent_end`: the text is a truncated prefix, not
        // a finished answer. Persist it (so the partial isn't lost) but settle
        // the run as not-completed — and *return* that failure, because a caller
        // that only sees `Ok` records the run as a success with an empty answer
        // (a cancelled or restarted run reached the task ledger as `completed`
        // exactly this way).
        Ok(response) if !response.complete => {
            let message = super::stream::termination_error(response.termination_kind.as_deref());
            // A run the Agent reports as *cancelled* was stopped on purpose —
            // by this client's phone, another paired desktop, or an IM `/stop`.
            // Record it as `cancelled` (the status the local stop already
            // writes) so every reader draws the stopped reply; a `failed` row
            // carrying this error read as "模型服务异常 / 请稍后重试" over a stop
            // the user chose (measured 2026-11-24). Any other incomplete ending
            // stays a failure.
            let cancelled = response.termination_kind.as_deref() == Some("run_cancelled");
            let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
                run_id: run_id.clone(),
                status: if cancelled { "cancelled" } else { "failed" }.to_string(),
                error_message: Some(message.to_string()),
                error_type: Some(if cancelled {
                    "cancelled".to_string()
                } else {
                    response
                        .termination_kind
                        .clone()
                        .unwrap_or_else(|| "stream_interrupted".to_string())
                }),
            });
            Err(crate::Error::Message(message.to_string()))
        }
        Ok(_) => {
            let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
                run_id: run_id.clone(),
                status: "completed".to_string(),
                error_message: None,
                error_type: None,
            });
            Ok(())
        }
        Err(error) => {
            let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
                run_id: run_id.clone(),
                status: "failed".to_string(),
                error_message: Some(error.to_string()),
                error_type: None,
            });
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{
        mock_agent, seed_thread, seed_workspace, stream_event, Reply, StreamScript, TestHome,
    };
    use super::*;

    fn headless_thread() -> (TestHome, crate::store::ThreadRecord) {
        let home = TestHome::new("headless");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-headless"));
        (home, thread)
    }

    /// `prepare_prompt_persisted` resolves the session from the thread's agent
    /// session and creates a local run row the caller can ack immediately.
    #[test]
    fn prepare_prompt_persisted_creates_run_and_resolves_session() {
        let (_home, thread) = headless_thread();
        let prepared = prepare_prompt_persisted(
            &thread,
            "hello".to_string(),
            Some("future/k3".to_string()),
            Some("high".to_string()),
            vec![],
        )
        .expect("prepare");

        assert_eq!(prepared.thread_id, thread.id);
        assert_eq!(prepared.session_id, "sess-headless");
        assert_eq!(prepared.message, "hello");
        assert_eq!(prepared.model_id.as_deref(), Some("future/k3"));
        assert_eq!(prepared.thinking_level.as_deref(), Some("high"));

        let run = crate::store::get_run(&prepared.run_id)
            .expect("get run")
            .expect("run exists");
        assert_eq!(run.thread_id, thread.id);
    }

    /// A thread without an agent session falls back to the thread id as the
    /// session the agent will use.
    #[test]
    fn prepare_prompt_persisted_falls_back_to_thread_id() {
        let home = TestHome::new("headless-no-session");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, None);
        let prepared = prepare_prompt_persisted(&thread, "hi".to_string(), None, None, vec![])
            .expect("prepare");
        assert_eq!(prepared.session_id, thread.id);
    }

    #[tokio::test]
    async fn cancelling_the_host_task_drops_its_prompt_collector_before_cleanup_returns() {
        let home = TestHome::new("headless-cancel-owned-collector");
        let mock = mock_agent();
        mock.default_data(
            "get_state",
            super::super::test_support::get_state_payload("session-cancel", false),
        );
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("session-cancel"));
        let prepared = prepare_prompt_persisted_with_trigger(
            &thread,
            "keep streaming".into(),
            None,
            None,
            vec![],
            Some("command-cancel".into()),
        )
        .unwrap();
        let run_id = prepared.run_id.clone();
        let (receipt, accepted) = tokio::sync::oneshot::channel();
        let task = crate::runtime::spawn(async move {
            let _ = run_prepared_prompt_with_acceptance(prepared, receipt).await;
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), accepted)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        // Real RPC acceptance and a live stream must have acquired the lease;
        // cancellation must drop that same collector, not just its parent.
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while super::super::AGENT_REPLICAS
                .canonical_for_local(&run_id)
                .is_none()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        crate::runtime::cancel_all_tasks(std::time::Duration::from_secs(3)).unwrap();
        assert_eq!(
            super::super::AGENT_REPLICAS.canonical_for_local(&run_id),
            None,
            "cleanup cannot return while a detached prompt collector still owns this run"
        );
    }

    #[tokio::test]
    async fn run_prepared_prompt_completes_a_clean_run() {
        let home = TestHome::new("headless-complete");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-headless-error"));
        let prepared = prepare_prompt_persisted(&thread, "hello".to_string(), None, None, vec![])
            .expect("prepare");
        let run_id = prepared.run_id.clone();

        mock.push_data(
            "get_state",
            serde_json::json!({ "sessionId": "sess-headless-error", "cwd": workspace.path }),
        );
        mock.push_stream(StreamScript::Events(
            vec![stream_event(
                &run_id,
                0,
                "agent_end",
                r#"{"reason":"complete"}"#,
            )],
            None,
        ));

        run_prepared_prompt(prepared).await.expect("run");
        assert_eq!(
            crate::store::get_run(&run_id)
                .expect("get run")
                .expect("some")
                .status,
            "completed"
        );
    }

    #[tokio::test]
    async fn remote_receipt_appears_only_after_agent_acceptance() {
        let home = TestHome::new("headless-remote-acceptance");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-headless-remote"));
        let command_id = "remote-command-1";
        let prepared = prepare_prompt_persisted_with_trigger(
            &thread,
            "hello".to_string(),
            None,
            None,
            vec![],
            Some(command_id.to_string()),
        )
        .expect("prepare");
        let run_id = prepared.run_id.clone();
        assert!(store::find_run_by_trigger_message_id(command_id)
            .expect("receipt query")
            .is_none());

        mock.push_data(
            "get_state",
            serde_json::json!({ "sessionId": "sess-headless-remote", "cwd": workspace.path }),
        );
        mock.push_stream(StreamScript::Events(
            vec![stream_event(
                &run_id,
                0,
                "agent_end",
                r#"{"reason":"complete"}"#,
            )],
            None,
        ));
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let execution = tokio::spawn(run_prepared_prompt_with_acceptance(prepared, accepted_tx));
        accepted_rx
            .await
            .expect("acceptance channel")
            .expect("durable receipt");
        assert_eq!(
            store::find_run_by_trigger_message_id(command_id)
                .expect("receipt query")
                .expect("accepted run")
                .id,
            run_id
        );
        execution.await.expect("join").expect("execution");
    }

    /// A remote command id that cannot be recorded is reported through the
    /// receipt instead of being invented — and the run still executes: the
    /// Agent already accepted it, so a bookkeeping failure must not turn into a
    /// lost run. The commit is retried once the prompt has finished (there is no
    /// further durable work to race then).
    #[tokio::test]
    async fn a_remote_receipt_that_cannot_be_persisted_is_reported_and_retried_after_the_run() {
        let home = TestHome::new("headless-receipt-retry");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-headless-receipt-retry"));
        // `mark_remote_prompt_accepted` refuses an empty command id, which is
        // how a receipt commit is made to fail without a hostile store.
        let prepared = prepare_prompt_persisted_with_trigger(
            &thread,
            "hello".to_string(),
            None,
            None,
            vec![],
            Some(String::new()),
        )
        .expect("prepare");
        let run_id = prepared.run_id.clone();

        mock.push_data(
            "get_state",
            serde_json::json!({ "sessionId": "sess-headless-receipt-retry", "cwd": workspace.path }),
        );
        mock.push_stream(StreamScript::Events(
            vec![stream_event(
                &run_id,
                0,
                "agent_end",
                r#"{"reason":"complete"}"#,
            )],
            None,
        ));

        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let execution = tokio::spawn(run_prepared_prompt_with_acceptance(prepared, accepted_tx));
        assert!(
            accepted_rx.await.expect("acceptance channel").is_err(),
            "an unpersistable command id is reported, not silently accepted"
        );
        execution
            .await
            .expect("join")
            .expect("the run still executed despite the failed receipt commit");
        assert_eq!(
            store::get_run(&run_id)
                .expect("get run")
                .expect("some")
                .status,
            "completed"
        );
    }

    #[tokio::test]
    async fn run_prepared_prompt_marks_an_incomplete_run_failed() {
        let home = TestHome::new("headless-incomplete");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-headless-incomplete"));
        let prepared = prepare_prompt_persisted(&thread, "hello".to_string(), None, None, vec![])
            .expect("prepare");
        let run_id = prepared.run_id.clone();

        mock.push_data(
            "get_state",
            serde_json::json!({ "sessionId": "sess-headless-incomplete", "cwd": workspace.path }),
        );
        mock.push_stream(StreamScript::Events(
            vec![stream_event(
                &run_id,
                0,
                "agent_end",
                r#"{"reason":"incomplete"}"#,
            )],
            None,
        ));

        // The run row is failed *and* the caller hears about it: a caller that
        // only saw `Ok` here recorded the run as a success with an empty answer
        // (a cancelled or restarted run reached the task ledger as `completed`).
        let error = run_prepared_prompt(prepared)
            .await
            .expect_err("an incomplete run is a failure, not a quiet success");
        assert!(
            error.to_string().contains("[RESPONSE_UNCONFIRMED]"),
            "{error}"
        );
        let run = crate::store::get_run(&run_id)
            .expect("get run")
            .expect("some");
        assert_eq!(run.status, "failed");
    }

    #[tokio::test]
    async fn run_prepared_prompt_records_a_stop_as_cancelled_not_a_failure() {
        let home = TestHome::new("headless-cancelled");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-headless-cancelled"));
        let prepared = prepare_prompt_persisted(&thread, "hello".to_string(), None, None, vec![])
            .expect("prepare");
        let run_id = prepared.run_id.clone();

        mock.push_data(
            "get_state",
            serde_json::json!({ "sessionId": "sess-headless-cancelled", "cwd": workspace.path }),
        );
        mock.push_stream(StreamScript::Events(
            vec![
                stream_event(&run_id, 0, "text_chunk", r#"{"text":"half an answer"}"#),
                stream_event(&run_id, 1, "agent_end", r#"{"state":"cancelled"}"#),
            ],
            None,
        ));

        // The Agent ended this run because somebody stopped it — the phone's
        // stop button, in the case this guards — so nothing in this process
        // asked. The caller still hears the reply did not finish, but the row
        // says *cancelled*: a `failed` row carrying `[RUN_CANCELLED]` drew
        // "模型服务异常 / 请稍后重试" over a stop the reader chose (2026-11-24).
        let error = run_prepared_prompt(prepared)
            .await
            .expect_err("a stopped run answers nothing");
        assert!(error.to_string().contains("[RUN_CANCELLED]"), "{error}");
        let run = crate::store::get_run(&run_id)
            .expect("get run")
            .expect("some");
        assert_eq!(run.status, "cancelled");
        assert_eq!(run.error_type.as_deref(), Some("cancelled"));
    }

    /// A remote (mobile) prompt the Agent accepts while another run is executing
    /// is *queued* — the one status the rest of the app reads as "alive, but not
    /// the run this conversation is watching stream".
    ///
    /// This wrapper used to know only complete/incomplete, so it restamped the
    /// row `failed` immediately after the prompt path had written `queued`; the
    /// settled-run repair then revived it to `running`, and the open
    /// conversation opened a second, empty "thinking" bubble for a turn that had
    /// not started (2026-11-26: two 未来助手 bubbles under one prompt, the lower
    /// one counting from the queued row's own start).
    ///
    /// Reverting the `run_queued` arm in `run_prepared_prompt_inner` turns this
    /// red: the row comes back `failed` with error_type `run_queued`.
    #[tokio::test]
    async fn a_remote_prompt_queued_behind_a_running_run_stays_queued() {
        let home = TestHome::new("headless-queued");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-queued"));
        mock.push_data(
            "get_state",
            serde_json::json!({ "sessionId": "sess-queued", "cwd": workspace.path }),
        );
        let prepared = prepare_prompt_persisted_with_trigger(
            &thread,
            "second question".to_string(),
            None,
            None,
            vec![],
            Some("remote-command-queued".to_string()),
        )
        .expect("prepare");
        let run_id = prepared.run_id.clone();

        // The Agent accepted the prompt and is holding it behind an older run:
        // a queued run has no execution epoch, so the attach is refused.
        mock.push_stream(StreamScript::AttachError(
            tonic::Code::FailedPrecondition,
            "run is not the active run",
        ));
        mock.push_run_state(
            &run_id,
            serde_json::json!({"queuedRuns": [{"runId": run_id, "queuePosition": 1}]}),
        );
        // Its user entry is already durable on the Agent; the queued path reads
        // it back instead of inventing a stub.
        mock.push_typed_data(
            "get_session_entries",
            serde_json::json!({ "entries": [{
                "id": "e-queued",
                "kind": "user",
                "role": "user",
                "createdAtMs": 1000,
                "runId": run_id,
                "blocks": [{"kind": "text", "text": "second question"}],
            }]}),
        );

        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let execution = tokio::spawn(run_prepared_prompt_with_acceptance(prepared, accepted_tx));
        accepted_rx
            .await
            .expect("acceptance channel")
            .expect("durable receipt");
        // The turn is still owed, so a caller still cannot record a finished
        // answer — but the row must not read as a failure either.
        assert!(execution.await.expect("join").is_err());

        let run = store::get_run(&run_id).expect("get run").expect("some");
        assert_eq!(
            run.status,
            store::QUEUED_RUN_STATUS,
            "a held turn is queued, not failed: {:?} / {:?}",
            run.error_type,
            run.error_message
        );
        assert!(
            run.error_message.is_none(),
            "no failure is recorded for a turn that has not run yet: {:?}",
            run.error_message
        );
        assert!(run.ended_at.is_none(), "a queued row has not ended");
    }

    #[tokio::test]
    async fn run_prepared_prompt_propagates_an_agent_error() {
        let home = TestHome::new("headless-error");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-headless-complete"));
        let prepared = prepare_prompt_persisted(&thread, "hello".to_string(), None, None, vec![])
            .expect("prepare");
        let run_id = prepared.run_id.clone();

        mock.push_data(
            "get_state",
            serde_json::json!({ "sessionId": "sess-headless-complete", "cwd": workspace.path }),
        );
        mock.push(
            "prompt",
            Reply::Status(tonic::Code::Internal, "prompt rejected"),
        );

        let error = run_prepared_prompt(prepared).await.expect_err("error");
        assert!(
            error.to_string().contains("Unable to send prompt"),
            "{error}"
        );
        let run = crate::store::get_run(&run_id)
            .expect("get run")
            .expect("some");
        assert_eq!(run.status, "failed");
        assert!(run
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("Unable to send prompt"));
    }
}
