//! Transcript hydration, in-memory repair and session reconstruction.
use super::Manager;
use crate::session::projection::hydrate_entry_projections;
use crate::session::repair::{
    dedupe_tool_entries, repair_dangling_tool_calls, strip_empty_assistants,
};
use crate::session::{Session, SessionEntry, ENTRY_TYPE_MODEL_CHANGE, ENTRY_TYPE_SESSION_INFO};
use anyhow::{anyhow, Context, Result};
use rusqlite::params;

impl Manager {
    /// Frequent state polling needs only counts and lineage. It must not reload
    /// configuration bodies or walk every historical message header each time.
    ///
    /// **Every storage format answers from bounded rows.** `entries` is the
    /// header index both a canonical and a legacy session keep — `history_shapes`
    /// holds the legacy *bodies*, not the headers — and the parent is part of the
    /// session row. The count used to fall through to `load_metadata` whenever
    /// the session was not canonical, which reads a whole transcript for a number
    /// a covering index answers: measured on a 14,531-entry legacy conversation,
    /// `get_state` cost 23 ms warm (72 ms at 50,000 entries) and this count was
    /// its only term that grew with the row count, while the two subqueries here
    /// answer in 7 µs and 3 µs.
    pub(crate) fn state_metadata(&self, id: &str) -> Result<(usize, String)> {
        let session = id.to_owned();
        self.storage()?.db.read(move |db| {
            let session = crate::session::identity::BoundIdentity::session(db, &session)?;
            let (count, parent): (i64, Option<String>) = db.query_row(
                "SELECT
                   (SELECT count(*) FROM entries WHERE session_id=?1 AND entry_type='user'),
                   (SELECT CASE WHEN json_type(current_metadata_json,'$.parent_session_id')='text'
                                THEN json_extract(current_metadata_json,'$.parent_session_id') END
                      FROM sessions WHERE id=?1 AND revision>=0)",
                [&session],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            Ok((usize::try_from(count)?, parent.unwrap_or_default()))
        })
    }

    pub(crate) fn serialize_entry(entry: &SessionEntry) -> Result<String> {
        let value = serde_json::to_value(entry).context("serialize entry")?;
        serde_json::to_string(&crate::session::entry_records::normalize_entry(value))
            .context("serialize entry")
    }

    /// Decode authoritative blocks without manufacturing lost-tool results.
    pub(crate) fn load_unrepaired_entries(&self, id: &str) -> Result<Vec<SessionEntry>> {
        self.prepare_session(id)?;
        let mut entries: Vec<SessionEntry> = self.storage()?.typed_entries(id)?;
        for entry in &mut entries {
            hydrate_entry_projections(entry);
        }
        Ok(entries)
    }

    pub fn load(&self, id: &str) -> Result<Session> {
        let mut entries = self.load_unrepaired_entries(id)?;
        // Runtime repair is a projection only; importing never calls this path.
        let stripped = strip_empty_assistants(&mut entries);
        let deduped = dedupe_tool_entries(&mut entries);
        let repaired = repair_dangling_tool_calls(&mut entries);
        if stripped || deduped || repaired {
            tracing::info!(
                "Healed session {id} in memory (stripped_empty={stripped}, \
                 deduped_tools={deduped}, repaired_dangling={repaired})"
            );
        }
        self.restore_session_times(Self::session_from_entries(id, entries)?)
    }

    /// Runtime configuration and identities; no historical message bodies.
    pub(crate) fn load_metadata(&self, id: &str) -> Result<Session> {
        let canonical = self.prepare_session(id)?;
        let store = self.storage()?;
        if canonical {
            let scope = crate::session::content::reader::Snapshot::capture(store, id)?;
            // Runtime restoration needs current configuration and the latest
            // model change, not a header round trip for every historical tool.
            let selected = scope.clone();
            let locators = store.db.read_as("session_metadata", move |db| {
                let key = selected.session_key();
                let mut query = db.prepare_cached("SELECT position,entry_id FROM entries WHERE session_id=?1 AND position IN (
                    SELECT min(position) FROM entries WHERE session_id=?1
                    UNION SELECT max(position) FROM entries WHERE session_id=?1 AND entry_type='session_info'
                    UNION SELECT max(position) FROM entries WHERE session_id=?1 AND entry_type='model_change'
                ) ORDER BY position")?;
                let rows = query.query_map([key], |row| Ok(super::super::content::reader::Entry { position: row.get(0)?, id: row.get(1)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })?;
            let mut entries = Vec::with_capacity(locators.len());
            for locator in locators {
                let mut value = scope.header(store, &locator)?;
                if value["type"] == ENTRY_TYPE_SESSION_INFO {
                    value = scope.entry(store, &locator)?;
                }
                entries.push(serde_json::from_value(value)?);
            }
            scope.verify(store)?;
            return self.restore_session_times(Self::session_from_entries(id, entries)?);
        }
        let session_id = id.to_owned();
        // Legacy bodies live in `history_shapes`, but the header index is the
        // same `entries` table, so the same three rows the canonical branch
        // reads (first entry, latest `session_info`, latest `model_change`)
        // answer the same questions. Reading every body instead cost 200 ms on a
        // 14,531-entry conversation — the whole cold `get_state` — for state a
        // metadata read never uses.
        let rows = self.storage()?.db.call(move |db| {
            crate::session::history::with_history_index(db, &session_id, |connection| {
                let mut stmt = connection.prepare(
                    "SELECT payload FROM history_shapes WHERE session_id=?1 AND position IN (
                       SELECT min(position) FROM entries WHERE session_id=?1
                       UNION SELECT max(position) FROM entries WHERE session_id=?1 AND entry_type='session_info'
                       UNION SELECT max(position) FROM entries WHERE session_id=?1 AND entry_type='model_change')
                     ORDER BY position",
                )?;
                let rows = stmt
                    .query_map([&session_id], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
        })?;
        let entries = rows
            .into_iter()
            .map(|row| serde_json::from_str(&row))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.restore_session_times(Self::session_from_entries(id, entries)?)
    }

    /// One page of a conversation's messages, read as a page.
    ///
    /// `get_messages` used to materialise a whole transcript and slice it in
    /// memory: 249 ms for a 40-message page of a 14,531-entry conversation, and
    /// a reply that grew with the conversation until a 25,000-entry one exceeded
    /// the 32 MiB gRPC limit outright. The index space is the messages
    /// [`entries_to_agent_messages`](crate::session::entries_to_agent_messages)
    /// produces — one per `user`/`system`/`assistant`/`tool` row, in position
    /// order — so an `offset` resolves to a row range in `entries` and only that
    /// range is hydrated. Returns the page's entries and how many messages the
    /// conversation holds.
    ///
    /// Two things it deliberately does *not* do:
    ///
    /// - It does not apply the cross-entry repairs `load` performs (dangling tool
    ///   calls, empty assistants, duplicate tools). Those shape what a provider
    ///   sees on an internally inconsistent transcript; a page a client reads
    ///   shows that transcript as stored.
    /// - Historical storage is migrated before serving its canonical page.
    ///   Once migrated, page selection never hydrates off-page messages.
    pub(crate) fn message_page(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<SessionEntry>, usize)> {
        const MESSAGE_ROLES: &str = "'user','system','assistant','tool'";
        enum Rows {
            Canonical(Vec<crate::session::content::reader::Entry>),
            NeedsMigration,
        }
        let store = self.storage()?;
        let requested = id.to_owned();
        let offset_rows = i64::try_from(offset)?;
        let limit_rows = i64::try_from(limit.max(1))?;
        // Probe format/pending conversion with the indexed page selection, so
        // a warm canonical read pays no separate migration-admission job.
        // Pending legacy receipts resume before any partial header is decoded.
        let (rows, total): (Rows, i64) = store.db.read(move |db| {
            let key = crate::session::identity::BoundIdentity::session(db, &requested)?;
            let total: i64 = db.query_row(
                &format!(
                    "SELECT count(*) FROM entries WHERE session_id=?1 AND entry_type IN ({MESSAGE_ROLES})"
                ),
                [&key],
                |row| row.get(0),
            )?;
            // `OFFSET` walks header rows, not bodies: both the count above and
            // this page are index-only reads even for a page deep into a
            // 50,000-entry conversation.
            let mut query = db.prepare_cached(&format!(
                "SELECT position,entry_id FROM entries
                  WHERE session_id=?1 AND entry_type IN ({MESSAGE_ROLES})
                  ORDER BY position LIMIT ?2 OFFSET ?3"
            ))?;
            let locators = query
                .query_map(params![&key, limit_rows, offset_rows], |row| {
                    Ok(crate::session::content::reader::Entry {
                        position: row.get(0)?,
                        id: row.get(1)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if crate::session::content::format(db, &requested)?
                == crate::session::content::FORMAT_VERSION
                && !db.query_row("SELECT EXISTS(SELECT 1 FROM legacy_conversions WHERE session_id=?1)",[&key],|row|row.get::<_,bool>(0))?
            {
                return Ok((Rows::Canonical(locators), total));
            }
            Ok((Rows::NeedsMigration, total))
        })?;
        let total = usize::try_from(total)?;
        match rows {
            Rows::NeedsMigration => {
                if self.prepare_session(id)? {
                    self.message_page(id, offset, limit)
                } else {
                    Ok((Vec::new(), total))
                }
            }
            Rows::Canonical(locators) if locators.is_empty() => Ok((Vec::new(), total)),
            Rows::Canonical(locators) => {
                let snapshot = crate::session::content::reader::Snapshot::capture(store, id)?;
                let values = snapshot.entries_page(store, &locators)?;
                let mut entries = values
                    .into_iter()
                    .map(serde_json::from_value)
                    .collect::<std::result::Result<Vec<SessionEntry>, _>>()?;
                for entry in &mut entries {
                    hydrate_entry_projections(entry);
                }
                Ok((entries, total))
            }
        }
    }

    /// Sequence restoration does not require decoding every run marker.
    pub(crate) fn next_sequence(&self, id: &str) -> Result<u64> {
        let id = id.to_owned();
        self.storage()?.db.read_as("session_next_sequence", move |db| {
            let key = crate::session::identity::BoundIdentity::session(db, &id)?;
            let previous: i64 = db.query_row("SELECT max(coalesce((SELECT max(run_sequence) FROM runs WHERE session_id=?1),0),(SELECT count(*) FROM entries WHERE session_id=?1 AND entry_type='run_started'))", [key], |row| row.get(0))?;
            Ok(u64::try_from(previous)?.saturating_add(1).max(1))
        })
    }

    pub(in crate::session) fn restore_session_times(
        &self,
        mut session: Session,
    ) -> Result<Session> {
        let id = session.id.clone();
        let (created, updated): (Option<i64>, Option<i64>) =
            self.storage()?.db.read(move |db| {
                let id = crate::session::identity::BoundIdentity::session(db, &id)?;
                Ok(db.query_row(
                    "SELECT created_at_ms,updated_at_ms FROM sessions WHERE id=?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?)
            })?;
        if let Some(created) = created.and_then(chrono::DateTime::from_timestamp_millis) {
            session.created_at = created.with_timezone(&chrono::Local);
        }
        if let Some(updated) = updated.and_then(chrono::DateTime::from_timestamp_millis) {
            session.updated_at = updated.with_timezone(&chrono::Local);
        }
        Ok(session)
    }

    pub(crate) fn session_from_entries(id: &str, entries: Vec<SessionEntry>) -> Result<Session> {
        let created_at = entries
            .first()
            .ok_or_else(|| anyhow!("session has no display entries"))?
            .timestamp;
        let updated_at = entries
            .iter()
            .map(|e| e.timestamp)
            .max()
            .unwrap_or(created_at);
        let cwd = entries
            .iter()
            .rev()
            .find_map(|e| {
                if e.entry_type == ENTRY_TYPE_SESSION_INFO {
                    e.content
                        .as_ref()
                        .and_then(|v| v.get("cwd"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let model = entries
            .iter()
            .find(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO)
            .and_then(|e| e.content.as_ref())
            .and_then(|c| c.get("model"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                entries.iter().rev().find_map(|e| {
                    if e.entry_type == ENTRY_TYPE_MODEL_CHANGE {
                        e.content
                            .as_ref()
                            .and_then(|c| c.get("model"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();
        let name = entries
            .iter()
            .rev()
            .find(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO)
            .and_then(|e| e.content.as_ref())
            .and_then(|c| c.get("session_name"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let parent_session_id = entries
            .iter()
            .rev()
            .find_map(|e| {
                if e.entry_type == ENTRY_TYPE_SESSION_INFO {
                    e.content
                        .as_ref()
                        .and_then(|v| v.get("parent_session_id"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let session = Session {
            id: id.to_string(),
            version: crate::session::CURRENT_SESSION_VERSION,
            cwd,
            model,
            name,
            parent_session_id,
            leaf_id: String::new(),
            entries,
            created_at,
            updated_at,
        };
        Ok(session)
    }
}
