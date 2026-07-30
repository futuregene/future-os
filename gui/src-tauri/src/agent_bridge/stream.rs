//! Consumes the Future Agent event stream for a single prompt: accumulates
//! assistant text, drives the approval wait-state, and forwards every event to
//! the persistence projection. Returns the assembled assistant text once the
//! agent signals `agent_end`.

use tokio::time::{sleep, timeout, Duration};
use tonic::Code;

use super::{connect_agent, persist::persist_run_event};
use crate::agent_proto::StreamRequest;

const AGENT_EVENT_STREAM_TIMEOUT_SECS: u64 = 600;
const STREAM_RECONNECT_ATTEMPTS: u32 = 6;

/// Outcome of collecting one canonical Agent run. `RunGone` is distinct from a
/// transient/`App` failure: the Agent explicitly does not recognize the run
/// (it rolled over, restarted, or already settled and dropped the ring), so the
/// caller must reconcile the local row from the Agent's journal (`get_state`)
/// instead of retrying the attach or marking the run failed.
pub(super) enum CollectError {
    App(crate::AppError),
    RunGone(String),
}

impl From<crate::AppError> for CollectError {
    fn from(error: crate::AppError) -> Self {
        CollectError::App(error)
    }
}

impl From<String> for CollectError {
    fn from(error: String) -> Self {
        CollectError::App(error.into())
    }
}

impl From<CollectError> for crate::AppError {
    fn from(error: CollectError) -> Self {
        match error {
            CollectError::App(error) => error,
            CollectError::RunGone(reason) => {
                crate::AppError::from(format!("Future Agent run no longer active: {reason}"))
            }
        }
    }
}

/// Why an `atomic_attach` failed. Only `FailedPrecondition` / `NotFound` mean the
/// run is gone; everything else (connect failure, unknown code) is a transient
/// the reconnect loop should retry.
enum AttachFailure {
    Transient(String),
    RunGone(String),
}

/// Persist a run event on a blocking thread, so the synchronous SQLite write
/// (and the occasional `git` fork on write/artifact events) doesn't stall the
/// async event loop. Awaited to preserve event order; errors are logged inside
/// `persist_run_event`.
async fn persist_run_event_off_thread(
    thread_id: &str,
    run_id: Option<&str>,
    event_type: String,
    data: String,
    sequence: i64,
) {
    let thread_id = thread_id.to_string();
    let run_id = run_id.map(str::to_string);
    let emitted_run_id = run_id.clone();
    let status = if event_type == "agent_end" {
        "finalizing"
    } else {
        "running"
    }
    .to_string();
    let _ = tokio::task::spawn_blocking(move || {
        persist_run_event(run_id.as_deref(), &event_type, &data, sequence);
        if let Some(run_id) = emitted_run_id {
            crate::emit_thread_runtime_updated(thread_id, run_id, status, false);
        }
    })
    .await;
}

async fn replace_projection_off_thread(
    thread_id: &str,
    local_run_id: Option<&str>,
    events: Vec<(String, String, i64)>,
) {
    let thread_id = thread_id.to_string();
    let local_run_id = local_run_id.map(str::to_string);
    let emitted_run_id = local_run_id.clone();
    let terminal = events
        .iter()
        .any(|(event_type, _, _)| event_type == "agent_end");
    let _ = tokio::task::spawn_blocking(move || {
        if let Some(run_id) = local_run_id.as_deref() {
            // A projection snapshot replaces, rather than extends, the local
            // replica. Closing/removing the old append log first prevents a
            // truncated prefix from being rendered alongside the snapshot.
            crate::store::clear_run_event_buffer(run_id);
            crate::store::delete_run_events_file(run_id);
            for (event_type, data, sequence) in events {
                persist_run_event(Some(run_id), &event_type, &data, sequence);
            }
        }
        if let Some(run_id) = emitted_run_id {
            crate::emit_thread_runtime_updated(
                thread_id,
                run_id,
                if terminal { "finalizing" } else { "running" }.to_string(),
                true,
            );
        }
    })
    .await;
}

/// The assembled assistant text plus whether the stream reached a clean
/// `agent_end`. `complete == false` means the stream ended (server closed it,
/// agent restarted mid-reply) before signalling completion — the text is a
/// prefix, not the whole answer, and the caller must mark the run `failed`
/// rather than persist a silently truncated reply as `completed`.
pub(super) struct AgentResponse {
    pub content: String,
    pub complete: bool,
}

