//! Live event-lane coalescing.
//!
//! A streaming run emits one event per provider token: a single 3-4 byte text
//! fragment per message, each wrapped in the full remote envelope (session,
//! run, epoch, event id, timestamp). Measured on the heaviest real runs that
//! envelope is about 80% of the bytes a phone receives, and the lane carries
//! 60-90 messages per second — the phone pays a decrypt, a JSON parse and a
//! commit for each one, while a cold open competes with the same lane for the
//! network and its JS thread.
//!
//! This merges a run's consecutive text fragments per stream inside a short
//! window into one event whose `data.text` is the concatenation the client
//! would have accumulated anyway. Merging is by concatenation, never by
//! replacement, so a client that appends each event's text ends up with exactly
//! the text it would have had; nothing about the rendered result changes.
//!
//! Four rules keep the merge honest:
//!
//! * Any non-fragment event flushes the buffer first, so a tool result can
//!   never overtake the arguments it follows. One stream's fragments are
//!   contiguous in practice (verified against real journals, and the stream key
//!   includes the tool id), so two tools can never be merged together.
//! * The merged event keeps the *newest* source event's identity — `idx`,
//!   `eventId`, `timestamp` — because its content now covers every source index
//!   up to that one. The covered count rides along in `coalescedCount`, which
//!   is how a client distinguishes a merged index jump from a lost range.
//! * Merging requires the *whole* identity the client routes and dedups on:
//!   session, run, epoch, event type, tool and reasoning block. This queue carries
//!   every session at once, so a key of type + tool alone merged one's tokens into
//!   another's event — published on whichever subject the last fragment
//!   belonged to, which the client attributes by subject.
//! * Merging requires *consecutive* source indices. `coalescedCount` is a claim
//!   that every index in the range arrived, so a group spanning a hole would
//!   silently cover an event the queue dropped — the client would advance past
//!   it and never heal. Non-consecutive fragments publish separately and the
//!   client's existing gap detection still recovers the hole.
//! * The merged payload stays inside the per-event byte budget: fragments are
//!   capped individually before the queue, and a burst of them must not
//!   reassemble into an event the transport refuses to publish.

use serde_json::Value;

use super::EventPublish;

/// Fragment events whose text accumulates on the client.
const FRAGMENT_TYPES: &[&str] = &[
    "text_chunk",
    "text_delta",
    "thinking_delta",
    "tool_delta",
    "toolcall_delta",
];

/// How long a fragment waits for company before being published alone. A
/// display frame is ~16ms and the client's own commit coalescing is ~80ms, so
/// this stays below what a reader can perceive while still merging the burst.
/// One value for every build: a test-only window would make the real-traffic
/// measurement describe something other than production.
pub(super) const COALESCE_WINDOW: std::time::Duration = std::time::Duration::from_millis(100);

/// Bound on fragments merged into one event. A reasoning burst can reach a few
/// thousand events per second; this keeps one merged payload small regardless.
const MAX_MERGED_FRAGMENTS: usize = 512;

/// Bound on the bytes a merged event may stand for, counted as the sum of the
/// source payloads — an upper bound of the merged payload, because the merge
/// keeps every source's text but only one envelope, and JSON escaping is
/// per-character so escaped concatenation equals concatenated escaping. Slack
/// covers the fields a merge adds (`coalescedCount`, `snapshot`).
const MAX_MERGED_BYTES: usize = super::MAX_EVENT_BYTES - 4096;

/// Identifies the stream a fragment belongs to: everything the client routes,
/// dedups and renders by. Fragments may only merge when all of it matches — a
/// narrower key leaks one session's tokens into another's event, or one run's
/// text into its successor's.
#[derive(PartialEq, Eq)]
struct StreamKey {
    session: String,
    run: String,
    epoch: i64,
    event_type: String,
    tool: String,
    // Reasoning blocks can interleave inside one run, even at adjacent indices.
    block: String,
}

#[derive(Default)]
pub(super) struct Coalescer {
    pending: Option<Pending>,
    /// Overridden only by the real-journal measurement, so the window versus
    /// bandwidth trade-off can be swept on real traffic instead of guessed.
    window: Option<std::time::Duration>,
}

impl Coalescer {
    /// Test-only override of the window; production always uses
    /// `COALESCE_WINDOW`. The real-journal measurement picks this up by running
    /// in the crate's test binary, so it needs no non-test caller.
    #[cfg(test)]
    pub(super) fn with_window(window: std::time::Duration) -> Self {
        Self {
            pending: None,
            window: Some(window),
        }
    }

    fn window(&self) -> std::time::Duration {
        self.window.unwrap_or(COALESCE_WINDOW)
    }
}

struct Pending {
    key: StreamKey,
    /// Index of the newest source event; the next merge must be its successor.
    newest_idx: i64,
    /// Sum of the source payload lengths, an upper bound of the merged payload.
    bytes: usize,
    /// The newest source event; its payload is the base for the merged body.
    event: EventPublish,
    /// Fragment text accumulated across every merged source.
    text: String,
    /// A source replaced the accumulated text (`snapshot: true`), so the group
    /// is no longer a pure concatenation and the flag must survive the merge.
    replaces: bool,
    /// Source events folded into `event` beyond itself.
    merged: usize,
    /// When this group opened; the window is measured from here.
    opened: std::time::Instant,
}

