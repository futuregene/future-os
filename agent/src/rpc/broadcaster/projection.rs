//! Folding live events into a bounded reconnect projection.
use super::SseEvent;

pub(super) fn is_buffered_delta(event_type: &str) -> bool {
    matches!(
        event_type,
        "thinking_delta" | "tool_delta" | "toolcall_delta" | "text_chunk" | "text_delta"
    )
}

/// Fold a run event into the durable-in-memory semantic projection.
///
/// The replay ring is intentionally bounded, while the projection must retain
/// enough information to rebuild the visible run after that ring truncates.
/// High-frequency deltas are coalesced into their preceding semantic segment;
/// lifecycle, tool terminal, approval, usage, error, and terminal events keep
/// their original ordering and cursor.
#[cfg(test)]
pub(in crate::rpc) fn apply_to_projection(projection: &mut Vec<SseEvent>, event: &SseEvent) {
    // `text_delta` (raw provider-stream token) duplicates `text_chunk` (the
    // on_text-derived token); consumers project the latter, so retaining both
    // would duplicate assistant output.
    if event.event_type == "text_delta" {
        return;
    }

    let coalescible = matches!(
        event.event_type.as_str(),
        "text_chunk" | "thinking_delta" | "toolcall_delta" | "tool_delta"
    );
    if coalescible {
        if let Some(previous) = projection
            .last_mut()
            .filter(|previous| previous.event_type == event.event_type)
        {
            if let (Ok(mut previous_data), Ok(next_data)) = (
                serde_json::from_str::<serde_json::Value>(&previous.data),
                serde_json::from_str::<serde_json::Value>(&event.data),
            ) {
                let same_tool_stream = crate::rpc::run_snapshot::same_delta_stream(
                    previous,
                    event,
                    &previous_data,
                    &next_data,
                );
                if let (Some(previous_text), Some(next_text)) = (
                    previous_data.get("text").and_then(|value| value.as_str()),
                    next_data.get("text").and_then(|value| value.as_str()),
                ) {
                    if !same_tool_stream {
                        projection.push(event.clone());
                        return;
                    }
                    let combined = format!("{previous_text}{next_text}");
                    previous_data["text"] = serde_json::Value::String(combined);
                    previous.data = serde_json::to_string(&previous_data).unwrap_or_default();
                    // The folded segment represents every source event through
                    // this cursor, so live resume starts strictly after it.
                    previous.idx = event.idx;
                    previous.event_id = event.event_id.clone();
                    previous.timestamp = event.timestamp.clone();
                    previous.run_sequence = event.run_sequence;
                    return;
                }
            }
        }
    }

    projection.push(event.clone());
}

/// Incremental live state. Closed segments are immutable and cheap to capture;
/// the only mutable text is bounded, and is serialized once when sealed.
#[derive(Default)]
pub(super) struct Projection {
    closed: Vec<std::sync::Arc<SseEvent>>,
    pending: Option<(SseEvent, serde_json::Value)>,
    bytes: usize,
    budget: Option<usize>,
    over_budget: bool,
}

pub(super) struct ProjectionView {
    closed: Vec<std::sync::Arc<SseEvent>>,
    pending: Option<(SseEvent, serde_json::Value)>,
}

const PROJECTION_SEGMENT_BYTES: usize = 16 * 1024;

impl Projection {
    pub(super) fn from_events(events: Vec<SseEvent>) -> Self {
        let mut projection = Self::default();
        for event in events {
            projection.push(&event);
        }
        projection
    }

    pub(super) fn clear(&mut self) {
        self.closed.clear();
        self.pending = None;
        self.bytes = 0;
        self.over_budget = false;
    }

    pub(super) fn limit(&mut self, bytes: usize) {
        self.budget = Some(bytes);
    }
    pub(super) fn over_budget(&self) -> bool {
        self.over_budget
    }

    fn flush(&mut self) {
        if let Some((mut event, data)) = self.pending.take() {
            event.data = data.to_string();
            self.closed.push(std::sync::Arc::new(event));
        }
    }

    pub(super) fn push(&mut self, event: &SseEvent) {
        if self.over_budget {
            return;
        }
        self.bytes = self.bytes.saturating_add(event.data.len() + 512);
        if self.budget.is_some_and(|budget| self.bytes > budget) {
            self.closed.clear();
            self.pending = None;
            self.over_budget = true;
            return;
        }
        if event.event_type == "text_delta" {
            return;
        }
        let delta = matches!(
            event.event_type.as_str(),
            "text_chunk" | "thinking_delta" | "tool_delta" | "toolcall_delta"
        );
        if delta {
            if let Ok(next) = serde_json::from_str::<serde_json::Value>(&event.data) {
                if let Some(text) = next.get("text").and_then(serde_json::Value::as_str) {
                    if let Some((prior, data)) = self.pending.as_mut() {
                        let bytes = data["text"].as_str().map_or(0, str::len);
                        if bytes.saturating_add(text.len()) <= PROJECTION_SEGMENT_BYTES
                            && crate::rpc::run_snapshot::same_delta_stream(
                                prior, event, data, &next,
                            )
                        {
                            if let serde_json::Value::String(current) = &mut data["text"] {
                                current.push_str(text);
                            }
                            if data.get("content_key").is_some() {
                                data["content_bytes"] = serde_json::json!(bytes + text.len());
                            }
                            prior.idx = event.idx;
                            prior.event_id.clone_from(&event.event_id);
                            prior.timestamp.clone_from(&event.timestamp);
                            prior.run_sequence = event.run_sequence;
                            prior.session_idx = event.session_idx;
                            return;
                        }
                    }
                    self.flush();
                    if text.len() <= PROJECTION_SEGMENT_BYTES {
                        let mut header = event.clone();
                        header.data.clear();
                        self.pending = Some((header, next));
                        return;
                    }
                }
            }
        }
        self.flush();
        self.closed.push(std::sync::Arc::new(event.clone()));
    }

    /// Capture references under the sequencing lock; materialize after release.
    pub(super) fn view(&self) -> ProjectionView {
        ProjectionView {
            closed: self.closed.clone(),
            pending: self.pending.clone(),
        }
    }
}

impl ProjectionView {
    pub(super) fn materialize(self) -> Vec<SseEvent> {
        let mut events: Vec<SseEvent> = self
            .closed
            .into_iter()
            .map(|event| (*event).clone())
            .collect();
        if let Some((mut event, data)) = self.pending {
            event.data = data.to_string();
            events.push(event);
        }
        events
    }
}
