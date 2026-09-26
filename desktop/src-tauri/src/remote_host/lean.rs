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

use serde_json::{json, Value};
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};

/// Capability a client declares on `secure_ready` to receive the lean lane.
/// Named so an older desktop ignores it and an older client never asks for it.
pub(crate) const LEAN_EVENTS_FEATURE: &str = "lean_events_v1";

/// Event types that exist only to stream content a phone does not render.
const DROPPED_EVENTS: [&str; 3] = ["thinking_delta", "tool_delta", "toolcall_delta"];

/// Tools whose arguments carry a body the phone never renders: a `write` its
/// whole file, an `edit` the text it replaces.
///
/// `read` keeps everything (its `offset`/`limit` are a few bytes) and `shell`
/// keeps its `command`, which *is* the row's label. A tool this build has never
/// heard of keeps its arguments too: nothing here can tell which of them a
/// client might read.
const BODY_BEARING_TOOLS: [&str; 2] = ["write", "edit"];

/// Argument keys a tool row's label is built from, mirroring the client's
/// `targetFromArgs`. Anything else a body-bearing tool passes is dropped.
const TARGET_ARG_KEYS: [&str; 3] = ["path", "file_path", "filePath"];

/// Body keys a lane subscriber reads. Everything else in the envelope is
/// dropped on the lean lane.
///
/// Verified against every consumer of this subject: the phone's `StreamEvent`
/// type and its `normalizeReplayEvents` copy `type`/`data`/`runId`/`idx`, the
/// desktop's own web verification client reads `type`/`runId`/`idx`/`data`, and
/// `coalescedCount` is the merge marker. The desktop does not subscribe to this
/// subject at all (it only publishes), and the TUI speaks gRPC instead.
///
/// The dropped keys stay recoverable from the agent, whose `get_events_since`
/// carries `event_id`/`timestamp`/`session_id`/`epoch`/`session_idx`/
/// `run_sequence` on every event -- the lane is a projection of that stream, not
/// the only copy of it.
const EVENT_BODY_KEYS: [&str; 5] = ["type", "data", "runId", "idx", "coalescedCount"];

/// Token counters a client reads out of a `usage` event.
///
/// The phone sums these into the settled reply's "N tokens" footer. The
/// provider's other counters and the credit cost are the desktop footer's
/// business, and the desktop does not read this lane (the agent is its source).
const USAGE_TOKEN_KEYS: [&str; 2] = ["completion_tokens", "output_tokens"];

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
        // parsing an `[exit: N]` footer out of the text -- except for the legacy
        // shape, where there is no outcome to read and that footer is the only
        // signal there is.
        "tool_end" | "tool_result" => {
            if keeps_legacy_exit_footer(data) {
                Some(Cow::Borrowed(data))
            } else {
                Some(without(data, &["text", "result"]))
            }
        }
        // The folded events are the same reasoning and argument fragments again.
        // The client treats this event as a "resync me" signal and never reads
        // them, so the whole array goes.
        "run_snapshot" => Some(without(data, &["snapshotEvents"])),
        // A tool call announces itself twice: an `input` phase when the model
        // begins writing the call, then an `execution` phase when the arguments
        // are complete and the tool runs. The first one goes entirely: it carries
        // no arguments (measured across six real runs, 1,608 input phases, none
        // with arguments) and therefore no label, so all it can do is make an
        // empty row appear ~331ms earlier (median; the row's target is unknown
        // until `execution` either way). An approval wait, the one case where
        // that gap is long, has its own `approval_request` event.
        "tool_start" | "toolcall_start" => {
            if is_tool_input_phase(data) {
                None
            } else {
                // The row's label comes from `path` (read/write/edit) or
                // `command` (shell) -- the file bodies a `write`/`edit` carries
                // are never rendered, and the history page drops them too.
                Some(without_tool_bodies(data))
            }
        }
        // The client reads one number out of a usage event; every provider
        // counter beside it is dead weight on this lane.
        "usage" => Some(without_extra_usage(data)),
        _ => Some(Cow::Borrowed(data)),
    }
}