impl Pending {
    /// Whether `fragment` may join this group: same stream, the very next
    /// source index, and still inside both merged-event bounds.
    fn accepts(&self, fragment: &Fragment, event: &EventPublish) -> bool {
        self.key == fragment.key
            && self.newest_idx.checked_add(1) == Some(fragment.idx)
            && self.merged + 1 < MAX_MERGED_FRAGMENTS
            && self.bytes + event.payload.len() <= MAX_MERGED_BYTES
    }
}

impl Coalescer {
    /// Offer one event observed at `now`, appending what may now be published
    /// to `out` (a window expiry, at most one stream switch, and the event).
    ///
    /// Time is injected so the merge can be driven by real event timestamps in
    /// a measurement instead of waiting out a real window.
    pub(super) fn offer(
        &mut self,
        event: EventPublish,
        now: std::time::Instant,
        out: &mut Vec<EventPublish>,
    ) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| now.duration_since(pending.opened) >= self.window())
        {
            self.flush(out);
        }
        let Some(fragment) = fragment_of(&event) else {
            self.flush(out);
            out.push(event);
            return;
        };
        let merges = self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.accepts(&fragment, &event));
        if self.pending.is_some() && !merges {
            self.flush(out);
        }
        match self.pending.as_mut() {
            None => {
                self.pending = Some(Pending {
                    key: fragment.key,
                    newest_idx: fragment.idx,
                    bytes: event.payload.len(),
                    event,
                    text: fragment.text,
                    replaces: fragment.replaces,
                    merged: 0,
                    opened: now,
                });
            }
            Some(pending) => {
                if fragment.replaces {
                    pending.text = fragment.text;
                } else {
                    pending.text.push_str(&fragment.text);
                }
                pending.replaces |= fragment.replaces;
                pending.merged += 1;
                pending.bytes += event.payload.len();
                // The newest source owns the identity: its index is the end of
                // the range this event now covers.
                pending.newest_idx = fragment.idx;
                pending.event = event;
            }
        }
    }

    /// Publish the buffered group, if any.
    pub(super) fn flush(&mut self, out: &mut Vec<EventPublish>) {
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        if pending.merged == 0 {
            // Nothing was merged: the source event is already exact.
            out.push(pending.event);
            return;
        }
        if let Some(payload) = merged_payload(
            &pending.event,
            &pending.text,
            pending.merged + 1,
            pending.replaces,
        ) {
            pending.event.payload = payload;
            out.push(pending.event);
        }
    }
}

/// The parts of a fragment event the coalescer needs.
struct Fragment {
    key: StreamKey,
    /// The source index this fragment occupies, for contiguity.
    idx: i64,
    text: String,
    /// `snapshot: true` means "replace the accumulated text".
    replaces: bool,
}

fn fragment_of(event: &EventPublish) -> Option<Fragment> {
    let body: Value = serde_json::from_slice(&event.payload).ok()?;
    let event_type = body.get("type")?.as_str()?;
    if !FRAGMENT_TYPES.contains(&event_type) {
        return None;
    }
    let text_of = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let data: Value = serde_json::from_str(body.get("data")?.as_str()?).ok()?;
    let text = data.get("text")?.as_str()?;
    let tool = data
        .get("tool_id")
        .or_else(|| data.get("tool_call_id"))
        .and_then(Value::as_str)
        .unwrap_or("");
    Some(Fragment {
        key: StreamKey {
            session: text_of("sessionId"),
            run: text_of("runId"),
            epoch: body
                .get("epoch")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            event_type: event_type.to_owned(),
            tool: tool.to_owned(),
            block: data
                .get("block_id")
                .and_then(Value::as_str)
                .or_else(|| data.get("blockId").and_then(Value::as_str))
                .unwrap_or_default()
                .to_owned(),
        },
        idx: body.get("idx").and_then(Value::as_i64).unwrap_or(-1),
        text: text.to_owned(),
        replaces: data.get("snapshot").and_then(Value::as_bool) == Some(true),
    })
}

