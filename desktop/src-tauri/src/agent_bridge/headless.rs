//! Prompt pipeline for headless callers (no webview driving the store): create
//! the local run, drive [`super::agent_prompt`], and finalize run metadata with
//! the SAME semantics the frontend
//! `handleSend` applies (CAS-guarded status writes; a stream that closes before
//! `agent_end` persists the partial text but fails the run). Keeping this in
//! `agent_bridge` means the finalization contract lives in one backend place —
//! remote (phone) prompts and any future headless path must not re-implement it.

use super::AttachmentInput;
use crate::store;

/// A prompt whose user message + run row are already persisted. Created by
/// [`prepare_prompt_persisted`] so callers can ack with the real identifiers
/// before the (long) agent call runs.
pub struct PreparedPrompt {
    pub thread_id: String,
    /// The session the agent (and event mirror) will actually use — resolved
    /// from the thread, never assumed from caller input.
    pub session_id: String,
    pub run_id: String,
    message: String,
    model_id: Option<String>,
    thinking_level: Option<String>,
    attachments: Vec<AttachmentInput>,
}

/// Create the run for `thread`, returning the
/// identifiers the caller can immediately ack to its client.
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
        message,
        model_id,
        thinking_level,
        attachments,
    })
}

/// Drive the agent for a [`PreparedPrompt`] and finalize local run metadata.
/// Conversation messages are persisted once, by the Agent JSONL writer.
pub async fn run_prepared_prompt(prepared: PreparedPrompt) -> Result<(), crate::AppError> {
    let PreparedPrompt {
        thread_id,
        session_id,
        run_id,
        message,
        model_id,
        thinking_level,
        attachments,
    } = prepared;

    let result = super::agent_prompt(
        message,
        Some(attachments),
        thread_id.clone(),
        Some(session_id),
        Some(run_id.clone()),
        model_id,
        thinking_level,
    )
    .await;

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
