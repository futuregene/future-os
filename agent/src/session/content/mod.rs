//! Canonical UTF-8 content shared by transcripts, tools and stream ranges.
//!
//! Every mutation takes the caller's transaction: a segment, its owner and its
//! durable watermark cannot commit separately. No API concatenates or rewrites
//! a complete growing body. Only bounded ranges are materialized by readers.
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};

pub(crate) mod checkpoint;
mod checkpoint_retirement;
pub(crate) mod compact;
pub(crate) mod entries;
mod entry_stages;
mod extensions;
pub(crate) mod fork;
pub(crate) mod journal;
pub(crate) mod metadata;
pub(crate) mod operations;
pub(crate) mod reader;
mod recovery;
pub(crate) mod reset;
pub(crate) mod stream;
pub(crate) mod stream_entry;
mod symbols;
pub(crate) mod terminal;
pub(crate) mod transport;

pub(crate) const FORMAT_VERSION: i64 = 2;
// Only the transaction inserting a genuinely new conversation selects this
// default. Existing rows and import receipts retain their original format.
pub(crate) const NEW_SESSION_FORMAT: i64 = FORMAT_VERSION;
pub(crate) const SEGMENT_BYTES: usize = 16 * 1024;
pub(crate) const WRITE_BYTES: usize = 64 * 1024;
pub(crate) const READ_BYTES: usize = 256 * 1024;

pub(crate) use checkpoint_retirement::advance as advance_history;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContentRange {
    pub id: i64,
    pub offset: i64,
    pub length: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ContentPage {
    pub text: String,
    pub next_offset: i64,
    pub total_bytes: i64,
    pub sealed: bool,
}

/// Private incremental state belongs to the single ordered stream producer.
/// It observes each accepted byte once, never rehashing a growing prefix.
#[derive(Clone, Default)]
pub(crate) struct ContentDigest {
    hash: Sha256,
    bytes: i64,
}

impl ContentDigest {
    pub(crate) fn push(&mut self, text: &str) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_add(i64::try_from(text.len())?)
            .context("content length overflow")?;
        self.hash.update(text.as_bytes());
        Ok(())
    }

    pub(crate) fn bytes(&self) -> i64 {
        self.bytes
    }

    pub(crate) fn finish(self) -> String {
        format!("{:x}", self.hash.finalize())
    }
}

pub(crate) fn format(db: &Connection, session: &str) -> Result<i64> {
    let stored = super::identity::session_key(db, session)?;
    Ok(db
        .query_row(
            "SELECT format FROM session_formats WHERE session_id=?1",
            [stored],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(1))
}

/// Called only in the transaction creating a session, never on a legacy open.
pub(crate) fn enable(tx: &Transaction<'_>, session: &str) -> Result<()> {
    let stored = super::identity::session_key(tx, session)?;
    let has_history: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM entries WHERE session_id=?1)",
        [&stored],
        |r| r.get(0),
    )?;
    ensure!(
        !has_history,
        "cannot change an existing conversation's storage format"
    );
    tx.execute(
        "INSERT INTO session_formats(session_id,format) VALUES (?1,?2)",
        params![stored, FORMAT_VERSION],
    )?;
    Ok(())
}

pub(crate) fn create(tx: &Transaction<'_>) -> Result<i64> {
    tx.execute("INSERT INTO content_objects DEFAULT VALUES", [])?;
    Ok(tx.last_insert_rowid())
}

