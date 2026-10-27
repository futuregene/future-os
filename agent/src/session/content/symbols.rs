//! Per-stream string interning. Long source identifiers are stored once even
//! when many immutable contexts refer to the same message/tool/block.
use crate::session::identity::StoredIdentity;
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Canonical UUIDs in new dictionary values have a binary representation; all
/// other strings remain opaque. FromSql keeps historical TEXT as TEXT.
pub(super) fn key(wire: &str) -> StoredIdentity {
    StoredIdentity::generated(wire).unwrap_or_else(|_| StoredIdentity::Legacy(wire.into()))
}

const FIELDS: &[&str] = &[
    "entry_id",
    "tool_id",
    "content_key",
    "block_id",
    "tool_name",
    "tool_type",
];
pub(super) type Symbols = HashMap<i64, (i64, String)>;

/// IDs never leave their publication transaction until COMMIT. String lookup
/// uses SQLite's exact BINARY equality, including empty strings and embedded NUL.
pub(super) fn intern(
    tx: &Transaction<'_>,
    stream: i64,
    mut descriptor: Value,
) -> Result<(String, usize)> {
    let mut symbols = Map::new();
    let mut bytes = 0usize;
    if let Some(data) = descriptor[0].get_mut("data").and_then(Value::as_object_mut) {
        for field in FIELDS {
            let Some(value) = data.get(*field).and_then(Value::as_str) else {
                continue;
            };
            bytes = bytes.saturating_add(value.len());
            let stored = key(value);
            let id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM stream_symbols WHERE stream_id=?1 AND value=?2 COLLATE BINARY",
                    params![stream, &stored],
                    |r| r.get(0),
                )
                .optional()?;
            let id = match id {
                Some(id) => id,
                None => {
                    tx.execute(
                        "INSERT INTO stream_symbols(stream_id,value) VALUES (?1,?2)",
                        params![stream, &stored],
                    )?;
                    tx.last_insert_rowid()
                }
            };
            symbols.insert((*field).into(), json!(id));
            data.remove(*field);
        }
    }
    let array = descriptor
        .as_array_mut()
        .context("invalid context descriptor")?;
    ensure!(array.len() == 4, "invalid context descriptor shape");
    let kind = array.pop().context("missing context kind")?;
    array.push(Value::Object(symbols));
    array.push(kind);
    let encoded = descriptor.to_string();
    ensure!(
        encoded.len() <= super::stream::CHANGE_METADATA_BYTES
            && bytes <= super::stream::CHANGE_METADATA_BYTES,
        "context exceeds symbol budget"
    );
    Ok((encoded, bytes))
}

/// One query for the page's distinct dictionary entries. No per-change lookup;
/// all returned values count against the same raw-page memory budget.
pub(super) fn read(db: &Connection, selection: &str, bytes: &mut usize) -> Result<Symbols> {
    let mut query=db.prepare_cached("SELECT s.id,s.stream_id,s.value FROM stream_symbols s WHERE s.id IN (SELECT j.value FROM stream_contexts x,json_each(x.descriptor_json,'$[3]') j WHERE x.id IN (SELECT context_id FROM stream_changes WHERE id IN (SELECT value FROM json_each(?1))))")?;
    let mut rows = query.query([selection])?;
    let mut symbols = HashMap::new();
    while let Some(row) = rows.next()? {
        let value = row.get::<_, StoredIdentity>(2)?.wire();
        *bytes = bytes.saturating_add(value.len());
        ensure!(
            *bytes <= super::READ_BYTES,
            "change page exceeds symbol byte budget"
        );
        symbols.insert(row.get(0)?, (row.get(1)?, value));
    }
    Ok(symbols)
}

pub(super) fn restore(descriptor: &mut Value, stream: i64, symbols: &Symbols) -> Result<()> {
    let references = descriptor[3]
        .as_object()
        .context("missing context symbol map")?
        .clone();
    for (field, id) in references {
        ensure!(
            FIELDS.contains(&field.as_str()),
            "unknown context symbol field"
        );
        let (owner, value) = symbols
            .get(&id.as_i64().context("invalid symbol ID")?)
            .context("missing stream symbol")?;
        ensure!(*owner == stream, "context symbol belongs to another stream");
        let data = descriptor[0]
            .get_mut("data")
            .and_then(Value::as_object_mut)
            .context("missing context data")?;
        ensure!(!data.contains_key(&field), "overlapping symbol field");
        data.insert(field, json!(value));
    }
    Ok(())
}