pub(super) async fn collect_agent_response(
    local_run_id: Option<&str>,
    canonical_run_id: &str,
    session_id: &str,
    thread_id: &str,
) -> Result<AgentResponse, CollectError> {
    let mut content = String::new();
    let mut waiting_for_approval = false;
    let cursor_run_id = local_run_id.map(str::to_string);
    let mut last_idx = tokio::task::spawn_blocking(move || {
        cursor_run_id
            .as_deref()
            .and_then(|run_id| crate::store::list_run_events(run_id).ok())
            .and_then(|events| events.into_iter().map(|event| event.sequence).max())
            .unwrap_or(-1)
    })
    .await
    .map_err(|error| format!("Unable to load local run cursor: {error}"))?;
    let mut reconnect_attempt = 0_u32;

    let clean_end = 'attach: loop {
        let attach_result = async {
            let mut client = connect_agent()
                .await
                .map_err(|error| AttachFailure::Transient(error.to_string()))?;
            client
                .stream_events(StreamRequest {
                    event_types: vec![],
                    session_id: session_id.to_string(),
                    run_id: canonical_run_id.to_string(),
                    after_idx: last_idx,
                    atomic_attach: true,
                })
                .await
                .map(|response| response.into_inner())
                .map_err(|status| match status.code() {
                    // The Agent no longer has this run (rolled over, restarted,
                    // or settled and dropped the ring). Not retryable: the run is
                    // gone, so the caller must reconcile from the journal.
                    Code::FailedPrecondition | Code::NotFound => {
                        AttachFailure::RunGone(status.to_string())
                    }
                    _ => AttachFailure::Transient(status.to_string()),
                })
        }
        .await;

        let mut stream = match attach_result {
            Ok(stream) => stream,
            Err(AttachFailure::RunGone(reason)) => return Err(CollectError::RunGone(reason)),
            Err(AttachFailure::Transient(stream_error)) => {
                reconnect_attempt += 1;
                if reconnect_attempt > STREAM_RECONNECT_ATTEMPTS {
                    return Err(format!(
                        "Future Agent stream could not be resumed after {STREAM_RECONNECT_ATTEMPTS} attempts: {stream_error}"
                    )
                    .into());
                }
                sleep(reconnect_delay(reconnect_attempt)).await;
                continue;
            }
        };

        let stream_error = loop {
            let next_event = if waiting_for_approval {
                stream.message().await
            } else {
                match timeout(
                    Duration::from_secs(AGENT_EVENT_STREAM_TIMEOUT_SECS),
                    stream.message(),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        break "Future Agent response timed out.".to_string();
                    }
                }
            };

            let next_event = match next_event {
                Ok(event) => event,
                Err(error) => {
                    break format!("Future Agent event stream failed: {error}");
                }
            };

            let Some(event) = next_event else {
                break "Future Agent event stream closed before the run terminal.".to_string();
            };
            reconnect_attempt = 0;

            if event.run_id != canonical_run_id {
                return Err(format!(
                    "Future Agent sent event for run {}, expected {canonical_run_id}",
                    event.run_id
                )
                .into());
            }

            if event.projection_snapshot {
                if event.snapshot_cursor < last_idx {
                    continue;
                }
                let snapshot_events: Vec<(String, String, i64)> = event
                    .snapshot_events
                    .iter()
                    .map(|projected| {
                        (
                            projected.r#type.clone(),
                            projected.data.clone(),
                            projected.idx,
                        )
                    })
                    .collect();
                replace_projection_off_thread(thread_id, local_run_id, snapshot_events).await;

                content.clear();
                waiting_for_approval = false;
                let mut snapshot_terminal = None;
                for projected in &event.snapshot_events {
                    match fold_response_event(
                        &projected.r#type,
                        &projected.data,
                        &mut content,
                        &mut waiting_for_approval,
                    )? {
                        FoldOutcome::Continue => {}
                        FoldOutcome::Terminal { clean } => {
                            snapshot_terminal = Some(clean);
                        }
                    }
                }
                last_idx = event.snapshot_cursor;
                if let Some(clean) = snapshot_terminal {
                    break 'attach clean;
                }
                continue;
            }

            if event.r#type == "stream_gap" {
                break "Future Agent reported a legacy stream gap without a projection snapshot"
                    .to_string();
            }
            if event.idx <= last_idx {
                continue;
            }
            if event.idx != last_idx.saturating_add(1) {
                // Drop this stream and atomically reattach from the last
                // confirmed cursor. The Agent will return either the missing
                // tail or a full projection snapshot if the ring rolled over.
                break format!(
                    "Future Agent event gap for run {canonical_run_id}: expected {}, received {}",
                    last_idx.saturating_add(1),
                    event.idx
                );
            }
            last_idx = event.idx;

            persist_run_event_off_thread(
                thread_id,
                local_run_id,
                event.r#type.clone(),
                event.data.clone(),
                event.idx,
            )
            .await;
            // Remote tap (Step B/P1): queue the event for mirroring to mobile/web
            // (no-op when no remote connection; never blocks this loop).
            crate::remote::publish_event(
                session_id,
                &event.r#type,
                &event.data,
                &event.run_id,
                event.idx,
            );
            match fold_response_event(
                &event.r#type,
                &event.data,
                &mut content,
                &mut waiting_for_approval,
            )? {
                FoldOutcome::Continue => {}
                FoldOutcome::Terminal { clean } => {
                    break 'attach clean;
                }
            }
        };

        reconnect_attempt += 1;
        if reconnect_attempt > STREAM_RECONNECT_ATTEMPTS {
            persist_run_event_off_thread(
                thread_id,
                local_run_id,
                "stream_disconnected".to_string(),
                serde_json::json!({"error": stream_error}).to_string(),
                last_idx.saturating_add(1),
            )
            .await;
            return Err(format!(
                "Future Agent stream could not be resumed after {STREAM_RECONNECT_ATTEMPTS} attempts: {stream_error}"
            )
            .into());
        }
        eprintln!(
            "FutureOS reattaching Agent run {canonical_run_id} after idx {last_idx} \
             (attempt {reconnect_attempt}): {stream_error}"
        );
        sleep(reconnect_delay(reconnect_attempt)).await;
    };

    Ok(AgentResponse {
        content,
        complete: clean_end,
    })
}