/// Whether a tool result's captured output has to survive the lean trim.
///
/// An agent older than `tool_end_semantics` reports a failed shell command as a
/// *successful* result whose only outcome signal is the `[exit: N]` footer in
/// the output. Dropping that output would leave the phone nothing to read, so
/// the row would show as completed -- a failure the lean feed invented. Keep
/// the output in exactly that case: a failing shell result is short, and a
/// successful one still loses it.
fn keeps_legacy_exit_footer(data: &str) -> bool {
    let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(data) else {
        return false;
    };
    if fields.contains_key("exit_code") || fields.contains_key("exitCode") {
        return false;
    }
    if fields.get("tool_name").and_then(Value::as_str) != Some("shell") {
        return false;
    }
    // `result` is the other spelling the client accepts for the same output.
    ["text", "result"]
        .iter()
        .filter_map(|key| fields.get(*key).and_then(Value::as_str))
        .any(has_non_zero_exit_footer)
}

/// The non-zero `[exit: N]` footer the shell tool appends, parsed the way the
/// client parses it (`nonZeroExitCode` in `thread-projection`): the last line of
/// the trimmed output, matching in full.
fn has_non_zero_exit_footer(output: &str) -> bool {
    let line = output.trim_end().rsplit('\n').next().unwrap_or_default();
    let Some(code) = line
        .strip_prefix("[exit: ")
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return false;
    };
    matches!(code.parse::<i64>(), Ok(code) if code != 0)
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

/// Whether this `tool_start` is the `input` phase (the model still writing the
/// call) rather than the `execution` phase (arguments complete, tool running).
///
/// An unparsable payload answers `false`: an unrecognised shape is forwarded
/// untouched rather than dropped, here as everywhere else in this module.
fn is_tool_input_phase(data: &str) -> bool {
    serde_json::from_str::<Value>(data)
        .ok()
        .and_then(|value| {
            value
                .get("phase")
                .and_then(Value::as_str)
                .map(|phase| phase == "input")
        })
        .unwrap_or(false)
}

/// Reduce a lane body to the keys a subscriber reads.
///
/// Measured on the three heaviest runs: the envelope was 72.9% of the lean lane
/// and 56.1% of the lane was fields no subscriber reads -- `eventId` alone
/// (a `{session}:{run}:{epoch}:{idx}` string, ~96 B per event) was 20.3%.
pub(crate) fn lean_event_body(body: &mut Value) {
    let Some(fields) = body.as_object_mut() else {
        return;
    };
    fields.retain(|key, _| EVENT_BODY_KEYS.contains(&key.as_str()));
}

/// A `tool_start`'s arguments, minus the bodies the phone never renders.
///
/// Only `write` and `edit` carry one; every other tool is forwarded verbatim
/// (see [`BODY_BEARING_TOOLS`]). `tool_args` is normally an object but the agent
/// has emitted a JSON string for it too, and the client accepts either, so both
/// shapes are filtered and the original shape is preserved. A payload with no
/// body in it is returned borrowed and untouched.
fn without_tool_bodies(data: &str) -> Cow<'_, str> {
    let Ok(mut value) = serde_json::from_str::<Value>(data) else {
        return Cow::Borrowed(data);
    };
    let name = value["tool_name"].as_str().unwrap_or_default();
    if !BODY_BEARING_TOOLS.contains(&name) {
        return Cow::Borrowed(data);
    }
    let Some(args) = value
        .as_object_mut()
        .and_then(|root| root.remove("tool_args"))
    else {
        return Cow::Borrowed(data);
    };
    let trimmed = match args {
        Value::Object(fields) => drop_bodies(fields).map(Value::Object),
        Value::String(text) => serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|parsed| match parsed {
                Value::Object(fields) => {
                    drop_bodies(fields).map(|kept| Value::String(Value::Object(kept).to_string()))
                }
                _ => None,
            }),
        _ => None,
    };
    let Some(trimmed) = trimmed else {
        return Cow::Borrowed(data);
    };
    value["tool_args"] = trimmed;
    Cow::Owned(value.to_string())
}

/// The path-only remainder of a body-bearing tool's arguments, or `None` when
/// there was no body to drop.
fn drop_bodies(fields: serde_json::Map<String, Value>) -> Option<serde_json::Map<String, Value>> {
    if fields
        .keys()
        .all(|key| TARGET_ARG_KEYS.contains(&key.as_str()))
    {
        return None;
    }
    Some(
        fields
            .into_iter()
            .filter(|(key, _)| TARGET_ARG_KEYS.contains(&key.as_str()))
            .collect(),
    )
}

