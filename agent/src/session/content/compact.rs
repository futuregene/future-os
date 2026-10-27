//! One physical codec for the unpublished format-2 layout. Contexts are
//! immutable and scoped to a stream; IDs are never retained across a rollback.
//! The protocol still sees the same metadata and scoped body ranges.
use super::{
    stream::{Change, Encoding, StoredChange},
    ContentRange,
};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub(crate) const LAYOUT: &str = "compact-context-v1";
// Frozen wire-independent codes. Append only; never use enum ordinals.
const KINDS: &[&str] = &[
    "agent_start",
    "agent_end",
    "message_start",
    "message_end",
    "text_start",
    "text_chunk",
    "text_end",
    "thinking_start",
    "thinking_delta",
    "thinking_end",
    "tool_start",
    "tool_delta",
    "tool_input_end",
    "tool_end",
    "tool_result",
    "usage",
    "model_changed",
    "tool_sandboxed",
];
fn kind_code(kind: &str) -> i64 {
    KINDS
        .iter()
        .position(|known| *known == kind)
        .map_or(0, |i| i as i64 + 1)
}

type Body = (String, ContentRange, Encoding);

fn encode(change: &Change, direct: Option<&Body>) -> Result<(String, String, i64)> {
    let mut remainder = change.metadata.clone();
    let mut stable = Map::new();
    if let Some(object) = remainder.as_object_mut() {
        if let Some(value) = object.remove("run_sequence") {
            stable.insert("run_sequence".into(), value);
        }
        if let Some(data) = object.get_mut("data").and_then(Value::as_object_mut) {
            let mut context = Map::new();
            for key in [
                "type",
                "entry_id",
                "tool_id",
                "tool_name",
                "tool_type",
                "tc_index",
                "content_key",
                "block_id",
            ] {
                if let Some(value) = data.remove(key) {
                    context.insert(key.into(), value);
                }
            }
            if !context.is_empty() {
                stable.insert("data".into(), Value::Object(context));
            }
        }
    }
    let mut flags = 0;
    if let Some((_, range, _)) = direct {
        if let Some(data) = remainder.get_mut("data").and_then(Value::as_object_mut) {
            for (bit, key, expected) in [
                (1, "content_offset", range.offset),
                (2, "content_bytes", range.length),
                (4, "content_first_idx", change.first_cursor),
            ] {
                if data.get(key).and_then(Value::as_i64) == Some(expected) {
                    data.remove(key);
                    flags |= bit;
                }
            }
        }
    }
    let (field, encoding) = direct.map_or((None, None), |(field, _, encoding)| {
        (Some(field.as_str()), Some(encoding.name()))
    });
    let descriptor = json!([stable, field, encoding]).to_string();
    ensure!(
        descriptor.len() <= super::stream::CHANGE_METADATA_BYTES,
        "context exceeds metadata budget"
    );
    Ok((descriptor, remainder.to_string(), flags))
}

fn decode(
    descriptor: &Value,
    mut remainder: Value,
    flags: i64,
    first: i64,
    direct: Option<ContentRange>,
) -> Result<Value> {
    let stable = descriptor[0]
        .as_object()
        .context("invalid stream context")?;
    let object = remainder
        .as_object_mut()
        .context("invalid change metadata")?;
    for (key, value) in stable {
        if key == "data" {
            let data = object
                .get_mut("data")
                .and_then(Value::as_object_mut)
                .context("missing change data")?;
            for (name, value) in value.as_object().context("invalid context data")? {
                ensure!(!data.contains_key(name), "overlapping context field");
                data.insert(name.clone(), value.clone());
            }
        } else {
            ensure!(!object.contains_key(key), "overlapping context field");
            object.insert(key.clone(), value.clone());
        }
    }
    if flags != 0 {
        let range = direct.context("range flags without direct body")?;
        let data = object
            .get_mut("data")
            .and_then(Value::as_object_mut)
            .context("missing range data")?;
        for (bit, key, value) in [
            (1, "content_offset", range.offset),
            (2, "content_bytes", range.length),
            (4, "content_first_idx", first),
        ] {
            if flags & bit != 0 {
                ensure!(!data.contains_key(key), "overlapping range field");
                data.insert(key.into(), json!(value));
            }
        }
    }
    Ok(remainder)
}

