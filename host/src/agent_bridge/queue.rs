//! What a conversation has in flight and waiting, read from the Agent.
//!
//! The queue belongs to the session, not to the client that filled it: a phone,
//! a computer driving that desktop, and the desktop's own window all see the same
//! rows, and a row exists whether or not the sender is still connected. So it is
//! read here, from the Agent's `get_state`, instead of being tracked per client —
//! a client-side copy could only ever be a guess about someone else's queue.

use super::client::{connect_agent, get_state_command};
use serde_json::{json, Value};

/// The queue of one conversation: the run in flight and the rows behind it.
///
/// An empty or unknown session answers with an empty queue rather than an error:
/// a view that has not sent anything yet has nothing to show, and "no queue" is
/// the truthful answer for it.
pub async fn session_queue_snapshot(session_id: &str) -> Result<Value, crate::Error> {
    if session_id.trim().is_empty() {
        return Ok(empty_queue());
    }
    let mut client = connect_agent().await?;
    super::verify_session_database(&mut client, session_id).await?;
    let response = client
        .execute_command(get_state_command(session_id.to_string()))
        .await
        .map_err(|error| format!("get_state: {error}"))?
        .into_inner();
    if !response.success {
        return Err(format!("Future Agent rejected get_state: {}", response.error).into());
    }
    let state: Value = future_rpc::decode::response_data(&response);
    Ok(queue_state_json(&state))
}

/// An idle conversation with nothing waiting.
pub fn empty_queue() -> Value {
    json!({ "activeRunId": Value::Null, "queued": [] })
}

/// The queue half of a `get_state` payload, normalised for clients.
///
/// Typed `get_state` emits canonical camelCase; the snake_case spelling is still
/// accepted for an older Agent. Kept apart from the RPC so the shape a client
/// relies on is testable without an Agent.
pub fn queue_state_json(state: &Value) -> Value {
    let field = |value: &Value, camel: &str, snake: &str| -> Option<Value> {
        value.get(camel).or_else(|| value.get(snake)).cloned()
    };
    let queued = field(state, "queuedRuns", "queued_runs")
        .and_then(|rows| rows.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|row| {
            json!({
                "runId": field(row, "runId", "run_id").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
                "position": field(row, "queuePosition", "queue_position").and_then(|v| v.as_i64()).unwrap_or(0),
                "acceptedAt": field(row, "acceptedAt", "accepted_at").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
                "text": field(row, "displayText", "display_text").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    let active_run_id = field(state, "activeRun", "active_run")
        .and_then(|run| field(&run, "runId", "run_id"))
        .and_then(|id| id.as_str().map(str::to_string));
    json!({ "activeRunId": active_run_id, "queued": queued })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The queue a client shows is the Agent's own, in the Agent's order, with the
    /// text and acceptance time it recorded.
    #[test]
    fn the_queue_reads_the_agents_order_and_text() {
        let state = json!({
            "activeRun": { "runId": "run_live" },
            "queuedRuns": [
                { "runId": "run_2", "queuePosition": 1, "acceptedAt": "2026-11-03T10:00:00Z", "displayText": "second" },
                { "runId": "run_3", "queuePosition": 2, "acceptedAt": "2026-11-03T10:00:01Z", "displayText": "third" }
            ]
        });
        let snapshot = queue_state_json(&state);
        assert_eq!(snapshot["activeRunId"], "run_live");
        let queued = snapshot["queued"].as_array().expect("a list");
        assert_eq!(queued.len(), 2);
        assert_eq!(queued[0]["runId"], "run_2");
        assert_eq!(queued[0]["position"], 1);
        assert_eq!(queued[0]["text"], "second");
        assert_eq!(queued[0]["acceptedAt"], "2026-11-03T10:00:00Z");
    }

    /// An older Agent spells the same fields with underscores.
    #[test]
    fn the_older_snake_case_spelling_still_reads() {
        let legacy = json!({
            "queued_runs": [
                { "run_id": "run_9", "queue_position": 1, "display_text": "old agent" }
            ]
        });
        let snapshot = queue_state_json(&legacy);
        assert!(
            snapshot["activeRunId"].is_null(),
            "an idle session has no active run"
        );
        assert_eq!(snapshot["queued"][0]["runId"], "run_9");
        assert_eq!(snapshot["queued"][0]["text"], "old agent");

        let idle = queue_state_json(&json!({}));
        assert_eq!(idle["queued"].as_array().expect("a list").len(), 0);
        assert_eq!(empty_queue(), idle);
    }
}
