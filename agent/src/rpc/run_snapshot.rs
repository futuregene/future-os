//! Batch equivalent of the broadcaster's semantic projection. Historical
//! snapshots parse each delta once and serialize each coalesced segment once,
//! rather than repeatedly serializing its growing text for every old token.
use super::SseEvent;
use serde_json::Value;

pub(super) fn fold_events(events: Vec<SseEvent>) -> Vec<SseEvent> {
    let mut output = Vec::new();
    let mut pending: Option<(SseEvent, Value, bool)> = None;
    let flush = |pending: &mut Option<(SseEvent, Value, bool)>, output: &mut Vec<SseEvent>| {
        if let Some((mut event, data, merged)) = pending.take() {
            if merged {
                event.data = serde_json::to_string(&data).expect("JSON value");
            }
            output.push(event);
        }
    };
    for event in events {
        // Raw provider deltas duplicate the on_text-derived text_chunk stream.
        if event.event_type == "text_delta" {
            continue;
        }
        let coalescible = matches!(
            event.event_type.as_str(),
            "text_chunk" | "thinking_delta" | "toolcall_delta" | "tool_delta"
        );
        let data = coalescible
            .then(|| serde_json::from_str::<Value>(&event.data).ok())
            .flatten();
        if let Some(next) = data.filter(|data| data.get("text").is_some_and(Value::is_string)) {
            if let Some((previous, previous_data, merged)) = pending.as_mut() {
                let same_stream = previous.event_type == event.event_type
                    && (!matches!(event.event_type.as_str(), "toolcall_delta" | "tool_delta")
                        || ["tool_id", "tc_index"]
                            .iter()
                            .all(|key| previous_data.get(key) == next.get(key)));
                if same_stream {
                    let Value::String(text) = &mut previous_data["text"] else {
                        unreachable!()
                    };
                    text.push_str(next["text"].as_str().expect("text checked"));
                    previous.idx = event.idx;
                    previous.event_id = event.event_id;
                    previous.timestamp = event.timestamp;
                    previous.run_sequence = event.run_sequence;
                    *merged = true;
                    continue;
                }
            }
            flush(&mut pending, &mut output);
            pending = Some((event, next, false));
        } else {
            flush(&mut pending, &mut output);
            output.push(event);
        }
    }
    flush(&mut pending, &mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::{protocol::apply_to_projection, SseBroadcaster};
    use serde_json::json;

    fn event(kind: &str, idx: i64, data: Value) -> SseEvent {
        SseEvent {
            idx,
            run_id: "r".into(),
            event_id: format!("r:{idx}"),
            timestamp: format!("t:{idx}"),
            run_sequence: 3,
            ..SseEvent::new(kind, data)
        }
    }

    #[test]
    fn batch_matches_live_folding_and_preserves_semantic_boundaries() {
        let mut events = vec![event("agent_start", 0, json!({"started_at_ms":1}))];
        for i in 1..=10_000 {
            events.push(event("text_chunk", i, json!({"text":"中文𠮷"})));
            events.push(event("text_delta", i, json!({"text":"duplicate"})));
        }
        events.extend([
            event("thinking_start", 10001, json!({})),
            event("thinking_delta", 10002, json!({"text":"reason"})),
            event("thinking_delta", 10003, json!({"text":" more"})),
            event("thinking_end", 10004, json!({})),
            event("tool_start", 10005, json!({"tool_call_id":"a"})),
            event(
                "toolcall_delta",
                10006,
                json!({"text":"{", "tool_id":"a", "tc_index":0}),
            ),
            event(
                "toolcall_delta",
                10007,
                json!({"text":"}", "tool_id":"a", "tc_index":0}),
            ),
            event(
                "toolcall_delta",
                10008,
                json!({"text":"other", "tool_id":"b", "tc_index":0}),
            ),
            event(
                "toolcall_delta",
                10009,
                json!({"text":"index", "tool_id":"b", "tc_index":1}),
            ),
            event("tool_end", 10010, json!({"tool_call_id":"a", "text":"ok"})),
            event(
                "approval_request",
                10011,
                json!({"approval_request_id":"p"}),
            ),
            event("usage", 10012, json!({"output_tokens":42})),
            event("agent_end", 10013, json!({"duration_ms":10})),
        ]);
        // Include malformed data and non-text deltas: these remain untouched.
        events.push(SseEvent {
            data: "bad json".into(),
            ..event("text_chunk", 10014, json!({}))
        });
        events.push(event("text_chunk", 10015, json!({"text":42})));
        let mut expected = Vec::new();
        for e in &events {
            apply_to_projection(&mut expected, e);
        }
        let folded = fold_events(events);
        assert_eq!(
            serde_json::to_value(&folded).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(folded[1].idx, 10000);
        assert!(folded.len() < 20);
    }

    #[test]
    fn concurrent_broadcasts_never_pair_a_new_cursor_with_an_old_projection() {
        let b = SseBroadcaster::new();
        b.start_run("r".into(), 1);
        b.broadcast(SseEvent::new("agent_start", json!({})));
        let writer = b.clone();
        let task = std::thread::spawn(move || {
            for _ in 0..1000 {
                writer.broadcast(SseEvent::new("text_chunk", json!({"text":"x"})));
                std::thread::yield_now();
            }
        });
        for _ in 0..100 {
            let snapshot = b.run_snapshot("r").unwrap();
            let text = snapshot
                .events
                .get(1)
                .map(|event| {
                    serde_json::from_str::<Value>(&event.data).unwrap()["text"]
                        .as_str()
                        .unwrap()
                        .len()
                })
                .unwrap_or(0);
            assert_eq!(snapshot.cursor as usize, text);
            std::thread::yield_now();
        }
        task.join().unwrap();
        assert_eq!(b.run_snapshot("r").unwrap().cursor, 1000);
    }

    #[test]
    fn active_snapshot_pins_cursor_and_does_not_change_after_broadcast_or_run_switch() {
        let b = SseBroadcaster::new();
        b.start_run("r".into(), 1);
        b.broadcast(SseEvent::new("agent_start", json!({})));
        for _ in 0..3000 {
            b.broadcast(SseEvent::new("text_chunk", json!({"text":"x"})));
        }
        let first = b.run_snapshot("r").unwrap();
        assert_eq!(first.cursor, 3000);
        assert_eq!(first.events.len(), 2);
        b.broadcast(SseEvent::new("text_chunk", json!({"text":"tail"})));
        assert_eq!(b.events_since("r", first.cursor).unwrap().1.len(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(&first.events[1].data).unwrap()["text"],
            "x".repeat(3000)
        );
        assert_eq!(b.run_snapshot("r").unwrap().cursor, 3001);
        b.start_run("new".into(), 2);
        assert!(b.run_snapshot("r").is_err());
        assert!(b.run_snapshot("").is_err());
    }
}
