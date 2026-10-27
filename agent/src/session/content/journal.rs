//! Ordered stream encoder. Consecutive live fragments become one semantic
//! range; the durable representation never retains their token envelopes.
use super::{
    entries::stage_named, stream::*, ContentDigest, ContentRange, FORMAT_VERSION, READ_BYTES,
    WRITE_BYTES,
};
use crate::session::sqlite_store::SqliteStore;
use anyhow::{ensure, Context, Result};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Default)]
pub(crate) struct Writer {
    active: HashMap<(String, i64, String), ContentDigest>,
}

struct Group {
    stream: StreamIdentity,
    appends: Vec<ContentAppend>,
    change: Change,
}

// Delta clocks denote the last fragment of a coalesced range. All other
// metadata must match; otherwise collapsing it would discard a semantic fact.
fn same_delta_metadata(left: &Value, right: &Value) -> bool {
    fn object_eq(left: &Value, right: &Value, skip: &[&str], data: bool) -> bool {
        let (Some(left), Some(right)) = (left.as_object(), right.as_object()) else {
            return left == right;
        };
        let keys = left
            .keys()
            .chain(right.keys())
            .filter(|key| !skip.contains(&key.as_str()));
        keys.into_iter()
            .all(|key| match (left.get(key), right.get(key)) {
                (Some(a), Some(b)) if data && key == "data" => object_eq(
                    a,
                    b,
                    &["content_offset", "content_bytes", "content_first_idx"],
                    false,
                ),
                (a, b) => a == b,
            })
    }
    object_eq(left, right, &["timestamp"], true)
}

