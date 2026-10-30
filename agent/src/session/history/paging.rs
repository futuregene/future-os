//! Bounded display-page IO and lossless history-entry fragments.
use super::display_projection::project_entries;
use super::fragment_cache::{self, Encoded, FragmentCache};
use crate::session::{content::reader, sqlite_store::SqliteStore};
use crate::session::{projection::hydrate_entry_projections, Manager, SessionEntry};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Arc;

pub(super) const HISTORY_PAGE_BYTE_BUDGET: usize = 2 * 1024 * 1024;

/// How many rows a conversation's *first* page may carry.
///
/// A page is asked for on every switch and every page-up, and what it costs is
/// what is *in* it: measured on this machine's store, the newest page of a
/// conversation whose history is one huge turn was 541 rows and 1.9 MB, i.e.
/// 250–310 ms before the reader could see anything — and the exchange count
/// could not see it at all, because that conversation has three exchanges in
/// total. A page is therefore bounded by rows or bytes, whichever comes first.
pub(super) const HISTORY_TAIL_ROWS: usize = 150;

/// How many rows a page *above* the first one may carry.
///
/// A reader scrolling up has already settled on the conversation and is reading;
/// what they want from a page-up is the rest of the turn they are looking at, in
/// one page rather than four. Measured both ways on the same store: with the
/// first-page ceiling applied to every page, walking the longest conversation in
/// it took 37 page-ups instead of 12.
pub(super) const HISTORY_PAGE_ROWS: usize = 600;

/// What the read loop may pull off disk before the page is cut: raw payload
/// bytes, before projection and JSON encoding (which can expand them). It sits
/// above the encoded ceiling on purpose — the encoded cut is the one the client
/// feels, and this one only stops a page of one enormous entry from reading
/// unbounded memory.
const HISTORY_PAGE_READ_BUDGET: usize = 2 * 1024 * 1024;

// JSON escaping of a fragment can expand each byte up to six times. A quarter
// of the page ceiling, so one fragment and its envelope always fit a page.
const ENTRY_FRAGMENT_BYTE_BUDGET: usize = 1024 * 1024;

/// How many materialized pages the agent keeps, and how many bytes of them.
///
/// A page is bounded by [`HISTORY_PAGE_BYTE_BUDGET`], so the count alone bounds
/// nothing useful; the byte ceiling is what bounds the memory, and the count
/// keeps a reader stepping back and forth through a long rail from evicting the
/// page they are about to return to.
const PAGE_CACHE_CAPACITY: usize = 8;
const PAGE_CACHE_BYTES: usize = 16 * 1024 * 1024;
/// A page larger than this is not worth a slot: one of them would take half the
/// cache for a conversation the reader is unlikely to re-read at once.
const PAGE_CACHE_MAX_PAGE: usize = PAGE_CACHE_BYTES / 4;

/// What one materialized page is an answer to.
///
/// The session revision is the one thing every write to a conversation moves —
/// an appended run, a rewrite, a rename — so a page read at revision `r` stays
/// `r`'s answer, and the next read of it costs one indexed query instead of
/// re-materializing every body in the page. The coordinates are part of the key
/// because the same conversation is read tail-first and then backwards.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::session) struct PageKey {
    session: String,
    revision: i64,
    before: Option<i64>,
    offset: Option<i64>,
    limit: Option<i64>,
    fragment_offset: Option<i64>,
}

impl PageKey {
    pub(in crate::session) fn new(
        session: &str,
        revision: i64,
        before: Option<i64>,
        offset: Option<i64>,
        limit: Option<i64>,
        fragment_offset: Option<i64>,
    ) -> Self {
        Self {
            session: session.to_string(),
            revision,
            before,
            offset,
            limit,
            fragment_offset,
        }
    }
}

/// Materialized pages, oldest first.
///
/// A switch re-reads the tail page to find out whether the conversation moved on
/// while the reader was away, so the same page is asked for again and again —
/// measured on a 147-conversation store, materializing one cost 40–50 ms and the
/// answer was byte-identical every time.
pub(in crate::session) struct PageCache {
    entries: VecDeque<(PageKey, Arc<Value>, usize)>,
    bytes: usize,
}

