//! Batch equivalent of the broadcaster's semantic projection. Historical
//! snapshots parse each delta once and serialize each coalesced segment once,
//! rather than repeatedly serializing its growing text for every old token.
use super::SseEvent;
use serde_json::Value;

pub(super) fn fold_events(events: Vec<SseEvent>) -> Vec<SseEvent> {
    strip_repeated_tool_arguments(coalesce_deltas(events))
}

/// Drop the duplicate copies of a tool call's argument JSON.
///
/// A tool's arguments reach a client three ways: the streamed `tool_delta`
/// fragments, the `input`-phase `tool_start` (which carries no arguments), and
/// the `execution`-phase `tool_start`, which carries the parsed object the
/// fragments spell out. Measured on real runs, the delta stream and the
/// execution start hold byte-equivalent JSON for every tool, so a snapshot paid
/// for the same arguments twice — 20-30% of a long run's snapshot, in the two
/// busiest event types a cold open has to download.
///
/// The delta entries add nothing a client can act on once the execution start
/// is present: both feed exactly one piece of state, the tool row's target, and
/// the start's parsed object is the authoritative form. This keeps the start
/// and removes the fragment stream it duplicates.
///
/// The equality is checked, never assumed. A tool whose fragments were
/// truncated, rewritten, or never followed by an execution start keeps its
/// deltas, so an interrupted tool call still shows what it had.
pub(super) fn strip_repeated_tool_arguments(events: Vec<SseEvent>) -> Vec<SseEvent> {
    let mut tools_with_arguments = Vec::new();
    for event in &events {
        if event.event_type != "tool_start" {
            continue;
        }
        let Ok(data) = serde_json::from_str::<serde_json::Value>(&event.data) else {
            continue;
        };
        let Some(tool) = tool_id_of(&data) else {
            continue;
        };
        if carries_arguments(&data) {
            tools_with_arguments.push((tool.to_owned(), data["tool_args"].clone()));
        }
    }
    if tools_with_arguments.is_empty() {
        return events;
    }
    let repeated: Vec<String> = tools_with_arguments
        .into_iter()
        .filter(|(tool, args)| {
            let streamed = streamed_arguments(&events, tool);
            serde_json::from_str::<serde_json::Value>(&streamed)
                .map(|value| value == *args)
                .unwrap_or(false)
        })
        .map(|(tool, _)| tool)
        .collect();
    if repeated.is_empty() {
        return events;
    }
    events
        .into_iter()
        .filter(|event| {
            if !matches!(event.event_type.as_str(), "tool_delta" | "toolcall_delta") {
                return true;
            }
            let Ok(data) = serde_json::from_str::<serde_json::Value>(&event.data) else {
                return true;
            };
            !tool_id_of(&data).is_some_and(|tool| repeated.iter().any(|known| known == tool))
        })
        .collect()
}

/// Rebuild the argument text a client accumulates from a tool's delta stream:
/// a `snapshot` delta replaces the text so far, any other fragment appends.
fn streamed_arguments(events: &[SseEvent], tool: &str) -> String {
    let mut streamed = String::new();
    for event in events {
        if !matches!(event.event_type.as_str(), "tool_delta" | "toolcall_delta") {
            continue;
        }
        let Ok(data) = serde_json::from_str::<serde_json::Value>(&event.data) else {
            continue;
        };
        if tool_id_of(&data) != Some(tool) {
            continue;
        }
        if data.get("snapshot").and_then(serde_json::Value::as_bool) == Some(true) {
            streamed.clear();
        }
        if let Some(text) = data.get("text").and_then(serde_json::Value::as_str) {
            streamed.push_str(text);
        }
    }
    streamed
}

/// Whether a `tool_start` actually carries the call's arguments: an `input`
/// start announces the call with none, and only a populated payload is worth
/// comparing against the streamed fragments.
fn carries_arguments(data: &Value) -> bool {
    match data.get("tool_args") {
        None | Some(Value::Null) => false,
        Some(Value::Object(arguments)) => !arguments.is_empty(),
        Some(Value::Array(arguments)) => !arguments.is_empty(),
        Some(Value::String(arguments)) => !arguments.is_empty(),
        Some(_) => true,
    }
}

