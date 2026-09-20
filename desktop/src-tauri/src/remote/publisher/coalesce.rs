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
//! Two rules keep ordering honest:
//!
//! * Any non-fragment event flushes the buffer first, so a tool result can
//!   never overtake the arguments it follows. One stream's fragments are
//!   contiguous in practice (verified against real journals, and the stream key
//!   includes the tool id), so two tools can never be merged together.
//! * The merged event keeps the *newest* source event's identity — `idx`,
//!   `eventId`, `timestamp` — because its content now covers every source index
//!   up to that one. The covered count rides along in `coalescedCount`, which
//!   is how a client distinguishes a merged index jump from a lost range.

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

/// Identifies the stream a fragment belongs to. Fragments may only merge when
/// the type and the tool identity match — an interleaved tool would otherwise
/// collect another tool's arguments.
type StreamKey = (String, String);

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
    /// The newest source event; its payload is the base for the merged body.
    event: EventPublish,
    /// Fragment text accumulated across every merged source.
    text: String,
    /// Source events folded into `event` beyond itself.
    merged: usize,
    /// When this group opened; the window is measured from here.
    opened: std::time::Instant,
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
        let same_stream = self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.key == fragment.key);
        if self.pending.is_some() && (!same_stream || self.full()) {
            self.flush(out);
        }
        match self.pending.as_mut() {
            None => {
                self.pending = Some(Pending {
                    key: fragment.key,
                    event,
                    text: fragment.text,
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
                pending.merged += 1;
                // The newest source owns the identity: its index is the end of
                // the range this event now covers.
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
        if let Some(payload) = merged_payload(&pending.event, &pending.text, pending.merged + 1) {
            pending.event.payload = payload;
            out.push(pending.event);
        }
    }

    fn full(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.merged + 1 >= MAX_MERGED_FRAGMENTS)
    }
}

/// The parts of a fragment event the coalescer needs.
struct Fragment {
    key: StreamKey,
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
    let data: Value = serde_json::from_str(body.get("data")?.as_str()?).ok()?;
    let text = data.get("text")?.as_str()?;
    let tool = data
        .get("tool_id")
        .or_else(|| data.get("tool_call_id"))
        .and_then(Value::as_str)
        .unwrap_or("");
    Some(Fragment {
        key: (event_type.to_owned(), tool.to_owned()),
        text: text.to_owned(),
        replaces: data.get("snapshot").and_then(Value::as_bool) == Some(true),
    })
}

/// Rebuild a published body around the merged text. Everything except `data`
/// comes from the newest source event, so the run identity, cursor and event id
/// stay consistent with the index the merged event now occupies.
fn merged_payload(event: &EventPublish, text: &str, covered: usize) -> Option<Vec<u8>> {
    let body: Value = serde_json::from_slice(&event.payload).ok()?;
    let raw = body.get("data")?.as_str()?.to_owned();
    let mut data: Value = serde_json::from_str(&raw).ok()?;
    let object = data.as_object_mut()?;
    object.insert("text".into(), Value::String(text.to_owned()));
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

    /// Real-traffic measurement, driven by `scripts/measure-live-lane.py`.
    ///
    /// Feeds one run's real journal through this real coalescer using the
    /// event's own timestamps as the clock, and reports what the phone would
    /// receive today versus with the coalesced lane. Two invariants are checked
    /// before the numbers are trusted: every fragment must survive into exactly
    /// one published event, and the published index range must cover the whole
    /// run with the newest event last.
    #[test]
    #[ignore = "driven by scripts/measure-live-lane.py with a real journal"]
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
}