impl Writer {
    /// A failed attempt retains the caller's immutable batch. Rolling hashes
    /// advance only after all commits succeed; partial commits retry verbatim.
    pub(crate) fn persist<'a>(
        &mut self,
        store: &SqliteStore,
        session: &str,
        events: impl IntoIterator<Item = &'a Value>,
    ) -> Result<()> {
        ensure!(
            store.storage_format(session)? == FORMAT_VERSION,
            "wrong journal format"
        );
        let mut next = self.clone();
        let mut group: Option<Group> = None;
        for event in events {
            if event["event_type"] == "message_end"
                || event["data"]["content_seal"] == true
                || event["data"].as_str().is_some_and(|data| {
                    data.len() > WRITE_BYTES || data.contains("\"content_seal\":true")
                })
            {
                if let Some(prior) = group.take() {
                    commit(store, prior)?;
                }
            }
            let (stream, mut change, appends) = next.encode(store, session, event)?;
            let combine = group.as_ref().is_some_and(|g| {
                g.stream == stream
                    && g.change.event_type == change.event_type
                    && same_delta_metadata(&g.change.metadata, &change.metadata)
                    && g.change.last_cursor + 1 == change.first_cursor
                    && g.change.bodies.len() == 1
                    && change.bodies.len() == 1
                    && g.change.bodies[0].key == change.bodies[0].key
                    && g.change.bodies[0].field == change.bodies[0].field
                    && g.change.bodies[0].offset + g.change.bodies[0].length
                        == change.bodies[0].offset
                    && g.appends.iter().map(|a| a.text.len()).sum::<usize>()
                        + appends.iter().map(|a| a.text.len()).sum::<usize>()
                        <= WRITE_BYTES
                    && matches!(
                        change.event_type.as_str(),
                        "text_chunk" | "thinking_delta" | "tool_delta"
                    )
                    && g.change.metadata["data"]["snapshot"] != true
                    && change.metadata["data"]["snapshot"] != true
            });
            if combine {
                let prior = group.as_mut().expect("checked group");
                let body = &mut prior.change.bodies[0];
                body.length += change.bodies[0].length;
                change.metadata["data"]["content_offset"] = json!(body.offset);
                change.metadata["data"]["content_bytes"] = json!(body.length);
                change.metadata["data"]["content_first_idx"] = json!(prior.change.first_cursor);
                prior.change.metadata = change.metadata;
                prior.change.last_cursor = change.last_cursor;
                prior.appends.extend(appends);
            } else {
                if let Some(prior) = group.take() {
                    commit(store, prior)?;
                }
                group = Some(Group {
                    stream,
                    appends,
                    change,
                });
            }
        }
        if let Some(group) = group {
            commit(store, group)?;
        }
        *self = next;
        Ok(())
    }

    fn encode(
        &mut self,
        store: &SqliteStore,
        session: &str,
        event: &Value,
    ) -> Result<(StreamIdentity, Change, Vec<ContentAppend>)> {
        let run = event["run_id"].as_str().context("missing run identity")?;
        ensure!(
            event["session_id"]
                .as_str()
                .is_none_or(|id| id.is_empty() || id == session),
            "event session mismatch"
        );
        let session_scope = run.is_empty() && event["session_idx"].as_i64().is_some_and(|n| n >= 0);
        let cursor = event[if session_scope { "session_idx" } else { "idx" }]
            .as_i64()
            .filter(|n| *n >= 0)
            .context("invalid event cursor")?;
        let epoch = if session_scope {
            -1
        } else {
            event["epoch"]
                .as_i64()
                .filter(|n| *n >= 0)
                .context("invalid event epoch")?
        };
        let stream = StreamIdentity {
            session_id: session.to_owned(),
            run_id: run.to_owned(),
            epoch,
        };
        let event_type = event["event_type"]
            .as_str()
            .context("missing event type")?
            .to_owned();
        let fields = event["content_fields"]
            .as_array()
            .map(|fields| {
                fields
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<std::collections::HashSet<_>>()
            })
            .unwrap_or_default();
        let mut metadata = event.clone();
        let data: Value = match metadata["data"].take() {
            Value::String(raw) => serde_json::from_str(&raw)?,
            data @ Value::Object(_) => data,
            _ => anyhow::bail!("invalid event data"),
        };
        let object = metadata.as_object_mut().context("invalid event envelope")?;
        for field in [
            "session_id",
            "run_id",
            "epoch",
            "idx",
            "session_idx",
            "event_type",
            "content_fields",
        ] {
            object.remove(field);
        }
        let default_id = if session_scope {
            format!("{session}:session:{cursor}")
        } else {
            format!("{session}:{run}:{epoch}:{cursor}")
        };
        if object.get("event_id").and_then(Value::as_str) == Some(&default_id) {
            object.remove("event_id");
        }
        metadata["data"] = data;
        if matches!(event_type.as_str(), "tool_end" | "tool_result") {
            metadata["data"]["is_error"] =
                json!(super::transport::event_is_error(&metadata["data"]));
        }
        let mut bodies = Vec::new();
        let mut appends = Vec::new();
        let key = metadata["data"]["content_key"].as_str().map(str::to_owned);
        if let Some(key) = key {
            let offset = metadata["data"]["content_offset"]
                .as_i64()
                .context("missing content offset")?;
            let state_key = (run.to_owned(), epoch, key.clone());
            ensure!(
                self.active.len() < 2048 || self.active.contains_key(&state_key),
                "too many active canonical blocks"
            );
            let digest = self.active.entry(state_key.clone()).or_default();
            ensure!(
                digest.bytes() == offset,
                "live content has a gap or conflicting retry"
            );
            let field = if event_type == "tool_start" {
                "tool_args"
            } else {
                "text"
            };
            let incremental = matches!(
                event_type.as_str(),
                "text_chunk" | "thinking_delta" | "tool_delta" | "tool_start"
            );
            if incremental {
                let value = &metadata["data"][field];
                let text = if value.is_null() {
                    String::new()
                } else {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string())
                };
                ensure!(
                    metadata["data"]["content_bytes"].as_u64() == Some(text.len() as u64),
                    "content length mismatch"
                );
                digest.push(&text)?;
                let encoding = if value.is_string() {
                    Encoding::Text
                } else {
                    Encoding::Json
                };
                if !text.is_empty() {
                    // Stage an oversized provider fragment before publishing
                    // its cursor, using the same bounded immutable segments.
                    let mut start = 0;
                    while start < text.len() {
                        let mut end = (start + WRITE_BYTES).min(text.len());
                        while !text.is_char_boundary(end) {
                            end -= 1;
                        }
                        let piece = ContentAppend {
                            key: key.clone(),
                            offset: offset + start as i64,
                            text: text[start..end].to_owned(),
                            seal: None,
                        };
                        if text.len() > WRITE_BYTES {
                            let scope = stream.clone();
                            store.db.call(move |db| {
                                commit_batch(db, &scope, vec![piece], Vec::new())
                            })?;
                        } else {
                            appends.push(piece);
                        }
                        start = end;
                    }
                    bodies.push(BodyLink {
                        field: format!("/data/{field}"),
                        encoding,
                        key: key.clone(),
                        offset,
                        length: text.len() as i64,
                    });
                    metadata["data"][field] = json!("");
                }
            }
            if metadata["data"]["content_seal"] == true {
                let hash = self.active.remove(&state_key).expect("active block");
                if event_type == "tool_input_end" && hash.bytes() > 0 {
                    let scope = stream.clone();
                    let named = owner(&stream, &key);
                    let range = store
                        .db
                        .read(move |db| {
                            super::resolve(db, &scope.session_id, "stream", &named, "")
                        })?
                        .context("tool argument stream is missing")?;
                    ensure!(
                        range.length == hash.bytes(),
                        "tool input boundary precedes its body"
                    );
                    let raw = read_range(store, range)?;
                    if serde_json::from_str::<Value>(&raw).ok().as_ref()
                        == Some(&metadata["data"]["tool_args"])
                    {
                        let verified = super::JsonContentDigest::verified(
                            &raw,
                            &metadata["data"]["tool_args"],
                        )?;
                        store.db.call(move |db| {
                            let tx = crate::session::database::begin_write_transaction(db)?;
                            super::seal_json(&tx, range.id, verified)?;
                            tx.commit()?;
                            Ok(())
                        })?;
                    }
                }
                appends.push(ContentAppend {
                    key,
                    offset: hash.bytes(),
                    text: String::new(),
                    seal: Some(hash),
                });
            }
        }
        if event_type == "message_end" {
            let id = metadata["data"]["entry_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .context("message boundary has no identity")?;
            let prefix = format!("{id}:");
            let keys = self
                .active
                .keys()
                .filter(|(r, e, key)| r == run && *e == epoch && key.starts_with(&prefix))
                .cloned()
                .collect::<Vec<_>>();
            for key in keys {
                let digest = self.active.remove(&key).expect("collected active block");
                appends.push(ContentAppend {
                    key: key.2,
                    offset: digest.bytes(),
                    text: String::new(),
                    seal: Some(digest),
                });
            }
        }
        // Semantic facts are owned once and can be reused by final entries.
        // Unknown provider payloads stay lossless; a large object is one JSON
        // body rather than an unbounded metadata row.
        if let Ok(facts) = serde_json::from_value::<future_rpc::shell_result::ShellResult>(
            metadata["data"]["shell_result"].clone(),
        ) {
            let key = format!("fact:{cursor}:shell_result");
            let range = stage_named(
                store,
                session,
                "stream",
                &owner(&stream, &key),
                "",
                &metadata["data"]["shell_result"].to_string(),
                true,
            )?;
            bodies.push(BodyLink {
                field: "/data/shell_result".into(),
                encoding: Encoding::Json,
                key: key.clone(),
                offset: 0,
                length: range.length,
            });
            metadata["data"]["shell_result"] = json!("");
            if metadata["data"]["text"].as_str()
                == Some(&future_rpc::shell_result::model_summary(&facts))
            {
                bodies.push(BodyLink {
                    field: "/data/text".into(),
                    encoding: Encoding::ShellSummary,
                    key,
                    offset: 0,
                    length: range.length,
                });
                metadata["data"]["text"] = json!("");
            }
        }
        let wire_fields = fields;
        let fields: Vec<String> = metadata["data"]
            .as_object()
            .into_iter()
            .flat_map(|o| o.keys())
            .filter(|field| {
                matches!(
                    field.as_str(),
                    "text" | "output" | "tool_args" | "shell_result" | "summary"
                ) || wire_fields.contains(*field)
                    || metadata["data"][*field].to_string().len() > 4096
            })
            .cloned()
            .collect();
        for field in fields {
            let pointer = format!("/data/{}", field.replace('~', "~0").replace('/', "~1"));
            if bodies.iter().any(|b| b.field == pointer) {
                continue;
            }
            let value = &metadata["data"][&field];
            if value.is_null() || value.as_str() == Some("") {
                continue;
            }
            let encoding = if value.is_string() {
                Encoding::Text
            } else {
                Encoding::Json
            };
            let text = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            let key = format!("fact:{cursor}:{field}");
            let range = stage_named(
                store,
                session,
                "stream",
                &owner(&stream, &key),
                "",
                &text,
                encoding == Encoding::Json,
            )?;
            bodies.push(BodyLink {
                field: pointer,
                encoding,
                key,
                offset: 0,
                length: range.length,
            });
            metadata["data"][&field] = json!("");
        }
        Ok((
            stream,
            Change {
                first_cursor: cursor,
                last_cursor: cursor,
                event_type,
                metadata,
                bodies,
            },
            appends,
        ))
    }
}

