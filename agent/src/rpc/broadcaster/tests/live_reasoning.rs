//! Reading a run's reasoning back while it is being written.
use super::*;

fn thinking(broadcaster: &SseBroadcaster, events: &[(&str, serde_json::Value)]) {
    for (kind, data) in events {
        broadcaster.broadcast(SseEvent::new(kind, data.clone()));
    }
}

fn live_thinking() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("thinking_start", serde_json::json!({"block_id": "b0"})),
        (
            "thinking_delta",
            serde_json::json!({"block_id": "b0", "text": "half a "}),
        ),
        (
            "thinking_delta",
            serde_json::json!({"block_id": "b0", "text": "thought"}),
        ),
    ]
}

/// The read a row opened mid-reasoning makes: the block the run is writing, in
/// the order the reply draws it — the same order the client's row index counts.
#[test]
fn the_run_being_written_answers_with_what_it_has_so_far() {
    let broadcaster = SseBroadcaster::new();
    broadcaster.start_run("live".into(), 1);
    thinking(&broadcaster, &live_thinking());

    assert_eq!(
        broadcaster.live_reasoning_blocks("live"),
        Some(vec![Some("half a thought".into())])
    );
}

/// Only the run this process is writing answers: an older or unknown run is the
/// stored record's business, and a kept projection has this text trimmed away.
#[test]
fn a_run_that_is_not_the_one_being_written_answers_nothing() {
    let broadcaster = SseBroadcaster::new();
    assert_eq!(broadcaster.live_reasoning_blocks("live"), None);

    broadcaster.start_run("live".into(), 1);
    thinking(&broadcaster, &live_thinking());
    assert_eq!(broadcaster.live_reasoning_blocks("another"), None);
}

/// Once a run's terminal is in the fold, its reply is the stored one: the index
/// a row asks with is counted against that record from then on, not against the
/// stream's blocks.
#[test]
fn a_finished_run_is_read_from_the_record_not_the_stream() {
    let broadcaster = SseBroadcaster::new();
    broadcaster.start_run("live".into(), 1);
    thinking(&broadcaster, &live_thinking());
    broadcaster.broadcast(SseEvent::new("agent_end", serde_json::json!({})));

    assert_eq!(broadcaster.live_reasoning_blocks("live"), None);
}
