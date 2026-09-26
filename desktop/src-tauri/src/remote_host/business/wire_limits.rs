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

    /// A single oversized *entry* — not page — is the case `cap_remote_item`
    /// exists for. Every stage has to converge on something smaller than the
    /// cap: first the text blocks, then the tool arguments, and only then a
    /// whole-item replacement that keeps the identity a client needs to render
    /// the row (id, role, kind, run, timestamps).
    #[test]
    fn an_oversized_entry_is_reduced_in_stages_until_it_fits() {
        // Tool arguments big enough to be replaced on their own.
        let mut item = json!({
            "id": "e1", "role": "assistant", "kind": "assistant",
            "runId": "r", "createdAtMs": 1, "usage": {"in": 1},
            "run": {"status": "completed"},
            "checkpoint": {
                "checkpointId": "cp1", "tokensBefore": 10, "tokensAfter": 20,
                "trigger": "auto", "unrelated": "y".repeat(50_000)
            },
            "blocks": [
                {"kind": "tool", "arguments": {"blob": "x".repeat(40_000)}},
                {"kind": "text", "text": "z".repeat(40_000)},
            ],
        });
        cap_remote_item(&mut item, 4096);
        assert!(serialized_len(&item) <= 4096, "{item}");
        assert_eq!(item["id"], json!("e1"), "identity survives the cap");
        assert_eq!(item["role"], json!("assistant"));
        assert_eq!(item["runId"], json!("r"));
        assert_eq!(item["createdAtMs"], json!(1));
        assert_eq!(item["usage"], json!({"in": 1}));
        assert_eq!(item["run"], json!({"status": "completed"}));
        assert_eq!(item["metadata"]["remoteTruncated"], json!(true));
        // The checkpoint keeps only what a merged-context row needs, and the
        // unrelated bulk is gone.
        assert_eq!(item["checkpoint"]["checkpointId"], json!("cp1"));
        assert_eq!(item["checkpoint"]["tokensBefore"], json!(10));
        assert_eq!(item["checkpoint"]["tokensAfter"], json!(20));
        assert_eq!(item["checkpoint"]["trigger"], json!("auto"));
        assert!(item["checkpoint"].get("unrelated").is_none());
    }

    /// An event (no `blocks`) is capped by rewriting its `data`, and an item
    /// with no `data` at all must still come back with a `data` *string*: the
    /// phone decodes that field, so an object where it expects JSON text would
    /// be unreadable. The relay budget is a bound on the *envelope*, and the
    /// replacement deliberately keeps a tail of the text (released clients read
    /// their `[exit: N]` footer there) — so the assertion is that the row shrinks
    /// by an order of magnitude and stays decodable, not that it fits one cap.
    #[test]
    fn an_oversized_event_has_its_data_cut_or_marked_missing() {
        // Large enough that one truncation pass runs, like the real relay
        // budget. A cap below the event size is what makes the pipeline rewrite
        // the payload rather than pass it through.
        const CAP: usize = 64 * 1024;
        let tail = "-[exit: 3]";
        let body = format!("{}{tail}", "x".repeat(120 * 1024));
        let original_data = json!({"text": body, "exit_code": 3, "tool_id": "c1"}).to_string();
        let mut with_data = json!({
            "id": "ev1",
            "type": "tool_end",
            "data": original_data.clone(),
        });
        let before = serialized_len(&with_data);
        cap_remote_item(&mut with_data, CAP);
        let after = serialized_len(&with_data);
        assert!(
            after * 4 < before,
            "the row must shrink, not merely re-serialise: {before} -> {after}"
        );
        assert_eq!(with_data["metadata"]["remoteTruncated"], json!(true));
        assert!(
            with_data["metadata"]["originalBytes"].as_u64().unwrap() as usize >= before,
            "the original size is recorded for the UI: {with_data}"
        );
        let data = with_data["data"].as_str().expect("data stays a string");
        let parsed: Value = serde_json::from_str(data).expect("data stays decodable JSON");
        assert_eq!(parsed["_truncated"], json!(true));
        assert_eq!(
            parsed["bytes"].as_u64().unwrap() as usize,
            original_data.len(),
            "bytes reports what was cut, so the client can tell how much it lost"
        );
        assert!(
            parsed["note"].as_str().unwrap().contains("get_messages"),
            "the marker must say where the full content still is: {parsed}"
        );
        assert_eq!(
            parsed["exit_code"],
            json!(3),
            "the outcome survives the cut"
        );
        assert_eq!(parsed["tool_id"], json!("c1"));
        assert!(
            parsed["text"].as_str().unwrap().ends_with(tail),
            "the tail is what a released client reads its exit footer from: {parsed}"
        );

        // No `data` at all: an event without a payload cannot be shrunk by
        // rewriting `data`, so the cap's job here is only to make the truncation
        // *visible*: the row gains a decodable marker recording the original size
        // instead of reaching the phone looking complete. (The oversized non-
        // payload field is not this function's to drop.)
        let mut without_data =
            json!({"id": "ev2", "type": "usage", "padding": "y".repeat(120 * 1024)});
        let before = serialized_len(&without_data);
        cap_remote_item(&mut without_data, CAP);
        let data: Value =
            serde_json::from_str(without_data["data"].as_str().expect("data is a string"))
                .expect("data is decodable JSON");
        assert_eq!(data["_truncated"], json!(true));
        assert!(
            data["bytes"].as_u64().unwrap() as usize >= before,
            "the marker records the size that was not delivered: {data}"
        );
    }

    /// Tool arguments reach the desktop in either wire shape: a JSON string
    /// (the older providers) or an object. Both must be reduced to the file
    /// target, and an entry that is already inside the cap must be returned
    /// untouched so a small row is never rewritten.
    #[test]
    fn oversized_tool_arguments_are_reduced_to_their_target() {
        let big = "x".repeat(40_000);
        for tool_args in [
            json!(json!({"command": "/bin/sh -c 'echo hi'", "content": big}).to_string()),
            json!({"path": "/tmp/a.txt", "content": big}),
        ] {
            let data = json!({
                "type": "tool_start",
                "tool_id": "c1",
                "tool_args": tool_args,
                "extra": big,
            })
            .to_string();
            let truncated = truncated_event_data(&data);
            let parsed: Value = serde_json::from_str(&truncated).unwrap();
            assert_eq!(parsed["tool_id"], json!("c1"));
            let target = if parsed["tool_args"].get("command").is_some() {
                "command"
            } else {
                "path"
            };
            assert!(
                parsed["tool_args"][target].is_string(),
                "the row's target survives: {parsed}"
            );
            assert!(
                parsed["tool_args"].get("content").is_none(),
                "the bulk is not forwarded twice: {parsed}"
            );
            assert!(truncated.len() < data.len());
        }

        // Inside the cap the row is returned byte-for-byte unchanged, so a small
        // row is never rewritten: this is the no-op contract of the cap, and it
        // is what keeps the marker (`_truncated`) meaningful — see
        // `an_oversized_event_has_its_data_cut_or_marked_missing` for the case
        // where the payload is over the budget instead.
        let mut item = json!({"id": "small", "blocks": [{"kind": "text", "text": "hi"}]});
        let before = item.clone();
        cap_remote_item(&mut item, 64 * 1024);
        assert_eq!(item, before, "a row inside the budget is untouched");
    }

    /// A tool call whose arguments are not a JSON object is not a write the
    /// desktop can reduce, so its arguments are replaced wholesale — but the
    /// `arguments` key must remain present, because the client branches on it
    /// to render the row as a tool call at all.
    #[test]
    fn unparsable_tool_arguments_are_marked_not_dropped() {
        let data = json!({
            "type": "tool_end",
            "tool_args": "this is not JSON at all, just a long string".repeat(500),
        })
        .to_string();
        let truncated = truncated_event_data(&data);
        let parsed: Value = serde_json::from_str(&truncated).unwrap();
        assert!(parsed.get("tool_args").is_some());
        // The rebuilt value is always an object, and it never carries more than
        // the write target: an unparsable input leaves it empty (nothing was
        // recoverable), a parseable one keeps the target — nothing else about
        // the call is forwarded a second time.
        assert_eq!(
            argument_keys(&parsed),
            Vec::<String>::new(),
            "a non-JSON argument string recovers no target: {parsed}"
        );
    }

    /// The keys of a rebuilt `tool_args`, sorted so a test compares the whole
    /// shape rather than a predicate whose body may never run.
    fn argument_keys(parsed: &Value) -> Vec<String> {
        let mut keys: Vec<String> = parsed["tool_args"]
            .as_object()
            .expect("tool_args stays an object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    /// A parseable write keeps the one thing the phone needs to render the row
    /// — its target — and drops the content it already has locally. This is the
    /// other half of the shape assertion above: the rebuilt object is small, not
    /// empty.
    #[test]
    fn a_parseable_tool_call_keeps_its_target_and_drops_its_content() {
        let data = json!({
            "type": "tool_end",
            "tool_args": {"content": "y".repeat(60_000), "path": "/tmp/big.txt"},
        })
        .to_string();
        let truncated = truncated_event_data(&data);
        let parsed: Value = serde_json::from_str(&truncated).unwrap();
        // `path` alone: the 60 KB of `content` is not forwarded a second time.
        assert_eq!(
            argument_keys(&parsed),
            vec!["path".to_string()],
            "only the write target survives: {parsed}"
        );
        assert_eq!(parsed["tool_args"]["path"], json!("/tmp/big.txt"));
        assert!(
            truncated.len() < data.len() / 10,
            "reducing a 60 KB write must actually shrink it"
        );
    }

    /// The arguments pass is the *second* stage: a row whose only bulk is tool
    /// arguments is still over the cap after the text pass, and the arguments
    /// pass is what brings it under. When that happens the row keeps its real
    /// identity (id/role/run) — the whole-row replacement below is for rows that
    /// even that cannot fit, and using it here would throw away the identity a
    /// client needs to match the row against its own tool call.
    #[test]
    fn an_item_that_fits_once_its_tool_arguments_are_reduced_keeps_its_identity() {
        let mut item = json!({
            "id": "tool-row", "role": "assistant", "kind": "assistant", "runId": "r",
            "createdAtMs": 1,
            "blocks": [{"kind": "tool", "arguments": {"blob": "x".repeat(40_000)}}],
        });
        assert!(serialized_len(&item) > 8 * 1024);
        cap_remote_item(&mut item, 8 * 1024);
        assert!(serialized_len(&item) <= 8 * 1024, "{item}");
        assert_eq!(item["id"], json!("tool-row"));
        assert_eq!(item["role"], json!("assistant"));
        assert_eq!(item["runId"], json!("r"));
        assert_eq!(item["createdAtMs"], json!(1));
        assert_eq!(item["blocks"][0]["arguments"]["truncated"], json!(true));
        assert!(
            item.get("metadata").is_none(),
            "no whole-row replacement was needed: {item}"
        );
    }

    /// The relay budget applies to whatever the Agent sent. A malformed row that
    /// is not an item object at all (`blocks` absent, no object form) has nothing
    /// the cap can rewrite, so it must come back byte-for-byte rather than being
    /// replaced by an envelope with invented fields.
    #[test]
    fn a_value_that_is_not_an_item_object_has_nothing_to_rewrite() {
        let mut item = json!((0..500).collect::<Vec<u32>>());
        let before = item.clone();
        assert!(serialized_len(&item) > 1024);
        cap_remote_item(&mut item, 1024);
        assert_eq!(
            item, before,
            "nothing to rewrite means nothing is rewritten"
        );
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
        let build = |lean: bool, enforce: bool| {
            let mut data = json!({ "entries": entries.clone() });
            if lean {
                if let Some(list) = data.get_mut("entries") {
                    crate::remote_host::lean::lean_entries(list);
                }
            }
            let page = prepare_backward_entries_page_with_cap(
                "measure", data, /* cap_item_content */ false, enforce,
            );
            let wire =
                sized(&page) + future_remote_crypto::HEADER_LEN + future_remote_crypto::TAG_LEN;
            (page["entries"].as_array().map(Vec::len).unwrap_or(0), wire)
        };

        // Two different effects, so measure them apart.
        //
        // Without the budget, both pages hold the requested entries and the
        // trim's own claim holds exactly: it only ever removes bytes.
        let (_, plain_wire) = build(false, false);
        let (_, trimmed_wire) = build(true, false);
        assert!(
            trimmed_wire <= plain_wire,
            "the trim may only shrink a page's bytes for the same entries \
             ({plain_wire} vs {trimmed_wire})"
        );

        // With the budget, the *entry* counts can differ, because the budget
        // sheds whole oldest exchanges until the page fits. A trim that brings
        // the page under the budget delivers it whole, while the undeclared page
        // is cut down — so comparing raw bytes across the two is meaningless:
        // the lean page can be larger precisely because it still holds the
        // exchanges the undeclared page had to drop. Measured on a real session
        // page: undeclared kept 41 of 769 entries in 76 KB, lean kept all 769 in
        // 276 KB. The invariant is therefore on the entry count, not the bytes.
        let (full_entries, full_wire) = build(false, true);
        let (lean_count, lean_wire) = build(true, true);
        assert!(
            lean_count >= full_entries,
            "the lean page must not deliver fewer entries than the undeclared one \
             ({full_entries} vs {lean_count})"
        );

        let source = entries.as_array().map(Vec::len).unwrap_or(0);
        // The wire size a phone actually pays: the reply body the command loop
        // builds, run through the shipping encoder with the phone's own
        // declaration (`reply_gzip_v1`). `wireBytes` above counts plain JSON +
        // crypto overhead, which is what an *undeclared* client pays on small
        // pages; a page at or above 32 KiB goes out gzipped.
        let encode = |lean: bool, enforce: bool| {
            let mut data = json!({ "entries": entries.clone() });
            if lean {
                if let Some(list) = data.get_mut("entries") {
                    crate::remote_host::lean::lean_entries(list);
                }
            }
            let page = prepare_backward_entries_page_with_cap(
                "measure", data, /* cap_item_content */ false, enforce,
            );
            let body = json!({ "type": "response", "success": true, "data": page, "error": null });
            let plain = crate::remote::commands::encode_reply_payload_with_gzip(
                &body, /* gzip */ false,
            );
            let gzipped = crate::remote::commands::encode_reply_payload_with_gzip(
                &body, /* gzip */ true,
            );
            // Over the decoded-reply limit the encoder answers with an error
            // body, not the page — so a size read off it would be nonsense. That
            // is reachable only without the budget (an untrimmed 3-exchange page
            // can exceed 1 MiB), which no client configuration asks for; it is
            // measured to value the trim, and is reported as unavailable here.
            let over_limit = serde_json::to_vec(&body)
                .map(|raw| raw.len() > future_remote_crypto::MAX_PLAINTEXT)
                .unwrap_or(true);
            (plain.len(), gzipped.len(), over_limit)
        };
        let (lean_plain, lean_gzip, _) = encode(true, true);
        // The trim's own value *after* compression, on the same entries: what it
        // removes (tool output, reasoning, commands, file bodies) is the part
        // that compresses worst, so its post-gzip share is the honest one.
        let (_, untrimmed_gzip, untrimmed_over) = encode(false, false);
        let (_, trimmed_gzip, _) = encode(true, false);
        let saved_gzip =
            (!untrimmed_over).then(|| 1.0 - (trimmed_gzip as f64 / untrimmed_gzip.max(1) as f64));

        println!(
            "VERIFY_E2E_PHONE_PAGE {}",
            json!({
                "sourceEntries": source,
                "plainBytes": plain_wire,
                "trimmedBytes": trimmed_wire,
                "saved": 1.0 - (trimmed_wire as f64 / plain_wire.max(1) as f64),
                "untrimmedGzip": untrimmed_gzip,
                "trimmedGzip": trimmed_gzip,
                "savedGzip": saved_gzip,
                "leanReplyPlain": lean_plain,
                "leanReplyGzip": lean_gzip,
                "undeclared": { "entries": full_entries, "wireBytes": full_wire },
                "declared": { "entries": lean_count, "wireBytes": lean_wire },
                "declaredWhole": lean_count == source,
            })
        );
    }
}
