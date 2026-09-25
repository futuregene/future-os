//! The lean event feed: what a phone that does not render reasoning or tool
//! argument text can do without.
//!
//! Measured on the three heaviest completed runs, a run's live lane costs 67-77
//! MB raw / 4.4-4.7 MB after coalescing, and two event types are almost all of
//! it: `thinking_delta` (57-84%) and `tool_delta` (4-41%). Both are per-token
//! streams whose content no phone surface shows — `thinking_start` alone opens
//! the "thinking" row, and a tool row's target already rides `tool_start`'s
//! complete `tool_args`.
//!
//! Dropping them is therefore not a fidelity trade, it is dead weight. Once
//! rewritten, the same runs deliver 0.9-1.4 MB raw / 0.5-0.9 MB coalesced.
//!
//! Two rules keep this honest:
//!
//! - **Only named types are touched.** Anything unrecognized is forwarded
//!   byte-for-byte, so a future event type cannot be silently reshaped.
//! - **Structure is preserved where a peer validates it.** The folded projection
//!   that rides a snapshot has to keep every event with its `idx` (both the
//!   desktop and the client reject an empty or reordered list), so there the
//!   text is blanked rather than the event removed.
//!
//! # Why the flag lives here
//!
//! The live lane (`remote::publisher::publish_event`, once per event) and the
//! replay/history replies (`remote_host::business::*`, once per request) cannot
//! share a handle without threading one through both layers, so the declaration
//! is recorded in a single [`AtomicBool`] — the same shape as
//! `store::CATALOG_DIRTY`. `remote::transport::build_transport` clears it on
//! every new connection: the declaration belongs to the connection that made it,
//! and an older client on the same pairing must not inherit it.

use serde_json::Value;
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};

/// Capability a client declares on `secure_ready` to receive the lean lane.
/// Named so an older desktop ignores it and an older client never asks for it.
pub(crate) const LEAN_EVENTS_FEATURE: &str = "lean_events_v1";

/// Event types that exist only to stream content a phone does not render.
const DROPPED_EVENTS: [&str; 3] = ["thinking_delta", "tool_delta", "toolcall_delta"];

static LEAN_EVENTS: AtomicBool = AtomicBool::new(false);

pub(crate) fn enabled() -> bool {
    LEAN_EVENTS.load(Ordering::Acquire)
}

pub(crate) fn set_enabled(enabled: bool) {
    LEAN_EVENTS.store(enabled, Ordering::Release);
}

/// Whether a `secure_ready` feature list asked for the lean feed.
pub(crate) fn feature_declared(features: &[String]) -> bool {
    features
        .iter()
        .any(|feature| feature == LEAN_EVENTS_FEATURE)
}

/// True when an event's type is streamed-only content.
fn is_dropped(event_type: &str) -> bool {
    DROPPED_EVENTS.contains(&event_type)
}

/// The lean rewrite of one event's `data`, or `None` when the event is dropped.
///
/// Reads `event_type` before any parsing, which is what keeps the hot path
/// cheap: the two highest-frequency types cost one slice compare each, and only
/// the per-call types (`tool_end`, `run_snapshot`) are parsed at all.
pub(crate) fn lean_event_data<'a>(event_type: &str, data: &'a str) -> Option<Cow<'a, str>> {
    if is_dropped(event_type) {
        return None;
    }
    match event_type {
        // Outcome fields (`exit_code`, `is_soft_fail`, `target_path`, `error`)
        // stay; only the captured output goes. The client reads those instead of
        // parsing an `[exit: N]` footer out of the text.
        "tool_end" | "tool_result" => Some(without(data, &["text", "result"])),
        // The folded events are the same reasoning and argument fragments again.
        // The client treats this event as a "resync me" signal and never reads
        // them, so the whole array goes.
        "run_snapshot" => Some(without(data, &["snapshotEvents"])),
        _ => Some(Cow::Borrowed(data)),
    }
}

/// Remove `keys` from a JSON object, leaving anything unparsable untouched.
fn without<'a>(data: &'a str, keys: &[&str]) -> Cow<'a, str> {
    let Ok(Value::Object(mut fields)) = serde_json::from_str::<Value>(data) else {
        return Cow::Borrowed(data);
    };
    for key in keys {
        fields.remove(*key);
    }
    Cow::Owned(serde_json::to_string(&Value::Object(fields)).expect("a Value always serializes"))
}