/// A `usage` event reduced to the counters a client reads.
///
/// The phone sums completion tokens for the settled reply's footer, falling back
/// to the `agent_end` total; the provider's prompt/cache/reasoning counters, the
/// credit cost and the stop reason are read by nobody on this lane.
fn without_extra_usage(data: &str) -> Cow<'_, str> {
    let Ok(mut value) = serde_json::from_str::<Value>(data) else {
        return Cow::Borrowed(data);
    };
    let Some(Value::Object(fields)) = value.get("usage").cloned() else {
        return Cow::Borrowed(data);
    };
    let kept: serde_json::Map<String, Value> = fields
        .into_iter()
        .filter(|(key, _)| USAGE_TOKEN_KEYS.contains(&key.as_str()))
        .collect();
    if value["usage"] == Value::Object(kept.clone()) {
        return Cow::Borrowed(data);
    }
    value["usage"] = Value::Object(kept);
    if let Some(root) = value.as_object_mut() {
        root.remove("stopReason");
    }
    Cow::Owned(value.to_string())
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
    // The trim removes events, so the page's own list no longer matches the
    // source range it covers. State how many events that range held *before*
    // the trim: a client that declared a trimmed feed still has to prove the
    // tail reached the pinned watermark, and counting what arrived cannot do
    // that once holes are expected. The count is what keeps that check exact
    // instead of relaxing it to "whatever arrived is enough".
    if let Some(count) = page.get("events").and_then(Value::as_array).map(Vec::len) {
        page["rawEvents"] = json!(count);
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
/// blocks rather than per-token events — but the same payloads dominate it,
/// measured on the three heaviest sessions:
///
/// | Trim | Share of the page |
/// | --- | --- |
/// | reasoning body | 25.6-32.2% |
/// | tool-result body | 22.0-26.5% |
/// | tool-call arguments | 12.3-17.8% (of which shell commands are most) |
///
/// Each is unread rather than merely unrendered, which is what makes this a
/// removal:
///
/// - `targetFromArgs` derives a **file** row's text from `path`/`file_path`/
///   `filePath` and never reads any other argument key, so keeping exactly
///   those three is behaviour-preserving for read/write/edit.
/// - A **shell** row's text is its `command`, and the row shows no text until
///   it is opened — a tap fetches the arguments back by identity
///   (`get_tool_call_args`) and caches them, so the whole `arguments` object
///   goes. The page carries the call's `toolCallId` (and its entry's `runId`),
///   which is all the fetch needs; no marker field is added, because "no
///   `arguments`" is exactly the state that means "ask for them".
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
                    // The file kinds keep only what their row label reads; every
                    // other name (shell, and any tool this client has never
                    // heard of — it renders those as shell) loses the whole key.
                    let keeps_path =
                        matches!(block["name"].as_str(), Some("read" | "write" | "edit"));
                    let Some(fields) = block.as_object_mut() else {
                        continue;
                    };
                    if !keeps_path {
                        fields.remove("arguments");
                        continue;
                    }
                    let Some(arguments) = fields.get_mut("arguments") else {
                        continue;
                    };
                    // A non-object argument list (a model that emitted the call
                    // as JSON text) is not ours to reshape.
                    if let Some(arguments) = arguments.as_object_mut() {
                        arguments.retain(|key, _| PATH_ARGUMENT_KEYS.contains(&key.as_str()));
                    }
                }
                // Every other block kind is forwarded verbatim, so a future
                // vocabulary the client renders is never silently reshaped.
                _ => {}
            }
        }
    }
}

