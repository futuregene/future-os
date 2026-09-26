use serde_json::{json, Value};

/// Reply budget for a `get_messages` page: comfortably under NATS's 1MB
/// user-JWT payload limit, leaving headroom for the reply envelope.
pub(crate) const MESSAGES_PAGE_BYTES: usize = 512 * 1024;
/// Backward mobile history keeps the requested page of user exchanges, with the
/// same 512 KiB wire budget as other remote history pages. Complete oldest
/// exchanges are deferred only when an unusually content-heavy page would exceed
/// that budget; a page never splits an exchange. (The phone asks for
/// `HISTORY_PAGE_USER_EXCHANGES` = 3 of them; this comment used to say "ten",
/// which was true before #607 lowered it.)
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
///
/// `cap_item_content` truncates a single oversized body: only the non-chunked
/// path needs that, because it must fit one reply. `enforce_page_bytes` drops
/// whole oldest exchanges until the page fits [`BACKWARD_HISTORY_PAGE_BYTES`],
/// which is what keeps a page inside one reply at all — a chunked reader is the
/// only caller that may turn it off, and only for a page nobody is waiting for.
#[cfg(test)]
pub(crate) fn prepare_backward_entries_page(_session_id: &str, data: Value) -> Value {
    prepare_backward_entries_page_with_cap(_session_id, data, true, true)
}

