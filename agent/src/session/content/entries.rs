//! Stage canonical entry bodies with bounded writer jobs, then publish headers.
//! Sealed streamed content is reused by digest; entry callbacks do not copy it.
use super::{
    append, create, find_json, find_sealed, format, resolve, seal, ContentDigest, ContentRange,
    FORMAT_VERSION, WRITE_BYTES,
};
use crate::session::sqlite_store::SqliteStore;
use anyhow::{ensure, Context, Result};
use rusqlite::params;
use serde_json::Value;

/// The digest of a sealed content object, read once per retried stage.
const OBJECT_SHA: &str = "SELECT sha256 FROM content_objects WHERE id=?1";

/// Keep the original shape and provider metadata; only known body fields move.
fn fields(value: &Value) -> Vec<(String, String)> {
    let Some(blocks) = value.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for (ordinal, block) in blocks.iter().enumerate() {
        let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
        let key = match kind {
            "text" | "reasoning" => "text",
            "tool_result" => "content",
            "tool_call" => "args",
            _ if block["text"].is_string() => "text",
            _ => continue,
        };
        let Some(body) = block.get(key) else { continue };
        let text = if key == "args" {
            body.to_string()
        } else {
            let Some(text) = body.as_str() else { continue };
            if text.is_empty()
                || text.starts_with(crate::session::repair::TOOL_LOST_PLACEHOLDER_PREFIX)
            {
                continue;
            }
            text.to_owned()
        };
        result.push((format!("/content/{ordinal}/{key}"), text));
    }
    result
}

pub(crate) fn prepare(
    store: &SqliteStore,
    session: &str,
    entries: Vec<Value>,
) -> Result<super::entry_stages::PreparedEntries> {
    let id = session.to_owned();
    if store.db.read(move |db| format(db, &id))? != FORMAT_VERSION {
        return Ok(super::entry_stages::PreparedEntries::legacy(entries));
    }
    super::entry_stages::PreparedEntries::stage(store, session, entries)
}

pub(super) fn prepare_unleased(
    store: &SqliteStore,
    session: &str,
    entries: Vec<Value>,
) -> Result<Vec<Value>> {
    prepare_owned(store, session, entries, "entry", None, false)
}

/// A reset stages independently owned references before publication. The
/// operation receipt is durable before this function can create any owner.
pub(crate) fn prepare_staged(
    store: &SqliteStore,
    session: &str,
    entries: Vec<Value>,
    prefix: &str,
) -> Result<Vec<Value>> {
    prepare_owned(store, session, entries, "checkpoint", Some(prefix), false)
}

/// Only a receipt-owned unpublished fork may supply trusted shared fields.
/// Public append callers must present their actual bodies for identity checks.
pub(crate) fn prepare_shared_fork(
    store: &SqliteStore,
    session: &str,
    entries: Vec<Value>,
) -> Result<Vec<Value>> {
    let id = session.to_owned();
    store.db.read(move |db| {
        let id=crate::session::identity::BoundIdentity::session(db,&id)?;
        let hidden:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM fork_operations f JOIN sessions s ON s.id=f.child_session_id JOIN session_formats v ON v.session_id=s.id WHERE s.id=?1 AND s.revision=-1 AND v.format=2)",[id],|row|row.get(0))?;
        ensure!(hidden,"shared entry preparation requires an unpublished fork receipt");Ok(())
    })?;
    prepare_owned(store, session, entries, "entry", None, true)
}