/// Rebuild a published body around the merged text. Everything except `data`
/// comes from the newest source event, so the run identity, cursor and event id
/// stay consistent with the index the merged event now occupies. The stream key
/// already guarantees that identity is the same for every source in the group.
fn merged_payload(
    event: &EventPublish,
    text: &str,
    covered: usize,
    replaces: bool,
) -> Option<Vec<u8>> {
    let body: Value = serde_json::from_slice(&event.payload).ok()?;
    let raw = body.get("data")?.as_str()?.to_owned();
    let mut data: Value = serde_json::from_str(&raw).ok()?;
    let object = data.as_object_mut()?;
    object.insert("text".into(), Value::String(text.to_owned()));
    if replaces {
        // A source in this group replaced the accumulated text, so the merged
        // text is the state *after* that replacement. Without the flag the
        // client would append it to its own accumulated text and render the
        // pre-replacement prefix twice (visible as duplicated arguments, and as
        // a tool target parsed out of half-merged JSON).
        object.insert("snapshot".into(), Value::Bool(true));
    }
    // The count belongs on the ENVELOPE, next to `idx`: the client reads
    // `event.coalescedCount` off the decoded event, and a copy inside `data` is
    // invisible to it. That mistake made every merged event look like a gap —
    // the client discarded it and re-fetched the same range over reconcile,
    // which is what the reader sees as a repeatedly flashing sync notice.
    let mut body = body;
    body.as_object_mut()?
        .insert("coalescedCount".into(), Value::from(covered));
    body.as_object_mut()?
        .insert("data".into(), Value::String(data.to_string()));
    serde_json::to_vec(&body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn body(kind: &str, data: Value, idx: i64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "sessionId": "s",
            "type": kind,
            "data": data.to_string(),
            "runId": "r",
            "idx": idx,
            "epoch": 1,
            "eventId": format!("s:r:1:{idx}"),
            "timestamp": "2026-09-19T15:00:00.000000+00:00",
            "sessionIdx": -1,
            "runSequence": 1,
        }))
        .unwrap()
    }

    fn event(kind: &str, data: Value, idx: i64) -> EventPublish {
        EventPublish {
            subject: "p.pair.evt.s".into(),
            payload: body(kind, data, idx),
            status_subject: None,
        }
    }

    fn text_of(published: &EventPublish) -> String {
        let body: Value = serde_json::from_slice(&published.payload).unwrap();
        let data: Value = serde_json::from_str(body["data"].as_str().unwrap()).unwrap();
        data["text"].as_str().unwrap().to_owned()
    }

    fn idx_of(published: &EventPublish) -> i64 {
        let body: Value = serde_json::from_slice(&published.payload).unwrap();
        body["idx"].as_i64().unwrap()
    }

    /// One event whose envelope identity can be varied, for the tests that pin
    /// which differences forbid a merge.
    fn identified(
        session: &str,
        run: &str,
        epoch: i64,
        kind: &str,
        data: Value,
        idx: i64,
    ) -> EventPublish {
        let mut body: Value = serde_json::from_slice(&body(kind, data, idx)).unwrap();
        body["sessionId"] = Value::String(session.into());
        body["runId"] = Value::String(run.into());
        body["epoch"] = Value::from(epoch);
        EventPublish {
            subject: format!("p.pair.evt.{session}"),
            payload: serde_json::to_vec(&body).unwrap(),
            status_subject: None,
        }
    }

    /// The event lane carries every session at once, and the client attributes
    /// an event to the *subject* the desktop published it on. A merge key that
    /// ignored the session therefore shipped one session's tokens as another
    /// session's event, and the session they belonged to never received them.
    #[test]
    fn fragments_from_different_sessions_never_merge() {
        let mut coalescer = Coalescer::default();
        let base = Instant::now();
        let mut out = Vec::new();
        coalescer.offer(
            identified(
                "A",
                "r",
                1,
                "text_chunk",
                serde_json::json!({"text":"A"}),
                11,
            ),
            base,
            &mut out,
        );
        coalescer.offer(
            identified(
                "B",
                "r",
                1,
                "text_chunk",
                serde_json::json!({"text":"B"}),
                // Consecutive indices isolate the session check from gap detection.
                12,
            ),
            base,
            &mut out,
        );
        coalescer.flush(&mut out);
        assert_eq!(out.len(), 2, "each session keeps its own event");
        assert_eq!(out[0].subject, "p.pair.evt.A");
        assert_eq!(text_of(&out[0]), "A");
        assert_eq!(out[1].subject, "p.pair.evt.B");
        assert_eq!(text_of(&out[1]), "B");
    }

    #[test]
    fn interleaved_thinking_blocks_keep_their_own_text_and_ranges() {
        for field in ["block_id", "blockId"] {
            let mut coalescer = Coalescer::default();
            let base = Instant::now();
            let mut out = Vec::new();
            let sources = [
                ("thinking_start", "a", ""),
                ("thinking_start", "b", ""),
                ("thinking_delta", "a", "alpha"),
                ("thinking_delta", "a", "!"),
                ("thinking_delta", "b", "beta"),
                ("thinking_delta", "b", "!"),
                ("thinking_delta", "a", "more"),
                ("thinking_delta", "a", "!"),
            ];
            for (idx, (kind, block, text)) in sources.into_iter().enumerate() {
                let mut data = serde_json::json!({"text": text});
                data[field] = Value::String(block.into());
                coalescer.offer(event(kind, data, idx as i64), base, &mut out);
            }
            coalescer.flush(&mut out);

            assert_eq!(out.len(), 5, "two starts and three separate block ranges");
            for (published, (block, text, idx)) in
                out[2..]
                    .iter()
                    .zip([("a", "alpha!", 3), ("b", "beta!", 5), ("a", "more!", 7)])
            {
                let body: Value = serde_json::from_slice(&published.payload).unwrap();
                let data: Value = serde_json::from_str(body["data"].as_str().unwrap()).unwrap();
                assert_eq!(data[field], block);
                assert_eq!(data["text"], text);
                assert_eq!(body["idx"], idx);
                assert_eq!(body["coalescedCount"], 2);
            }
        }
    }

    #[test]
    fn thinking_block_aliases_merge_but_an_unidentified_block_stays_separate() {
        let mut coalescer = Coalescer::default();
        let base = Instant::now();
        let mut out = Vec::new();
        for (idx, data) in [
            serde_json::json!({"block_id":"a", "text":"alpha"}),
            serde_json::json!({"blockId":"a", "text":"!"}),
            serde_json::json!({"text":"unidentified"}),
            serde_json::json!({"block_id":"b", "text":"beta"}),
        ]
        .into_iter()
        .enumerate()
        {
            coalescer.offer(event("thinking_delta", data, idx as i64), base, &mut out);
        }
        coalescer.flush(&mut out);
        assert_eq!(out.len(), 3);
        assert_eq!(text_of(&out[0]), "alpha!");
        assert_eq!(text_of(&out[1]), "unidentified");
        assert_eq!(text_of(&out[2]), "beta");
        let first: Value = serde_json::from_slice(&out[0].payload).unwrap();
        assert_eq!(first["idx"], 1);
        assert_eq!(first["coalescedCount"], 2);
        let data: Value = serde_json::from_str(first["data"].as_str().unwrap()).unwrap();
        assert_eq!(data["blockId"], "a");
    }

    /// A new run in the same session must not collect the previous run's
    /// tokens: the merged event would carry the new run's id and index while
    /// rendering the old run's text.
    #[test]
    fn fragments_from_different_runs_or_epochs_never_merge() {
        for (run, epoch) in [("next", 1), ("r", 2)] {
            let mut coalescer = Coalescer::default();
            let base = Instant::now();
            let mut out = Vec::new();
            coalescer.offer(
                identified(
                    "A",
                    "r",
                    1,
                    "text_chunk",
                    serde_json::json!({"text":"first"}),
                    4,
                ),
                base,
                &mut out,
            );
            coalescer.offer(
                identified(
                    "A",
                    run,
                    epoch,
                    "text_chunk",
                    serde_json::json!({"text":"second"}),
                    5,
                ),
                base,
                &mut out,
            );
            coalescer.flush(&mut out);
            assert_eq!(out.len(), 2, "run {run} epoch {epoch}");
            assert_eq!(text_of(&out[1]), "second");
            let last: Value = serde_json::from_slice(&out[1].payload).unwrap();
            assert_eq!(last["runId"], Value::String(run.into()));
            assert_eq!(last["epoch"], Value::from(epoch));
        }
    }

    /// `coalescedCount` claims that every index in the range arrived, so a
    /// group must not span a hole: a dropped event would be silently covered,
    /// and the client would advance past it instead of healing the gap.
    #[test]
    fn a_merge_never_spans_a_source_index_hole() {
        let mut coalescer = Coalescer::default();
        let base = Instant::now();
        let mut out = Vec::new();
        for idx in [7, 8, 10] {
            coalescer.offer(
                event("text_chunk", serde_json::json!({"text":"x"}), idx),
                base,
                &mut out,
            );
        }
        coalescer.flush(&mut out);
        assert_eq!(out.len(), 2, "the hole splits the burst");
        let first: Value = serde_json::from_slice(&out[0].payload).unwrap();
        assert_eq!(first["idx"], 8);
        assert_eq!(first["coalescedCount"], 2, "covers exactly 7 and 8");
        // The fragment after the hole is published as itself: index 10 claiming
        // a range would have hidden index 9 from the client's gap detection.
        let second: Value = serde_json::from_slice(&out[1].payload).unwrap();
        assert_eq!(second["idx"], 10);
        assert!(second.get("coalescedCount").is_none());
    }

    /// Fragments are size-capped one by one *before* the queue, so a burst of
    /// legal events could reassemble into one the transport refuses to publish
    /// — and a failed publish ends the lane for every session on the
    /// connection.
    #[test]
    fn a_merge_stays_within_the_event_byte_budget() {
        let mut coalescer = Coalescer::default();
        let base = Instant::now();
        let mut out = Vec::new();
        // Each source is legal on its own (under the event cap); two of them
        // together are too big for one merged event.
        let fragment = "x".repeat(super::MAX_MERGED_BYTES / 2 - 1024);
        assert!(fragment.len() < super::super::MAX_EVENT_BYTES);
        for idx in 0..4 {
            coalescer.offer(
                event("text_chunk", serde_json::json!({"text": fragment}), idx),
                base,
                &mut out,
            );
        }
        coalescer.flush(&mut out);
        assert_eq!(out.len(), 2, "the burst splits into budget-sized groups");
        for published in &out {
            assert!(
                published.payload.len() <= super::super::MAX_EVENT_BYTES,
                "merged event of {} bytes exceeds the publish budget",
                published.payload.len()
            );
        }
        assert!(
            out[0].payload.len() > fragment.len(),
            "the first group merged"
        );
        let merged: String = out.iter().map(text_of).collect();
        assert_eq!(merged, fragment.repeat(4), "no text may be lost");
    }

    /// A provider that resends the accumulated arguments mid-stream makes the
    /// group a replacement, not a concatenation. The merged event must keep
    /// that meaning, or a client that appends renders the pre-replacement
    /// prefix twice.
    #[test]
    fn a_replacing_source_keeps_its_replacement_semantics_through_a_merge() {
        let mut coalescer = Coalescer::default();
        let base = Instant::now();
        let mut out = Vec::new();
        coalescer.offer(
            event(
                "tool_delta",
                serde_json::json!({"text":"NEW", "tool_id":"t", "snapshot":true}),
                1,
            ),
            base,
            &mut out,
        );
        coalescer.offer(
            event(
                "tool_delta",
                serde_json::json!({"text":"!", "tool_id":"t"}),
                2,
            ),
            base,
            &mut out,
        );
        coalescer.flush(&mut out);
        assert_eq!(out.len(), 1, "one group, one event");
        let published: Value = serde_json::from_slice(&out[0].payload).unwrap();
        let data: Value = serde_json::from_str(published["data"].as_str().unwrap()).unwrap();
        assert_eq!(data["text"], "NEW!");
        // The flag is the whole point: without it a client holding `OLD`
        // appends and renders `OLDNEW!`, with it the client replaces and
        // renders `NEW!` — the text the provider actually sent.
        assert_eq!(
            data["snapshot"], true,
            "the merged text replaces the client's accumulation"
        );
    }

    /// The merge must be invisible to a client that accumulates text: what it
    /// would have concatenated arrives as one event with the same characters.
    #[test]
    fn merged_text_equals_the_concatenation_a_client_would_have_built() {
        let mut coalescer = Coalescer::default();
        let base = Instant::now();
        let fragments = ["The ", "user ", "wants ", "zoom"];
        let mut out = Vec::new();
        for (index, fragment) in fragments.iter().enumerate() {
            coalescer.offer(
                event(
                    "text_chunk",
                    serde_json::json!({"text": fragment}),
                    index as i64,
                ),
                base + Duration::from_millis(index as u64 * 5),
                &mut out,
            );
        }
        coalescer.flush(&mut out);
        assert_eq!(out.len(), 1, "a burst within one window must publish once");
        assert_eq!(text_of(&out[0]), fragments.concat());
        // The merged event occupies the newest source index and declares how
        // many sources it stands for, which is what lets a client tell a merged
        // jump from a lost range.
        assert_eq!(idx_of(&out[0]), 3);
        let body: Value = serde_json::from_slice(&out[0].payload).unwrap();
        // On the ENVELOPE, beside `idx`: that is where the client reads it, and
        // a copy inside `data` is invisible to `nextEvent`.
        assert_eq!(body["coalescedCount"], 4, "count must ride on the envelope");
        let data: Value = serde_json::from_str(body["data"].as_str().unwrap()).unwrap();
        assert_eq!(data["text"], fragments.concat());
        assert_eq!(body["eventId"], "s:r:1:3");
    }

    #[test]
    fn a_non_fragment_event_flushes_first_and_keeps_its_order() {
        let mut coalescer = Coalescer::default();
        let base = Instant::now();
        let mut out = Vec::new();
        coalescer.offer(
            event("text_chunk", serde_json::json!({"text": "a"}), 0),
            base,
            &mut out,
        );
        coalescer.offer(
            event("text_chunk", serde_json::json!({"text": "b"}), 1),
            base,
            &mut out,
        );
        // The tool ends the text run: the merged text must be published before
        // the result that follows it.
        coalescer.offer(
            event("tool_end", serde_json::json!({"text": "result"}), 2),
            base,
            &mut out,
        );
        coalescer.flush(&mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(text_of(&out[0]), "ab");
        assert_eq!(idx_of(&out[0]), 1);
        assert_eq!(idx_of(&out[1]), 2);
        let second: Value = serde_json::from_slice(&out[1].payload).unwrap();
        assert_eq!(second["type"], "tool_end");
        // An untouched event keeps its exact bytes: no re-serialization drift.
        assert_eq!(
            out[1].payload,
            body("tool_end", serde_json::json!({"text": "result"}), 2)
        );
    }

    #[test]
    fn a_window_expiry_publishes_what_it_has_and_a_lone_fragment_is_untouched() {
        let mut coalescer = Coalescer::default();
        let base = Instant::now();
        let mut out = Vec::new();
        coalescer.offer(
            event("thinking_delta", serde_json::json!({"text": "one"}), 0),
            base,
            &mut out,
        );
        // Past the window: the next fragment must not merge with the stale one.
        coalescer.offer(
            event("thinking_delta", serde_json::json!({"text": "two"}), 1),
            base + COALESCE_WINDOW,
            &mut out,
        );
        coalescer.flush(&mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(text_of(&out[0]), "one");
        assert_eq!(text_of(&out[1]), "two");
        assert_eq!(
            out[0].payload,
            body("thinking_delta", serde_json::json!({"text": "one"}), 0)
        );
    }

    /// Two tools must never share accumulated text even if a provider
    /// interleaves their streams; the stream key carries the tool identity.
    #[test]
    fn interleaved_tool_streams_never_merge_into_each_other() {
        let mut coalescer = Coalescer::default();
        let base = Instant::now();
        let mut out = Vec::new();
        for (idx, (tool, text)) in [
            ("a", "{\"pa"),
            ("b", "{\"pa"),
            ("a", "th\":1}"),
            ("b", "th\":2}"),
        ]
        .iter()
        .enumerate()
        {
            coalescer.offer(
                event(
                    "tool_delta",
                    serde_json::json!({"text": text, "tool_id": tool}),
                    idx as i64,
                ),
                base,
                &mut out,
            );
        }
        coalescer.flush(&mut out);
        // A stream switch publishes what it had, so an interleaved provider gets
        // more events — never mixed text. Each tool's own fragments still
        // concatenate to exactly its arguments.
        let mut per_tool: std::collections::BTreeMap<String, String> = Default::default();
        for published in &out {
            let body: Value = serde_json::from_slice(&published.payload).unwrap();
            let data: Value = serde_json::from_str(body["data"].as_str().unwrap()).unwrap();
            per_tool
                .entry(data["tool_id"].as_str().unwrap().to_owned())
                .or_default()
                .push_str(data["text"].as_str().unwrap());
        }
        assert_eq!(per_tool["a"], "{\"path\":1}");
        assert_eq!(per_tool["b"], "{\"path\":2}");
    }

    /// A provider that re-sends the whole accumulated text marks it `snapshot`;
    /// that replaces rather than appends, and merging must respect it.
    #[test]
    fn snapshot_fragments_replace_instead_of_appending() {
        let mut coalescer = Coalescer::default();
        let base = Instant::now();
        let mut out = Vec::new();
        coalescer.offer(
            event(
                "tool_delta",
                serde_json::json!({"text": "{\"a", "tool_id": "t"}),
                0,
            ),
            base,
            &mut out,
        );
        coalescer.offer(
            event(
                "tool_delta",
                serde_json::json!({"text": "{\"a\":1}", "tool_id": "t", "snapshot": true}),
                1,
            ),
            base,
            &mut out,
        );
        coalescer.flush(&mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(text_of(&out[0]), "{\"a\":1}");
    }

    /// Non-fragment and unparsable traffic must pass through byte-identical:
    /// the lane exists for token fragments, not for rewriting the protocol.
    #[test]
    fn everything_else_passes_through_unchanged() {
        let mut coalescer = Coalescer::default();
        let mut out = Vec::new();
        let plain = event("agent_end", serde_json::json!({"duration_ms": 5}), 9);
        coalescer.offer(plain.clone(), Instant::now(), &mut out);
        let malformed = EventPublish {
            subject: "p.pair.evt.s".into(),
            payload: b"not json".to_vec(),
            status_subject: Some("p.pair.state.events".into()),
        };
        coalescer.offer(malformed.clone(), Instant::now(), &mut out);
        coalescer.flush(&mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].payload, plain.payload);
        assert_eq!(out[1].payload, malformed.payload);
        assert_eq!(out[1].subject, malformed.subject);
    }

    /// Bounded merge: a burst larger than the fragment cap is split rather than
    /// grown without limit.
    #[test]
    fn a_burst_larger_than_the_cap_splits_into_bounded_events() {
        let mut coalescer = Coalescer::default();
        let base = Instant::now();
        let mut out = Vec::new();
        let total = MAX_MERGED_FRAGMENTS * 2 + 1;
        for idx in 0..total {
            coalescer.offer(
                event("text_chunk", serde_json::json!({"text": "x"}), idx as i64),
                base,
                &mut out,
            );
        }
        coalescer.flush(&mut out);
        assert!(out.len() >= 2, "a long burst must be split");
        let merged: String = out.iter().map(text_of).collect();
        assert_eq!(merged.len(), total, "no character may be lost");
        assert_eq!(idx_of(out.last().unwrap()), (total - 1) as i64);
    }

    /// Real-traffic measurement, driven by `scripts/measure/measure-live-lane.py`.
    ///
    /// Feeds one run's real journal through this real coalescer using the
    /// event's own timestamps as the clock, and reports what the phone would
    /// receive today versus with the coalesced lane. Two invariants are checked
    /// before the numbers are trusted: every fragment must survive into exactly
    /// one published event, and the published index range must cover the whole
    /// run with the newest event last.
    #[test]
    #[ignore = "driven by scripts/measure/measure-live-lane.py with a real journal"]
    fn measure_real_journal() {
        let path = std::env::var("SYNC_MEASURE_JOURNAL").expect("SYNC_MEASURE_JOURNAL");
        let session = std::env::var("SYNC_MEASURE_SESSION").expect("SYNC_MEASURE_SESSION");
        let run = std::env::var("SYNC_MEASURE_RUN").expect("SYNC_MEASURE_RUN");
        let journal = std::fs::read_to_string(path).expect("journal readable");
        let base = Instant::now();
        let mut first_stamp: Option<i64> = None;

        let window = std::env::var("SYNC_MEASURE_WINDOW_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or(COALESCE_WINDOW);
        let mut coalescer = Coalescer::with_window(window);
        let mut published = Vec::new();
        let mut today_bytes = 0usize;
        let mut today_data_bytes = 0usize;
        let mut events = 0usize;
        // Every fragment must reappear in the merged output exactly once.
        let mut source_characters = 0usize;
        let mut merged_characters = 0usize;
        let mut first_idx = i64::MAX;
        let mut last_idx = i64::MIN;

        for line in journal.lines().filter(|line| !line.trim().is_empty()) {
            let raw: Value = serde_json::from_str(line).expect("journal line");
            let event_type = raw["event_type"].as_str().unwrap_or_default();
            let data = raw["data"].as_str().unwrap_or("{}");
            let idx = raw["idx"].as_i64().unwrap_or(0);
            let payload = super::super::build_event_body(
                &session,
                event_type,
                data,
                &run,
                idx,
                raw["epoch"].as_i64().unwrap_or(0),
                "",
                raw["timestamp"].as_str().unwrap_or_default(),
                raw["session_idx"].as_i64().unwrap_or(-1),
                raw["run_sequence"].as_i64().unwrap_or(0),
            );
            let bytes = serde_json::to_vec(&payload).expect("body serializes");
            today_bytes += bytes.len();
            // Data versus envelope: the envelope is everything the body carries
            // besides the event's own `data` string (identity, ids, timestamps),
            // which is what a phone pays per event regardless of payload size.
            today_data_bytes += data.len();
            events += 1;
            if let Some(text) = serde_json::from_str::<Value>(data)
                .ok()
                .and_then(|parsed| parsed["text"].as_str().map(str::to_owned))
            {
                source_characters += text.chars().count();
            }
            first_idx = first_idx.min(idx);
            last_idx = last_idx.max(idx);
            // The window is driven by the events' own clock, so a 17-minute run
            // measures in milliseconds while still exercising the real window.
            let stamp = raw["timestamp"]
                .as_str()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.timestamp_millis())
                .unwrap_or(idx * 100);
            let first = *first_stamp.get_or_insert(stamp);
            let now = base + Duration::from_millis((stamp - first).max(0) as u64);
            coalescer.offer(
                EventPublish {
                    subject: format!("p.pair.evt.{session}"),
                    payload: bytes,
                    status_subject: None,
                },
                now,
                &mut published,
            );
        }
        coalescer.flush(&mut published);

        let mut coalesced_bytes = 0usize;
        let mut coalesced_data_bytes = 0usize;
        let mut parties = 0usize;
        let mut newest_idx = i64::MIN;
        for event in &published {
            coalesced_bytes += event.payload.len();
            let body: Value = serde_json::from_slice(&event.payload).expect("published body");
            coalesced_data_bytes += body["data"].as_str().unwrap_or("").len();
            let data: Value =
                serde_json::from_str(body["data"].as_str().unwrap_or("{}")).unwrap_or(Value::Null);
            // The count is envelope metadata; the measurement must read it there
            // or it under-counts the events each publish stands for.
            parties += body["coalescedCount"].as_u64().unwrap_or(1) as usize;
            if let Some(text) = data["text"].as_str() {
                merged_characters += text.chars().count();
            }
            newest_idx = newest_idx.max(body["idx"].as_i64().unwrap_or(0));
        }
        assert_eq!(parties, events, "every source event must be accounted for");
        assert_eq!(
            merged_characters, source_characters,
            "no character may be lost or duplicated"
        );
        assert_eq!(
            newest_idx, last_idx,
            "the newest index must still be published"
        );
        assert!(
            coalesced_bytes < today_bytes,
            "coalescing must not grow the lane"
        );
        let _ = first_idx;
        println!(
            "LIVE_LANE {}",
            serde_json::json!({
                "events": events,
                "todayBytes": today_bytes,
                "todayDataBytes": today_data_bytes,
                "coalescedBytes": coalesced_bytes,
                "coalescedDataBytes": coalesced_data_bytes,
                "published": published.len(),
                "parties": parties,
                "windowMs": window.as_millis(),
                "ratio": today_bytes as f64 / coalesced_bytes.max(1) as f64,
            })
        );
    }

    /// The same measurement for the lean lane, over the same real journal and
    /// through the same shipping code: every event is first rewritten by
    /// [`crate::remote_host::lean::lean_event_data`], exactly as
    /// `remote::publisher::publish_event` rewrites it, and then coalesced.
    ///
    /// Run it through `scripts/measure/measure-live-lane.py`, which supplies the three
    /// environment variables.
    ///
    /// Unlike the full-lane measurement it cannot assert "every source event is
    /// accounted for" — dropping content is the point. It asserts what must stay
    /// true instead: nothing of a dropped type reaches the lane, and the newest
    /// source index is still published, so a client's dedup cursor still reaches
    /// the end of the run.
    #[test]
    #[ignore = "measurement: needs SYNC_MEASURE_JOURNAL/SESSION/RUN"]
    fn measure_real_journal_lean() {
        use crate::remote_host::lean::lean_event_data;
        let path = std::env::var("SYNC_MEASURE_JOURNAL").expect("SYNC_MEASURE_JOURNAL");
        let session = std::env::var("SYNC_MEASURE_SESSION").expect("SYNC_MEASURE_SESSION");
        let run = std::env::var("SYNC_MEASURE_RUN").expect("SYNC_MEASURE_RUN");
        let journal = std::fs::read_to_string(path).expect("journal readable");
        let base = Instant::now();
        let mut first_stamp: Option<i64> = None;
        let window = std::env::var("SYNC_MEASURE_WINDOW_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or(COALESCE_WINDOW);
        let mut coalescer = Coalescer::with_window(window);
        let mut published = Vec::new();
        let mut full_bytes = 0usize;
        let mut lean_bytes = 0usize;
        // Per-type totals, so a report can say which event types still carry the
        // lane rather than only how big it is.
        let mut full_by_type: std::collections::BTreeMap<String, usize> = Default::default();
        let mut lean_by_type: std::collections::BTreeMap<String, usize> = Default::default();
        let mut events = 0usize;
        let mut delivered = 0usize;
        let mut last_idx = i64::MIN;

        for line in journal.lines().filter(|line| !line.trim().is_empty()) {
            let raw: Value = serde_json::from_str(line).expect("journal line");
            let event_type = raw["event_type"].as_str().unwrap_or_default();
            let data = raw["data"].as_str().unwrap_or("{}");
            let idx = raw["idx"].as_i64().unwrap_or(0);
            events += 1;
            last_idx = last_idx.max(idx);
            let full_body = serde_json::to_vec(&super::super::build_event_body(
                &session,
                event_type,
                data,
                &run,
                idx,
                raw["epoch"].as_i64().unwrap_or(0),
                "",
                raw["timestamp"].as_str().unwrap_or_default(),
                raw["session_idx"].as_i64().unwrap_or(-1),
                raw["run_sequence"].as_i64().unwrap_or(0),
            ))
            .expect("body serializes");
            *full_by_type.entry(event_type.to_string()).or_default() += full_body.len();
            full_bytes += full_body.len();

            let Some(lean) = lean_event_data(event_type, data) else {
                continue;
            };
            delivered += 1;
            let payload = super::super::build_event_body(
                &session,
                event_type,
                &lean,
                &run,
                idx,
                raw["epoch"].as_i64().unwrap_or(0),
                "",
                raw["timestamp"].as_str().unwrap_or_default(),
                raw["session_idx"].as_i64().unwrap_or(-1),
                raw["run_sequence"].as_i64().unwrap_or(0),
            );
            let lean_body = serde_json::to_vec(&payload).expect("body serializes");
            *lean_by_type.entry(event_type.to_string()).or_default() += lean_body.len();
            lean_bytes += lean_body.len();
            let stamp = raw["timestamp"]
                .as_str()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.timestamp_millis())
                .unwrap_or(idx * 100);
            let first = *first_stamp.get_or_insert(stamp);
            let now = base + Duration::from_millis((stamp - first).max(0) as u64);
            coalescer.offer(
                EventPublish {
                    subject: format!("p.pair.evt.{session}"),
                    payload: serde_json::to_vec(&payload).expect("body serializes"),
                    status_subject: None,
                },
                now,
                &mut published,
            );
        }
        coalescer.flush(&mut published);

        let mut coalesced_bytes = 0usize;
        let mut newest_idx = i64::MIN;
        let mut published_by_type: std::collections::BTreeMap<String, usize> = Default::default();
        for event in &published {
            coalesced_bytes += event.payload.len();
            let body: Value = serde_json::from_slice(&event.payload).expect("published body");
            let event_type = body["type"].as_str().unwrap_or_default();
            *published_by_type.entry(event_type.to_string()).or_default() += event.payload.len();
            assert!(
                lean_event_data(event_type, "{}").is_some(),
                "{event_type} streams content the lean lane must not carry"
            );
            newest_idx = newest_idx.max(body["idx"].as_i64().unwrap_or(0));
        }
        assert_eq!(
            newest_idx, last_idx,
            "the newest index must still be published, or the client's cursor stalls"
        );
        println!(
            "LEAN_LANE {}",
            serde_json::json!({
                "events": events,
                "delivered": delivered,
                "fullBytes": full_bytes,
                "leanBytes": lean_bytes,
                "fullByType": full_by_type,
                "leanByType": lean_by_type,
                "publishedByType": published_by_type,
                "published": published.len(),
                "coalescedBytes": coalesced_bytes,
                "windowMs": window.as_millis(),
                "reduction": 1.0 - (lean_bytes as f64 / full_bytes.max(1) as f64),
            })
        );
    }
}
