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
    metadata_bytes: usize,
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
        let mut pending = Pending::default();
        for event in events {
            let (stream, change, appends) = next.encode(store, session, event, &mut pending)?;
            pending.push(
                store,
                Group {
                    stream,
                    appends,
                    change,
                    metadata_bytes: 0,
                },
            )?;
        }
        pending.flush(store)?;
        *self = next;
        Ok(())
    }

    fn encode(
        &mut self,
        store: &SqliteStore,
        session: &str,
        event: &Value,
        pending: &mut Pending,
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
                            pending.flush(store)?;
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
                    pending.flush(store)?;
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
            pending.flush(store)?;
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
            pending.flush(store)?;
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

/// Exact compact JSON byte length for admission, without serializing token
/// metadata repeatedly. The actual publication still enforces its byte budget.
pub(super) fn json_bytes(value: &Value) -> usize {
    fn string_bytes(value: &str) -> usize {
        value.bytes().fold(2usize, |n, b| {
            n.saturating_add(match b {
                b'"' | b'\\' | 8 | 9 | 10 | 12 | 13 => 2,
                0..=31 => 6,
                _ => 1,
            })
        })
    }
    match value {
        Value::Null => 4,
        Value::Bool(value) => {
            if *value {
                4
            } else {
                5
            }
        }
        Value::Number(value) => {
            let digits = |n: u64| if n == 0 { 1 } else { n.ilog10() as usize + 1 };
            if let Some(n) = value.as_u64() {
                digits(n)
            } else if let Some(n) = value.as_i64() {
                digits(n.unsigned_abs()) + usize::from(n < 0)
            } else {
                value.to_string().len()
            }
        }
        Value::String(value) => string_bytes(value),
        Value::Array(values) => values.iter().fold(
            2usize.saturating_add(values.len().saturating_sub(1)),
            |n, v| n.saturating_add(json_bytes(v)),
        ),
        Value::Object(values) => values.iter().fold(
            2usize.saturating_add(values.len().saturating_sub(1)),
            |n, (k, v)| {
                n.saturating_add(string_bytes(k))
                    .saturating_add(1)
                    .saturating_add(json_bytes(v))
            },
        ),
    }
}

/// The producer already admitted a bounded batch. Semantic grouping controls
/// replay facts, not transaction boundaries: unrelated facts can share a commit
/// without merging their metadata. Pending work is flushed before encoder reads
/// or stages bodies, and never survives persist()'s acknowledgement.
#[derive(Default)]
struct Pending {
    groups: Vec<Group>,
    bytes: usize,
    metadata_bytes: usize,
    appends: usize,
}
impl Pending {
    fn flush(&mut self, store: &SqliteStore) -> Result<()> {
        if self.groups.is_empty() {
            return Ok(());
        }
        let groups = std::mem::take(&mut self.groups);
        self.bytes = 0;
        self.metadata_bytes = 0;
        self.appends = 0;
        let stream = groups[0].stream.clone();
        let mut changes = Vec::with_capacity(groups.len());
        let mut packed: Vec<ContentAppend> = Vec::new();
        for group in groups {
            changes.push(group.change);
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
        }
        while packed.len() > CHANGES_PER_BATCH {
            let batch = packed.drain(..CHANGES_PER_BATCH).collect::<Vec<_>>();
            let scope = stream.clone();
            store
                .db
                .call(move |db| commit_batch(db, &scope, batch, Vec::new()).map(|_| ()))?;
        }
        store
            .db
            .call(move |db| commit_batch(db, &stream, packed, changes).map(|_| ()))
    }

    fn push(&mut self, store: &SqliteStore, mut group: Group) -> Result<()> {
        let bytes = group.appends.iter().map(|a| a.text.len()).sum::<usize>();
        let metadata = json_bytes(&group.change.metadata);
        group.metadata_bytes = metadata;
        if !self.groups.is_empty()
            && (self.groups[0].stream != group.stream
                || self.groups.len() >= CHANGES_PER_BATCH
                || self.bytes.saturating_add(bytes) > WRITE_BYTES
                || self.metadata_bytes.saturating_add(metadata) > CHANGE_METADATA_BYTES
                || self.appends.saturating_add(group.appends.len()) > CHANGES_PER_BATCH)
        {
            self.flush(store)?;
        }
        let terminal = group.change.event_type == "agent_end";
        let combine = self.groups.last().is_some_and(|g| {
            g.stream == group.stream
                && g.change.event_type == group.change.event_type
                && same_delta_metadata(&g.change.metadata, &group.change.metadata)
                && g.change.last_cursor + 1 == group.change.first_cursor
                && g.change.bodies.len() == 1
                && group.change.bodies.len() == 1
                && g.change.bodies[0].key == group.change.bodies[0].key
                && g.change.bodies[0].field == group.change.bodies[0].field
                && g.change.bodies[0].offset + g.change.bodies[0].length
                    == group.change.bodies[0].offset
                && matches!(
                    group.change.event_type.as_str(),
                    "text_chunk" | "thinking_delta" | "tool_delta"
                )
                && g.change.metadata["data"]["snapshot"] != true
                && group.change.metadata["data"]["snapshot"] != true
        });
        self.bytes += bytes;
        self.appends += group.appends.len();
        if combine {
            let prior = self.groups.last_mut().expect("checked group");
            self.metadata_bytes -= prior.metadata_bytes;
            let body = &mut prior.change.bodies[0];
            body.length += group.change.bodies[0].length;
            group.change.metadata["data"]["content_offset"] = json!(body.offset);
            group.change.metadata["data"]["content_bytes"] = json!(body.length);
            group.change.metadata["data"]["content_first_idx"] = json!(prior.change.first_cursor);
            prior.change.metadata = group.change.metadata;
            prior.change.last_cursor = group.change.last_cursor;
            prior.appends.extend(group.appends);
            prior.metadata_bytes = json_bytes(&prior.change.metadata);
            self.metadata_bytes += prior.metadata_bytes;
        } else {
            self.metadata_bytes += metadata;
            self.groups.push(group);
        }
        if terminal {
            self.flush(store)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct BodyCursor {
    range: ContentRange,
    cursor: i64,
}

struct InlineBody {
    change: usize,
    field: String,
    body: BodyCursor,
}

fn inline_body_requests(changes: &[StoredChange]) -> Vec<InlineBody> {
    let mut requests = Vec::new();
    for (index, change) in changes.iter().enumerate() {
        let fragment = matches!(
            change.event_type.as_str(),
            "text_chunk" | "thinking_delta" | "tool_delta"
        );
        let mut inline_bytes = 0usize;
        for (field, range, _) in &change.bodies {
            let size = usize::try_from(range.length).unwrap_or(usize::MAX);
            if !fragment
                && (size > super::transport::INLINE_FIELD_BYTES
                    || inline_bytes.saturating_add(size.saturating_mul(6))
                        > super::transport::INLINE_EVENT_BYTES / 2)
            {
                continue;
            }
            inline_bytes = inline_bytes.saturating_add(size.saturating_mul(6));
            requests.push(InlineBody {
                change: index,
                field: field.clone(),
                body: BodyCursor {
                    range: *range,
                    cursor: change.last_cursor,
                },
            });
        }
    }
    requests
}

/// Resolve and read every inline body for one replay page in a single read
/// transaction. Each reference is compared with the range the page was decoded
/// from, so a concurrently deleted/recycled object can never be served as
/// authority for the old cursor.
fn read_inline_bodies(
    store: &SqliteStore,
    stream: &StreamIdentity,
    identity: &future_rpc::content_range::StorageIdentity,
    requests: Vec<InlineBody>,
) -> Result<std::collections::HashMap<(usize, String), String>> {
    if requests.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let (session_id, run_id, epoch) = (
        stream.session_id.clone(),
        stream.run_id.clone(),
        stream.epoch,
    );
    let expected = identity.clone();
    store.db.read(move |db| {
        ensure!(
            super::transport::storage_identity(db)? == expected,
            "content database/restore identity changed"
        );
        ensure!(
            super::format(db, &session_id)? == super::FORMAT_VERSION,
            "conversation does not support canonical content ranges"
        );
        let tx = db.transaction()?;
        let session = crate::session::identity::BoundIdentity::session(&tx, &session_id)?;
        let run = crate::session::identity::BoundIdentity::run(&tx, &session_id, &run_id)?;
        let mut statement = tx.prepare_cached(
            "SELECT b.content_id,b.byte_offset,b.byte_length FROM stream_runs r \
             JOIN stream_changes c ON c.stream_id=r.id \
             JOIN stream_content_ranges b ON b.change_id=c.id \
             WHERE r.session_id=?1 AND r.run_id=?2 AND r.epoch=?3 AND c.last_cursor=?4 \
               AND c.last_cursor<=r.durable_cursor AND b.field=?5 \
               AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=r.id)",
        )?;
        let mut bodies = std::collections::HashMap::with_capacity(requests.len());
        for request in requests {
            let current: Option<ContentRange> = statement
                .query_row(
                    rusqlite::params![session, run, epoch, request.body.cursor, request.field,],
                    |row| {
                        Ok(ContentRange {
                            id: row.get(0)?,
                            offset: row.get(1)?,
                            length: row.get(2)?,
                        })
                    },
                )
                .optional()?;
            ensure!(
                current == Some(request.body.range),
                "event content reference changed while reading"
            );
            let page = super::read(
                &tx,
                request.body.range.id,
                request.body.range.offset,
                request.body.range.length.try_into()?,
            )?;
            ensure!(
                page.next_offset == request.body.range.offset + request.body.range.length,
                "inline event content is incomplete"
            );
            bodies.insert((request.change, request.field), page.text);
        }
        drop(statement);
        tx.commit()?;
        Ok(bodies)
    })
}

/// Materialize one page, reading all inline bodies in one transaction and doing
/// JSON assembly after it commits.
fn materialize_page(
    store: &SqliteStore,
    stream: &StreamIdentity,
    identity: &future_rpc::content_range::StorageIdentity,
    changes: Vec<StoredChange>,
) -> Result<Vec<Value>> {
    let requests = inline_body_requests(&changes);
    let mut bodies = read_inline_bodies(store, stream, identity, requests)?;
    changes
        .into_iter()
        .enumerate()
        .map(|(index, change)| {
            materialize_with(stream, identity, change, |field, _| {
                bodies
                    .remove(&(index, field.to_owned()))
                    .context("inline event body was not staged")
            })
        })
        .collect()
}

fn materialize_with(
    stream: &StreamIdentity,
    identity: &future_rpc::content_range::StorageIdentity,
    change: StoredChange,
    mut read_body: impl FnMut(&str, BodyCursor) -> Result<String>,
) -> Result<Value> {
    ensure!(
        change.first_cursor <= change.last_cursor,
        "invalid canonical change range"
    );
    let cursor = change.last_cursor;
    let mut event = change.metadata;
    if event["data"].get("content_key").is_some() {
        event["data"]["content_first_idx"] = json!(change.first_cursor);
    }
    let fragment = matches!(
        change.event_type.as_str(),
        "text_chunk" | "thinking_delta" | "tool_delta"
    );
    let mut inline_bytes = 0usize;
    for (field, range, encoding) in change.bodies {
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
                "epoch":stream.epoch,"cursor":cursor,"field":field,"offset":0,"maxBytes":64*1024
            });
            continue;
        }
        let text = read_body(&field, BodyCursor { range, cursor })?;
        inline_bytes = inline_bytes.saturating_add(text.len().saturating_mul(6));
        let value = match encoding {
            Encoding::Text => json!(text),
            Encoding::Json => serde_json::from_str(&text)?,
            Encoding::ShellSummary => json!(future_rpc::shell_result::model_summary(
                &serde_json::from_str(&text)?
            )),
            Encoding::ToolArgumentsStringV1 => {
                let mut value = json!(text);
                future_rpc::tool_arguments::repair_string_v1(&mut value);
                value
            }
        };
        *event
            .pointer_mut(&field)
            .context("missing semantic content slot")? = value;
    }
    event["data"]["content_format"] = json!(super::FORMAT_VERSION);
    let scoped = stream.run_id.is_empty() && stream.epoch == -1;
    event["session_id"] = json!(stream.session_id);
    event["run_id"] = json!(stream.run_id);
    event["epoch"] = json!(if scoped { 0 } else { stream.epoch });
    event["idx"] = json!(if scoped { -1 } else { cursor });
    event["session_idx"] = json!(if scoped { cursor } else { -1 });
    event["event_type"] = json!(change.event_type);
    if event.get("event_id").is_none() {
        event["event_id"] = json!(if scoped {
            format!("{}:session:{}", stream.session_id, cursor)
        } else {
            format!(
                "{}:{}:{}:{}",
                stream.session_id, stream.run_id, stream.epoch, cursor
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
        ensure!(
            page.changes
                .iter()
                .all(|change| change.last_cursor <= page.durable_cursor),
            "change exceeds durable watermark"
        );
        let mut events = Vec::new();
        let mut bytes = 0usize;
        for event in materialize_page(self, stream, &identity, page.changes)? {
            let event: crate::rpc::SseEvent = serde_json::from_value(event)?;
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
                ensure!(
                    page.changes
                        .iter()
                        .all(|change| change.last_cursor <= page.durable_cursor),
                    "change exceeds durable watermark"
                );
                let cursors = page
                    .changes
                    .iter()
                    .map(|change| change.last_cursor)
                    .collect::<Vec<_>>();
                let values = materialize_page(self, &stream, &storage_identity, page.changes)?;
                for (value, last_cursor) in values.into_iter().zip(cursors) {
                    cursor = last_cursor;
                    let event: crate::rpc::SseEvent = serde_json::from_value(value)?;
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