fn prepare_owned(
    store: &SqliteStore,
    session: &str,
    entries: Vec<Value>,
    owner_kind: &'static str,
    prefix: Option<&str>,
    shared: bool,
) -> Result<Vec<Value>> {
    let mut prepared = Vec::with_capacity(entries.len());
    for value in entries {
        let mut value = crate::session::entry_records::normalize_entry(value);
        ensure!(value["meta"].is_null() || value["meta"].is_object(),
            "canonical entry meta must be an object or null; refusing to overwrite opaque data while assigning run ownership");
        if value["type"] == "session_info" {
            super::metadata::validate_input(&value["content"])?;
        }
        let entry = value["id"]
            .as_str()
            .context("entry identity is required")?
            .to_owned();
        let entry = prefix.map_or(entry.clone(), |prefix| format!("{prefix}{entry}"));
        // References copied from byte-identical retained source fields are
        // already authoritative, including unsealed recovered prefixes. Do
        // not rehash, copy or seal their shared content object.
        let prelinked = if prefix.is_some() || shared {
            let mut fields = std::collections::HashMap::new();
            let mut after = String::new();
            loop {
                let (session, owner, cursor) = (session.to_owned(), entry.clone(), after.clone());
                let page=store.db.read(move |db| {
                    let owner=super::owner_key(db,&session,owner_kind,&owner)?;
                    let session=crate::session::identity::session_key(db,&session)?;
                    let mut query=db.prepare("SELECT field,encoding FROM content_references WHERE session_id=?1 AND owner_kind=?2 AND owner_id=?3 AND field>?4 ORDER BY field LIMIT 64")?;
                    let rows=query.query_map(params![session,owner_kind,owner,cursor],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                    ensure!(rows.iter().map(|(field,encoding)|field.len()+encoding.len()).sum::<usize>()<=super::READ_BYTES,"shared field identity page exceeds budget");Ok(rows)
                })?;
                if page.is_empty() {
                    break;
                }
                for (field, encoding) in page {
                    after = field.clone();
                    fields.insert(field, encoding);
                }
            }
            fields
        } else {
            std::collections::HashMap::new()
        };
        for (field, encoding) in &prelinked {
            let arguments = field
                .strip_suffix("/args")
                .and_then(|parent| value.pointer(parent))
                .is_some_and(|block| block["type"] == "tool_call");
            let slot = value
                .pointer_mut(field)
                .context("staged reset reference has no matching retained field")?;
            *slot = if arguments || field == "/meta/shell_result" {
                Value::String(String::new())
            } else if encoding == "json" {
                super::extensions::json_placeholder(slot)
            } else if field == "/content/error" {
                Value::String(
                    slot.as_str()
                        .unwrap_or_default()
                        .chars()
                        .take(512)
                        .collect(),
                )
            } else {
                Value::String(String::new())
            };
        }
        let run = value
            .pointer("/meta/run_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let metadata = value.get_mut("meta").and_then(Value::as_object_mut);
        let (keys, epoch) = if let Some(metadata) = metadata {
            (
                metadata.remove(crate::types::AgentMessage::STREAM_BLOCKS_KEY),
                metadata
                    .remove(crate::types::AgentMessage::STREAM_EPOCH_KEY)
                    .and_then(|value| value.as_i64()),
            )
        } else {
            (None, None)
        };
        if owner_kind == "entry" && prefix.is_none() && !shared {
            let (session, id) = (session.to_owned(), entry.clone());
            let published = store.db.read(move |db| {
                use rusqlite::OptionalExtension;
                let tx = db.transaction()?;
                let id = crate::session::identity::BoundIdentity::entry(&tx, &session, &id)?;
                let stored_session = crate::session::identity::session_key(&tx, &session)?;
                let position: Option<i64> = tx
                    .query_row(
                        "SELECT position FROM entries WHERE session_id=?1 AND entry_id=?2",
                        params![stored_session, &id],
                        |row| row.get(0),
                    )
                    .optional()?;
                let published = position
                    .map(|position| -> Result<_> {
                        Ok((
                            super::reader::Snapshot::capture_from(&tx, &session)?,
                            super::reader::Entry { position, id },
                        ))
                    })
                    .transpose()?;
                tx.commit()?;
                Ok(published)
            })?;
            if let Some((scope, locator)) = published {
                scope.verify_existing_intent(store, &locator, &value)?;
            }
        }
        let shell = value
            .pointer("/meta/shell_result")
            .filter(|value| value.is_object())
            .cloned();
        let shell = if let Some(shell) = shell {
            let facts =
                serde_json::from_value::<future_rpc::shell_result::ShellResult>(shell.clone()).ok();
            let range = stage_named(
                store,
                session,
                owner_kind,
                &entry,
                "/meta/shell_result",
                &shell.to_string(),
                true,
            )?;
            value["meta"]["shell_result"] = Value::String(String::new());
            facts.map(|facts| (range, future_rpc::shell_result::model_summary(&facts)))
        } else {
            None
        };
        for (field, text) in fields(&value) {
            if prelinked.contains_key(&field) {
                continue;
            }
            if let Some((range, summary)) = &shell {
                if field.ends_with("/content") && text == *summary {
                    let (session, entry, reference_field, range) =
                        (session.to_owned(), entry.clone(), field.clone(), *range);
                    store.db.call_as("content_store", move |db| {
                        let tx = crate::session::database::begin_write_transaction(db)?;
                        super::bind_encoded(
                            &tx,
                            &session,
                            owner_kind,
                            &entry,
                            &reference_field,
                            range,
                            super::stream::Encoding::ShellSummary,
                        )?;
                        tx.commit()?;
                        Ok(())
                    })?;
                    *value.pointer_mut(&field).expect("known field") = Value::String(String::new());
                    continue;
                }
            }
            if let (Some(run), Some(epoch), Some(keys)) =
                (&run, epoch, keys.as_ref().and_then(Value::as_array))
            {
                if let Some(key) = field
                    .strip_prefix("/content/")
                    .and_then(|path| path.split('/').next())
                    .and_then(|ordinal| ordinal.parse::<usize>().ok())
                    .and_then(|ordinal| keys.get(ordinal))
                    .and_then(Value::as_str)
                {
                    let scope = super::stream::StreamIdentity {
                        session_id: session.to_owned(),
                        run_id: run.clone(),
                        epoch,
                    };
                    let owner = super::stream::owner(&scope, key);
                    let mut digest = ContentDigest::default();
                    digest.push(&text)?;
                    let hash = digest.finish();
                    let (session, entry, reference_field, bytes) = (
                        session.to_owned(),
                        entry.clone(),
                        field.clone(),
                        text.len() as i64,
                    );
                    let linked=store.db.call_as("content_store", move|db| {
                        let tx=crate::session::database::begin_write_transaction(db)?;
                        let range=resolve(&tx,&session,"stream",&owner,"")?;
                        if let Some(range)=range {
                            let same:bool=tx.query_row("SELECT sealed=1 AND sha256=?2 AND byte_length=?3 FROM content_objects WHERE id=?1",params![range.id,hash,bytes],|row|row.get(0))?;
                            if same {
                                let encoding=if reference_field.ends_with("/args") {super::stream::Encoding::Json} else {super::stream::Encoding::Text};
                                super::bind_encoded(&tx,&session,owner_kind,&entry,&reference_field,range,encoding)?;
                                tx.commit()?;return Ok(true);
                            }
                        }
                        tx.commit()?;Ok(false)
                    })?;
                    if linked {
                        *value.pointer_mut(&field).expect("collected field exists") =
                            Value::String(String::new());
                        continue;
                    }
                }
            }
            stage_named(
                store,
                session,
                owner_kind,
                &entry,
                &field,
                &text,
                field.ends_with("/args"),
            )?;
            *value.pointer_mut(&field).expect("collected field exists") =
                Value::String(String::new());
        }
        for field in super::extensions::fields(&value) {
            if prelinked.contains_key(&field.pointer) {
                continue;
            }
            stage_named(
                store,
                session,
                owner_kind,
                &entry,
                &field.pointer,
                &field.body,
                field.json,
            )?;
            *value
                .pointer_mut(&field.pointer)
                .context("extension field disappeared while staging")? = if field.json {
                super::extensions::json_placeholder(
                    value.pointer(&field.pointer).expect("collected extension"),
                )
            } else {
                Value::String(field.preview.unwrap_or_default())
            };
        }
        if value["type"] == "session_info" {
            super::metadata::validate_thin(&value["content"])?;
        }
        prepared.push(value);
    }
    Ok(prepared)
}

/// Stage one immutable owned body without occupying the writer for its full size.
/// Both stream facts and transcript headers use this path, so sealed bodies can
/// be shared rather than copied when the final message arrives.
pub(crate) fn stage_named(
    store: &SqliteStore,
    session: &str,
    kind: &'static str,
    owner: &str,
    field: &str,
    text: &str,
    json: bool,
) -> Result<ContentRange> {
    let mut digest = ContentDigest::default();
    digest.push(text)?;
    let hash = digest.clone().finish();
    let bytes = digest.bytes();
    let semantic = if json {
        Some(super::json_digest(&serde_json::from_str::<Value>(text)?)?)
    } else {
        None
    };
    let json_seal = if json {
        Some(super::JsonContentDigest::verified(
            text,
            &serde_json::from_str::<Value>(text)?,
        )?)
    } else {
        None
    };
    let (session, owner, field) = (session.to_owned(), owner.to_owned(), field.to_owned());
    let scope = (session.clone(), owner.clone(), field.clone());
    let encoding = if json {
        super::stream::Encoding::Json
    } else {
        super::stream::Encoding::Text
    };
    let (body, mut offset, complete) = store.db.call_as("content_store", move |db| {
        let tx = crate::session::database::begin_write_transaction(db)?;
        let stored_session = crate::session::identity::session_key(&tx, &scope.0)?;
        let prior = super::resolve_encoded(&tx, &scope.0, kind, &scope.1, &scope.2)?;
        let result = if let Some((prior, prior_encoding)) = prior {
            ensure!(
                prior_encoding == encoding,
                "conflicting content field encoding"
            );
            let sealed: Option<String> = tx.query_row(OBJECT_SHA, [prior.id], |r| r.get(0))?;
            if kind == "entry" && sealed.is_none() {
                let owner_key = super::owner_key(&tx, &scope.0, kind, &scope.1)?;
                let published: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM entries WHERE session_id=?1 AND entry_id=?2)",
                    params![stored_session, &owner_key],
                    |row| row.get(0),
                )?;
                ensure!(
                    !published,
                    "published entry content is immutable; preserve its committed prefix"
                );
            }
            if json {
                let json_hash: Option<String> = tx.query_row(
                    "SELECT json_sha256 FROM content_objects WHERE id=?1",
                    [prior.id],
                    |r| r.get(0),
                )?;
                if let Some(json_hash) = json_hash {
                    ensure!(
                        Some(&json_hash) == semantic.as_ref() && sealed.is_some(),
                        "conflicting entry JSON identity"
                    );
                    tx.commit()?;
                    return Ok((prior.id, prior.length, true));
                }
            }
            ensure!(
                prior.offset == 0 && prior.length <= bytes,
                "conflicting entry content length"
            );
            if let Some(sealed) = &sealed {
                ensure!(
                    *sealed == hash && prior.length == bytes,
                    "conflicting entry content identity"
                );
            }
            (prior.id, prior.length, sealed.is_some())
        } else if let Some(range) = if json {
            find_json(&tx, semantic.as_ref().expect("JSON digest prepared"))?
                .or(find_sealed(&tx, &hash, bytes)?)
        } else {
            find_sealed(&tx, &hash, bytes)?
        } {
            super::bind_encoded(&tx, &scope.0, kind, &scope.1, &scope.2, range, encoding)?;
            (range.id, range.length, true)
        } else {
            let id = create(&tx)?;
            super::bind_encoded(
                &tx,
                &scope.0,
                kind,
                &scope.1,
                &scope.2,
                ContentRange {
                    id,
                    offset: 0,
                    length: 0,
                },
                encoding,
            )?;
            (id, 0, false)
        };
        tx.commit()?;
        Ok(result)
    })?;
    if complete {
        return Ok(ContentRange {
            id: body,
            offset: 0,
            length: offset,
        });
    }
    // A retry of staging an unsealed body must verify the prior prefix in
    // bounded read pages; a failed earlier append cannot authorize new bytes.
    let mut verified = 0usize;
    while verified < usize::try_from(offset)? {
        let start = verified;
        let budget = (usize::try_from(offset)? - verified).min(WRITE_BYTES);
        let page = store
            .db
            .read(move |db| super::read(db, body, start as i64, budget))?;
        ensure!(
            !page.text.is_empty()
                && text.get(start..start + page.text.len()) == Some(page.text.as_str()),
            "conflicting staged entry prefix"
        );
        verified += page.text.len();
    }
    while usize::try_from(offset)? < text.len() {
        let start = usize::try_from(offset)?;
        let mut end = (start + WRITE_BYTES).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let piece = text[start..end].to_owned();
        let scope = StagingOwner {
            session: session.clone(),
            kind,
            owner: owner.clone(),
            field: field.clone(),
            body,
            encoding,
        };
        let next = i64::try_from(end)?;
        store
            .db
            .call(move |db| append_piece(db, &scope, offset, &piece))?;
        offset = next;
    }
    store.db.call_as("content_store", move |db| {
        let tx = crate::session::database::begin_write_transaction(db)?;
        match json_seal {
            Some(verified) => {
                super::seal_json(&tx, body, verified)?;
            }
            None => {
                seal(&tx, body, digest)?;
            }
        }
        tx.commit()?;
        Ok(ContentRange {
            id: body,
            offset: 0,
            length: offset,
        })
    })
}