fn commit(store: &SqliteStore, mut group: Group) -> Result<()> {
    // Segment rows represent bounded text batches, not provider tokens. Merge
    // adjacent accepted appends before SQLite splits them into 16 KiB pieces.
    let mut packed: Vec<ContentAppend> = Vec::new();
    for piece in group.appends {
        if let Some(last) = packed.last_mut().filter(|last| {
            last.key == piece.key
                && last.seal.is_none()
                && last.offset + last.text.len() as i64 == piece.offset
                && last.text.len() + piece.text.len() <= WRITE_BYTES
        }) {
            last.text.push_str(&piece.text);
            last.seal = piece.seal;
        } else {
            packed.push(piece);
        }
    }
    while packed.len() > CHANGES_PER_BATCH {
        let batch = packed.drain(..CHANGES_PER_BATCH).collect::<Vec<_>>();
        let stream = group.stream.clone();
        store
            .db
            .call(move |db| commit_batch(db, &stream, batch, Vec::new()).map(|_| ()))?;
    }
    group.appends = packed;
    store.db.call(move |db| {
        commit_batch(db, &group.stream, group.appends, vec![group.change]).map(|_| ())
    })
}

/// Range materialization happens off the ordered writer, in bounded read jobs.
/// Object IDs never escape the session-scoped change/reference lookup.
pub(crate) fn materialize(
    store: &SqliteStore,
    stream: &StreamIdentity,
    identity: &future_rpc::content_range::StorageIdentity,
    change: StoredChange,
) -> Result<Value> {
    ensure!(
        change.first_cursor <= change.last_cursor,
        "invalid canonical change range"
    );
    let mut event = change.metadata;
    if event["data"].get("content_key").is_some() {
        event["data"]["content_first_idx"] = json!(change.first_cursor);
    }
    let fragment = matches!(
        change.event_type.as_str(),
        "text_chunk" | "thinking_delta" | "tool_delta"
    );
    let mut inline_bytes = 0usize;
    for (field, range, _) in change.bodies {
        // Fragment ranges are already capped by WRITE_BYTES. Facts can be
        // arbitrarily large and must never be hydrated just to page a replay.
        let size = usize::try_from(range.length)?;
        if !fragment
            && (size > super::transport::INLINE_FIELD_BYTES
                || inline_bytes.saturating_add(size.saturating_mul(6))
                    > super::transport::INLINE_EVENT_BYTES / 2)
        {
            let name = field
                .strip_prefix("/data/")
                .context("invalid event body field")?
                .replace("~1", "/")
                .replace("~0", "~");
            event["data"]["content_refs"][name] = json!({
                "databaseId":identity.database_id,"restoreGeneration":identity.restore_generation,
                "epoch":stream.epoch,"cursor":change.last_cursor,"field":field,"offset":0,"maxBytes":64*1024
            });
            continue;
        }
        // Re-resolve the session/run/field in the SAME transaction as each
        // body read. A concurrent delete must not turn a recycled object ID
        // from the metadata page into authority for unrelated content.
        let page = store.event_content_range(
            &stream.session_id,
            &stream.run_id,
            future_rpc::content_range::ContentRequest {
                database_id: identity.database_id.clone(),
                restore_generation: identity.restore_generation.clone(),
                epoch: stream.epoch,
                cursor: change.last_cursor,
                field: field.clone(),
                offset: 0,
                max_bytes: WRITE_BYTES as i64,
            },
        )?;
        ensure!(
            page["hasMore"] == false,
            "inline event content exceeds its read budget"
        );
        let text = page["text"]
            .as_str()
            .context("invalid inline content page")?;
        inline_bytes = inline_bytes.saturating_add(text.len().saturating_mul(6));
        *event
            .pointer_mut(&field)
            .context("missing semantic content slot")? = if page["encoding"] == "json" {
            serde_json::from_str(text)?
        } else {
            json!(text)
        };
    }
    event["data"]["content_format"] = json!(super::FORMAT_VERSION);
    let scoped = stream.run_id.is_empty() && stream.epoch == -1;
    event["session_id"] = json!(stream.session_id);
    event["run_id"] = json!(stream.run_id);
    event["epoch"] = json!(if scoped { 0 } else { stream.epoch });
    event["idx"] = json!(if scoped { -1 } else { change.last_cursor });
    event["session_idx"] = json!(if scoped { change.last_cursor } else { -1 });
    event["event_type"] = json!(change.event_type);
    if event.get("event_id").is_none() {
        event["event_id"] = json!(if scoped {
            format!("{}:session:{}", stream.session_id, change.last_cursor)
        } else {
            format!(
                "{}:{}:{}:{}",
                stream.session_id, stream.run_id, stream.epoch, change.last_cursor
            )
        });
    }
    event["data"] = Value::String(serde_json::to_string(&event["data"])?);
    Ok(event)
}

