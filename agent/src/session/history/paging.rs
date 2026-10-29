//! Bounded display-page IO and lossless history-entry fragments.
use super::display_projection::project_entries;
use super::fragment_cache::{self, Encoded, FragmentCache};
use crate::session::{content::reader, sqlite_store::SqliteStore};
use crate::session::{projection::hydrate_entry_projections, Manager, SessionEntry};
use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub(super) const HISTORY_PAGE_BYTE_BUDGET: usize = 8 * 1024 * 1024;
// JSON escaping of a fragment can expand each byte up to six times.
const ENTRY_FRAGMENT_BYTE_BUDGET: usize = 1024 * 1024;

pub(super) struct DisplayPageRow {
    ordinal: i64,
    index_payload: String,
    entry_payload: Option<String>,
    canonical_entry: Option<reader::Entry>,
}

pub(super) struct DisplayHistoryPage {
    pub(super) rows: Vec<DisplayPageRow>,
    has_more: bool,
    next_offset: i64,
    snapshot: Option<reader::Snapshot>,
}

pub(super) fn read_display_page(
    db: &Connection,
    session: &str,
    before: Option<i64>,
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<DisplayHistoryPage> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    let canonical =
        crate::session::content::format(db, session)? == crate::session::content::FORMAT_VERSION;
    let snapshot = canonical
        .then(|| reader::Snapshot::capture_from(db, session))
        .transpose()?;
    let total: i64 = db.query_row(
        "SELECT COALESCE(MAX(ordinal)+1,0) FROM history_display WHERE session_id=?1",
        [&stored_session],
        |r| r.get(0),
    )?;
    let (start, end) = if let Some(before) = before {
        let end = before.clamp(0, total);
        let count = limit.unwrap_or(10).clamp(1, 100);
        let start: i64 = db.query_row("SELECT COALESCE((SELECT ordinal FROM history_display WHERE session_id=?1 AND is_user=1 AND ordinal<?2 ORDER BY ordinal DESC LIMIT 1 OFFSET ?3),0)",params![&stored_session,end,count-1],|r|r.get(0))?;
        (start, end)
    } else {
        let start = offset.unwrap_or(0).clamp(0, total);
        (
            start,
            (start + limit.unwrap_or(250).clamp(1, 1000)).min(total),
        )
    };
    let direction = if before.is_some() { "DESC" } else { "ASC" };
    // The canonical branch must never touch the compatibility body views:
    // even asking SQLite for a sliced payload first assembles their full JSON.
    let sql = if canonical {
        let outcome = "json_object('status',CASE r.status WHEN 'error' THEN 'failed' WHEN 'incomplete' THEN 'interrupted' WHEN 'interrupted_by_restart' THEN 'interrupted' ELSE r.status END,'error',r.error,'durationMs',r.duration_ms)";
        format!("SELECT d.ordinal,
        CASE WHEN r.run_id IS NOT NULL THEN
          CASE WHEN header.entry_type='assistant' AND header.position=(SELECT max(a.position) FROM entries a WHERE a.session_id=header.session_id AND a.run_id=header.run_id AND a.entry_type='assistant')
          THEN json_set(d.payload,'$.run',{outcome},'$.usage',json_object('inputTokens',r.input_tokens,'outputTokens',r.output_tokens,'cacheReadTokens',r.cache_read_tokens,'cacheWriteTokens',r.cache_write_tokens))
          ELSE json_set(d.payload,'$.run',{outcome}) END
        ELSE d.payload END,NULL,header.position,header.entry_id
        FROM history_display d
        LEFT JOIN entries header ON header.session_id=d.session_id AND header.position=d.source_position
        LEFT JOIN runs r ON r.session_id=header.session_id AND r.run_id=header.run_id
          AND EXISTS(SELECT 1 FROM entries terminal WHERE terminal.session_id=header.session_id AND terminal.run_id=header.run_id AND terminal.entry_type='run_terminal')
        WHERE d.session_id=?1 AND d.ordinal>=?2 AND d.ordinal<?3 ORDER BY d.ordinal {direction}")
    } else {
        format!("SELECT d.ordinal,d.payload,e.payload,NULL,NULL
        FROM history_display d LEFT JOIN entry_records e ON e.session_id=d.session_id AND e.position=d.source_position
        WHERE d.session_id=?1 AND d.ordinal>=?2 AND d.ordinal<?3 ORDER BY d.ordinal {direction}")
    };
    // A backward page is bounded by the *exchange* window its caller named:
    // `start` is the oldest requested user turn, so cutting the range by a row
    // count hands back fewer exchanges than were asked for. One busy run is
    // dozens of rows on its own (an assistant step and a tool entry per call), so
    // a fixed row ceiling made a page that should hold ten exchanges hold nothing
    // but the newest reply — to the reader, the conversation above it was gone.
    //
    // Forward pages keep the ceiling: their `limit` counts rows and the client
    // resumes them through `nextOffset`.
    let sql = if canonical && before.is_none() {
        format!("{sql} LIMIT 64")
    } else {
        sql
    };
    // Canonical rows carry no bodies — the text a row ships is read at
    // `materialize_display_page` — so their payload sizes say nothing about what
    // the page will weigh. With the row ceiling gone for backward reads the page
    // is bounded where the bodies are: that materializer's byte budget, which
    // splits an oversized entry into fragments instead of dropping it.
    let read_budget = HISTORY_PAGE_BYTE_BUDGET;
    let mut stmt = db.prepare(&sql)?;
    let mut cursor = stmt.query(params![&stored_session, start, end])?;
    let mut rows = Vec::new();
    let mut body_bytes = 0usize;
    let mut next = if before.is_some() { end } else { start };
    while let Some(row) = cursor.next()? {
        let ordinal: i64 = row.get(0)?;
        let overlay: String = row.get(1)?;
        let body: Option<String> = row.get(2)?;
        let canonical_entry = if let Some(position) = row.get::<_, Option<i64>>(3)? {
            Some(reader::Entry {
                position,
                id: row.get(4)?,
            })
        } else {
            None
        };
        let size = overlay
            .len()
            .saturating_add(body.as_ref().map_or(0, String::len))
            .saturating_add(
                canonical_entry
                    .as_ref()
                    .map_or(0, |entry| entry.id.len() + 32),
            );
        if !rows.is_empty() && body_bytes.saturating_add(size) > read_budget {
            break;
        }
        anyhow::ensure!(
            !canonical || size <= crate::session::content::READ_BYTES,
            "canonical display header exceeds the read budget"
        );
        body_bytes = body_bytes.saturating_add(size);
        next = if before.is_some() {
            ordinal
        } else {
            ordinal + 1
        };
        rows.push(DisplayPageRow {
            ordinal,
            index_payload: overlay,
            entry_payload: body,
            canonical_entry,
        });
        if body_bytes >= read_budget {
            break;
        }
    }
    Ok(DisplayHistoryPage {
        rows,
        has_more: if before.is_some() {
            next > 0
        } else {
            next < total
        },
        next_offset: next,
        snapshot,
    })
}

/// Decode selected bodies outside the database worker. Oversized entries are
/// losslessly transported as UTF-8 JSON fragments, then assembled by clients.
fn materialize_display_page(
    page: DisplayHistoryPage,
    before: Option<i64>,
    fragment_offset: Option<i64>,
    store: Option<&SqliteStore>,
    cache: Option<&parking_lot::Mutex<FragmentCache>>,
) -> Result<Value> {
    let mut entries = Vec::new();
    let mut bytes = 64usize; // page envelope and commas
    let mut next = page.next_offset;
    let mut more = page.has_more;
    let row_count = page.rows.len();
    for DisplayPageRow {
        ordinal,
        index_payload: overlay,
        entry_payload: raw,
        canonical_entry,
    } in page.rows
    {
        let key = page
            .snapshot
            .as_ref()
            .zip(canonical_entry.as_ref())
            .map(|(scope, entry)| scope.render_key(entry, &overlay));
        let cached = key
            .as_ref()
            .zip(cache)
            .and_then(|(key, cache)| cache.lock().get(key));
        let (payload, encoded) = if let Some(body) = cached {
            (None, Encoded::Large(body))
        } else {
            let overlay: Value = serde_json::from_str(&overlay)?;
            let canonical = canonical_entry.is_some();
            let entry = if let Some(locator) = canonical_entry {
                Some(serde_json::from_value(
                    page.snapshot
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("canonical page has no snapshot"))?
                        .display(
                            store.ok_or_else(|| anyhow::anyhow!("canonical page has no reader"))?,
                            &locator,
                        )?,
                )?)
            } else if overlay["kind"] == "session_info" {
                None
            } else {
                raw.map(|raw| serde_json::from_str::<SessionEntry>(&raw))
                    .transpose()?
            };
            let payload = if let Some(mut entry) = entry {
                if !canonical {
                    hydrate_entry_projections(&mut entry);
                }
                let mut value = project_entries(&[entry]).pop().ok_or_else(|| {
                    anyhow::anyhow!("history index refers to a non-display entry")
                })?;
                for key in ["usage", "run"] {
                    if let Some(annotation) = overlay.get(key) {
                        value[key] = annotation.clone();
                    }
                }
                value
            } else {
                overlay
            };
            let encoded = fragment_cache::encode(&payload, HISTORY_PAGE_BYTE_BUDGET - 65)?;
            if let (Some(key), Some(cache), Encoded::Large(body)) = (key, cache, &encoded) {
                cache.lock().insert(key, body.clone());
            }
            (Some(payload), encoded)
        };
        if encoded.len().saturating_add(bytes + 1) > HISTORY_PAGE_BYTE_BUDGET {
            if !entries.is_empty() {
                next = if before.is_some() {
                    ordinal + 1
                } else {
                    ordinal
                };
                more = true;
                break;
            }
            let Encoded::Large(body) = encoded else {
                anyhow::bail!("oversized entry was not spooled");
            };
            let offset = usize::try_from(fragment_offset.unwrap_or(0))?;
            let fragment = body.read(offset, ENTRY_FRAGMENT_BYTE_BUDGET)?;
            if let Some(scope) = &page.snapshot {
                scope.verify(
                    store.ok_or_else(|| anyhow::anyhow!("canonical page has no reader"))?,
                )?;
            }
            return Ok(json!({"entries":[],
                "hasMore":page.has_more || row_count > 1,
                "nextOffset":if before.is_some() {ordinal} else {ordinal + 1},
                "fragment":{"entryId":body.entry_id,"ordinal":ordinal,"offset":offset,
                    "totalBytes":body.len,"json":fragment,"sha256":body.sha256}}));
        }
        anyhow::ensure!(
            fragment_offset.unwrap_or(0) == 0,
            "history changed during fragmented read"
        );
        bytes += encoded.len() + 1;
        entries
            .push(payload.ok_or_else(|| anyhow::anyhow!("cached entry unexpectedly fits inline"))?);
    }
    if before.is_some() {
        entries.reverse();
    }
    if let Some(scope) = &page.snapshot {
        scope.verify(store.ok_or_else(|| anyhow::anyhow!("canonical page has no reader"))?)?;
    }
    Ok(json!({"entries":entries,"hasMore":more,"nextOffset":next}))
}

impl Manager {
    #[cfg(test)]
    pub(crate) fn history_page(
        &self,
        id: &str,
        before: Option<i64>,
        offset: Option<i64>,
        limit: Option<i64>,
    ) -> Result<serde_json::Value> {
        self.read_history_page(id, before, offset, limit, None)
    }

    pub(crate) fn read_history_page(
        &self,
        id: &str,
        before: Option<i64>,
        offset: Option<i64>,
        limit: Option<i64>,
        fragment_offset: Option<i64>,
    ) -> Result<serde_json::Value> {
        let store = self.storage()?;
        // Lease a warm query-only connection for this response. Its WAL read
        // transaction spans the page locators and bounded body reads, so a
        // startup configuration update cannot replace a locator underneath us.
        let snapshot = store.db.read_snapshot()?;
        let ready = snapshot.read(|db| super::recovery::history_index_ready(db, id))?;
        let snapshot = if ready {
            snapshot
        } else {
            // Rebuild disposable projections before capturing the page. No
            // page or body has been read yet, and this is not a failed-read retry.
            drop(snapshot);
            let owned = id.to_owned();
            store
                .db
                .call(move |db| super::with_history_index(db, &owned, |_| Ok(())))?;
            store.db.read_snapshot()?
        };
        let mut page = snapshot.read(|db| {
            let tx = db.savepoint()?;
            let skipped: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='skipped')",
                [id], |row| row.get(0),
            )?;
            anyhow::ensure!(!skipped, "session migration was skipped");
            let page = read_display_page(&tx, id, before, offset, limit)?;
            tx.commit()?;
            Ok(page)
        })?;
        if let Some(scope) = &mut page.snapshot {
            scope.use_read_snapshot(snapshot);
        }
        materialize_display_page(
            page,
            before,
            fragment_offset,
            Some(store),
            Some(&self.history_fragments),
        )
    }
}

#[cfg(test)]
#[path = "paging_tests.rs"]
mod tests;