struct StagingOwner {
    session: String,
    kind: &'static str,
    owner: String,
    field: String,
    body: i64,
    encoding: super::stream::Encoding,
}

/// A repeated semantic boundary must retain the complete referenced-field set.
/// Placeholder equality cannot distinguish an old externalized array from a
/// new inline empty array. The header fingerprint is checked by insertion.
pub(crate) fn verify_boundary_retry(
    db: &rusqlite::Connection,
    session: &str,
    prefix: &str,
    entry: &Value,
) -> Result<()> {
    let id = entry["id"]
        .as_str()
        .context("boundary entry identity missing")?;
    let stored_id = crate::session::identity::BoundIdentity::entry(db, session, id)?;
    let stored_session = crate::session::identity::session_key(db, session)?;
    let existing: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM entries WHERE session_id=?1 AND entry_id=?2)",
        params![stored_session, stored_id],
        |row| row.get(0),
    )?;
    if !existing {
        return Ok(());
    }
    let mut query=db.prepare("SELECT field,content_id,byte_offset,byte_length,encoding FROM content_references WHERE session_id=?1 AND owner_kind=?2 AND owner_id=?3 ORDER BY field LIMIT 129")?;
    type BoundaryReference = (String, i64, i64, i64, String);
    let mut read = |kind: &str, owner: &str| -> Result<Vec<BoundaryReference>> {
        let owner = super::owner_key(db, session, kind, owner)?;
        Ok(query
            .query_map(params![stored_session, kind, owner], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    };
    let old = read("entry", id)?;
    let staged = read("checkpoint", &format!("{prefix}{id}"))?;
    ensure!(
        old.len() <= 128 && old == staged,
        "conflicting boundary content identity"
    );
    Ok(())
}

fn append_piece(
    db: &mut rusqlite::Connection,
    scope: &StagingOwner,
    offset: i64,
    piece: &str,
) -> Result<()> {
    let tx = crate::session::database::begin_write_transaction(db)?;
    let (current, encoding) =
        super::resolve_encoded(&tx, &scope.session, scope.kind, &scope.owner, &scope.field)?
            .context("staged body owner changed")?;
    ensure!(current.id == scope.body, "staged body owner changed");
    ensure!(
        encoding == scope.encoding,
        "staged content encoding changed"
    );
    let next = offset
        .checked_add(i64::try_from(piece.len())?)
        .context("staged content length overflow")?;
    if scope.kind == "entry" && next > current.length {
        let owner_key = super::owner_key(&tx, &scope.session, scope.kind, &scope.owner)?;
        let stored_session = crate::session::identity::session_key(&tx, &scope.session)?;
        let published: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM entries WHERE session_id=?1 AND entry_id=?2)",
            params![stored_session, &owner_key],
            |row| row.get(0),
        )?;
        ensure!(
            !published,
            "published entry content is immutable; preserve its committed prefix"
        );
    }
    append(&tx, scope.body, offset, piece)?;
    // An identical retry may have captured an older staging offset before a
    // concurrent retry finished/published. Its already-committed prefix is
    // idempotent; it must never move a visible reference length backwards.
    super::extend_reference(
        &tx,
        &scope.session,
        scope.kind,
        &scope.owner,
        &scope.field,
        scope.body,
        next,
    )?;
    tx.commit()?;
    Ok(())
}