pub(crate) fn read_range(store: &SqliteStore, range: ContentRange) -> Result<String> {
    let mut text = String::new();
    let mut offset = range.offset;
    let end = range
        .offset
        .checked_add(range.length)
        .context("range overflow")?;
    while offset < end {
        let budget = usize::try_from(end - offset)?.min(READ_BYTES);
        let page = store
            .db
            .read(move |db| super::read(db, range.id, offset, budget))?;
        ensure!(!page.text.is_empty(), "canonical range is incomplete");
        offset = page.next_offset;
        text.push_str(&page.text);
    }
    Ok(text)
}

impl SqliteStore {
    /// One exact epoch, bounded by count and wire bytes. Used by atomic
    /// stream attachment so previous epochs can never answer the same cursor.
    pub(crate) fn segmented_stream_page(
        &self,
        stream: &StreamIdentity,
        since: i64,
        limit: usize,
    ) -> Result<Vec<crate::rpc::SseEvent>> {
        let scope = stream.clone();
        let (page, identity) = self.db.read(move |db| {
            let tx = db.transaction()?;
            let identity = super::transport::storage_identity(&tx)?;
            let page = read_encoded_changes(&tx, &scope, since, limit)?;
            tx.commit()?;
            Ok((page, identity))
        })?;
        let page = page.decode()?;
        let mut events = Vec::new();
        let mut bytes = 0usize;
        for change in page.changes {
            ensure!(
                change.last_cursor <= page.durable_cursor,
                "change exceeds durable watermark"
            );
            let event: crate::rpc::SseEvent =
                serde_json::from_value(materialize(self, stream, &identity, change)?)?;
            bytes += event.data.len() + super::transport::EVENT_WIRE_OVERHEAD;
            events.push(event);
            if bytes > super::transport::REPLAY_PAGE_BYTES {
                break;
            }
        }
        Ok(events)
    }