/// All callers are in the publication transaction. A lookup always compares
/// the complete descriptor, including field presence/null and direct encoding.
/// No cross-transaction ID cache exists to leak an aborted allocation.
fn context(tx: &Transaction<'_>, stream: i64, descriptor: &str) -> Result<i64> {
    let digest = Sha256::digest(descriptor.as_bytes());
    let mut stmt = tx.prepare_cached(
        "SELECT id,descriptor_json FROM stream_contexts WHERE stream_id=?1 AND digest=?2 LIMIT 9",
    )?;
    let candidates = stmt
        .query_map(params![stream, digest.as_slice()], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(
        candidates.len() < 9,
        "stream context digest collision budget exceeded"
    );
    if let Some((id, _)) = candidates.iter().find(|(_, stored)| stored == descriptor) {
        return Ok(*id);
    }
    tx.execute(
        "INSERT INTO stream_contexts(stream_id,digest,descriptor_json) VALUES (?1,?2,?3)",
        params![stream, digest.as_slice(), descriptor],
    )?;
    Ok(tx.last_insert_rowid())
}

pub(crate) fn publish(
    tx: &Transaction<'_>,
    stream: i64,
    change: &Change,
    bodies: Vec<Body>,
) -> Result<()> {
    ensure!(bodies.len() <= 128, "change field budget exceeded");
    if let Some(id) = tx
        .query_row(
            "SELECT id FROM stream_changes WHERE stream_id=?1 AND last_cursor=?2",
            params![stream, change.last_cursor],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    {
        let mut prior = read_ids(tx, &[id])?;
        let prior = prior.pop().context("missing retry change")?;
        ensure!(
            prior.first_cursor == change.first_cursor
                && prior.event_type == change.event_type
                && prior.metadata == change.metadata,
            "conflicting semantic change retry"
        );
        let mut old = prior.bodies;
        let mut new = bodies;
        old.sort_by(|a, b| a.0.cmp(&b.0));
        new.sort_by(|a, b| a.0.cmp(&b.0));
        ensure!(old == new, "conflicting semantic content link");
        return Ok(());
    }
    let direct = (bodies.len() == 1).then(|| &bodies[0]);
    let (descriptor, metadata, flags) = encode(change, direct)?;
    let context = context(tx, stream, &descriptor)?;
    let code = kind_code(&change.event_type);
    let range = direct.map(|(_, range, _)| *range);
    tx.execute("INSERT INTO stream_changes(stream_id,context_id,first_cursor,last_cursor,event_kind,unknown_kind,flags,metadata_json,content_id,byte_offset,byte_length) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![stream,context,change.first_cursor,change.last_cursor,code,(code==0).then_some(&change.event_type),flags,metadata,range.map(|r|r.id),range.map(|r|r.offset),range.map(|r|r.length)])?;
    if direct.is_none() {
        let id = tx.last_insert_rowid();
        for (field, range, encoding) in bodies {
            tx.execute("INSERT INTO stream_change_content(change_id,field,encoding,content_id,byte_offset,byte_length) VALUES (?1,?2,?3,?4,?5,?6)",params![id,field,encoding.name(),range.id,range.offset,range.length])?;
        }
    }
    Ok(())
}

#[derive(Debug)]
struct Row {
    id: i64,
    context: i64,
    first: i64,
    last: i64,
    kind: String,
    flags: i64,
    metadata: String,
    direct: Option<ContentRange>,
}

#[derive(Debug, Default)]
pub(crate) struct EncodedPage {
    rows: Vec<Row>,
    contexts: Vec<(i64, String)>,
    links: HashMap<i64, Vec<Body>>,
}

/// Single-fact decoding at transactional semantic boundaries.
/// Streaming/recovery consumers fetch raw pages and decode off the DB worker.
pub(crate) fn read_ids(db: &Connection, ids: &[i64]) -> Result<Vec<StoredChange>> {
    read_encoded(db, ids)?.decode()
}

/// Three bounded queries for a page. Descriptors are fetched once per context;
/// the worker returns strings/ranges without parsing or assembling JSON.
pub(crate) fn read_encoded(db: &Connection, ids: &[i64]) -> Result<EncodedPage> {
    ensure!(ids.len() <= 257, "change selection exceeds page budget");
    if ids.is_empty() {
        return Ok(EncodedPage::default());
    }
    let selection = serde_json::to_string(ids)?;
    let mut query = db.prepare_cached("SELECT c.id,c.context_id,c.first_cursor,c.last_cursor,c.event_type,c.flags,c.metadata_json,c.content_id,c.byte_offset,c.byte_length FROM stream_changes c JOIN json_each(?1) selected ON c.id=selected.value ORDER BY CAST(selected.key AS INTEGER)")?;
    let mut bytes = 0usize;
    let mapped = query.query_map([&selection], |r| {
        let content: Option<i64> = r.get(7)?;
        Ok(Row {
            id: r.get(0)?,
            context: r.get(1)?,
            first: r.get(2)?,
            last: r.get(3)?,
            kind: r.get(4)?,
            flags: r.get(5)?,
            metadata: r.get(6)?,
            direct: content
                .map(|id| {
                    Ok::<_, rusqlite::Error>(ContentRange {
                        id,
                        offset: r.get(8)?,
                        length: r.get(9)?,
                    })
                })
                .transpose()?,
        })
    })?;
    let mut rows = Vec::new();
    for row in mapped {
        let row = row?;
        bytes = bytes.saturating_add(row.metadata.len());
        ensure!(
            bytes <= super::READ_BYTES,
            "change page exceeds metadata byte budget"
        );
        rows.push(row);
    }
    let mut query = db.prepare_cached("SELECT x.id,x.descriptor_json FROM stream_contexts x WHERE x.id IN (SELECT context_id FROM stream_changes WHERE id IN (SELECT value FROM json_each(?1)))")?;
    let mapped = query.query_map([&selection], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut contexts = Vec::new();
    for context in mapped {
        let (id, raw) = context?;
        bytes = bytes.saturating_add(raw.len());
        ensure!(
            bytes <= super::READ_BYTES,
            "change page exceeds context byte budget"
        );
        contexts.push((id, raw));
    }
    let mut query = db.prepare_cached("SELECT change_id,field,content_id,byte_offset,byte_length,encoding FROM stream_change_content WHERE change_id IN (SELECT value FROM json_each(?1)) ORDER BY change_id,field")?;
    let mut links: HashMap<i64, Vec<Body>> = HashMap::new();
    let mut fields = query.query([&selection])?;
    while let Some(r) = fields.next()? {
        let field: String = r.get(1)?;
        bytes = bytes.saturating_add(field.len() + 64);
        ensure!(
            bytes <= super::READ_BYTES,
            "change page exceeds field byte budget"
        );
        links.entry(r.get(0)?).or_default().push((
            field,
            ContentRange {
                id: r.get(2)?,
                offset: r.get(3)?,
                length: r.get(4)?,
            },
            Encoding::parse(&r.get::<_, String>(5)?)?,
        ));
    }
    Ok(EncodedPage {
        rows,
        contexts,
        links,
    })
}

impl EncodedPage {
    pub(crate) fn decode(self) -> Result<Vec<StoredChange>> {
        let Self {
            rows,
            contexts,
            mut links,
        } = self;
        let parsed = contexts
            .into_iter()
            .map(|(id, raw)| Ok((id, serde_json::from_str::<Value>(&raw)?)))
            .collect::<Result<HashMap<_, _>>>()?;
        rows.into_iter()
            .map(|row| {
                let descriptor = parsed
                    .get(&row.context)
                    .context("missing immutable stream context")?;
                let mut bodies = links.remove(&row.id).unwrap_or_default();
                if let Some(range) = row.direct {
                    ensure!(bodies.is_empty(), "direct body has extra field links");
                    bodies.push((
                        descriptor[1]
                            .as_str()
                            .context("missing direct field")?
                            .into(),
                        range,
                        Encoding::parse(
                            descriptor[2].as_str().context("missing direct encoding")?,
                        )?,
                    ));
                }
                Ok(StoredChange {
                    first_cursor: row.first,
                    last_cursor: row.last,
                    event_type: row.kind,
                    metadata: decode(
                        descriptor,
                        serde_json::from_str(&row.metadata)?,
                        row.flags,
                        row.first,
                        row.direct,
                    )?,
                    bodies,
                })
            })
            .collect()
    }
}

/// Must run before WAL/schema writes for an already-current database. Version
/// 8 alone cannot distinguish two unpublished candidates. Never repair by
/// creating an empty table over an incompatible or damaged authoritative store.
pub(crate) fn validate_layout(db: &Connection) -> Result<()> {
    let marker: Option<String> = db.query_row("SELECT layout FROM stream_layout WHERE singleton=1",[],|r|r.get(0)).optional()
        .context("unsupported unpublished Agent storage layout; preserve agent.db and use the matching candidate or a released backup")?;
    ensure!(marker.as_deref()==Some(LAYOUT),"unsupported unpublished Agent storage layout; preserve agent.db and use the matching candidate or a released backup");
    for (name, columns) in [
        ("stream_contexts_identity", vec!["id", "stream_id"]),
        ("stream_contexts_digest", vec!["stream_id", "digest"]),
        ("stream_contexts_entry", vec!["stream_id", "entry_id", "id"]),
        (
            "stream_contexts_tool",
            vec!["stream_id", "entry_id", "tool_id", "id"],
        ),
        (
            "stream_contexts_key",
            vec!["stream_id", "entry_id", "content_key", "id"],
        ),
        (
            "stream_changes_context",
            vec!["context_id", "event_kind", "last_cursor"],
        ),
        ("stream_changes_body", vec!["content_id"]),
        ("stream_changes_pricing", vec!["stream_id", "id"]),
        ("stream_change_content_body", vec!["content_id"]),
    ] {
        let mut query = db.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?;
        let actual = query
            .query_map([name], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            actual == columns,
            "incomplete compact storage index {name}; preserve database for recovery"
        );
        let expected = include_str!("schema.sql")
            .split(';')
            .find(|sql| sql.contains(&format!("INDEX IF NOT EXISTS {name} ")))
            .context("missing index contract")?;
        let expected = expected
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join(" ");
        let installed: String = db.query_row(
            "SELECT sql FROM sqlite_schema WHERE type='index' AND name=?1",
            [name],
            |r| r.get(0),
        )?;
        let normalize = |sql: &str| {
            sql.replace("IF NOT EXISTS ", "")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        };
        ensure!(
            normalize(&installed) == normalize(&expected),
            "incompatible compact storage index contract {name}; preserve database for recovery"
        );
    }
    for (table, column, target) in [
        ("stream_changes", "context_id", "stream_contexts"),
        ("stream_changes", "stream_id", "stream_runs"),
        ("stream_changes", "content_id", "content_objects"),
        ("stream_contexts", "stream_id", "stream_runs"),
    ] {
        let valid: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_list(?1) WHERE \"from\"=?2 AND \"table\"=?3 AND \"to\"='id')",params![table,column,target],|r|r.get(0))?;
        ensure!(
            valid,
            "incomplete compact storage foreign key; preserve database for recovery"
        );
    }
    for trigger in [
        "stream_direct_removed",
        "stream_content_removed",
        "stream_context_immutable",
    ] {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='trigger' AND name=?1)",
            [trigger],
            |r| r.get(0),
        )?;
        ensure!(
            exists,
            "incomplete compact storage collection trigger; preserve database for recovery"
        );
    }
    db.prepare("SELECT change_id,field,encoding,content_id,byte_offset,byte_length FROM stream_content_ranges LIMIT 0")?;
    Ok(())
}

#[cfg(test)]
#[path = "compact_tests.rs"]
mod tests;
