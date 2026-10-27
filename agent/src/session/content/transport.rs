//! Scoped, bounded transport of canonical event bodies. Readers resolve the
//! immutable semantic reference before touching a content object.
use super::{stream::Encoding, ContentRange, READ_BYTES};
use crate::session::{sqlite_store::SqliteStore, Manager};
use anyhow::{ensure, Context, Result};
use future_rpc::content_range::{ContentRequest, StorageIdentity};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) const REPLAY_PAGE_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const EVENT_WIRE_OVERHEAD: usize = 320;

pub(crate) const INLINE_EVENT_BYTES: usize = 64 * 1024;
pub(crate) const INLINE_FIELD_BYTES: usize = 16 * 1024;
pub(crate) const LIVE_TEXT_BYTES: usize = 4 * 1024;

/// Construct the bounded preview before acquiring the broadcast sequencing
/// lock. Large JSON fields are represented explicitly, never silently clipped.
pub(crate) fn prepare_wire_data(data: &Value, event_type: &str) -> Value {
    let Some(fields) = data.as_object() else {
        return data.clone();
    };
    let mut result = serde_json::Map::new();
    let mut references = serde_json::Map::new();
    let mut inline_bytes = 0usize;
    for (field, value) in fields {
        let size = value
            .as_str()
            .map(str::len)
            .unwrap_or_else(|| value.to_string().len());
        if size > INLINE_FIELD_BYTES
            || inline_bytes.saturating_add(size.saturating_mul(6) + field.len())
                > INLINE_EVENT_BYTES / 2
        {
            references.insert(
                field.clone(),
                json!({"field":format!("/data/{}",field.replace('~',"~0").replace('/',"~1"))}),
            );
            let preview = if field == "tool_args" {
                argument_preview(value)
            } else if value.is_string() {
                json!("")
            } else {
                Value::Null
            };
            result.insert(field.clone(), preview);
        } else {
            inline_bytes = inline_bytes.saturating_add(size.saturating_mul(6) + field.len());
            result.insert(field.clone(), value.clone());
        }
    }
    if matches!(event_type, "tool_end" | "tool_result") {
        let error = event_is_error(data);
        result.insert("is_error".into(), json!(error));
    }
    if !references.is_empty() {
        result.insert("content_refs".into(), Value::Object(references));
    }
    Value::Object(result)
}
pub(crate) fn event_is_error(data: &Value) -> bool {
    data["is_error"]
        .as_bool()
        .or_else(|| data["shell_result"]["is_error"].as_bool())
        .unwrap_or_else(|| {
            data["error"]
                .as_str()
                .is_some_and(|error| !error.is_empty())
                || (data["exit_code"].as_i64().is_some_and(|code| code != 0)
                    && data["is_soft_fail"] != true)
        })
}

fn argument_preview(value: &Value) -> Value {
    let Some(fields) = value.as_object() else {
        return json!("");
    };
    let mut preview = serde_json::Map::new();
    let mut remaining = 2048usize;
    for field in ["path", "file_path", "filePath", "target_path", "cwd"] {
        if let Some(value) = fields
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| value.len() <= remaining)
        {
            remaining -= value.len();
            preview.insert(field.into(), json!(value));
        }
    }
    Value::Object(preview)
}

/// Add only coordinator-owned small fields to an already bounded preview.
pub(crate) fn stamp_wire_data(
    data: &mut Value,
    source: &Value,
    identity: &StorageIdentity,
    epoch: i64,
    cursor: i64,
) {
    data["content_format"] = json!(super::FORMAT_VERSION);
    for field in [
        "entry_id",
        "content_key",
        "content_offset",
        "content_bytes",
        "content_first_idx",
        "content_seal",
    ] {
        if let Some(value) = source.get(field) {
            data[field] = value.clone();
        }
    }
    if let Some(references) = data.get_mut("content_refs").and_then(Value::as_object_mut) {
        for value in references.values_mut() {
            value["databaseId"] = json!(identity.database_id);
            value["restoreGeneration"] = json!(identity.restore_generation);
            value["epoch"] = json!(epoch);
            value["cursor"] = json!(cursor);
            value["offset"] = json!(0);
            value["maxBytes"] = json!(64 * 1024);
        }
    }
}

impl Manager {
    pub(crate) fn event_content_range(
        &self,
        session: &str,
        run: &str,
        request: ContentRequest,
    ) -> Result<Value> {
        self.storage()?.event_content_range(session, run, request)
    }
}
impl SqliteStore {
    pub(crate) fn storage_identity(&self) -> Result<StorageIdentity> {
        self.db.read(|db| storage_identity(db))
    }