    /// Reconstruct only a short tail, never the complete old run. Runtime
    /// leases must mint a new run ID when their epoch changes; silently moving
    /// the same run to another epoch would make resume cursors ambiguous.
    pub(crate) fn segmented_run_tail(
        &self,
        session: &str,
        run: &str,
        epoch: i64,
    ) -> Result<Vec<crate::rpc::SseEvent>> {
        let (s, r) = (session.to_owned(), run.to_owned());
        let first = self.db.read(move |db| {
            let r=crate::session::identity::BoundIdentity::run(db,&s,&r)?;
            let s=crate::session::identity::BoundIdentity::session(db,&s)?;
            let latest: Option<(i64,i64)> = db.query_row("SELECT id,epoch FROM stream_runs WHERE session_id=?1 AND run_id=?2 AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=stream_runs.id) ORDER BY epoch DESC LIMIT 1", rusqlite::params![s,r], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
            let Some((id, stored_epoch)) = latest else { return Ok(None); };
            ensure!(stored_epoch == epoch, "a restarted run requires a new identity; stored run epoch differs");
            Ok(db.query_row("SELECT min(first_cursor) FROM (SELECT first_cursor FROM stream_changes WHERE stream_id=?1 ORDER BY last_cursor DESC LIMIT 8)", [id], |row| row.get::<_, Option<i64>>(0))?)
        })?;
        match first {
            Some(first) => self.segmented_stream_page(
                &StreamIdentity {
                    session_id: session.into(),
                    run_id: run.into(),
                    epoch,
                },
                first - 1,
                8,
            ),
            None => Ok(Vec::new()),
        }
    }

    pub(crate) fn segmented_replay(
        &self,
        session: &str,
        run: &str,
        since: i64,
        session_scope: bool,
        limit: Option<usize>,
    ) -> Result<crate::session::journal_compaction::RunReplayPage> {
        let (session, run) = (session.to_owned(), run.to_owned());
        let (s, r) = (session.clone(), run.clone());
        let epochs = self.db.read(move |db| {
            let r=crate::session::identity::BoundIdentity::run(db,&s,&r)?;
            let s=crate::session::identity::BoundIdentity::session(db,&s)?;
            let mut query = db.prepare("SELECT epoch FROM stream_runs WHERE session_id=?1 AND run_id=?2 AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=stream_runs.id) AND (?3=0 OR epoch=-1 OR ?4 < -1) ORDER BY epoch")?;
            let epochs = query.query_map(rusqlite::params![s,r,session_scope,since],|row|row.get::<_,i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(epochs)
        })?;
        let known = !epochs.is_empty();
        let storage_identity = self.storage_identity()?;
        let mut events = Vec::new();
        let mut bytes = 0usize;
        'epochs: for epoch in epochs {
            let stream = StreamIdentity {
                session_id: session.clone(),
                run_id: run.clone(),
                epoch,
            };
            let mut cursor = since;
            loop {
                let remaining = limit.unwrap_or(usize::MAX).saturating_sub(events.len());
                if remaining == 0 {
                    break;
                }
                let identity = stream.clone();
                let expected_storage = storage_identity.clone();
                let page = self.db.read(move |db| {
                    let tx = db.transaction()?;
                    ensure!(
                        super::transport::storage_identity(&tx)? == expected_storage,
                        "storage identity changed during replay"
                    );
                    let page = read_encoded_changes(&tx, &identity, cursor, remaining.min(128))?;
                    tx.commit()?;
                    Ok(page)
                })?;
                let page = page.decode()?;
                for change in page.changes {
                    ensure!(
                        change.last_cursor <= page.durable_cursor,
                        "change exceeds durable watermark"
                    );
                    cursor = change.last_cursor;
                    let event: crate::rpc::SseEvent = serde_json::from_value(materialize(
                        self,
                        &stream,
                        &storage_identity,
                        change,
                    )?)?;
                    bytes = bytes
                        .saturating_add(event.data.len() + super::transport::EVENT_WIRE_OVERHEAD);
                    events.push(event);
                    if bytes > super::transport::REPLAY_PAGE_BYTES {
                        ensure!(
                            limit.is_some(),
                            "run replay exceeds memory budget; use paged get_events_since"
                        );
                        // Keep one real lookahead event beyond the byte budget.
                        // The RPC cuts it and returns has_more; it is read again
                        // from the last delivered cursor on the next request.
                        break 'epochs;
                    }
                }
                if !page.has_more {
                    break;
                }
            }
            if limit.is_some_and(|n| events.len() >= n) {
                break;
            }
        }
        Ok(crate::session::journal_compaction::RunReplayPage {
            known,
            events,
            snapshot: None,
        })
    }
}
