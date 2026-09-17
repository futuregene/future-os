use serde_json::{json, Value};

/// Reply budget for a `get_messages` page: comfortably under NATS's 1MB
/// user-JWT payload limit, leaving headroom for the reply envelope.
pub(crate) const MESSAGES_PAGE_BYTES: usize = 512 * 1024;
/// Backward mobile history keeps the requested ten-exchange semantic maximum,
/// with the same 512 KiB wire budget as other remote history pages. Complete
/// oldest exchanges are deferred only when an unusually content-heavy ten-turn
/// page would exceed that budget; a page never splits an exchange.
pub(crate) const BACKWARD_HISTORY_PAGE_BYTES: usize = 512 * 1024;
/// A single persisted message can embed a huge tool result; cap its content so
/// one oversized message can't push a page past the payload limit on its own.
pub(crate) const MESSAGE_CONTENT_CAP_BYTES: usize = 256 * 1024;
/// Default page size when the client doesn't ask for one.
pub(crate) const DEFAULT_MESSAGE_PAGE_LIMIT: usize = 100;

/// Extract the `messages` array from an agent `get_messages` reply.
pub(crate) fn messages_vec(data: Value) -> Vec<Value> {
    data.get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Extract the `entries` array from an agent `get_session_entries` reply.
pub(crate) fn entries_vec(data: Value) -> Vec<Value> {
    data.get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn paginate_messages(messages: Vec<Value>, offset: usize, limit: usize) -> Value {
    paginate_items(messages, offset, limit, "messages")
}

/// Apply the remote wire caps to a backward Agent page without losing its
/// cursor. If ten unusually large exchanges exceed the NATS page budget, drop
/// complete oldest exchanges until the page fits and advance the returned
/// cursor past those omitted rows; they remain reachable on the next pull.
#[cfg(test)]
pub(crate) fn prepare_backward_entries_page(_session_id: &str, data: Value) -> Value {
    prepare_backward_entries_page_with_cap(_session_id, data, true)
}

pub(crate) fn prepare_backward_entries_page_with_cap(
    _session_id: &str,
    data: Value,
    cap_items: bool,
) -> Value {
    let mut entries = entries_vec(data.clone());
    if cap_items {
        for entry in &mut entries {
            cap_remote_item(entry, MESSAGE_CONTENT_CAP_BYTES);
        }
    }
    let agent_start = data
        .get("nextOffset")
        .or_else(|| data.get("next_offset"))
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize;
    let agent_has_more = data
        .get("hasMore")
        .or_else(|| data.get("has_more"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut removed = 0usize;
    while cap_items
        && serde_json::to_vec(&entries).map_or(0, |bytes| bytes.len()) > BACKWARD_HISTORY_PAGE_BYTES
    {
        let Some(next_user) = entries
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(index, entry)| {
                (entry.get("role").and_then(Value::as_str) == Some("user")).then_some(index)
            })
        else {
            break;
        };
        entries.drain(..next_user);
        removed += next_user;
    }
    let next_offset = agent_start.saturating_add(removed);
    json!({
        "offset": next_offset,
        "nextOffset": next_offset,
        "hasMore": agent_has_more || removed > 0,
        "entries": entries,
    })
}

/// Page a full item list into a reply that fits the NATS payload cap.
///
/// Each item is content-capped first (so no single item is huge), then items
/// are accumulated from `offset` until the serialized page would exceed
/// [`MESSAGES_PAGE_BYTES`] or `limit` is reached (always at least one item —
/// it's already capped). Returns the page (under `key`) plus cursor fields the
/// client uses to fetch the remainder.
pub(crate) fn paginate_items(items: Vec<Value>, offset: usize, limit: usize, key: &str) -> Value {
    paginate_items_with_cap(items, offset, limit, key, true)
}

pub(crate) fn paginate_items_with_cap(
    mut items: Vec<Value>,
    offset: usize,
    limit: usize,
    key: &str,
    cap_items: bool,
) -> Value {
    if cap_items {
        for item in items.iter_mut() {
            cap_remote_item(item, MESSAGE_CONTENT_CAP_BYTES);
        }
    }
    let total = items.len();
    let start = offset.min(total);
    let mut end = start;
    let mut bytes = 0usize;
    for (index, item) in items.iter().skip(start).enumerate() {
        let size = serde_json::to_vec(item)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        if index > 0 && (index >= limit || (cap_items && bytes + size > MESSAGES_PAGE_BYTES)) {
            break;
        }
        bytes += size;
        end += 1;
    }
    let page: Vec<Value> = items.drain(start..end).collect();
    let mut value = json!({
        "offset": start,
        "nextOffset": end,
        "total": total,
        "hasMore": end < total,
    });
    value[key] = json!(page);
    value
}

/// Page a session's replay event tail into a reply that fits the NATS payload
/// cap, mirroring `paginate_items` (each event's `data` is capped, then events
/// accumulate until the page would exceed [`MESSAGES_PAGE_BYTES`]). The reply
/// keeps the envelope's non-event fields (`runId`, `projection`, `truncated`)
/// on every page so the client can distinguish a ring-overflow projection from
/// a plain tail replay regardless of which page it lands on.
pub(crate) fn paginate_events(mut data: Value, offset: usize, limit: usize) -> Value {
    let run_id = data.get("runId").cloned().unwrap_or(Value::Null);
    let projection = data.get("projection").cloned().unwrap_or(Value::Null);
    let truncated = data.get("truncated").cloned().unwrap_or(Value::Null);
    let events = data
        .get_mut("events")
        .and_then(Value::as_array_mut)
        .map(std::mem::take)
        .unwrap_or_default();
    let mut page = paginate_items(events, offset, limit, "events");
    if !run_id.is_null() {
        page["runId"] = run_id;
    }
    if !projection.is_null() {
        page["projection"] = projection;
    }
    if !truncated.is_null() {
        page["truncated"] = truncated;
    }
    page
}

/// Bound presentation payloads without changing the stored record.
pub(crate) fn truncate_message_content(message: &mut Value, cap: usize) {
    if serialized_len(message) <= cap {
        return;
    }
    let original_bytes = serialized_len(message);
    let mut content_truncated = false;
    if let Some(blocks) = message.get_mut("blocks").and_then(Value::as_array_mut) {
        let mut remaining = cap;
        for block in blocks {
            if let Some(Value::String(text)) = block.get_mut("text") {
                let (end, truncated) = byte_cut(text, remaining);
                if truncated {
                    let mut cut = text[..end].to_owned();
                    cut.push('…');
                    *text = cut;
                    content_truncated = true;
                }
                remaining = remaining.saturating_sub(text.len());
            }
        }
    } else if let Some(Value::String(data)) = message.get_mut("data") {
        if data.len() > cap {
            *data = json!({"_truncated":true,"bytes":data.len()}).to_string();
            content_truncated = true;
        }
    }
    if content_truncated {
        let metadata = message.as_object_mut().and_then(|object| {
            object
                .entry("metadata")
                .or_insert_with(|| json!({}))
                .as_object_mut()
        });
        if let Some(metadata) = metadata {
            metadata.insert("remoteTruncated".into(), Value::Bool(true));
            metadata.insert("originalBytes".into(), json!(original_bytes));
        }
    }
}

pub(crate) fn cap_remote_item(item: &mut Value, cap: usize) {
    truncate_message_content(item, cap.saturating_sub(16 * 1024));
    if serialized_len(item) <= cap {
        return;
    }
    if let Some(blocks) = item.get_mut("blocks").and_then(Value::as_array_mut) {
        for block in blocks {
            if let Some(arguments) = block.get_mut("arguments") {
                let bytes = serialized_len(arguments);
                if bytes > 8 * 1024 {
                    *arguments = json!({"truncated":true,"bytes":bytes});
                }
            }
        }
    }
    if serialized_len(item) <= cap {
        return;
    }
    let original_bytes = serialized_len(item);
    if item.get("blocks").is_some() {
        let mut replacement = serde_json::Map::new();
        for key in ["id", "role", "kind", "runId", "createdAtMs", "usage", "run"] {
            if let Some(value) = item.get(key) {
                replacement.insert(key.into(), value.clone());
            }
        }
        if let Some(checkpoint) = item.get("checkpoint").and_then(Value::as_object) {
            let minimal: serde_json::Map<String, Value> =
                ["checkpointId", "tokensBefore", "tokensAfter", "trigger"]
                    .into_iter()
                    .filter_map(|key| checkpoint.get(key).map(|value| (key.into(), value.clone())))
                    .collect();
            replacement.insert("checkpoint".into(), Value::Object(minimal));
        }
        replacement.insert(
            "metadata".into(),
            json!({"remoteTruncated":true,"originalBytes":original_bytes}),
        );
        replacement.insert(
            "blocks".into(),
            json!([{"kind":"text","text":"[…远程条目过大，已截断；完整内容见本机会话…]"}]),
        );
        *item = Value::Object(replacement);
    } else if let Some(object) = item.as_object_mut() {
        object.insert(
            "data".into(),
            Value::String(json!({"_truncated":true,"bytes":original_bytes}).to_string()),
        );
    }
}

pub(crate) fn serialized_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(0, |bytes| bytes.len())
}

/// Return a byte index at a char boundary, not exceeding `max_bytes`, and
/// whether the string had to be cut.
pub(crate) fn byte_cut(text: &str, max_bytes: usize) -> (usize, bool) {
    if text.len() <= max_bytes {
        return (text.len(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (end, true)
}