/// Retry with an already committed range is idempotent. A conflicting overlap
/// is rejected; append never guesses whether different text supersedes data.
pub(crate) fn append(
    tx: &Transaction<'_>,
    id: i64,
    offset: i64,
    text: &str,
) -> Result<ContentRange> {
    ensure!(
        offset >= 0 && text.len() <= WRITE_BYTES,
        "invalid content append budget"
    );
    let (length, sealed): (i64, bool) = tx.query_row(
        "SELECT byte_length,sealed FROM content_objects WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let added = i64::try_from(text.len())?;
    let end = offset
        .checked_add(added)
        .context("content offset overflow")?;
    if offset < length || (sealed && end <= length) {
        ensure!(end <= length, "content append overlaps the committed tail");
        let existing = read(tx, id, offset, text.len())?;
        ensure!(existing.text == text, "conflicting content append");
        return Ok(ContentRange {
            id,
            offset,
            length: added,
        });
    }
    ensure!(
        !sealed && offset == length,
        "content is sealed or append has a gap"
    );
    let mut written = 0;
    while written < text.len() {
        let mut end = (written + SEGMENT_BYTES).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        tx.execute(
            "INSERT INTO content_segments(content_id,byte_offset,body) VALUES (?1,?2,?3)",
            params![id, offset + i64::try_from(written)?, &text[written..end]],
        )?;
        written = end;
    }
    tx.execute(
        "UPDATE content_objects SET byte_length=?2 WHERE id=?1",
        params![id, end],
    )?;
    Ok(ContentRange {
        id,
        offset,
        length: added,
    })
}

/// The producer's digest must cover precisely the durable body. Sealing only
/// changes metadata; it never reads, packs, copies or deletes body segments.
pub(crate) fn seal(tx: &Transaction<'_>, id: i64, digest: ContentDigest) -> Result<ContentRange> {
    let bytes = digest.bytes();
    let hash = digest.finish();
    let changed = tx.execute(
        "UPDATE content_objects SET sealed=1,sha256=?3 WHERE id=?1 AND byte_length=?2 AND sealed=0",
        params![id, bytes, hash],
    )?;
    if changed == 0 {
        let existing: (i64, Option<String>) = tx.query_row(
            "SELECT byte_length,sha256 FROM content_objects WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            existing == (bytes, Some(hash)),
            "content seal does not match durable bytes"
        );
    }
    Ok(ContentRange {
        id,
        offset: 0,
        length: bytes,
    })
}

pub(crate) fn find_sealed(
    db: &Connection,
    sha256: &str,
    bytes: i64,
) -> Result<Option<ContentRange>> {
    let id: Option<i64>=db.query_row("SELECT id FROM content_objects WHERE sealed=1 AND sha256=?1 AND byte_length=?2 ORDER BY id LIMIT 1",params![sha256,bytes],|r|r.get(0)).optional()?;
    Ok(id.map(|id| ContentRange {
        id,
        offset: 0,
        length: bytes,
    }))
}

/// Construct outside the database writer, using the exact streamed bytes.
/// A provider's revised final arguments must never alias different raw input.
pub(crate) struct JsonContentDigest {
    raw: ContentDigest,
    canonical: String,
}

impl JsonContentDigest {
    pub(crate) fn verified(raw: &str, value: &serde_json::Value) -> Result<Self> {
        ensure!(
            serde_json::from_str::<serde_json::Value>(raw)? == *value,
            "streamed and final JSON arguments differ"
        );
        let mut digest = ContentDigest::default();
        digest.push(raw)?;
        Ok(Self {
            raw: digest,
            canonical: json_digest(value)?,
        })
    }
}

/// A deterministic semantic JSON hash. Keys are ordered without cloning any
/// value/body; primitive JSON encoding streams directly into SHA-256.
pub(crate) fn json_digest(value: &serde_json::Value) -> Result<String> {
    struct HashWriter(Sha256);
    impl std::io::Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn write(value: &serde_json::Value, out: &mut HashWriter) -> Result<()> {
        use std::io::Write;
        match value {
            serde_json::Value::Object(object) => {
                out.write_all(b"{")?;
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort_unstable();
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        out.write_all(b",")?;
                    }
                    serde_json::to_writer(&mut *out, key)?;
                    out.write_all(b":")?;
                    write(&object[key], out)?;
                }
                out.write_all(b"}")?;
            }
            serde_json::Value::Array(values) => {
                out.write_all(b"[")?;
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.write_all(b",")?;
                    }
                    write(value, out)?;
                }
                out.write_all(b"]")?;
            }
            _ => serde_json::to_writer(out, value)?,
        }
        Ok(())
    }
    let mut output = HashWriter(Sha256::new());
    write(value, &mut output)?;
    Ok(format!("{:x}", output.0.finalize()))
}

/// Parsed arguments can differ in insignificant JSON whitespace from streamed
/// bytes. Record a verified semantic digest without copying or rewriting them.
pub(crate) fn seal_json(
    tx: &Transaction<'_>,
    id: i64,
    digest: JsonContentDigest,
) -> Result<ContentRange> {
    let range = seal(tx, id, digest.raw)?;
    let existing: Option<String> = tx.query_row(
        "SELECT json_sha256 FROM content_objects WHERE id=?1",
        [id],
        |r| r.get(0),
    )?;
    if let Some(existing) = existing {
        ensure!(
            existing == digest.canonical,
            "conflicting semantic JSON digest"
        );
    } else {
        tx.execute(
            "UPDATE content_objects SET json_sha256=?2 WHERE id=?1 AND sealed=1",
            params![id, digest.canonical],
        )?;
    }
    Ok(range)
}

pub(crate) fn find_json(db: &Connection, sha256: &str) -> Result<Option<ContentRange>> {
    Ok(db.query_row("SELECT id,byte_length FROM content_objects WHERE sealed=1 AND json_sha256=?1 ORDER BY id LIMIT 1",[sha256],|r|Ok(ContentRange{id:r.get(0)?,offset:0,length:r.get(1)?})).optional()?)
}

