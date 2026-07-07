//! Prompt → persist pipeline for headless callers (no webview driving the
//! store): create the run + user message, drive [`super::agent_prompt`], and
//! finalize the run + assistant message with the SAME semantics the frontend
//! `handleSend` applies (CAS-guarded status writes; a stream that closes before
//! `agent_end` persists the partial text but fails the run). Keeping this in
//! `agent_bridge` means the finalization contract lives in one backend place —
//! remote (phone) prompts and any future headless path must not re-implement it.

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
}

/// Persist the user message and create the run for `thread`, returning the
/// identifiers the caller can immediately ack to its client.
pub fn prepare_prompt_persisted(
    thread: &store::ThreadRecord,
    message: String,
) -> Result<PreparedPrompt, crate::AppError> {
    let session_id = thread
        .agent_session_id
        .clone()
        .unwrap_or_else(|| thread.id.clone());

    let user_msg = store::append_message(store::AppendMessageInput {
        thread_id: thread.id.clone(),
        run_id: None,
        role: "user".to_string(),
        content_type: Some("markdown".to_string()),
        content: message.clone(),
        status: Some("complete".to_string()),
    })?;

    let run = store::create_run(store::CreateRunInput {
        thread_id: thread.id.clone(),
        trigger_message_id: Some(user_msg.id),
        model_provider: thread.model_provider.clone(),
        model_id: thread.model_id.clone(),
    })?;

    Ok(PreparedPrompt {
        thread_id: thread.id.clone(),
        session_id,
        run_id: run.id,
        message,
        model_id: thread.model_id.clone(),
        thinking_level: thread.thinking_level.clone(),
    })
}

/// Drive the agent for a [`PreparedPrompt`] and finalize run + assistant
/// message (mirrors the frontend `handleSend` settle branches).
pub async fn run_prepared_prompt(prepared: PreparedPrompt) -> Result<(), crate::AppError> {
    let PreparedPrompt {
        thread_id,
        session_id,
        run_id,
        message,
        model_id,
        thinking_level,
    } = prepared;

    let result = super::agent_prompt(
        message,
        None,
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
            let _ = store::append_message(store::AppendMessageInput {
                thread_id,
                run_id: Some(run_id),
                role: "assistant".to_string(),
                content_type: Some("markdown".to_string()),
                content: response.content,
                status: Some("failed".to_string()),
            });
            Ok(())
        }
        Ok(response) => {
            let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
                run_id: run_id.clone(),
                status: "completed".to_string(),
                error_message: None,
                error_type: None,
            });
            let content = if response.content.trim().is_empty() {
                "Future Agent completed but returned no text.".to_string()
            } else {
                response.content
            };
            let _ = store::append_message(store::AppendMessageInput {
                thread_id,
                run_id: Some(run_id),
                role: "assistant".to_string(),
                content_type: Some("markdown".to_string()),
                content,
                status: Some("complete".to_string()),
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
            let _ = store::append_message(store::AppendMessageInput {
                thread_id,
                run_id: Some(run_id),
                role: "assistant".to_string(),
                content_type: Some("markdown".to_string()),
                content: format!("Future Agent error: {error}"),
                status: Some("failed".to_string()),
            });
            Err(error)
        }
    }
}
