//! Bounded display-page IO and lossless history-entry fragments.
use super::display_projection::project_entries;
use crate::session::{projection::hydrate_entry_projections, Manager, SessionEntry};
use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) const HISTORY_PAGE_BYTE_BUDGET: usize = 8 * 1024 * 1024;
// JSON escaping of a fragment can expand each byte up to six times.
const ENTRY_FRAGMENT_BYTE_BUDGET: usize = 1024 * 1024;

pub(super) struct DisplayPageRow {
    ordinal: i64,
    index_payload: String,
    entry_payload: Option<String>,
}

pub(super) struct DisplayHistoryPage {
    pub(super) rows: Vec<DisplayPageRow>,
    has_more: bool,
    next_offset: i64,
}

pub(super) fn read_display_page(
    db: &Connection,
    session: &str,
    before: Option<i64>,
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<DisplayHistoryPage> {
    let total: i64 = db.query_row(
        "SELECT COALESCE(MAX(ordinal)+1,0) FROM history_display WHERE session_id=?1",
        [session],
        |r| r.get(0),
    )?;
    let (start, end) = if let Some(before) = before {
        let end = before.clamp(0, total);
        let count = limit.unwrap_or(10).clamp(1, 100);
        let start: i64 = db.query_row("SELECT COALESCE((SELECT ordinal FROM history_display WHERE session_id=?1 AND is_user=1 AND ordinal<?2 ORDER BY ordinal DESC LIMIT 1 OFFSET ?3),0)",params![session,end,count-1],|r|r.get(0))?;
        (start, end)
    } else {
        let start = offset.unwrap_or(0).clamp(0, total);
        (
            start,
            (start + limit.unwrap_or(250).clamp(1, 1000)).min(total),
        )
    };
    let direction = if before.is_some() { "DESC" } else { "ASC" };
    let mut stmt = db.prepare(&format!("SELECT d.ordinal,d.payload,e.payload FROM history_display d LEFT JOIN entry_records e ON e.session_id=d.session_id AND e.position=d.source_position WHERE d.session_id=?1 AND d.ordinal>=?2 AND d.ordinal<?3 ORDER BY d.ordinal {direction}"))?;
    let mut cursor = stmt.query(params![session, start, end])?;
    let mut rows = Vec::new();
    let mut body_bytes = 0usize;
    let mut next = if before.is_some() { end } else { start };
    while let Some(row) = cursor.next()? {
        let ordinal: i64 = row.get(0)?;
        let overlay: String = row.get(1)?;
        let body: Option<String> = row.get(2)?;
        let size = overlay
            .len()
            .saturating_add(body.as_ref().map_or(0, String::len));
        if !rows.is_empty() && body_bytes.saturating_add(size) > HISTORY_PAGE_BYTE_BUDGET {
            break;
        }
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
        });
        if body_bytes >= HISTORY_PAGE_BYTE_BUDGET {
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
    })
}

/// Decode selected bodies outside the database worker. Oversized entries are
/// losslessly transported as UTF-8 JSON fragments, then assembled by clients.
fn materialize_display_page(
    page: DisplayHistoryPage,
    before: Option<i64>,
    fragment_offset: Option<i64>,
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
    } in page.rows
    {
        let overlay: Value = serde_json::from_str(&overlay)?;
        let payload = if let Some(raw) = raw.filter(|_| overlay["kind"] != "session_info") {
            let mut entry: SessionEntry = serde_json::from_str(&raw)?;
            hydrate_entry_projections(&mut entry);
            let mut value = project_entries(&[entry])
                .pop()
                .ok_or_else(|| anyhow::anyhow!("history index refers to a non-display entry"))?;
            for key in ["usage", "run"] {
                if let Some(annotation) = overlay.get(key) {
                    value[key] = annotation.clone();
                }
            }
            value
        } else {
            overlay
        };
        let serialized = serde_json::to_string(&payload)?;
        if serialized.len().saturating_add(bytes + 1) > HISTORY_PAGE_BYTE_BUDGET {
            if !entries.is_empty() {
                next = if before.is_some() {
                    ordinal + 1
                } else {
                    ordinal
                };
                more = true;
                break;
            }
            let offset = usize::try_from(fragment_offset.unwrap_or(0))?;
            anyhow::ensure!(
                offset < serialized.len() && serialized.is_char_boundary(offset),
                "invalid history fragment offset"
            );
            let mut end = offset
                .saturating_add(ENTRY_FRAGMENT_BYTE_BUDGET)
                .min(serialized.len());
            while !serialized.is_char_boundary(end) {
                end -= 1;
            }
            return Ok(json!({"entries":[],
                "hasMore":page.has_more || row_count > 1,
                "nextOffset":if before.is_some() {ordinal} else {ordinal + 1},
                "fragment":{"entryId":payload["id"],"ordinal":ordinal,"offset":offset,
                    "totalBytes":serialized.len(),"json":&serialized[offset..end],
                    "sha256":format!("{:x}", Sha256::digest(serialized.as_bytes()))}}));
        }
        anyhow::ensure!(
            fragment_offset.unwrap_or(0) == 0,
            "history changed during fragmented read"
        );
        bytes += serialized.len() + 1;
        entries.push(payload);
    }
    if before.is_some() {
        entries.reverse();
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
        let id = id.to_owned();
        let page = self.storage()?.db.call(move |db| super::with_history_index(db, &id, |connection| {
            let skipped: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='skipped')", [&id], |r| r.get(0))?;
            if skipped { anyhow::bail!("session migration was skipped"); }
            read_display_page(connection, &id, before, offset, limit)
        }))?;
        materialize_display_page(page, before, fragment_offset)
    }
}

#[cfg(test)]
#[path = "paging_tests.rs"]
mod tests;
