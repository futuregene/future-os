//! Prompt pipeline for headless callers (no webview driving the store): create
//! the local run, drive [`super::agent_prompt_with_acceptance`], and finalize run metadata with
//! the SAME semantics the frontend
//! `handleSend` applies (CAS-guarded status writes; a stream that closes before
//! `agent_end` persists the partial text but fails the run). Keeping this in
//! `agent_bridge` means the finalization contract lives in one backend place —
//! remote (phone) prompts and any future headless path must not re-implement it.

use super::AttachmentInput;
use crate::store;

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
}

/// Create the run for `thread` and return its stable local identifiers.
#[cfg(test)]
pub fn prepare_prompt_persisted(
    thread: &store::ThreadRecord,
    message: String,
    model_id: Option<String>,
    thinking_level: Option<String>,
    attachments: Vec<AttachmentInput>,
) -> Result<PreparedPrompt, crate::AppError> {
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
) -> Result<PreparedPrompt, crate::AppError> {
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
    })
}

/// Drive the agent for a [`PreparedPrompt`] and finalize local run metadata.
/// Conversation messages are persisted once, by the Agent JSONL writer.
#[cfg(test)]
pub async fn run_prepared_prompt(prepared: PreparedPrompt) -> Result<(), crate::AppError> {
    run_prepared_prompt_inner(prepared, None).await
}

/// Execute a remote prompt and report when its acceptance receipt is durable
/// in both stores. Admission failures are sent back to the caller, so a local
/// run row cannot be mistaken for an Agent-accepted message.
pub async fn run_prepared_prompt_with_acceptance(
    prepared: PreparedPrompt,
    receipt: tokio::sync::oneshot::Sender<Result<(), String>>,
) -> Result<(), crate::AppError> {
    run_prepared_prompt_inner(prepared, Some(receipt)).await
}

async fn run_prepared_prompt_inner(
    prepared: PreparedPrompt,
    receipt: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
) -> Result<(), crate::AppError> {
    let PreparedPrompt {
        thread_id,
        session_id,
        run_id,
        remote_command_id,
        message,
        model_id,
        thinking_level,
        attachments,
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
    };
    let (agent_accepted_tx, agent_accepted_rx) = tokio::sync::oneshot::channel();
    // Poll the long-running prompt concurrently with its early acceptance
    // signal. Awaiting the signal while leaving the prompt future unpolled
    // would deadlock before the command could ever reach the Agent.
    let prompt = tokio::spawn(super::agent_prompt_with_acceptance(
        request,
        Some(agent_accepted_tx),
    ));
    let acceptance = agent_accepted_rx.await;
    let result = match acceptance {
        Ok(()) => {
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
            let result = prompt.await.map_err(|error| {
                crate::AppError::Message(format!("Agent prompt task failed: {error}"))
            })?;
            if retry_receipt_commit {
                if let Some(command_id) = remote_command_id.as_deref() {
                    let _ = store::mark_remote_prompt_accepted(&run_id, command_id);
                }
            }
            result
        }
        Err(_) => {
            let result = prompt.await.map_err(|error| {
                crate::AppError::Message(format!("Agent prompt task failed: {error}"))
            })?;
            if let Some(receipt) = receipt {
                let error = result
                    .as_ref()
                    .err()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "Future Agent did not acknowledge the prompt.".to_string());
                let _ = receipt.send(Err(error));
            }
            result
        }
    };

    match result {
        // Stream closed before `agent_end`: the text is a truncated prefix, not
        // a finished answer. Persist it (so the partial isn't lost) but mark the
        // run failed rather than completed.
        Ok(response) if !response.complete => {
            let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
                run_id: run_id.clone(),
                status: "failed".to_string(),
                error_message: Some("Response interrupted before completion.".to_string()),
                error_type: Some("stream_interrupted".to_string()),
            });
            Ok(())
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

        run_prepared_prompt(prepared).await.expect("run");
        let run = crate::store::get_run(&run_id)
            .expect("get run")
            .expect("some");
        assert_eq!(run.status, "failed");
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
