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