impl PageCache {
    pub(in crate::session) fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
        }
    }

    /// The page for `key`, promoted to the newest slot.
    pub(in crate::session) fn get(&mut self, key: &PageKey) -> Option<Arc<Value>> {
        let index = self
            .entries
            .iter()
            .position(|(cached, _, _)| cached == key)?;
        let entry = self.entries.remove(index)?;
        let page = entry.1.clone();
        self.entries.push_back(entry);
        Some(page)
    }

    pub(in crate::session) fn insert(&mut self, key: PageKey, page: Arc<Value>, bytes: usize) {
        // One page per conversation and coordinate set: a newer revision of the
        // same page replaces the old one rather than sitting beside it.
        let mut replaced = 0usize;
        self.entries.retain(|(cached, _, size)| {
            let same_page = cached.session == key.session
                && cached.before == key.before
                && cached.offset == key.offset
                && cached.limit == key.limit
                && cached.fragment_offset == key.fragment_offset;
            if same_page {
                replaced += *size;
            }
            !same_page
        });
        self.bytes = self.bytes.saturating_sub(replaced);
        if bytes > PAGE_CACHE_MAX_PAGE {
            return;
        }
        self.entries.push_back((key, page, bytes));
        self.bytes += bytes;
        while self.entries.len() > PAGE_CACHE_CAPACITY || self.bytes > PAGE_CACHE_BYTES {
            let Some((_, _, size)) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(size);
        }
    }

    /// How many pages are held — the meter the cache's tests read.
    #[cfg(test)]
    pub(in crate::session) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(in crate::session) fn bytes(&self) -> usize {
        self.bytes
    }
}

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
    let read_budget = HISTORY_PAGE_READ_BUDGET;
    let rows_cap = if before == Some(i64::MAX) {
        HISTORY_TAIL_ROWS
    } else {
        HISTORY_PAGE_ROWS
    };
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
        // The row ceiling is the page's own bound, and it only means anything
        // for a backward read (a forward page is already row-bounded by its
        // `LIMIT`). The *first* page is the one a switch waits for, so it is the
        // one that has to stay small; a page-up may carry a whole turn, because
        // the reader is already reading and the read no longer holds the screen.
        // `before == i64::MAX` is the protocol's "from the newest row" marker.
        if before.is_some() && rows.len() >= rows_cap {
            break;
        }
    }
    // A page that was cut short may begin in the middle of a tool sequence, and
    // a tool *result* row carries only its call's id — the name and arguments it
    // is read with live on the call row above it. Nudging the cut up to the next
    // call row was tried and reverted: a turn is hundreds of consecutive results,
    // so the nudge ate the whole page and the next request advanced by one row
    // (measured: 392 pages for a 541-row conversation). The page keeps the rows
    // it read; the one at its top renders with its call id until the page above
    // arrives.
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
) -> Result<(Value, usize)> {
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
            return Ok((
                json!({"entries":[],
                "hasMore":page.has_more || row_count > 1,
                "nextOffset":if before.is_some() {ordinal} else {ordinal + 1},
                "fragment":{"entryId":body.entry_id,"ordinal":ordinal,"offset":offset,
                    "totalBytes":body.len,"json":fragment,"sha256":body.sha256}}),
                body.len.saturating_add(256),
            ));
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
    Ok((
        json!({"entries":entries,"hasMore":more,"nextOffset":next}),
        bytes,
    ))
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
        // The page a conversation's revision names is the page this read would
        // rebuild: every write to it moves the revision, so the cache can be
        // asked before any body is read. A conversation whose derived index is
        // being repaired is not cacheable — that repair is what the caller is
        // owed, and it runs below.
        let owned = id.to_owned();
        let (revision, ready) = store.db.read(move |db| {
            // The stored key is not always the wire id (native sessions are a
            // binary identity), so it is resolved the way every other reader
            // resolves it.
            let stored = crate::session::identity::session_key(db, &owned)?;
            let revision = db
                .query_row(
                    "SELECT revision FROM sessions WHERE id=?1 AND revision>=0",
                    [&stored],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            let ready = super::recovery::history_index_ready(db, &owned)?;
            Ok((revision, ready))
        })?;
        let revision =
            revision.ok_or_else(|| anyhow::anyhow!("session is missing or not published"))?;
        let key = PageKey::new(id, revision, before, offset, limit, fragment_offset);
        if ready {
            if let Some(page) = self.history_pages.lock().get(&key) {
                return Ok(page.as_ref().clone());
            }
        }
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
        let (value, bytes) = materialize_display_page(
            page,
            before,
            fragment_offset,
            Some(store),
            Some(&self.history_fragments),
        )?;
        // Only a page still at the revision read above is worth keeping: a write
        // that landed while this one was being materialized already has its own
        // revision, and caching this page under the newer one would be a lie.
        if store.revision(id)? == Some(revision) {
            self.history_pages
                .lock()
                .insert(key, Arc::new(value.clone()), bytes);
        }
        Ok(value)
    }
}

#[cfg(test)]
#[path = "paging_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "page_cache_tests.rs"]
mod page_cache_tests;
