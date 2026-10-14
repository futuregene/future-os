//! Transactional session storage. This facade owns durability boundaries;
//! transcript/event modules own their SQL and update derived indexes within
//! the same write transaction.

use super::database::Database;
use anyhow::{bail, Result};
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::path::Path;

#[derive(Clone)]
pub struct SqliteStore {
    pub(crate) db: Database,
}

impl SqliteStore {
    /// A transcript can be absent while its session-scoped events exist.
    pub fn contains(&self, session: &str) -> Result<bool> {
        let session = session.to_owned();
        self.db.call(move |db| Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND revision>=0) OR EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='skipped')", [session], |r| r.get(0))?))
    }

    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            db: Database::open(path)?,
        })
    }

    fn request_reclamation_after_success(&self, operation_succeeded: bool) {
        if operation_succeeded {
            self.db.request_space_reclamation();
        }
    }

    /// Commit transcript changes, derived run state and terminal events as a
    /// single durability boundary. All values are owned by the writer job.
    pub fn commit(&self, session: &str, entries: Vec<Value>, events: Vec<Value>) -> Result<()> {
        let session = session.to_owned();
        self.db.call(move |db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            if tx.execute(
                "UPDATE sessions SET revision=revision+1 WHERE id=?1 AND revision>=0",
                [&session],
            )? == 0
            {
                bail!("session does not exist");
            }
            insert_transcript_entries(&tx, &session, entries)?;
            for event in events {
                insert_event_record(&tx, &session, event)?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn revision(&self, session: &str) -> Result<Option<i64>> {
        let session = session.to_owned();
        self.db.call(move |db| {
            Ok(db
                .query_row(
                    "SELECT revision FROM sessions WHERE id=?1 AND revision>=0",
                    [session],
                    |r| r.get(0),
                )
                .optional()?)
        })
    }

    pub fn ids(&self, include_skipped: bool) -> Result<Vec<String>> {
        self.db.call(move |db| {
            let mut statement = db.prepare(if include_skipped {
                "SELECT id FROM sessions WHERE revision>=0 UNION SELECT session_id FROM legacy_imports WHERE status='skipped' ORDER BY 1"
            } else { "SELECT id FROM sessions WHERE revision>=0 ORDER BY id" })?;
            let ids = statement.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(ids)
        })
    }

    pub fn delete(&self, session: &str) -> Result<()> {
        self.delete_many(vec![session.to_owned()])
    }

    /// One durability boundary and one reclamation pass for a bounded batch.
    /// A failed commit leaves every session retryable, including import tombstones.
    pub(crate) fn delete_many(&self, sessions: Vec<String>) -> Result<()> {
        let result = self.db.call(move |db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            for session in sessions {
                tx.execute("DELETE FROM sessions WHERE id=?1", [&session])?;
                tx.execute(
                    "UPDATE legacy_imports SET status='deleted' WHERE session_id=?1",
                    [&session],
                )?;
            }
            tx.commit()?;
            Ok(())
        });
        // A session's events are its largest storage, and a delete frees all of
        // them: this is the case the reclamation exists for.
        self.request_reclamation_after_success(result.is_ok());
        result
    }
}

mod events;
mod transcript;
pub(crate) use events::{
    decode_event_row, insert_event_record, is_event_rejected, read_compaction_event_rows,
    read_replay_event_rows,
};
pub(crate) use transcript::{insert_transcript_entries, read_run_markers};

#[cfg(test)]
mod deletion_tests;
#[cfg(test)]
mod event_selection_tests;
#[cfg(test)]
mod reclamation_tests;
#[cfg(test)]
mod replay_tests;
