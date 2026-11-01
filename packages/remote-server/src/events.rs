//! Ordered event publishing and lossless stream coalescing.
use crate::{diagnostics::FailureEpisode, secure};
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
mod coalesce;

pub fn is_catalog_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "agent_start"
            | "agent_end"
            | "approval_request"
            | "approval_decision"
            | "session_name_changed"
            | "provider_config_changed"
            | "model_visibility_changed"
            | "app_settings_changed"
            | "skills_changed"
            | "run_snapshot"
            | "error"
    )
}

pub const MAX_EVENT_BYTES: usize = 900 * 1024;

/// One agent event queued for publishing, in agent-emission order.
pub struct EventPublish {
    pub subject: String,
    pub payload: Vec<u8>,
    pub status_subject: Option<String>,
}

impl Clone for EventPublish {
    fn clone(&self) -> Self {
        Self {
            subject: self.subject.clone(),
            payload: self.payload.clone(),
            status_subject: self.status_subject.clone(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_event_body(
    session_id: &str,
    event_type: &str,
    data: &str,
    run_id: &str,
    idx: i64,
    epoch: i64,
    event_id: &str,
    timestamp: &str,
    session_idx: i64,
    run_sequence: i64,
) -> serde_json::Value {
    let data = cap_event_data(data);
    json!({
        "schemaVersion": 2,
        "sessionId": session_id,
        "type": event_type,
        "data": data,
        "runId": run_id,
        "idx": idx,
        "epoch": epoch,
        "eventId": event_id,
        "timestamp": timestamp,
        "sessionIdx": session_idx,
        "runSequence": run_sequence,
    })
}

pub fn cap_event_data(data: &str) -> std::borrow::Cow<'_, str> {
    if data.len() <= MAX_EVENT_BYTES {
        return std::borrow::Cow::Borrowed(data);
    }
    std::borrow::Cow::Owned(crate::wire_limits::truncated_event_data(data))
}

/// Poll the single ordered lane until its senders close or transport fails.
/// The host owns this future and may cancel it as part of generation shutdown.
pub async fn run_event_publisher(
    client: async_nats::Client,
    mut rx: tokio::sync::mpsc::Receiver<EventPublish>,
    security: secure::Transport,
    coalesce: Arc<AtomicBool>,
) {
    let failures = FailureEpisode::default();

    let mut coalescer = coalesce::Coalescer::default();
    let mut out: Vec<EventPublish> = Vec::new();
    loop {
        let coalescing = coalesce.load(Ordering::Relaxed);
        let received = if coalescing {
            // The window is what makes merging possible at all: an
            // unmerged fragment is published alone, so a slow stream keeps
            // its latency and only bursts are merged.
            tokio::time::timeout(coalesce::COALESCE_WINDOW, rx.recv())
                .await
                .ok()
        } else {
            // Leaving the lane must not strand a buffered group.
            coalescer.flush(&mut out);
            Some(rx.recv().await)
        };
        match received {
            None => coalescer.flush(&mut out),
            Some(None) => {
                coalescer.flush(&mut out);
                if publish_batch(&client, &security, &mut out, &failures)
                    .await
                    .is_err()
                {
                    return;
                }
                return;
            }
            Some(Some(event)) => {
                if coalescing {
                    coalescer.offer(event, std::time::Instant::now(), &mut out);
                } else {
                    out.push(event);
                }
            }
        }
        if publish_batch(&client, &security, &mut out, &failures)
            .await
            .is_err()
        {
            return;
        }
    }
}

/// Publish one batch in order, stopping at the first transport failure. The
/// caller ends the drain then, exactly as the per-event loop did.
async fn publish_batch(
    client: &async_nats::Client,
    security: &secure::Transport,
    batch: &mut Vec<EventPublish>,
    failures: &FailureEpisode,
) -> Result<(), crate::Error> {
    if batch.is_empty() {
        return Ok(());
    }
    for event in std::mem::take(batch) {
        let sent = async {
            if let Some(subject) = event.status_subject {
                secure::publish(client, security, subject, event.payload.clone()).await?;
            }
            secure::publish(client, security, event.subject, event.payload).await
        }
        .await;
        match sent {
            // `Ok(false)` is a payload the secure channel could not seal yet
            // (pre-handshake, or just after a credential refresh). The event lane
            // keeps its existing contract: the client recovers the gap through
            // `get_events_since` backfill.
            Ok(_) => {
                if let Some(line) = failures.recovered() {
                    future_remote_protocol::remote_log!("{line}");
                }
            }
            Err(error) => {
                if let Some(line) = failures.record("event_publish", error) {
                    future_remote_protocol::remote_log!("{line}");
                }
                return Err(crate::Error::Message("event publish failed".into()));
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nats_v2_event_keeps_every_v1_field_unchanged() {
        let body = build_event_body(
            "session-1",
            "text_chunk",
            r#"{"text":"hi"}"#,
            "run-1",
            7,
            2,
            "evt-1",
            "2026-08-02T00:00:00Z",
            -1,
            11,
        );
        assert_eq!(body["type"], "text_chunk");
        assert_eq!(body["data"], r#"{"text":"hi"}"#);
        assert_eq!(body["runId"], "run-1");
        assert_eq!(body["idx"], 7);
        assert_eq!(body["schemaVersion"], 2);
        assert_eq!(body["sessionId"], "session-1");
        assert_eq!(body["epoch"], 2);
        assert_eq!(body["eventId"], "evt-1");
        assert_eq!(body["runSequence"], 11);
    }
    #[test]
    fn nats_session_event_has_independent_cursor() {
        let body = build_event_body(
            "session-1",
            "model_changed",
            "{}",
            "",
            -1,
            0,
            "session-1:session:4",
            "2026-08-02T00:00:00Z",
            4,
            -1,
        );
        assert_eq!(body["runId"], "");
        assert_eq!(body["sessionIdx"], 4);
        assert_eq!(body["runSequence"], -1);
    }
}

#[cfg(test)]
mod lane_tests {
    use super::*;
    #[test]
    fn catalog_lane_keeps_status_and_approvals_but_not_token_traffic() {
        for event in [
            "agent_start",
            "agent_end",
            "approval_request",
            "approval_decision",
            "error",
            "session_name_changed",
            "provider_config_changed",
            "model_visibility_changed",
            "app_settings_changed",
            "skills_changed",
            "run_snapshot",
        ] {
            assert!(is_catalog_event(event), "{event}");
        }
        for event in [
            "text_chunk",
            "thinking_delta",
            "tool_delta",
            "toolcall_delta",
            "tool_end",
        ] {
            assert!(!is_catalog_event(event), "{event}");
        }
    }
}
