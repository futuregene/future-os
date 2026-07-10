//! Consumes the Future Agent event stream for a single prompt: accumulates
//! assistant text, drives the approval wait-state, and forwards every event to
//! the persistence projection. Returns the assembled assistant text once the
//! agent signals `agent_end`.

use tokio::time::{timeout, Duration};

use super::persist::persist_run_event;
use crate::store;

const AGENT_EVENT_STREAM_TIMEOUT_SECS: u64 = 600;

/// Persist a run event on a blocking thread, so the synchronous SQLite write
/// (and the occasional `git` fork on write/artifact events) doesn't stall the
/// async event loop. Awaited to preserve event order; errors are logged inside
/// `persist_run_event`.
async fn persist_run_event_off_thread(
    run_id: Option<&str>,
    event_type: String,
    data: String,
    sequence: i64,
) {
    let run_id = run_id.map(str::to_string);
    let _ = tokio::task::spawn_blocking(move || {
        persist_run_event(run_id.as_deref(), &event_type, &data, sequence);
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
    stream: &mut tonic::Streaming<crate::agent_proto::StreamEvent>,
    run_id: Option<&str>,
    session_id: &str,
) -> Result<AgentResponse, crate::AppError> {
    let mut content = String::new();
    let mut saw_agent_end = false;
    let mut waiting_for_approval = false;
    let mut sequence = 0_i64;

    loop {
        let next_event = if waiting_for_approval {
            stream
                .message()
                .await
                .map_err(|error| format!("Future Agent event stream failed: {error}"))?
        } else {
            match timeout(
                Duration::from_secs(AGENT_EVENT_STREAM_TIMEOUT_SECS),
                stream.message(),
            )
            .await
            {
                Ok(result) => {
                    result.map_err(|error| format!("Future Agent event stream failed: {error}"))?
                }
                Err(_) => {
                    persist_run_event_off_thread(
                        run_id,
                        "timeout".to_string(),
                        r#"{"error":"Future Agent response timed out."}"#.to_string(),
                        sequence,
                    )
                    .await;
                    return Err("Future Agent response timed out.".to_string().into());
                }
            }
        };

        let Some(event) = next_event else {
            break;
        };

        persist_run_event_off_thread(run_id, event.r#type.clone(), event.data.clone(), sequence)
            .await;
        // Remote tap (Step B/P1): mirror events to mobile/web (no-op when no remote connection).
        crate::remote::publish_event(
            session_id,
            &event.r#type,
            &event.data,
            &event.run_id,
            event.idx,
        )
        .await;
        sequence += 1;

        match event.r#type.as_str() {
            "approval_request" => {
                waiting_for_approval = true;
            }
            "approval_decision" => {
                waiting_for_approval = false;
            }
            "text_chunk" => {
                if let Some(text) = event_text(&event.data) {
                    content.push_str(&text);
                }
            }
            "agent_end" => {
                saw_agent_end = true;
                if let Some(rid) = run_id {
                    store::clear_run_event_buffer(rid);
                }
                break;
            }
            "error" => {
                return Err(event_error(&event.data)
                    .unwrap_or_else(|| "Future Agent returned an error event.".to_string())
                    .into());
            }
            _ => {}
        }
    }

    if content.trim().is_empty() && !saw_agent_end {
        Err("Future Agent finished without returning any text."
            .to_string()
            .into())
    } else {
        Ok(AgentResponse {
            content,
            complete: saw_agent_end,
        })
    }
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
