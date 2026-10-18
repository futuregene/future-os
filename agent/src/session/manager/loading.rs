//! Transcript hydration, in-memory repair and session reconstruction.
use super::Manager;
use crate::session::projection::hydrate_entry_projections;
use crate::session::repair::{
    dedupe_tool_entries, repair_dangling_tool_calls, strip_empty_assistants,
};
use crate::session::{Session, SessionEntry, ENTRY_TYPE_MODEL_CHANGE, ENTRY_TYPE_SESSION_INFO};
use anyhow::{anyhow, Context, Result};

impl Manager {
    pub(crate) fn serialize_entry(entry: &SessionEntry) -> Result<String> {
        let value = serde_json::to_value(entry).context("serialize entry")?;
        serde_json::to_string(&crate::session::entry_records::normalize_entry(value))
            .context("serialize entry")
    }

    pub fn load(&self, id: &str) -> Result<Session> {
        let mut entries: Vec<SessionEntry> = self.storage()?.typed_entries(id)?;
        for entry in &mut entries {
            hydrate_entry_projections(entry);
        }
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

    /// Runtime metadata and identities only; never opens historical bodies.
    pub(crate) fn load_metadata(&self, id: &str) -> Result<Session> {
        let session_id = id.to_owned();
        let rows = self.storage()?.db.call(move |db| {
            crate::session::history::with_history_index(db, &session_id, |connection| {
                let mut stmt = connection.prepare(
                    "SELECT payload FROM history_shapes WHERE session_id=?1 ORDER BY position",
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

    pub(in crate::session) fn restore_session_times(
        &self,
        mut session: Session,
    ) -> Result<Session> {
        let id = session.id.clone();
        let (created, updated): (Option<i64>, Option<i64>) =
            self.storage()?.db.call(move |db| {
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
