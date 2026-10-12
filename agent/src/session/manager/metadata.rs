//! Atomic updates of the authoritative current session metadata.
use super::Manager;
use crate::session::SessionEntry;
use anyhow::{anyhow, Context, Result};
use rusqlite::OptionalExtension;

impl Manager {
    pub fn update_session_info(&self, id: &str, key: &str, value: serde_json::Value) -> Result<()> {
        self.update_session_info_fields(id, [(key.to_owned(), value)].into_iter().collect())
    }

    /// Save one metadata snapshot so related usage counters survive restart together.
    /// The journal payloads needed to price a session's spend request by
    /// request; see `SqliteStore::pricing_event_payloads`.
    pub(crate) fn pricing_event_payloads(&self, id: &str) -> Result<Vec<String>> {
        self.storage()?.pricing_event_payloads(id)
    }

    pub(crate) fn update_session_info_fields(
        &self,
        id: &str,
        fields: serde_json::Map<String, serde_json::Value>,
    ) -> Result<()> {
        let id = id.to_owned();
        self.storage()?.db.call(move |db| {
            // Reads the current metadata before it writes: take the write lock
            // at BEGIN, or a concurrent connection's commit invalidates this
            // snapshot and the upgrade fails instantly (SQLITE_BUSY_SNAPSHOT).
            let tx = crate::session::database::begin_write_transaction(db)?;
            // The authoritative current metadata is `sessions.current_metadata_json`
            // — `entry_records` *projects* a `session_info` entry's content from
            // it (`entry_records.rs` `ENTRY_RECORD_VIEWS_SQL`), so reading the entry adds nothing and
            // makes the update depend on an entry that may not exist. A
            // legacy-imported transcript has no `session_info` entry at all, so
            // the previous `query_row(..)` on that entry failed with a bare
            // "Query returned no rows": `future session set --title/--thinking`
            // on an imported session reported success while persisting nothing.
            //
            // Three distinct cases, deliberately not collapsed:
            //   no row          -> the session does not exist (caller's bug)
            //   NULL column     -> nothing recorded yet; start empty (the
            //                      legacy-import case this fixes)
            //   non-object JSON -> corrupt metadata; report it rather than
            //                      silently replacing it and dropping every
            //                      other field it held
            // Outer `Option` = is there a row; inner = is the column NULL.
            let existing: Option<Option<String>> = tx
                .query_row(
                    "SELECT current_metadata_json FROM sessions WHERE id=?1",
                    [&id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .context("session metadata could not be read")?;
            let Some(existing) = existing else {
                // The row itself is missing, so the appended entry would fail on
                // its foreign key. Name the real cause instead of surfacing a
                // constraint error.
                return Err(anyhow!("session not found: no row for {id}"));
            };
            let mut info = match existing {
                None => serde_json::Map::new(),
                Some(json) => serde_json::from_str::<serde_json::Value>(&json)
                    .ok()
                    .and_then(|value| value.as_object().cloned())
                    .ok_or_else(|| anyhow!("session has no session_info object"))?,
            };
            info.extend(fields);
            let entry = SessionEntry::session_info(
                serde_json::Value::Object(info),
                String::new(),
                String::new(),
            );
            crate::session::sqlite_store::insert_transcript_entries(
                &tx,
                &id,
                Self::encoded(&[entry])?,
            )?;
            tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1", [&id])?;
            tx.commit()?;
            Ok(())
        })
    }
}