/// Compatibility view: historical columns remain untouched; canonical bodies
/// are reconstructed only by readers, scoped through their owning entry.
pub(crate) const RESOLVED_BLOCKS_SQL: &str = r#"
DROP VIEW IF EXISTS resolved_message_blocks;
DROP VIEW IF EXISTS resolved_content_references;
CREATE VIEW resolved_content_references AS
WITH raw AS (
 SELECT r.*,coalesce((SELECT group_concat(piece,'') FROM (
 SELECT CAST(substr(CAST(s.body AS BLOB),max(1,r.byte_offset-s.byte_offset+1),
 min(length(CAST(s.body AS BLOB)),r.byte_offset+r.byte_length-s.byte_offset)-max(0,r.byte_offset-s.byte_offset)) AS TEXT) AS piece
 FROM content_segments s WHERE s.content_id=r.content_id
 AND s.byte_offset<r.byte_offset+r.byte_length AND s.byte_offset+length(CAST(s.body AS BLOB))>r.byte_offset ORDER BY s.byte_offset)),'') AS raw_body
 FROM content_references r
)
SELECT raw.*,CASE WHEN encoding='shell_summary' THEN future_shell_summary(raw_body)
 WHEN encoding='tool_args_string_v1' THEN future_tool_args_string_v1(raw_body)
 ELSE raw_body END AS body FROM raw;