pub(crate) fn bind(
    tx: &Transaction<'_>,
    session: &str,
    kind: &str,
    owner: &str,
    field: &str,
    range: ContentRange,
) -> Result<()> {
    bind_encoded(
        tx,
        session,
        kind,
        owner,
        field,
        range,
        stream::Encoding::Text,
    )
}

/// Entry owners share the authoritative entry key representation. Stream and
/// operation staging names remain opaque until their structured owner adapter.
pub(crate) fn owner_key(
    db: &Connection,
    session: &str,
    kind: &str,
    owner: &str,
) -> Result<super::identity::BoundIdentity> {
    if kind == "entry" {
        super::identity::BoundIdentity::entry(db, session, owner)
    } else {
        Ok(super::identity::BoundIdentity::from_stored(
            super::identity::StoredIdentity::Legacy(owner.into()),
        ))
    }
}

pub(crate) fn bind_encoded(
    tx: &Transaction<'_>,
    session: &str,
    kind: &str,
    owner: &str,
    field: &str,
    range: ContentRange,
    encoding: stream::Encoding,
) -> Result<()> {
    let stored_session = super::identity::session_key(tx, session)?;
    let stored_owner = owner_key(tx, session, kind, owner)?;
    ensure!(
        range.offset >= 0 && range.length >= 0,
        "invalid content reference"
    );
    let total: i64 = tx.query_row(
        "SELECT byte_length FROM content_objects WHERE id=?1",
        [range.id],
        |r| r.get(0),
    )?;
    let end = range
        .offset
        .checked_add(range.length)
        .context("content range overflow")?;
    ensure!(end <= total, "content reference exceeds durable bytes");
    let old: Option<(i64,i64,i64,String)>=tx.query_row("SELECT content_id,byte_offset,byte_length,encoding FROM content_references WHERE session_id=?1 AND owner_kind=?2 AND owner_id=?3 AND field=?4",params![&stored_session,kind,&stored_owner,field],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    if let Some(old) = old {
        ensure!(
            old == (
                range.id,
                range.offset,
                range.length,
                encoding.name().to_owned()
            ),
            "conflicting content reference"
        );
    } else {
        tx.execute("INSERT INTO content_references(session_id,owner_kind,owner_id,field,content_id,byte_offset,byte_length,encoding) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",params![&stored_session,kind,&stored_owner,field,range.id,range.offset,range.length,encoding.name()])?;
    }
    Ok(())
}

/// Scope the reference lookup before reading a shared object. Raw object IDs
/// are internal and never sufficient authority for an RPC content read.
pub(crate) fn resolve(
    db: &Connection,
    session: &str,
    kind: &str,
    owner: &str,
    field: &str,
) -> Result<Option<ContentRange>> {
    let stored_session = super::identity::session_key(db, session)?;
    let stored_owner = owner_key(db, session, kind, owner)?;
    Ok(db.query_row("SELECT content_id,byte_offset,byte_length FROM content_references WHERE session_id=?1 AND owner_kind=?2 AND owner_id=?3 AND field=?4",params![stored_session,kind,stored_owner,field],|r|Ok(ContentRange{id:r.get(0)?,offset:r.get(1)?,length:r.get(2)?})).optional()?)
}

pub(crate) fn read(db: &Connection, id: i64, offset: i64, max_bytes: usize) -> Result<ContentPage> {
    ensure!(
        offset >= 0 && max_bytes <= READ_BYTES,
        "invalid content read budget"
    );
    let (total, sealed): (i64, bool) = db.query_row(
        "SELECT byte_length,sealed FROM content_objects WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure!(offset <= total, "content read starts past durable bytes");
    let end = offset.saturating_add(i64::try_from(max_bytes)?).min(total);
    let mut stmt=db.prepare("SELECT byte_offset,body FROM content_segments WHERE content_id=?1 AND byte_offset>=?2 AND byte_offset<?3 ORDER BY byte_offset")?;
    let mut rows = stmt.query(params![
        id,
        offset.saturating_sub(i64::try_from(SEGMENT_BYTES)?).max(0),
        end
    ])?;
    let mut text = String::with_capacity(usize::try_from(end - offset)?);
    let mut next = offset;
    let mut utf8_cut = false;
    while let Some(row) = rows.next()? {
        let at: i64 = row.get(0)?;
        let body: String = row.get(1)?;
        let last = at + i64::try_from(body.len())?;
        if last <= next {
            continue;
        }
        ensure!(at <= next, "stored content has a gap");
        let from = usize::try_from(next - at)?;
        ensure!(
            body.is_char_boundary(from),
            "content offset is not a UTF-8 boundary"
        );
        let requested = usize::try_from((end - at).min(i64::try_from(body.len())?))?;
        let mut to = requested;
        while !body.is_char_boundary(to) {
            to -= 1;
        }
        text.push_str(&body[from..to]);
        next = at + i64::try_from(to)?;
        if next < last {
            utf8_cut = to < requested;
            break;
        }
    }
    if next < end && !utf8_cut {
        bail!("stored content is missing its durable tail");
    }
    Ok(ContentPage {
        text,
        next_offset: next,
        total_bytes: total,
        sealed,
    })
}

/// Delete only caller-identified candidates after removing their references.
/// Never sweep or repack historical content on open, idle or run completion.
pub(crate) fn collect(tx: &Transaction<'_>, candidates: &[i64]) -> Result<usize> {
    ensure!(
        candidates.len() <= 256,
        "content collection batch is too large"
    );
    let mut deleted = 0;
    let mut remaining_segments = 4i64;
    for id in candidates {
        tx.execute("INSERT OR IGNORE INTO content_collection(content_id) SELECT id FROM content_objects WHERE id=?1",[id])?;
        let shared:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM content_references WHERE content_id=?1) OR EXISTS(SELECT 1 FROM stream_content_ranges WHERE content_id=?1)",[id],|r|r.get(0))?;
        if shared {
            tx.execute("DELETE FROM content_collection WHERE content_id=?1", [id])?;
            continue;
        }
        if remaining_segments == 0 {
            continue;
        }
        // Stop digest reuse before deleting any segments. The object has no
        // owners; a later append cannot resurrect a partially collected body.
        tx.execute(
            "UPDATE content_objects SET sealed=0,sha256=NULL,json_sha256=NULL WHERE id=?1",
            [id],
        )?;
        let removed=tx.execute("DELETE FROM content_segments WHERE content_id=?1 AND byte_offset IN (SELECT byte_offset FROM content_segments WHERE content_id=?1 ORDER BY byte_offset LIMIT ?2)",params![id,remaining_segments])?;
        remaining_segments -= i64::try_from(removed)?;
        deleted+=tx.execute("DELETE FROM content_objects WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM content_segments WHERE content_id=?1)",[id])?;
    }
    Ok(deleted)
}

/// One idle slice processes only explicitly queued candidates. It does not scan
/// bodies, and frees at most 64 KiB of segment payload per transaction.
pub(crate) fn reclaim(db: &mut Connection) -> Result<bool> {
    let retired_done = reset::reclaim_retired(db)?;
    let checkpoints_done = checkpoint_retirement::reclaim(db)?;
    let ids = {
        let mut stmt =
            db.prepare("SELECT content_id FROM content_collection ORDER BY content_id LIMIT 16")?;
        let rows = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        rows
    };
    if ids.is_empty() {
        return Ok(retired_done && checkpoints_done);
    }
    crate::session::database::try_maintenance_transaction(db, |tx| {
        collect(tx, &ids)?;
        Ok(())
    })?;
    Ok(retired_done
        && checkpoints_done
        && !db.query_row("SELECT EXISTS(SELECT 1 FROM content_collection)", [], |r| {
            r.get::<_, bool>(0)
        })?)
}

#[cfg(test)]
mod tests;

/// A pure projection registered on every application connection. It reads no
/// database state and never changes the canonical shell execution facts.
pub(crate) fn install_functions(db: &Connection) -> Result<()> {
    use rusqlite::functions::FunctionFlags;
    db.create_scalar_function(
        "future_shell_summary",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| {
            let raw: String = context.get(0)?;
            let result: future_rpc::shell_result::ShellResult = serde_json::from_str(&raw)
                .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
            Ok(future_rpc::shell_result::model_summary(&result))
        },
    )?;
    db.create_scalar_function(
        "future_tool_args_string_v1",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| {
            let raw: String = context.get(0)?;
            let mut value = serde_json::Value::String(raw);
            future_rpc::tool_arguments::repair_string_v1(&mut value);
            serde_json::to_string(&value)
                .map_err(|error| rusqlite::Error::UserFunctionError(error.into()))
        },
    )?;
    db.create_scalar_function(
        "future_restore_content_json",
        2,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| {
            let document: String = context.get(0)?;
            let patches: String = context.get(1)?;
            super::content::extensions::restore(&document, &patches)
                .map_err(|error| rusqlite::Error::UserFunctionError(error.into()))
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod acceptance_tests;
#[cfg(test)]
mod extension_tests;
#[cfg(test)]
mod journal_tests;
#[cfg(test)]
mod stream_entry_tests;

#[cfg(test)]
mod native_identity_tests;