fn tool_id_of(data: &serde_json::Value) -> Option<&str> {
    data.get("tool_id")
        .or_else(|| data.get("tool_call_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
}

fn coalesce_deltas(events: Vec<SseEvent>) -> Vec<SseEvent> {
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

    /// A `tool_start` only carries arguments when the payload is present and
    /// non-empty; every other shape means "no arguments yet".
    #[test]
    fn tool_start_arguments_are_only_significant_when_present_and_non_empty() {
        for (data, expected) in [
            (json!({}), false),
            (json!({"tool_args": null}), false),
            (json!({"tool_args": {}}), false),
            (json!({"tool_args": []}), false),
            (json!({"tool_args": ""}), false),
            (json!({"tool_args": {"path": "a"}}), true),
            (json!({"tool_args": [1]}), true),
            (json!({"tool_args": "{}"}), true),
            // A non-JSON scalar still counts as supplied arguments.
            (json!({"tool_args": 7}), true),
        ] {
            assert_eq!(carries_arguments(&data), expected, "for {data}");
        }
    }

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
        // A snapshot is the live projection plus the argument de-duplication
        // both product branches apply at read time.
        assert_eq!(
            serde_json::to_value(&folded).unwrap(),
            serde_json::to_value(strip_repeated_tool_arguments(expected)).unwrap()
        );
        assert_eq!(folded[1].idx, 10000);
        assert!(folded.len() < 20);
    }

    /// One tool call whose argument fragments spell out exactly what the
    /// execution start carries, written the way the agent emits it.
    fn tool_call_events(tool: &str, args: &str, base: i64) -> Vec<SseEvent> {
        let mut events = vec![event(
            "tool_start",
            base,
            json!({"tool_id":tool, "tool_name":"shell", "phase":"input", "tool_args":""}),
        )];
        for (index, character) in args.char_indices() {
            events.push(event(
                "tool_delta",
                base + 1 + index as i64,
                json!({"tool_id":tool, "text":character.to_string()}),
            ));
        }
        events.push(event(
            "tool_start",
            base + 1000,
            json!({"tool_id":tool, "tool_name":"shell", "phase":"execution",
                   "tool_args": serde_json::from_str::<Value>(args).unwrap()}),
        ));
        events.push(event(
            "tool_end",
            base + 1001,
            json!({"tool_id":tool, "text":"done"}),
        ));
        events
    }

    #[test]
    fn repeated_tool_arguments_are_stripped_only_when_they_match() {
        let mut events = vec![event("agent_start", 0, json!({}))];
        events.extend(tool_call_events("a", "{\"path\":\"/tmp/one\"}", 10));
        // A second tool whose fragments stop short of its execution start: the
        // fragments are the only copy of what it had, so they must survive.
        let mut partial = tool_call_events("b", "{\"path\":\"/tmp/two\"}", 2000);
        let dropped = partial
            .iter()
            .rposition(|event| event.event_type == "tool_delta")
            .unwrap();
        partial.remove(dropped);
        events.extend(partial);
        events.push(event("agent_end", 9000, json!({})));

        let stripped = strip_repeated_tool_arguments(events.clone());

        let tool = |id: &str, kind: &str| {
            stripped
                .iter()
                .filter(|event| {
                    event.event_type == kind
                        && serde_json::from_str::<Value>(&event.data)
                            .map(|data| data["tool_id"] == id)
                            .unwrap_or(false)
                })
                .count()
        };
        assert_eq!(tool("a", "tool_delta"), 0, "duplicate arguments were kept");
        assert_eq!(tool("a", "tool_start"), 2, "both starts describe the call");
        assert!(
            tool("b", "tool_delta") > 0,
            "partial fragments were dropped"
        );
        // Order and cursor stay valid: the client validates a strictly
        // increasing index, and the strip only removes interior events.
        let idxs: Vec<i64> = stripped.iter().map(|event| event.idx).collect();
        assert!(idxs.windows(2).all(|pair| pair[1] > pair[0]));
        assert_eq!(stripped.last().unwrap().idx, 9000);
        assert_eq!(stripped.first().unwrap().idx, 0);
        // Idempotent: a second pass has nothing left to remove.
        assert_eq!(
            serde_json::to_value(strip_repeated_tool_arguments(stripped.clone())).unwrap(),
            serde_json::to_value(&stripped).unwrap()
        );
    }

    #[test]
    fn snapshot_style_argument_fragments_are_reconstructed_before_comparing() {
        // A provider that re-sends the whole accumulated arguments marks the
        // fragment `snapshot: true`; that replaces the text so far instead of
        // appending, and the de-duplication has to read it the same way.
        let args = "{\"command\":\"ls -la\"}";
        let deltas = vec![
            event(
                "tool_delta",
                21,
                json!({"tool_id":"c", "snapshot":true, "text":"{old"}),
            ),
            event(
                "tool_delta",
                22,
                json!({"tool_id":"c", "snapshot":true, "text":args}),
            ),
        ];
        let mut events = vec![event(
            "tool_start",
            20,
            json!({"tool_id":"c", "tool_name":"shell", "phase":"input", "tool_args":""}),
        )];
        events.extend(deltas);
        events.push(event(
            "tool_start",
            30,
            json!({"tool_id":"c", "phase":"execution",
                   "tool_args": serde_json::from_str::<Value>(args).unwrap()}),
        ));
        let stripped = strip_repeated_tool_arguments(events);
        assert!(stripped
            .iter()
            .all(|event| event.event_type != "tool_delta"));
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