/// The argument keys a file row's target (`path`) is derived from. A shell
/// row's `command` is fetched on demand instead — see `lean_entries`.
const PATH_ARGUMENT_KEYS: [&str; 3] = ["path", "file_path", "filePath"];

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

    /// A `write`/`edit` announces its body on the live lane too, and the phone
    /// never renders it: the row's label is the path. Shell and read keep their
    /// arguments (the command *is* a shell row's label), and an unknown tool
    /// keeps everything, because nothing here knows what a future client reads.
    #[test]
    fn tool_start_drops_the_bodies_only_write_and_edit_carry() {
        let write = json!({
            "type": "tool_start",
            "phase": "execution",
            "tool_name": "write",
            "tool_id": "call_1",
            "tool_args": {"path": "/tmp/x.md", "content": "a".repeat(4096)},
        })
        .to_string();
        let lean: Value =
            serde_json::from_str(&lean_event_data("tool_start", &write).unwrap()).unwrap();
        assert_eq!(
            lean["tool_args"],
            json!({"path": "/tmp/x.md"}),
            "path survives"
        );
        assert_eq!(lean["tool_id"], json!("call_1"), "identity survives");
        assert_eq!(lean["phase"], json!("execution"));

        let edit = json!({
            "type": "tool_start",
            "tool_name": "edit",
            "tool_args": {"path": "a.ts", "oldText": "x", "newText": "y"},
        })
        .to_string();
        let lean: Value =
            serde_json::from_str(&lean_event_data("tool_start", &edit).unwrap()).unwrap();
        assert_eq!(lean["tool_args"], json!({"path": "a.ts"}));

        // The other argument spelling the client accepts for a path.
        let alias = json!({
            "type": "tool_start",
            "tool_name": "write",
            "tool_args": {"file_path": "b.ts", "content": "y"},
        })
        .to_string();
        let lean: Value =
            serde_json::from_str(&lean_event_data("tool_start", &alias).unwrap()).unwrap();
        assert_eq!(lean["tool_args"], json!({"file_path": "b.ts"}));

        // A shell row's label is its command: nothing may go.
        let shell = json!({
            "type": "tool_start",
            "tool_name": "shell",
            "tool_args": {"command": "ls -la", "timeout": 5000},
        })
        .to_string();
        let lean = lean_event_data("tool_start", &shell).unwrap();
        assert_eq!(
            lean.as_ref(),
            shell.as_str(),
            "a shell command is the row's label, so it stays"
        );

        // `read`'s arguments are a path and two small numbers.
        let read = json!({
            "type": "tool_start",
            "tool_name": "read",
            "tool_args": {"path": "c.ts", "offset": 10, "limit": 20},
        })
        .to_string();
        assert_eq!(
            lean_event_data("tool_start", &read).unwrap().as_ref(),
            read.as_str()
        );

        // A tool this build does not know keeps its arguments.
        let unknown = json!({
            "type": "tool_start",
            "tool_name": "mcp__whatever",
            "tool_args": {"content": "keep me"},
        })
        .to_string();
        assert_eq!(
            lean_event_data("tool_start", &unknown).unwrap().as_ref(),
            unknown.as_str()
        );

        // The `input` phase goes entirely: it never carries arguments (measured
        // across six real runs: 1,608 input phases, none with arguments), so it
        // can only make an empty row appear earlier.
        let input = json!({
            "type": "tool_start",
            "phase": "input",
            "tool_name": "write",
            "tool_args": "",
        })
        .to_string();
        assert!(
            lean_event_data("tool_start", &input).is_none(),
            "the argument-less input phase goes entirely"
        );

        // The `execution` phase is the one that carries the label, so it stays.
        let execution = json!({
            "type": "tool_start",
            "phase": "execution",
            "tool_name": "write",
            "tool_args": {"path": "f.ts"},
        })
        .to_string();
        assert!(lean_event_data("tool_start", &execution).is_some());

        // A `tool_start` with no phase is not the input phase, so it is
        // forwarded rather than dropped: an unrecognised shape never vanishes.
        let phaseless = json!({"type": "tool_start", "tool_name": "shell"}).to_string();
        assert!(lean_event_data("tool_start", &phaseless).is_some());

        // A body-less `write` (already trimmed, or a shape that never had one)
        // is returned borrowed rather than rewritten.
        let bare =
            json!({"type": "tool_start", "tool_name": "write", "tool_args": {"path": "d.ts"}})
                .to_string();
        let lean = lean_event_data("tool_start", &bare).unwrap();
        assert!(
            matches!(lean, Cow::Borrowed(_)),
            "nothing to drop means no rewrite"
        );
    }

    /// A JSON *string* carrying the arguments is filtered too, and stays a
    /// string: the client parses either shape, so the lane must not change it.
    #[test]
    fn tool_start_filters_a_stringified_argument_object() {
        let data = json!({
            "type": "tool_start",
            "tool_name": "write",
            "tool_args": "{\"content\":\"big body\",\"path\":\"e.ts\"}",
        })
        .to_string();
        let lean: Value =
            serde_json::from_str(&lean_event_data("tool_start", &data).unwrap()).unwrap();
        let args: Value = serde_json::from_str(lean["tool_args"].as_str().unwrap()).unwrap();
        assert_eq!(args, json!({"path": "e.ts"}));
    }

    /// The phone sums one counter out of a usage event into the settled reply's
    /// footer; the rest of the provider's report is read by nobody on this lane.
    #[test]
    fn usage_keeps_only_the_counter_a_client_reads() {
        let data = json!({
            "type": "usage",
            "stopReason": "tool_calls",
            "usage": {
                "prompt_tokens": 8079,
                "completion_tokens": 276,
                "total_tokens": 8355,
                "cache_read_tokens": 3584,
                "reasoning_tokens": 182,
                "credit_cost": 0.01134136,
            },
        })
        .to_string();
        let lean: Value = serde_json::from_str(&lean_event_data("usage", &data).unwrap()).unwrap();
        assert_eq!(lean["usage"], json!({"completion_tokens": 276}));
        assert!(lean.get("stopReason").is_none());

        // The alias the client also reads, and a payload that already carries
        // only what is needed (returned borrowed, not rewritten).
        let alias = json!({"type": "usage", "usage": {"output_tokens": 5}}).to_string();
        let lean = lean_event_data("usage", &alias).unwrap();
        assert_eq!(lean.as_ref(), alias.as_str());

        // A usage event with no `usage` object is not this function's business.
        let flat = json!({"type": "usage", "completion_tokens": 7}).to_string();
        assert_eq!(
            lean_event_data("usage", &flat).unwrap().as_ref(),
            flat.as_str()
        );
    }

    /// The tool row's outcome must survive: dropping the captured output is only
    /// safe because these fields replace the `[exit: N]` footer the text used to
    /// carry, and because `target_path` is the row's target on a result event.
    /// The lean envelope keeps the keys a subscriber reads and nothing else.
    /// Every dropped key is recoverable from the agent's own event stream, so
    /// the lane stays a projection rather than the only copy.
    #[test]
    fn lean_event_body_keeps_only_the_keys_a_subscriber_reads() {
        let mut body = json!({
            "schemaVersion": 2,
            "sessionId": "s-1",
            "type": "tool_end",
            "data": "{\"tool_id\":\"c1\"}",
            "runId": "r-1",
            "idx": 42,
            "epoch": 1,
            "eventId": "s-1:r-1:1:42",
            "timestamp": "2026-01-01T00:00:00+00:00",
            "sessionIdx": -1,
            "runSequence": 3,
        });
        lean_event_body(&mut body);
        assert_eq!(
            body,
            json!({"type": "tool_end", "data": "{\"tool_id\":\"c1\"}", "runId": "r-1", "idx": 42})
        );

        // The coalescer's merge marker must survive: the client reads it to know
        // `idx` ends a range instead of stepping by one.
        let mut merged = json!({
            "sessionId": "s-1",
            "type": "text_chunk",
            "data": "{}",
            "runId": "r-1",
            "idx": 50,
            "coalescedCount": 7,
            "eventId": "s-1:r-1:1:50",
        });
        lean_event_body(&mut merged);
        assert_eq!(merged["coalescedCount"], json!(7));
        assert!(merged.get("eventId").is_none());

        // A non-object body is left alone rather than replaced.
        let mut scalar = json!("not an object");
        lean_event_body(&mut scalar);
        assert_eq!(scalar, json!("not an object"));
    }

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

    /// The legacy shape: an agent older than `tool_end_semantics` leaves only
    /// the `[exit: N]` footer in the output. Keeping it is what stops a failed
    /// command from reading as completed on the lean feed.
    #[test]
    fn legacy_shell_outcome_keeps_the_footer_that_is_its_only_signal() {
        let failing = json!({
            "type": "tool_end",
            "tool_id": "call_1",
            "tool_name": "shell",
            "text": "ls: cannot access '/nope': No such file\n\n[exit: 2]",
        })
        .to_string();
        let lean = lean_event_data("tool_end", &failing).unwrap();
        assert!(
            lean.contains("[exit: 2]"),
            "the only failure signal must survive: {lean}"
        );

        // A soft failure keeps it too: the client needs the code to apply its own
        // exemption (`grep` exit 1 is a no-match, not an error).
        let soft = json!({
            "type": "tool_end",
            "tool_name": "shell",
            "text": "0\n[exit: 1]",
        })
        .to_string();
        assert!(lean_event_data("tool_end", &soft)
            .unwrap()
            .contains("[exit: 1]"));

        // `result` is the other spelling the client reads the same way.
        let aliased = json!({
            "type": "tool_result",
            "tool_name": "shell",
            "result": "x\n[exit: 4]",
        })
        .to_string();
        assert!(lean_event_data("tool_result", &aliased)
            .unwrap()
            .contains("[exit: 4]"));

        // A successful legacy result has nothing to report, so its output goes.
        let succeeded = json!({
            "type": "tool_end",
            "tool_name": "shell",
            "text": "hi\n[exit: 0]",
        })
        .to_string();
        assert!(
            !lean_event_data("tool_end", &succeeded)
                .unwrap()
                .contains("[exit: 0]"),
            "a successful legacy result still drops its output"
        );

        // Every other shape drops the output: a structured outcome replaces the
        // footer, and a non-shell tool cannot report an outcome this way.
        for shape in [
            json!({"type": "tool_end", "tool_name": "shell", "text": "x\n[exit: 2]", "exit_code": 2}),
            json!({"type": "tool_end", "tool_name": "shell", "text": "x\n[exit: 2]", "exitCode": 2}),
            json!({"type": "tool_result", "tool_name": "read", "text": "body\n[exit: 2]"}),
            json!({"type": "tool_end", "tool_name": "shell", "text": "[exit: 2]\nmore"}),
        ] {
            let data = shape.to_string();
            let lean = lean_event_data("tool_end", &data).unwrap();
            assert!(
                !lean.contains("[exit: 2]") && !lean.contains("body"),
                "this shape keeps its structured outcome, not the text: {lean}"
            );
        }
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
        // …and the client is told how many source events the range held, so its
        // "did this tail reach the watermark" check stays exact even though two
        // of the four were dropped. Without this the client counts arrivals and
        // rejects every trimmed tail (`replay_prefix_invalid`), which is what
        // pinned the phone in a retry loop.
        assert_eq!(page["rawEvents"], json!(4));

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
        // A peer that does not trim must not be told to expect holes: its events
        // are one per index, and the strict client check is what proves it.
        assert!(page.get("rawEvents").is_none());
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
                    {"kind": "tool_call", "name": "shell", "toolCallId": "c2",
                     "arguments": {"command": "rm -rf build", "timeout": 30}},
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
        // A file row keeps exactly the keys its target can be read from.
        assert_eq!(blocks[2]["arguments"], json!({"path": "/tmp/a"}));
        assert_eq!(blocks[2]["toolCallId"], json!("c1"));
        // A shell row keeps its identity and loses the whole argument object:
        // the command is fetched when the row is opened, and the row's absence
        // of arguments is exactly the signal that it can be.
        assert_eq!(blocks[3]["toolCallId"], json!("c2"));
        assert!(
            blocks[3].get("arguments").is_none(),
            "a shell call's arguments ride the lazy fetch, not the page"
        );
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
        // A string argument list on a file tool (a model that emitted the call
        // as JSON text) is not ours to reshape.
        let mut entries = json!([{"id": "e", "kind": "assistant",
            "blocks": [{"kind": "tool_call", "name": "read", "arguments": "{\"path\":\"/a\"}"}]}]);
        let before = entries.clone();
        lean_entries(&mut entries);
        assert_eq!(entries, before);
        // Not an array at all: no panic.
        let mut not_entries = json!({"entries": {"nope": true}});
        let before = not_entries.clone();
        lean_entries(&mut not_entries["entries"]);
        assert_eq!(not_entries, before);
    }

    /// The two rules split by tool name, and the client reads a file row's path
    /// from the page while a shell row fetches its command on demand — so a trim
    /// can never strand a target the client could have shown directly.
    #[test]
    fn a_trimmed_argument_list_keeps_only_what_each_row_label_reads() {
        // File kinds: the path keys survive, every other argument goes.
        for (name, arguments, expected) in [
            (
                "read",
                json!({"path": "/a/b", "offset": 5, "limit": 10}),
                json!({"path": "/a/b"}),
            ),
            (
                "write",
                json!({"file_path": "/a/b", "content": "body"}),
                json!({"file_path": "/a/b"}),
            ),
            (
                "edit",
                json!({"filePath": "/a/b", "command": "not read", "edits": []}),
                json!({"filePath": "/a/b"}),
            ),
        ] {
            let mut entries = json!([{"id": "e", "kind": "assistant", "blocks": [
                {"kind": "tool_call", "name": name, "toolCallId": "c", "arguments": arguments},
            ]}]);
            lean_entries(&mut entries);
            assert_eq!(
                entries[0]["blocks"][0]["arguments"], expected,
                "{name} must keep exactly the path a target can come from"
            );
        }
        // Shell, and any name the client has never seen (it renders those as
        // shell too): the whole key goes, identity stays for the lazy fetch.
        for (name, arguments) in [
            ("shell", json!({"command": "ls -la", "timeout": 30})),
            ("future_tool", json!({"command": "do", "secret": "s"})),
            ("future_query_tool", json!({"query": "s"})),
        ] {
            let mut entries = json!([{"id": "e", "kind": "assistant", "blocks": [
                {"kind": "tool_call", "name": name, "toolCallId": "c", "arguments": arguments},
            ]}]);
            lean_entries(&mut entries);
            let block = &entries[0]["blocks"][0];
            assert!(
                block.get("arguments").is_none(),
                "{name} must lose its arguments; the row fetches them on open"
            );
            assert_eq!(block["toolCallId"], json!("c"));
        }
        // A tool name the agent reports but the client's `asToolKind` has no
        // file kind for keeps the same rule even with no name at all.
        let mut entries = json!([{"id": "e", "kind": "assistant", "blocks": [
            {"kind": "tool_call", "toolCallId": "c", "arguments": {"command": "x"}},
        ]}]);
        lean_entries(&mut entries);
        assert!(entries[0]["blocks"][0].get("arguments").is_none());
    }

    /// The outcome shape the agent writes today, which a render history
    /// fixture has to carry: a recorded failure says `isError: true` (the only
    /// failure signal a trimmed history row keeps) and nothing says the
    /// uninformative `false` (#849 stops emitting it). `lean_entries` forwards
    /// the flag untouched, so the full page and the derived lean page are both
    /// checked — a fixture that regressed to the pre-#848/#849 shape would
    /// make the render scenario pass against a payload the phone never gets.
    fn assert_recorded_tool_outcomes(entries: &Value) {
        let mut failures = 0;
        for block in entries
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|entry| entry["blocks"].as_array().into_iter().flatten())
        {
            if block["kind"] != "tool_result" {
                continue;
            }
            match block.get("isError").and_then(Value::as_bool) {
                Some(true) => failures += 1,
                Some(false) => panic!("a history result must not send isError: false: {block}"),
                None => {}
            }
        }
        assert!(
            failures > 0,
            "the render fixture must keep a recorded failure for the render checks"
        );
    }

    /// Derive the mobile render fixtures' lean pages from their full-feed
    /// counterparts **through the shipping trims**.
    ///
    /// Driven by `scripts/screenshots/gen-lean-render-fixtures.mjs`, which writes
    /// the full-feed fixture files (the wire shapes `rpc/prompt_helpers.rs` and
    /// the history relay produce) and then runs this test. The lean files are the
    /// output of [`lean_event_data`] / [`lean_entries`] themselves, so a render
    /// capture cannot quietly test a hand-trimmed paraphrase of the wire: if a
    /// trim changes, regenerating the fixtures changes what the phone is fed.
    ///
    /// The assertions below are the point of that claim — a generation run that
    /// silently produced an untrimmed file would make every render check vacuous.
    #[test]
    #[ignore = "fixture generation: needs LEAN_RENDER_FIXTURES"]
    fn generate_render_fixtures() {
        let dir = std::env::var("LEAN_RENDER_FIXTURES").expect("LEAN_RENDER_FIXTURES");
        let dir = std::path::Path::new(&dir);

        for (input, output) in [
            (
                "render-full-lane-events.json",
                "render-lean-lane-events.json",
            ),
            (
                "render-full-lane-mid-events.json",
                "render-lean-lane-mid-events.json",
            ),
        ] {
            // The live lane: one event at a time, exactly like `publish_event`.
            let raw = std::fs::read_to_string(dir.join(input)).expect("full lane events readable");
            let full: Value = serde_json::from_str(&raw).expect("full lane events json");
            let mut lean_events = Vec::new();
            for event in full.as_array().expect("events array") {
                let event_type = event["type"].as_str().unwrap_or_default().to_string();
                let data = event["data"].as_str().unwrap_or_default().to_string();
                let Some(rewritten) = lean_event_data(&event_type, &data) else {
                    continue;
                };
                let mut copy = event.clone();
                if let Cow::Owned(trimmed) = rewritten {
                    copy["data"] = Value::String(trimmed);
                }
                // Same envelope the publisher sends: a fixture that kept keys
                // production drops would verify the client against a payload the
                // phone never receives.
                lean_event_body(&mut copy);
                lean_events.push(copy);
            }
            assert!(
                !lean_events.is_empty(),
                "{input} produced an empty lean lane"
            );
            for event in &lean_events {
                let event_type = event["type"].as_str().unwrap_or_default();
                assert!(
                    !is_dropped(event_type),
                    "{event_type} is streamed-only content and must not survive {input}"
                );
                for key in event.as_object().into_iter().flatten().map(|(key, _)| key) {
                    assert!(
                        EVENT_BODY_KEYS.contains(&key.as_str()),
                        "{input}: the lean envelope must not carry {key}"
                    );
                }
            }
            std::fs::write(
                dir.join(output),
                serde_json::to_string_pretty(&Value::Array(lean_events)).expect("serializes"),
            )
            .expect("lean lane events writable");
        }

        let raw = std::fs::read_to_string(dir.join("render-lean-lane-events.json"))
            .expect("lean lane events readable");
        let lean_lane: Value = serde_json::from_str(&raw).expect("lean lane events json");
        let lean_events = lean_lane.as_array().expect("events array");

        // The history page: an in-place trim of the entries array, exactly like
        // `history.rs` does for a page it is about to relay.
        let raw = std::fs::read_to_string(dir.join("render-full-history-entries.json"))
            .expect("full history entries readable");
        let mut lean_entries_value: Value =
            serde_json::from_str(&raw).expect("full history entries json");
        assert_recorded_tool_outcomes(&lean_entries_value);
        lean_entries(&mut lean_entries_value);
        assert_recorded_tool_outcomes(&lean_entries_value);

        // Self-checks: the generated lean files must actually be lean, and must
        // still carry what the phone reads instead.
        for event in lean_events {
            let event_type = event["type"].as_str().unwrap_or_default();
            if event_type == "tool_end" || event_type == "tool_result" {
                let data: Value = serde_json::from_str(event["data"].as_str().unwrap_or("{}"))
                    .expect("tool event data json");
                // The output is dropped everywhere except the legacy shell shape,
                // where the `[exit: N]` footer is the client's only outcome. So a
                // fixture that still has one has to be that shape, and has to
                // justify it by carrying the footer it exists for.
                let kept: Vec<&str> = ["text", "result"]
                    .iter()
                    .filter_map(|key| data.get(*key).and_then(Value::as_str))
                    .collect();
                if !kept.is_empty() {
                    assert_eq!(
                        data.get("tool_name").and_then(Value::as_str),
                        Some("shell"),
                        "tool output may only survive as a legacy shell result"
                    );
                    assert!(
                        data.get("exit_code").is_none() && data.get("exitCode").is_none(),
                        "a surviving output has no structured outcome to read instead"
                    );
                    assert!(
                        kept.iter().any(|output| has_non_zero_exit_footer(output)),
                        "a surviving output must end with the non-zero exit footer"
                    );
                }
            }
        }
        for entry in lean_entries_value.as_array().expect("entries array") {
            for block in entry["blocks"].as_array().into_iter().flatten() {
                match block["kind"].as_str().unwrap_or_default() {
                    "reasoning" => assert!(
                        block.get("text").is_none(),
                        "a lean reasoning block carries no body"
                    ),
                    "tool_result" => assert!(
                        block.get("text").is_none(),
                        "a lean tool_result carries no body"
                    ),
                    "tool_call" => {
                        let kept = block["arguments"].as_object();
                        if matches!(block["name"].as_str(), Some("read" | "write" | "edit")) {
                            // A file row keeps exactly the keys its target is
                            // derived from.
                            for key in kept.into_iter().flatten() {
                                assert!(
                                    PATH_ARGUMENT_KEYS.contains(&key.0.as_str()),
                                    "argument {key:?} is not one the target derivation reads"
                                );
                            }
                        } else {
                            // A shell row keeps none: the command is what the
                            // on-open fetch returns, so a fixture that still
                            // carried it would render a page the phone never sees.
                            assert!(
                                kept.is_none(),
                                "a shell row's arguments are fetched on open, not in the page"
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        assert!(
            lean_events.iter().any(|event| event["type"] == "tool_end"),
            "the settled lane fixture has to keep at least one tool_end for the render checks"
        );

        std::fs::write(
            dir.join("render-lean-history-entries.json"),
            serde_json::to_string_pretty(&lean_entries_value).expect("serializes"),
        )
        .expect("lean history entries writable");
        println!("LEAN_RENDER_FIXTURES written to {}", dir.display());
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

        // When asked, keep the trimmed page so the trim can be audited on real
        // data (which keys a shell row kept, which it lost) instead of only
        // trusted as a size.
        if let Ok(path) = std::env::var("LEAN_HISTORY_OUT") {
            std::fs::write(path, serde_json::to_vec(&entries).expect("serializes"))
                .expect("trimmed entries writable");
        }

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