CREATE VIEW resolved_message_blocks AS
SELECT b.session_id,b.entry_position,b.ordinal,b.kind,
coalesce(b.text,(SELECT r.body FROM resolved_content_references r
 WHERE r.session_id=b.session_id AND r.owner_kind='entry' AND r.owner_id=e.entry_id
 AND r.encoding IN ('text','shell_summary') AND r.field=printf('/content/%d/%s',b.ordinal,CASE WHEN b.kind='tool_result' THEN 'content' ELSE 'text' END))) AS text,
b.tool_call_id,b.tool_name,
CASE WHEN b.kind='tool_call' THEN coalesce(b.arguments_json,(SELECT r.body FROM resolved_content_references r
 WHERE r.session_id=b.session_id AND r.owner_kind='entry' AND r.owner_id=e.entry_id
 AND r.field=printf('/content/%d/args',b.ordinal))) ELSE b.arguments_json END AS arguments_json,
b.is_error,
CASE WHEN EXISTS(SELECT 1 FROM content_references r WHERE r.session_id=b.session_id AND r.owner_kind='entry' AND r.owner_id=e.entry_id
 AND r.field LIKE printf('/content/%d/%%',b.ordinal)
 AND (r.encoding='json' OR r.field<>printf('/content/%d/%s',b.ordinal,CASE WHEN b.kind='tool_result' THEN 'content' ELSE 'text' END))
 AND (b.kind IS NOT 'tool_call' OR r.field<>printf('/content/%d/args',b.ordinal)))