fn reconnect_delay(attempt: u32) -> Duration {
    Duration::from_millis((100_u64 << attempt.min(4)).min(2_000))
}

enum FoldOutcome {
    Continue,
    Terminal { clean: bool },
}

fn fold_response_event(
    event_type: &str,
    data: &str,
    content: &mut String,
    waiting_for_approval: &mut bool,
) -> Result<FoldOutcome, crate::AppError> {
    match event_type {
        "approval_request" => {
            *waiting_for_approval = true;
        }
        "approval_decision" => {
            *waiting_for_approval = false;
        }
        "text_chunk" => {
            if let Some(text) = event_text(data) {
                content.push_str(&text);
            }
        }
        "agent_end" => {
            return Ok(FoldOutcome::Terminal {
                clean: !agent_end_incomplete(data),
            });
        }
        "error" => {
            return Err(event_error(data)
                .unwrap_or_else(|| "Future Agent returned an error event.".to_string())
                .into());
        }
        _ => {}
    }
    Ok(FoldOutcome::Continue)
}

/// Returns true when an `agent_end` event's data marks the turn as incomplete —
/// i.e. the LLM stream was truncated before a genuine finish. Such a reply is a
/// prefix and must not be persisted as a clean completion.
fn agent_end_incomplete(data: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|value| {
            value
                .get("reason")
                .and_then(|reason| reason.as_str())
                .map(str::to_string)
        })
        .is_some_and(|reason| reason == "incomplete")
}

fn event_text(data: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|value| {
            value
                .get("text")
                .and_then(|text| text.as_str())
                .map(str::to_string)
        })
}

fn event_error(data: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .or_else(|| value.get("message"))
                .and_then(|error| error.as_str())
                .map(str::to_string)
        })
}

#[cfg(test)]
mod tests {
    use super::{
        agent_end_incomplete, fold_response_event, reconnect_delay, FoldOutcome,
        STREAM_RECONNECT_ATTEMPTS,
    };

    #[test]
    fn incomplete_reason_marks_truncated() {
        // Truncated stream: run loop emits agent_end reason "incomplete".
        assert!(agent_end_incomplete(r#"{"reason":"incomplete"}"#));
        assert!(agent_end_incomplete(
            r#"{"reason":"incomplete","stop_reason":"truncated"}"#
        ));
    }

    #[test]
    fn clean_reasons_are_not_truncated() {
        assert!(!agent_end_incomplete(r#"{"reason":"complete"}"#));
        assert!(!agent_end_incomplete(r#"{"reason":"stop_condition"}"#));
        assert!(!agent_end_incomplete(r#"{"reason":"interrupted"}"#));
        // Missing / malformed reason must default to clean, not truncated.
        assert!(!agent_end_incomplete(r#"{"usage":{}}"#));
        assert!(!agent_end_incomplete("not json"));
    }

    #[test]
    fn projection_events_rebuild_response_and_terminal_state() {
        let mut content = String::new();
        let mut waiting = false;
        assert!(matches!(
            fold_response_event(
                "text_chunk",
                r#"{"text":"restored answer"}"#,
                &mut content,
                &mut waiting
            )
            .unwrap(),
            FoldOutcome::Continue
        ));
        assert_eq!(content, "restored answer");
        assert!(matches!(
            fold_response_event(
                "agent_end",
                r#"{"reason":"complete"}"#,
                &mut content,
                &mut waiting
            )
            .unwrap(),
            FoldOutcome::Terminal { clean: true }
        ));
    }

    #[test]
    fn reconnect_backoff_is_bounded() {
        assert!(reconnect_delay(1) < reconnect_delay(2));
        assert_eq!(
            reconnect_delay(STREAM_RECONNECT_ATTEMPTS),
            std::time::Duration::from_millis(1_600)
        );
        assert_eq!(
            reconnect_delay(u32::MAX),
            std::time::Duration::from_millis(1_600)
        );
    }
}