pub(crate) fn prepare_backward_entries_page_with_cap(
    _session_id: &str,
    data: Value,
    cap_item_content: bool,
    enforce_page_bytes: bool,
) -> Value {
    let mut entries = entries_vec(data.clone());
    if cap_item_content {
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
    while enforce_page_bytes
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
        // The page shed whole oldest exchanges to fit the byte budget. A
        // chunked reader (mobile) uses this to backfill the missing exchanges
        // instead of showing a window that silently starts mid-history.
        "trimmed": removed > 0,
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

/// Shrink an oversized event without losing the identity/status needed to close
/// a tool row. Live publishing and replay must use the same representation: a
/// bare truncation marker consumes the event cursor but leaves the tool running.
pub(crate) fn truncated_event_data(data: &str) -> String {
    let mut truncated = json!({
        "_truncated": true,
        "bytes": data.len(),
        "note": "event exceeded the relay payload limit and was truncated; full content is available via get_messages",
    });
    if let Ok(Value::Object(payload)) = serde_json::from_str::<Value>(data) {
        // Keep only bounded protocol metadata, never arbitrary large objects.
        for key in [
            "type",
            "phase",
            "tool_id",
            "toolID",
            "tool_call_id",
            "tool_name",
            "toolName",
            "name",
            "exit_code",
            "exitCode",
            "is_error",
            "isError",
            "status",
            "tc_index",
        ] {
            if let Some(value) = payload.get(key) {
                if (value.is_string() || value.is_number() || value.is_boolean())
                    && serialized_len(value) <= 1024
                {
                    truncated[key] = value.clone();
                }
            }
        }
        // Preserve output tails, including the [exit: N] footer read by released
        // mobile clients. Error text must remain nonempty even if it ends in a
        // large amount of whitespace. Bound UTF-8 before JSON serialization.
        for key in ["text", "result", "error", "errorText"] {
            if let Some(text) = payload.get(key).and_then(Value::as_str) {
                let text = if matches!(key, "error" | "errorText") {
                    text.trim()
                } else {
                    text.trim_end()
                };
                let mut start = text.len().saturating_sub(4096);
                while !text.is_char_boundary(start) {
                    start += 1;
                }
                truncated[key] = json!(if start > 0 {
                    format!("…{}", &text[start..])
                } else {
                    text.to_owned()
                });
            }
        }
        // Large write inputs still need their file target; don't forward the
        // content again. Accept both object and JSON-string argument formats.
        for key in ["tool_args", "toolArgs", "arguments"] {
            if let Some(args) = payload.get(key) {
                let parsed;
                let args = if let Some(text) = args.as_str() {
                    parsed = serde_json::from_str::<Value>(text).unwrap_or(Value::Null);
                    &parsed
                } else {
                    args
                };
                let mut targets = serde_json::Map::new();
                for target in ["command", "path", "file_path", "filePath"] {
                    if let Some(value) = args.get(target).filter(|v| v.is_string()) {
                        if serialized_len(value) <= 4096 {
                            targets.insert(target.into(), value.clone());
                        }
                    }
                }
                truncated[key] = Value::Object(targets);
            }
        }
    }
    truncated.to_string()
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
            *data = truncated_event_data(data);
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
        let data = object
            .get("data")
            .and_then(Value::as_str)
            .map(truncated_event_data)
            .unwrap_or_else(|| json!({"_truncated":true,"bytes":original_bytes}).to_string());
        object.insert("data".into(), Value::String(data));
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Fixture: `exchanges` user/assistant pairs whose assistant body is
    /// `bytes` long. Backward pages page in *exchanges*, never inside one.
    fn exchanges(count: usize, bytes: usize) -> Value {
        let entries: Vec<Value> = (0..count)
            .flat_map(|index| {
                [
                    json!({"id": format!("u{index}"), "role": "user",
                           "blocks": [{"kind": "text", "text": "q"}]}),
                    json!({"id": format!("a{index}"), "role": "assistant",
                           "blocks": [{"kind": "text", "text": "x".repeat(bytes)}]}),
                ]
            })
            .collect();
        json!({"entries": entries, "nextOffset": 100, "hasMore": true})
    }

    fn size(page: &Value) -> usize {
        serde_json::to_vec(&page["entries"]).unwrap().len()
    }

    #[test]
    fn the_byte_budget_is_what_keeps_a_page_inside_one_reply() {
        let source = exchanges(6, 150_000);
        let bounded = prepare_backward_entries_page_with_cap("s", source.clone(), true, true);
        let unbounded = prepare_backward_entries_page_with_cap("s", source, true, false);
        assert!(
            size(&bounded) <= BACKWARD_HISTORY_PAGE_BYTES,
            "a page that pays the budget must fit it"
        );
        // This is the case a non-chunked client cannot survive: the entries are
        // capped individually (`cap_remote_item`) and the page is still larger
        // than the reply it has to fit in, because the cap bounds one *item*,
        // not the page.
        assert!(
            size(&unbounded) > BACKWARD_HISTORY_PAGE_BYTES,
            "without the budget the page is unbounded"
        );
        // Paying the budget defers whole exchanges and keeps them reachable: the
        // page still starts at a user turn, so no user/assistant pair is split,
        // and the advanced cursor still points at the first deferred row.
        let kept = bounded["entries"].as_array().unwrap();
        assert!(kept.len() < 12);
        assert_eq!(kept[0]["role"], "user");
        assert_eq!(kept.len() % 2, 0);
        assert_eq!(bounded["nextOffset"], 100 + 12 - kept.len());
        assert_eq!(bounded["hasMore"], true);
    }
}

#[cfg(test)]
mod measure_phone_page_tests {
    use super::*;
    use serde_json::{json, Value};

    /// Measure a *phone page* through the shipping trim and page budget.
    ///
    /// The whole-session measurements size payloads a phone never reads: the
    /// phone asks the agent for the newest `HISTORY_PAGE_USER_EXCHANGES`
    /// exchanges (`before` + `limit`, agent-side paging) and only then meets the
    /// bridge's trim and 512 KiB budget. The budget sheds whole oldest exchanges,
    /// so applying it to a whole session yields "as many newest exchanges as fit
    /// in 512 KiB" — which equals the phone's page only when that page already
    /// exceeds the budget, and overstates it when the exchanges are small.
    ///
    /// Driven by `scripts/measure/measure-lean-history.py --phone-page`, which
    /// dumps exactly that page. Trim and budget here are the shipping ones.
    #[test]
    #[ignore = "measurement: needs VERIFY_E2E_PHONE_PAGE"]
    fn measure_phone_page() {
        let path = std::env::var("VERIFY_E2E_PHONE_PAGE").expect("VERIFY_E2E_PHONE_PAGE");
        let raw = std::fs::read_to_string(path).expect("phone page readable");
        let entries: Value = serde_json::from_str(&raw).expect("phone page json");
        let sized = |value: &Value| serde_json::to_vec(value).expect("serializes").len();

        // The phone is a chunked reader (it reassembles `readChunk`s) and reads
        // the newest page, so it does not pay the per-item content cap but does
        // pay the byte budget.
        let build = |lean: bool| {
            let mut data = json!({ "entries": entries.clone() });
            if lean {
                if let Some(list) = data.get_mut("entries") {
                    crate::remote_host::lean::lean_entries(list);
                }
            }
            let page = prepare_backward_entries_page_with_cap(
                "measure", data, /* cap_item_content */ false, /* enforce */ true,
            );
            let wire =
                sized(&page) + future_remote_crypto::HEADER_LEN + future_remote_crypto::TAG_LEN;
            (page["entries"].as_array().map(Vec::len).unwrap_or(0), wire)
        };

        let (full_entries, full_wire) = build(false);
        let (lean_count, lean_wire) = build(true);
        let source = entries.as_array().map(Vec::len).unwrap_or(0);
        // The byte claim holds entry-wise: the trim only ever removes bytes. The
        // *entry* count may grow, because the budget behind it sheds whole oldest
        // exchanges until the page fits — so a leaner page keeps more of them.
        assert!(
            lean_wire <= full_wire,
            "the trim may only shrink the bytes of a page ({full_wire} vs {lean_wire})"
        );
        println!(
            "VERIFY_E2E_PHONE_PAGE {}",
            json!({
                "sourceEntries": source,
                "undeclared": { "entries": full_entries, "wireBytes": full_wire },
                "declared": { "entries": lean_count, "wireBytes": lean_wire },
                "saved": 1.0 - (lean_wire as f64 / full_wire.max(1) as f64),
            })
        );
    }
}