THEN future_restore_content_json(b.metadata_json,(SELECT json_group_array(json_object('field',substr(r.field,length(printf('/content/%d',b.ordinal))+1),'encoding',r.encoding,'body',r.body))
 FROM resolved_content_references r WHERE r.session_id=b.session_id AND r.owner_kind='entry' AND r.owner_id=e.entry_id
 AND r.field LIKE printf('/content/%d/%%',b.ordinal)
 AND (r.encoding='json' OR r.field<>printf('/content/%d/%s',b.ordinal,CASE WHEN b.kind='tool_result' THEN 'content' ELSE 'text' END))
 AND (b.kind IS NOT 'tool_call' OR r.field<>printf('/content/%d/args',b.ordinal))))
ELSE b.metadata_json END AS metadata_json
FROM message_blocks b JOIN entries e ON e.session_id=b.session_id AND e.position=b.entry_position;
"#;

/// The same lossless envelope with body slots left as their staging sentinels.
/// Forks read this metadata view, then share immutable references in bounded
/// transactions; no fork job reads or copies a parent's body on the writer.
pub(crate) fn header_views_sql() -> String {
    let mut sql = String::from(
        r#"
DROP VIEW IF EXISTS header_message_blocks;
DROP VIEW IF EXISTS header_content_references;
CREATE VIEW header_message_blocks AS
SELECT b.session_id,b.entry_position,b.ordinal,b.kind,
CASE WHEN b.text IS NULL AND EXISTS(SELECT 1 FROM content_references r WHERE r.session_id=b.session_id AND r.owner_kind='entry' AND r.owner_id=e.entry_id AND r.encoding IN ('text','shell_summary') AND r.field=printf('/content/%d/%s',b.ordinal,CASE WHEN b.kind='tool_result' THEN 'content' ELSE 'text' END)) THEN '' ELSE b.text END AS text,
b.tool_call_id,b.tool_name,
CASE WHEN b.arguments_json IS NULL AND EXISTS(SELECT 1 FROM content_references r WHERE r.session_id=b.session_id AND r.owner_kind='entry' AND r.owner_id=e.entry_id AND r.field=printf('/content/%d/args',b.ordinal)) THEN '""' ELSE b.arguments_json END AS arguments_json,
b.is_error,b.metadata_json FROM message_blocks b JOIN entries e ON e.session_id=b.session_id AND e.position=b.entry_position;
"#,
    );
    // Reuse the relational assembly, excluding the final body-restoration
    // SELECT. Header-only operations retain typed staging sentinels/previews.
    let (assembly, _) = crate::session::entry_records::ENTRY_RECORD_VIEWS_SQL
        .split_once("\nSELECT a.session_id,a.position,a.entry_id,a.entry_type,\n")
        .expect("entry view has a final restoration select");
    sql.push_str(
        &assembly
            .replace("entry_records", "entry_header_records")
            .replace("block_records", "block_header_records")
            .replace("resolved_message_blocks", "header_message_blocks"),
    );
    sql.push_str(
        "\nSELECT a.session_id,a.position,a.entry_id,a.entry_type,a.payload FROM assembled a;\n",
    );
    crate::session::entry_records::render_id_expressions(&sql)
}

#[cfg(test)]
#[path = "entry_staging_tests.rs"]
mod staging_tests;