    pub(crate) fn event_content_range(
        &self,
        session: &str,
        run: &str,
        request: ContentRequest,
    ) -> Result<Value> {
        ensure!(
            !session.is_empty() && request.cursor >= 0 && request.epoch >= -1,
            "invalid content scope"
        );
        ensure!(
            request.offset >= 0 && (1..=READ_BYTES as i64).contains(&request.max_bytes),
            "invalid content page budget"
        );
        ensure!(
            request.field.starts_with("/data/") && request.field.len() <= 512,
            "invalid content field"
        );
        let (session, run) = (session.to_owned(), run.to_owned());
        let (s, r, scope) = (session.clone(), run.clone(), request.clone());
        let (range, encoding) = self.db.read(move |db| {
            let tx = db.transaction()?;
            let resolved = resolve_scoped(&tx, &s, &r, &scope)?;
            tx.commit()?;
            Ok(resolved)
        })?;
        let tag = format!(
            "{:x}",
            Sha256::digest(
                format!(
                    "{}:{}:{}:{}:{}:{}",
                    request.database_id,
                    request.restore_generation,
                    range.id,
                    range.offset,
                    range.length,
                    request.field
                )
                .as_bytes()
            )
        );
        let (text, next, total, encoding) = match encoding {
            Encoding::Text | Encoding::Json => {
                ensure!(
                    request.offset <= range.length,
                    "content offset exceeds published range"
                );
                let offset = range.offset + request.offset;
                let budget = (range.length - request.offset).min(request.max_bytes) as usize;
                let page = self.read_scoped_page(
                    &session,
                    &run,
                    &request,
                    (range, encoding),
                    offset,
                    budget,
                )?;
                let next = page.next_offset - range.offset;
                (
                    page.text,
                    next,
                    range.length,
                    if encoding == Encoding::Json {
                        "json"
                    } else {
                        "text"
                    },
                )
            }
            Encoding::ToolArgumentsStringV1 => {
                anyhow::bail!("entry-only encoding cannot be used by a stream content request");
            }
            Encoding::ShellSummary => {
                let mut raw = String::new();
                let end = range
                    .offset
                    .checked_add(range.length)
                    .context("content range overflow")?;
                let mut offset = range.offset;
                while offset < end {
                    let page = self.read_scoped_page(
                        &session,
                        &run,
                        &request,
                        (range, encoding),
                        offset,
                        (end - offset).min(READ_BYTES as i64) as usize,
                    )?;
                    ensure!(!page.text.is_empty(), "canonical facts range is incomplete");
                    offset = page.next_offset;
                    raw.push_str(&page.text);
                }
                let summary = future_rpc::shell_result::model_summary(&serde_json::from_str(&raw)?);
                let offset = usize::try_from(request.offset)?;
                ensure!(
                    offset <= summary.len() && summary.is_char_boundary(offset),
                    "invalid summary content offset"
                );
                let end = summary
                    .floor_char_boundary((offset + request.max_bytes as usize).min(summary.len()));
                (
                    summary[offset..end].to_owned(),
                    end as i64,
                    summary.len() as i64,
                    "text",
                )
            }
        };
        ensure!(
            next > request.offset || next == total,
            "content page budget cannot fit a UTF-8 character"
        );
        Ok(
            json!({"text":text,"nextOffset":next,"totalBytes":total,"hasMore":next<total,"encoding":encoding,"etag":tag,"databaseId":request.database_id,"restoreGeneration":request.restore_generation}),
        )
    }
    fn read_scoped_page(
        &self,
        session: &str,
        run: &str,
        request: &ContentRequest,
        (expected, encoding): (ContentRange, Encoding),
        offset: i64,
        budget: usize,
    ) -> Result<super::ContentPage> {
        let (session, run, request) = (session.to_owned(), run.to_owned(), request.clone());
        self.db.read(move |db| {
            let tx = db.transaction()?;
            let current = resolve_scoped(&tx, &session, &run, &request)?;
            ensure!(
                current == (expected, encoding),
                "event content reference changed while reading"
            );
            let page = super::read(&tx, expected.id, offset, budget)?;
            tx.commit()?;
            Ok(page)
        })
    }
}

/// Resolve and read under the same read transaction: deletion/collection may
/// recycle an object ID between jobs, but cannot authorize a different body.
fn resolve_scoped(
    db: &rusqlite::Connection,
    session: &str,
    run: &str,
    scope: &ContentRequest,
) -> Result<(ContentRange, Encoding)> {
    let identity = storage_identity(db)?;
    ensure!(
        identity.database_id == scope.database_id
            && identity.restore_generation == scope.restore_generation,
        "content database/restore identity changed"
    );
    ensure!(
        super::format(db, session)? == super::FORMAT_VERSION,
        "conversation does not support canonical content ranges"
    );
    let row: Option<(i64,i64,i64,String)> = db.query_row(
        "SELECT b.content_id,b.byte_offset,b.byte_length,b.encoding FROM stream_runs r JOIN stream_changes c ON c.stream_id=r.id JOIN stream_content_ranges b ON b.change_id=c.id WHERE r.session_id=?1 AND r.run_id=?2 AND r.epoch=?3 AND c.last_cursor=?4 AND c.last_cursor<=r.durable_cursor AND b.field=?5 AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=r.id)",
        params![session,run,scope.epoch,scope.cursor,scope.field], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
    let (id, offset, length, encoding) =
        row.context("event content is not committed or no longer exists")?;
    Ok((
        ContentRange { id, offset, length },
        Encoding::parse(&encoding)?,
    ))
}
pub(crate) fn storage_identity(db: &rusqlite::Connection) -> Result<StorageIdentity> {
    Ok(StorageIdentity {
        database_id: db.query_row(
            "SELECT value FROM storage_meta WHERE key='database_id'",
            [],
            |row| row.get(0),
        )?,
        restore_generation: db.query_row(
            "SELECT value FROM storage_meta WHERE key='restore_generation'",
            [],
            |row| row.get(0),
        )?,
    })
}
