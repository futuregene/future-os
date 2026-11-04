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
    json!({ "activeRunId": Value::Null, "queued": [], "recentTerminalAcks": [] })
}

/// How many terminal acks a queue answer carries.
///
/// They say what became of something that left the queue without running, so a
/// handful is enough to cover the rows a reader can still see. The Agent keeps
/// its own (bounded) history for reconnect recovery; this is a display window,
/// and an unbounded one would keep growing the response body for nobody.
pub const QUEUE_ACK_WINDOW: usize = 20;

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
                // "queued" runs at the turn boundary; "stale" waited too long and
                // waits for a person instead. Same row, one different sentence.
                "state": field(row, "state", "state").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "queued".to_string()),
            })
        })
        .collect::<Vec<_>>();
    let active_run_id = field(state, "activeRun", "active_run")
        .and_then(|run| field(&run, "runId", "run_id"))
        .and_then(|id| id.as_str().map(str::to_string));
    // Why a queued prompt stopped being queued without running: folded into the
    // run ahead (`merged`), dropped by the reader, or refused before it started.
    // Without these a row would simply vanish, and the reader would believe it
    // had been sent.
    let recent = field(state, "recentTerminalAcks", "recent_terminal_acks")
        .and_then(|rows| rows.as_array().cloned())
        .unwrap_or_default();
    // The same window also holds how the run that *did* run ended (`settled`).
    // That is the reply the reader just watched finish, not a row that left the
    // queue, so it is not news a client can announce: forwarded, it says a
    // waiting message went missing when nothing was waiting. Dropped before the
    // cap, so finished turns cannot push the real acks out of the window.
    let mut recent = recent
        .iter()
        .filter(|row| {
            field(row, "reason", "reason")
                .and_then(|v| v.as_str().map(str::to_string))
                .as_deref()
                != Some("settled")
        })
        .map(|row| {
            json!({
                "runId": field(row, "runId", "run_id").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
                "state": field(row, "state", "state").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
                "reason": field(row, "reason", "reason").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    // A display window, not an audit log: keep the newest ones and drop the rest,
    // so one long-lived conversation cannot grow every queue answer without
    // bound.
    recent.drain(..recent.len().saturating_sub(QUEUE_ACK_WINDOW));
    json!({ "activeRunId": active_run_id, "queued": queued, "recentTerminalAcks": recent })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The state and the window: a row says whether it is waiting to run or
    /// parked, and the acks are capped so one long conversation cannot grow every
    /// queue answer.
    #[test]
    fn the_queue_reports_why_a_row_is_waiting_and_caps_its_acks() {
        let state = json!({
            "queuedRuns": [
                { "runId": "run_auto", "queuePosition": 1, "state": "queued", "displayText": "runs at the boundary" },
                { "runId": "run_old", "queuePosition": 2, "state": "stale:stale", "displayText": "waited too long" },
                { "runId": "run_odd", "queuePosition": 3, "state": "stale:unknown_age", "displayText": "age unknown" }
            ],
            "recentTerminalAcks": (0..40)
                .map(|index| json!({"runId": format!("run_{index}"), "state": "terminal", "reason": "merged"}))
                .collect::<Vec<_>>()
        });
        let snapshot = queue_state_json(&state);
        let queued = snapshot["queued"].as_array().expect("a list");
        assert_eq!(queued[0]["state"], "queued");
        assert_eq!(queued[1]["state"], "stale:stale");
        assert_eq!(
            queued[2]["state"], "stale:unknown_age",
            "no readable age is its own answer, not 'long ago'"
        );

        let acks = snapshot["recentTerminalAcks"].as_array().expect("a list");
        assert_eq!(acks.len(), QUEUE_ACK_WINDOW);
        assert_eq!(
            acks.last().expect("an ack")["runId"],
            "run_39",
            "the newest are kept"
        );
        assert_eq!(
            acks.first().expect("an ack")["runId"],
            "run_20",
            "the oldest fall out of the window"
        );
    }

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

    /// The run that actually ran ends as `settled`. A client reads this list as
    /// "rows that left the queue without running", so that ack says the wrong
    /// thing: a waiting message would go missing when nothing was waiting. It is
    /// dropped, and the reasons that are news pass through untouched.
    #[test]
    fn a_finished_turn_is_not_a_row_that_left_the_queue() {
        let state = json!({
            "recentTerminalAcks": [
                { "runId": "run_ran", "state": "terminal", "reason": "settled" },
                { "runId": "run_merged", "state": "terminal", "reason": "merged" },
                { "runId": "run_taken_back", "state": "cancelled", "reason": "cancelled" }
            ]
        });
        let acks = queue_state_json(&state)["recentTerminalAcks"]
            .as_array()
            .expect("a list")
            .clone();
        let reasons: Vec<&str> = acks
            .iter()
            .map(|ack| ack["reason"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(
            reasons,
            vec!["merged", "cancelled"],
            "only the rows that left the queue without running are news"
        );
    }

    /// Finished turns are the common case, so they must not spend the window: a
    /// long conversation would otherwise push out every real ack it still wants
    /// announced.
    #[test]
    fn finished_turns_do_not_crowd_out_the_rows_that_really_left() {
        let mut acks: Vec<Value> = (0..30)
            .map(|index| json!({"runId": format!("run_{index}"), "state": "terminal", "reason": "settled"}))
            .collect();
        acks.push(json!({"runId": "run_last", "state": "terminal", "reason": "merged"}));
        let snapshot = queue_state_json(&json!({ "recentTerminalAcks": acks }));
        let rows = snapshot["recentTerminalAcks"].as_array().expect("a list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["runId"], "run_last");
    }
}