/// Blank the `text` of a folded projection event, keeping the event itself.
fn blank_folded_text(event: &mut Value) {
    let Some(data) = event["data"].as_str() else {
        return;
    };
    let Ok(Value::Object(mut fields)) = serde_json::from_str::<Value>(data) else {
        return;
    };
    fields.insert("text".into(), Value::String(String::new()));
    if let Ok(blanked) = serde_json::to_string(&Value::Object(fields)) {
        event["data"] = Value::String(blanked);
    }
}

/// Rewrite the events of one replay page for the lean feed.
///
/// **Call this after a page's cursor fields are computed.** Dropping an event
/// must not move `nextSinceIdx` or `watermark`: the client resumes from the
/// cursor it was given and treats it as "I hold everything through this index",
/// so shrinking the window to the last surviving event would leave it
/// re-fetching a range whose events are always dropped — an endless loop.
///
/// Two shapes are handled, and deliberately differently:
///
/// - `events` — raw journal deltas. Dropped, because here the envelope is the
///   cost (per-token events repeat ~450 B of identity per fragment).
/// - `projection.events` — the folded semantic projection. Its events carry
///   `idx` values the client validates for strict ordering and bounds against
///   the snapshot cursor, and both the desktop and the client reject an empty
///   list, so the text is blanked and the event kept.
pub(crate) fn lean_replay_page(page: &mut Value, lean: bool) {
    if !lean {
        return;
    }
    if let Some(events) = page.get_mut("events").and_then(Value::as_array_mut) {
        events.retain_mut(|event| {
            let Some(event_type) = event["type"].as_str().map(str::to_owned) else {
                return true;
            };
            let Some(data) = event["data"].as_str().map(str::to_owned) else {
                return true;
            };
            match lean_event_data(&event_type, &data) {
                None => false,
                Some(Cow::Borrowed(_)) => true,
                Some(Cow::Owned(lean)) => {
                    event["data"] = Value::String(lean);
                    true
                }
            }
        });
    }
    if let Some(events) = page
        .pointer_mut("/projection/events")
        .and_then(Value::as_array_mut)
    {
        for event in events {
            if is_dropped(event["type"].as_str().unwrap_or_default()) {
                blank_folded_text(event);
            }
        }
    }
}

/// Trim one history page's `entries` array for the lean feed.
///
/// A history page has a different shape from the live lane — entries holding
/// blocks rather than per-token events — but the same three payloads dominate
/// it, measured on the three heaviest sessions:
///
/// | Trim | Share of the page |
/// | --- | --- |
/// | reasoning body | 25.6-32.2% |
/// | tool-result body | 22.0-26.5% |
/// | tool-call arguments beyond the four the client reads | 12.3-17.8% |
///
/// Together they are 64.4-70.7% of a page. Each is unread rather than merely
/// unrendered, which is what makes this a removal:
///
/// - `targetFromArgs` derives a tool row's text from `command`/`path`/
///   `file_path`/`filePath` and **never** reads any other argument key, for any
///   tool name, so keeping exactly those four is behaviour-preserving.
/// - `foldToolEntry` reads a result block's `toolCallId` and `isError` and
///   nothing else.
/// - A reasoning block's body is only ever rendered when the row is expanded.
///
/// Nothing here adds, removes or reorders an entry or a block: entries keep their
/// identity and count, so the page's `nextOffset`/`hasMore`/flush-cursor logic
/// and the client's gap-fill are unaffected. Only field payload shrinks.
pub(crate) fn lean_entries(entries: &mut Value) {
    let Some(entries) = entries.as_array_mut() else {
        return;
    };
    for entry in entries {
        let Some(blocks) = entry.get_mut("blocks").and_then(Value::as_array_mut) else {
            continue;
        };
        for block in blocks {
            match block["kind"].as_str().unwrap_or_default() {
                // The body goes; the block stays, so the row still appears.
                "reasoning" => {
                    if let Some(fields) = block.as_object_mut() {
                        fields.remove("text");
                    }
                }
                "tool_result" => {
                    if let Some(fields) = block.as_object_mut() {
                        fields.remove("text");
                    }
                }
                "tool_call" => {
                    let Some(arguments) = block.get_mut("arguments") else {
                        continue;
                    };
                    if let Some(fields) = arguments.as_object_mut() {
                        fields.retain(|key, _| TARGET_ARGUMENT_KEYS.contains(&key.as_str()));
                    }
                }
                // Every other block kind is forwarded verbatim, so a future
                // vocabulary the client renders is never silently reshaped.
                _ => {}
            }
        }
    }
}

