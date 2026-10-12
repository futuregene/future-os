//! Transactional, idempotent creation of a materialized session fork.
use super::Manager;
use crate::session::SessionEntry;
use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};

impl Manager {
    /// Resolve, materialize, and record a fork under one SQLite transaction.
    /// Replaying the same request returns the same child; reusing a request id
    /// for different semantics is rejected instead of silently duplicating or
    /// returning the wrong branch.
    pub fn create_fork(
        &self,
        request: crate::session::ForkRequest,
    ) -> Result<crate::session::ForkResult> {
        if request.request_id.trim().is_empty() {
            return Err(anyhow!("fork request id is required"));
        }
        if request.parent_session_id.trim().is_empty() {
            return Err(anyhow!("fork parent session id is required"));
        }
        let fingerprint = serde_json::to_string(&request)?;
        let request_id = request.request_id.clone();
        let parent_id = request.parent_session_id.clone();
        let point = request.point.clone();
        let created_by = request.created_by.clone();
        let creator_id = request.creator_id.clone();
        let fingerprint_for_tx = fingerprint.clone();
        let outcome = self.storage()?.db.call(move |db| {
            // Reads the operation and the parent's entries before it writes:
            // take the write lock at BEGIN, or a concurrent connection's commit
            // invalidates this snapshot and the upgrade fails instantly.
            let tx = crate::session::database::begin_write_transaction(db)?;
            if let Some((stored_fingerprint, child_id)) = tx
                .query_row(
                    "SELECT request_fingerprint, child_session_id FROM fork_operations WHERE request_id=?1",
                    [&request_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?
            {
                if stored_fingerprint != fingerprint_for_tx {
                    return Err(anyhow!("fork request id was already used with different parameters"));
                }
                tx.commit()?;
                return Ok((child_id, false));
            }

            let rows = {
                let mut statement = tx.prepare(
                    "SELECT payload FROM entry_records WHERE session_id=?1 ORDER BY position",
                )?;
                let rows = statement
                    .query_map([&parent_id], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            if rows.is_empty() {
                return Err(anyhow!("Session not found on disk — it may have been deleted or moved."));
            }
            let entries = rows
                .into_iter()
                .map(|row| serde_json::from_str::<SessionEntry>(&row))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let parent = Self::session_from_entries(&parent_id, entries)?;
            let mut child = crate::session::build_fork_session(&parent, &point)?;
            crate::session::set_creation_provenance(&mut child, &created_by, &creator_id);
            let child_id = child.id.clone();
            let forked_at_ms = child.created_at.timestamp_millis();
            let values = Self::encoded(&child.entries)?;
            tx.execute("INSERT INTO sessions(id) VALUES (?1)", [&child_id])?;
            crate::session::sqlite_store::insert_transcript_entries(&tx, &child_id, values)?;
            // Copied entries intentionally retain their historical timestamps,
            // but the session itself was created now. `insert_transcript_entries` derives
            // `created_at_ms` as the minimum entry timestamp, so restore the
            // fork operation boundary after the snapshot is materialized.
            tx.execute(
                "UPDATE sessions SET created_at_ms=?2 WHERE id=?1",
                params![child_id, forked_at_ms],
            )?;
            tx.execute(
                "INSERT INTO fork_operations(
                    request_id, request_fingerprint, parent_session_id,
                    child_session_id, created_at_ms
                 ) VALUES (?1,?2,?3,?4,?5)",
                params![
                    request_id,
                    fingerprint_for_tx,
                    parent_id,
                    child_id,
                    forked_at_ms
                ],
            )?;
            tx.commit()?;
            Ok((child_id, true))
        })?;

        self.invalidate_display_projection(&outcome.0);
        Ok(crate::session::ForkResult {
            session: self.load(&outcome.0)?,
            created: outcome.1,
        })
    }
}