/// The argument keys `targetFromArgs` reads. Anything else in `arguments`
/// cannot affect what the client displays.
const TARGET_ARGUMENT_KEYS: [&str; 4] = ["command", "path", "file_path", "filePath"];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn features(declared: &[&str]) -> Vec<String> {
        declared.iter().map(|f| (*f).to_string()).collect()
    }

    #[test]
    fn only_an_explicit_declaration_enables_the_feed() {
        assert!(feature_declared(&features(&["lean_events_v1"])));
        assert!(feature_declared(&features(&[
            "event_coalescing_v1",
            "lean_events_v1"
        ])));
        assert!(!feature_declared(&features(&[])));
        assert!(!feature_declared(&features(&["event_coalescing_v1"])));
        // Not a prefix match: a near-miss name must not enable it.
        assert!(!feature_declared(&features(&["lean_events_v1_preview"])));
    }

    #[test]
    fn streamed_content_drops_and_decision_events_do_not() {
        for event_type in ["thinking_delta", "tool_delta", "toolcall_delta"] {
            assert!(
                lean_event_data(event_type, r#"{"text":"x"}"#).is_none(),
                "{event_type} streams content the phone does not render"
            );
        }
        for event_type in [
            "text_chunk",
            "tool_start",
            "thinking_start",
            "thinking_end",
            "agent_start",
            "agent_end",
            "usage",
            "error",
        ] {
            let data = r#"{"text":"kept"}"#;
            assert_eq!(
                lean_event_data(event_type, data).as_deref(),
                Some(data),
                "{event_type} must be forwarded byte-for-byte"
            );
        }
    }

    /// The tool row's outcome must survive: dropping the captured output is only
    /// safe because these fields replace the `[exit: N]` footer the text used to
    /// carry, and because `target_path` is the row's target on a result event.
    #[test]
    fn tool_end_keeps_its_outcome_and_identity() {
        let data = json!({
            "type": "tool_end",
            "tool_id": "call_1",
            "tool_name": "shell",
            "text": "a\nb\nc\n[exit: 3]",
            "exit_code": 3,
            "is_soft_fail": false,
            "target_path": "/tmp/x",
            "error": "boom",
        })
        .to_string();
        let lean: Value =
            serde_json::from_str(&lean_event_data("tool_end", &data).unwrap()).unwrap();
        assert!(lean.get("text").is_none(), "the output itself is dropped");
        assert_eq!(lean["exit_code"], json!(3));
        assert_eq!(lean["is_soft_fail"], json!(false));
        assert_eq!(lean["target_path"], json!("/tmp/x"));
        assert_eq!(lean["error"], json!("boom"));
        assert_eq!(lean["tool_id"], json!("call_1"));

        // `result` is the other spelling the client accepts for the same field.
        let alt = json!({"type": "tool_result", "result": "out", "exit_code": 1}).to_string();
        let lean: Value =
            serde_json::from_str(&lean_event_data("tool_result", &alt).unwrap()).unwrap();
        assert!(lean.get("result").is_none(), "the output field is dropped");
        assert_eq!(lean["exit_code"], json!(1));
    }

    #[test]
    fn unparsable_and_unexpected_shapes_are_forwarded() {
        // A truncated/oversized payload marker, or any non-object data, must not
        // be reshaped into something the client cannot read.
        assert_eq!(
            lean_event_data("tool_end", "not json").as_deref(),
            Some("not json")
        );
        assert_eq!(
            lean_event_data("tool_end", r#"["array"]"#).as_deref(),
            Some(r#"["array"]"#)
        );
        assert_eq!(
            lean_event_data("run_snapshot", "\u{0}").as_deref(),
            Some("\u{0}")
        );
    }

    #[test]
    fn run_snapshot_drops_only_the_folded_events() {
        let data = json!({
            "snapshotEvents": [{"type": "thinking_delta", "data": "{\"text\":\"long\"}"}],
            "snapshotCursor": 42,
        })
        .to_string();
        let lean: Value =
            serde_json::from_str(&lean_event_data("run_snapshot", &data).unwrap()).unwrap();
        assert!(lean.get("snapshotEvents").is_none());
        assert_eq!(lean["snapshotCursor"], json!(42));
    }

    /// Raw page events are dropped (the envelope is the cost), while the folded
    /// projection keeps its events with blanked text (the client validates them).
    #[test]
    fn replay_page_trims_each_shape_the_way_its_consumer_requires() {
        let mut page = json!({
            "runId": "r1",
            "watermark": 30,
            "nextSinceIdx": 30,
            "events": [
                {"type": "thinking_delta", "data": json!({"text": "think"}).to_string(), "idx": 10},
                {"type": "tool_start", "data": json!({"tool_args": {"path": "/a"}}).to_string(), "idx": 20},
                {"type": "tool_delta", "data": json!({"text": "/a", "tool_id": "c1"}).to_string(), "idx": 25},
                {"type": "tool_end", "data": json!({"text": "out", "exit_code": 1}).to_string(), "idx": 30},
            ],
            "projection": {"cursor": 30, "events": [
                {"type": "thinking_delta", "data": json!({"text": "think"}).to_string(), "idx": 10},
                {"type": "tool_end", "data": json!({"text": "out", "exit_code": 1}).to_string(), "idx": 30},
            ]},
        });
        lean_replay_page(&mut page, true);

        let events = page["events"].as_array().unwrap();
        let types: Vec<&str> = events
            .iter()
            .map(|event| event["type"].as_str().unwrap())
            .collect();
        assert_eq!(types, vec!["tool_start", "tool_end"]);
        // The cursor contract is untouched by the trim.
        assert_eq!(page["nextSinceIdx"], json!(30));
        assert_eq!(page["watermark"], json!(30));

        let folded = page["projection"]["events"].as_array().unwrap();
        assert_eq!(folded.len(), 2, "the folded list keeps its length and idx");
        assert_eq!(folded[0]["idx"], json!(10));
        let blanked: Value = serde_json::from_str(folded[0]["data"].as_str().unwrap()).unwrap();
        assert_eq!(blanked["text"], json!(""));
        // Non-delta folded events are untouched.
        assert!(folded[1]["data"].as_str().unwrap().contains("out"));
    }

    #[test]
    fn replay_page_is_untouched_when_the_client_did_not_declare() {
        let before = json!({
            "events": [{"type": "thinking_delta", "data": json!({"text": "think"}).to_string(), "idx": 1}],
        });
        let mut page = before.clone();
        lean_replay_page(&mut page, false);
        assert_eq!(page, before);
    }

    /// The three trims, on the shapes a real page carries.
    #[test]
    fn lean_entries_trims_the_three_unread_payloads() {
        let mut entries = json!([
            {
                "id": "a1",
                "kind": "assistant",
                "role": "assistant",
                "createdAtMs": 1,
                "blocks": [
                    {"kind": "reasoning", "text": "a long private monologue"},
                    {"kind": "text", "text": "visible answer"},
                    {"kind": "tool_call", "name": "write", "toolCallId": "c1",
                     "arguments": {"path": "/tmp/a", "content": "x".repeat(2000)}},
                ],
            },
            {
                "id": "t1",
                "kind": "tool",
                "role": "tool",
                "createdAtMs": 2,
                "blocks": [
                    {"kind": "tool_result", "toolCallId": "c1", "text": "y".repeat(2000)},
                ],
            },
        ]);
        lean_entries(&mut entries);

        let blocks = entries[0]["blocks"].as_array().unwrap();
        // The reasoning block survives with no body, so the row still appears.
        assert_eq!(blocks[0]["kind"], json!("reasoning"));
        assert!(blocks[0].get("text").is_none(), "reasoning body is dropped");
        // Visible text is untouched.
        assert_eq!(blocks[1]["text"], json!("visible answer"));
        // Only the keys the target derivation can read survive.
        assert_eq!(blocks[2]["arguments"], json!({"path": "/tmp/a"}));
        assert_eq!(blocks[2]["toolCallId"], json!("c1"));
        // The result block keeps its identity and loses its body.
        let result = &entries[1]["blocks"][0];
        assert_eq!(result["toolCallId"], json!("c1"));
        assert!(result.get("text").is_none(), "tool output is dropped");
    }

    /// Entry identity and count are what the page's cursor arithmetic depends
    /// on, so the trim must not touch them.
    #[test]
    fn lean_entries_preserves_entry_and_block_counts() {
        let mut entries = json!([
            {"id": "e1", "kind": "user", "role": "user", "createdAtMs": 1,
             "blocks": [{"kind": "text", "text": "q"}]},
            {"id": "e2", "kind": "assistant", "role": "assistant", "createdAtMs": 2,
             "blocks": [{"kind": "reasoning", "text": "t"}, {"kind": "text", "text": "a"}]},
            {"id": "e3", "kind": "tool", "role": "tool", "createdAtMs": 3, "blocks": []},
        ]);
        let before = entries.clone();
        lean_entries(&mut entries);
        let ids = |value: &Value| {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["id"].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&entries),
            ids(&before),
            "entries keep identity and order"
        );
        for (after, original) in entries
            .as_array()
            .unwrap()
            .iter()
            .zip(before.as_array().unwrap())
        {
            assert_eq!(
                after["blocks"].as_array().unwrap().len(),
                original["blocks"].as_array().unwrap().len(),
                "blocks are never added or removed"
            );
            assert_eq!(after["createdAtMs"], original["createdAtMs"]);
        }
    }

    /// Fail closed on anything unrecognized: an argument list that is not an
    /// object, blocks that are not an array, and block kinds this code has never
    /// heard of all pass through untouched.
    #[test]
    fn lean_entries_leaves_unrecognized_shapes_alone() {
        for kind in ["text", "image", "future_kind"] {
            let block = json!({"kind": kind, "text": "kept", "data": {"x": 1}});
            let mut entries = json!([{"id": "e", "kind": "assistant", "blocks": [block]}]);
            let before = entries.clone();
            lean_entries(&mut entries);
            assert_eq!(entries, before, "{kind} must be forwarded verbatim");
        }
        // A string argument list (a model that emitted the call as JSON text) is
        // not ours to reshape.
        let mut entries = json!([{"id": "e", "kind": "assistant",
            "blocks": [{"kind": "tool_call", "arguments": "{\"content\":\"x\"}"}]}]);
        let before = entries.clone();
        lean_entries(&mut entries);
        assert_eq!(entries, before);
        // Not an array at all: no panic.
        let mut not_entries = json!({"entries": {"nope": true}});
        let before = not_entries.clone();
        lean_entries(&mut not_entries["entries"]);
        assert_eq!(not_entries, before);
    }

    /// Unknown tool names are treated as `shell` by the client and read only
    /// `command`, so trimming cannot strand a target the client would have shown.
    #[test]
    fn a_trimmed_argument_list_still_yields_every_target_the_client_can_read() {
        let cases = [
            ("shell", json!({"command": "ls -la", "timeout": 30})),
            ("read", json!({"path": "/a/b", "offset": 5, "limit": 10})),
            ("write", json!({"path": "/a/b", "content": "body"})),
            ("edit", json!({"path": "/a/b", "edits": [{"oldText": "x"}]})),
            // A tool name the client has never seen: `asToolKind` calls it shell
            // and reads `command`, which survives if it was there.
            ("future_tool", json!({"command": "do", "secret": "s"})),
            ("future_query_tool", json!({"query": "s"})),
        ];
        for (name, arguments) in cases {
            let expected = trimmed(arguments.clone());
            let mut entries = json!([{"id": "e", "kind": "assistant", "blocks": [
                {"kind": "tool_call", "name": name, "toolCallId": "c", "arguments": arguments},
            ]}]);
            lean_entries(&mut entries);
            assert_eq!(
                entries[0]["blocks"][0]["arguments"], expected,
                "{name} must keep exactly the keys a target can come from"
            );
        }
    }

    fn trimmed(arguments: Value) -> Value {
        let mut entries = json!([{"id": "e", "kind": "assistant", "blocks": [
            {"kind": "tool_call", "arguments": arguments},
        ]}]);
        lean_entries(&mut entries);
        entries[0]["blocks"][0]["arguments"].clone()
    }

    /// Measure the trim against real history, through the shipping code.
    ///
    /// Driven by `scripts/measure/measure-lean-history.py`, which dumps one session's
    /// `get_session_entries` reply to a file and supplies its path. Runs against
    /// the live (read-only) agent, so the numbers describe real sessions rather
    /// than a synthetic payload.
    #[test]
    #[ignore = "measurement: needs LEAN_HISTORY_ENTRIES"]
    fn measure_real_entries() {
        let path = std::env::var("LEAN_HISTORY_ENTRIES").expect("LEAN_HISTORY_ENTRIES");
        let raw = std::fs::read_to_string(path).expect("entries readable");
        let mut entries: Value = serde_json::from_str(&raw).expect("entries json");

        let sized = |value: &Value| serde_json::to_vec(value).expect("serializes").len();
        let before = sized(&entries);

        // Per-kind byte totals, so the report says where the bytes were rather
        // than only how many went away.
        let mut by_kind: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut block_counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        if let Some(list) = entries.as_array() {
            for entry in list {
                for block in entry["blocks"].as_array().into_iter().flatten() {
                    let kind = block["kind"].as_str().unwrap_or("<none>").to_string();
                    *block_counts.entry(kind.clone()).or_default() += 1;
                    *by_kind.entry(kind).or_default() += sized(block);
                }
            }
        }

        lean_entries(&mut entries);
        let after = sized(&entries);

        println!(
            "LEAN_HISTORY {}",
            serde_json::json!({
                "entries": entries.as_array().map(Vec::len).unwrap_or(0),
                "beforeBytes": before,
                "afterBytes": after,
                "saved": 1.0 - (after as f64 / before.max(1) as f64),
                "blockCounts": block_counts,
                "blockBytes": by_kind,
            })
        );
    }
}
