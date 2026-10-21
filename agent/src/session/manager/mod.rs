//! SQLite session persistence, ordered appends and transactional run recovery.
//!
//! Summary scanning and listing live in [`super::summary`].

use super::entry::SessionEntry;
use super::model::Session;
use super::run_journal::RUN_STATE_INTERRUPTED_BY_RESTART;
use super::sqlite_store::SqliteStore;
use crate::utils::default_session_dir;
#[cfg(test)]
use anyhow::anyhow;
use anyhow::Result;
use rusqlite::params;
use std::path::PathBuf;
use std::sync::OnceLock;

pub struct Manager {
    pub dir: PathBuf,
    store: OnceLock<SqliteStore>,
    initialization: parking_lot::Mutex<()>,
    journal_compactor: parking_lot::Mutex<Option<super::journal_compactor::CompletedRunCompactor>>,
    stopping: std::sync::atomic::AtomicBool,
    /// Bounded LRU for legacy whole-history reads, keyed by transcript revision.
    /// Paged reads use the persisted display index directly.
    display_projection_cache: parking_lot::Mutex<Vec<DisplayProjectionCacheEntry>>,
    /// Test-only save-failure injection (number of saves left to fail).
    #[cfg(test)]
    pub(crate) fail_saves_remaining: std::sync::atomic::AtomicU64,
}

impl Manager {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            stopping: std::sync::atomic::AtomicBool::new(false),
            dir,
            store: OnceLock::new(),
            initialization: parking_lot::Mutex::new(()),
            journal_compactor: parking_lot::Mutex::new(None),
            display_projection_cache: parking_lot::Mutex::new(Vec::new()),
            #[cfg(test)]
            fail_saves_remaining: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn default_for(cwd: &str) -> Self {
        Self::new(default_session_dir(cwd))
    }

    #[cfg(test)]
    pub(crate) fn test_execute(&self, sql: &str) {
        let sql = sql.to_owned();
        self.storage()
            .unwrap()
            .db
            .call(move |db| {
                db.execute_batch(&sql)?;
                Ok(())
            })
            .unwrap();
    }

    pub fn database_path(&self) -> PathBuf {
        if self.dir.file_name().and_then(|s| s.to_str()) == Some("sessions") {
            self.dir.parent().unwrap_or(&self.dir).join("agent.db")
        } else {
            self.dir.join("agent.db")
        }
    }

    /// Called before serving RPC, while the Agent instance lock is held.
    pub fn initialize(&self) -> Result<()> {
        self.initialize_cancellable(&crate::service::StartupCancellation::default())
    }

    pub(crate) fn initialize_cancellable(
        &self,
        cancellation: &crate::service::StartupCancellation,
    ) -> Result<()> {
        self.storage_cancellable(cancellation)?;
        cancellation.check()?;
        self.start_journal_compactor()
    }

    fn start_journal_compactor(&self) -> Result<()> {
        let mut journal_compactor = self.journal_compactor.lock();
        anyhow::ensure!(
            !self.stopping.load(std::sync::atomic::Ordering::Acquire),
            "session manager is stopping"
        );
        if journal_compactor.is_none() {
            *journal_compactor = Some(super::journal_compactor::CompletedRunCompactor::start(
                self.storage()?.clone(),
            )?);
        }
        Ok(())
    }

    pub(crate) fn request_journal_compaction(&self, session: &str, run: &str) {
        if let Err(error) = self.start_journal_compactor() {
            tracing::warn!(%error, "could not start journal compactor");
            return;
        }
        if let Some(journal_compactor) = self.journal_compactor.lock().as_ref() {
            journal_compactor.request(session, run);
        }
    }

    pub(crate) fn journal_compaction_activity(&self) -> Option<usize> {
        self.journal_compactor.try_lock().map(|compactor| {
            compactor
                .as_ref()
                .map_or(0, |compactor| compactor.pending_count())
        })
    }

    pub(crate) fn shutdown(&self) -> Result<()> {
        self.stopping
            .store(true, std::sync::atomic::Ordering::Release);
        // Stop the optional builder before closing its database connection.
        if let Some(compactor) = self.journal_compactor.lock().take() {
            compactor.shutdown()?;
        }
        if let Some(store) = self.store.get() {
            store.db.shutdown()?;
        }
        Ok(())
    }

    pub(crate) fn storage(&self) -> Result<&SqliteStore> {
        self.storage_cancellable(&crate::service::StartupCancellation::default())
    }

    fn storage_cancellable(
        &self,
        cancellation: &crate::service::StartupCancellation,
    ) -> Result<&SqliteStore> {
        cancellation.check()?;
        anyhow::ensure!(
            !self.stopping.load(std::sync::atomic::Ordering::Acquire),
            "session storage is closed"
        );
        if let Some(store) = self.store.get() {
            return Ok(store);
        }
        let _guard = self.initialization.lock();
        if self.store.get().is_none() {
            // Emitted where the work actually happens, so the CLI's own
            // initialization paths report the same progress. `agent.db` and the
            // legacy history can both live on slow shared storage.
            tracing::info!(
                database = %self.database_path().display(),
                "session storage: opening"
            );
            let store = SqliteStore::open_cancellable(&self.database_path(), cancellation.clone())?;
            store.import_legacy_cancellable(&self.dir, None, cancellation)?;
            store.recover_queued_receipts(None, false)?;
            cancellation.check()?;
            let _ = self.store.set(store);
            tracing::info!("session storage: ready");
        }
        Ok(self.store.get().expect("initialized SQLite store"))
    }

    pub fn retry_legacy_import(&self, id: &str) -> Result<()> {
        self.storage()?.import_legacy(&self.dir, Some(id))
    }

    pub fn import_records(&self) -> Result<Vec<super::ImportRecord>> {
        self.storage()?.import_records()
    }

    pub fn contains(&self, id: &str) -> Result<bool> {
        self.storage()?.contains(id)
    }

    fn encoded(entries: &[SessionEntry]) -> Result<Vec<serde_json::Value>> {
        entries
            .iter()
            .map(|entry| Ok(serde_json::from_str(&Self::serialize_entry(entry)?)?))
            .collect()
    }

    pub fn append_entries(&self, id: &str, entries: &[SessionEntry]) -> Result<()> {
        self.storage()?.append(id, Self::encoded(entries)?)?;
        self.invalidate_display_projection(id);
        Ok(())
    }

    pub fn append_entries_synced(&self, id: &str, entries: &[SessionEntry]) -> Result<()> {
        self.append_entries(id, entries)
    }

    pub fn append_run_start(
        &self,
        id: &str,
        user: SessionEntry,
        started: SessionEntry,
    ) -> Result<()> {
        let id = id.to_owned();
        let run_id = started
            .content
            .as_ref()
            .and_then(|content| content["run_id"].as_str())
            .unwrap_or_default()
            .to_owned();
        let merged = user
            .meta
            .as_ref()
            .and_then(|meta| meta["coalesced_run_ids"].as_array())
            .cloned()
            .unwrap_or_default();
        let values = Self::encoded(&[user, started])?;
        self.storage()?.db.call(move |db| {
            // Reads the existing markers before it writes: take the write lock
            // at BEGIN, or a concurrent connection's commit invalidates this
            // snapshot and the upgrade fails instantly (SQLITE_BUSY_SNAPSHOT).
            let tx = crate::session::database::begin_write_transaction(db)?;
            let existing: Vec<SessionEntry> = super::sqlite_store::read_run_markers(&tx, &id)?
                .into_iter()
                .map(serde_json::from_value)
                .collect::<std::result::Result<_, _>>()?;
            if let Some(run) = super::find_unterminated_run(&existing) {
                let terminal =
                    SessionEntry::run_terminal(&run, RUN_STATE_INTERRUPTED_BY_RESTART, 0, 0, None);
                super::sqlite_store::insert_transcript_entries(
                    &tx,
                    &id,
                    Self::encoded(&[terminal])?,
                )?;
            }
            super::sqlite_store::insert_transcript_entries(&tx, &id, values)?;
            tx.execute("UPDATE queued_run_receipts SET status='started',payload_json=NULL,user_entry_json=NULL WHERE session_id=?1 AND run_id=?2",params![id,run_id])?;
            for merged_run in merged.iter().filter_map(serde_json::Value::as_str) {
                tx.execute("UPDATE queued_run_receipts SET status='merged',payload_json=NULL,user_entry_json=NULL WHERE session_id=?1 AND run_id=?2",params![id,merged_run])?;
            }
            tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1", [&id])?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Append a continuation run's `run_started` marker.
    ///
    /// Same transaction as [`Self::append_run_start`] minus the user entry,
    /// because a continuation has no user message: the run continues an
    /// interrupted one rather than answering something new. Closing the prior
    /// unterminated run as `interrupted_by_restart` stays here for the same
    /// reason it is in `append_run_start` — the journal must never carry two
    /// open runs.
    pub fn append_run_continuation(&self, id: &str, started: SessionEntry) -> Result<()> {
        let id = id.to_owned();
        let run_id = started
            .content
            .as_ref()
            .and_then(|content| content["run_id"].as_str())
            .unwrap_or_default()
            .to_owned();
        let values = Self::encoded(&[started])?;
        self.storage()?.db.call(move |db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            let existing: Vec<SessionEntry> = super::sqlite_store::read_run_markers(&tx, &id)?
                .into_iter()
                .map(serde_json::from_value)
                .collect::<std::result::Result<_, _>>()?;
            if let Some(run) = super::find_unterminated_run(&existing) {
                let terminal =
                    SessionEntry::run_terminal(&run, RUN_STATE_INTERRUPTED_BY_RESTART, 0, 0, None);
                super::sqlite_store::insert_transcript_entries(
                    &tx,
                    &id,
                    Self::encoded(&[terminal])?,
                )?;
            }
            super::sqlite_store::insert_transcript_entries(&tx, &id, values)?;
            tx.execute(
                "UPDATE queued_run_receipts SET status='started',payload_json=NULL,user_entry_json=NULL WHERE session_id=?1 AND run_id=?2",
                params![id, run_id],
            )?;
            tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1", [&id])?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn unterminated_run_id(&self, id: &str) -> Result<Option<String>> {
        if !self.contains(id)? {
            return Ok(None);
        }
        let id = id.to_owned();
        let entries: Vec<SessionEntry> = self
            .storage()?
            .db
            .call(move |db| super::sqlite_store::read_run_markers(db, &id))?
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?;
        Ok(super::find_unterminated_run(&entries))
    }

    pub(crate) fn run_terminal(&self, id: &str, run_id: &str) -> Result<Option<serde_json::Value>> {
        let (id, run_id) = (id.to_owned(), run_id.to_owned());
        self.storage()?.db.call(move |db| {
            let entries = super::sqlite_store::read_run_markers(db, &id)?
                .into_iter()
                .map(serde_json::from_value)
                .collect::<std::result::Result<Vec<SessionEntry>, _>>()?;
            Ok(super::find_run_terminal(&entries, &run_id))
        })
    }

    /// Atomic recovery outcome: an acknowledged or ambiguously committed
    /// terminal wins over every retry. Only the exact unfinished run is closed.
    pub(crate) fn settle_run_if_unfinished(
        &self,
        id: &str,
        run_id: &str,
        state: &str,
        reason: &str,
    ) -> Result<serde_json::Value> {
        let (id, run_id, state, reason) = (
            id.to_owned(),
            run_id.to_owned(),
            state.to_owned(),
            reason.to_owned(),
        );
        let session_id = id.clone();
        let result = self.storage()?.db.call(move |db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            let entries = super::sqlite_store::read_run_markers(&tx, &id)?
                .into_iter()
                .map(serde_json::from_value)
                .collect::<std::result::Result<Vec<SessionEntry>, _>>()?;
            if let Some(terminal) = super::find_run_terminal(&entries, &run_id) {
                return Ok(terminal);
            }
            let terminal = SessionEntry::run_terminal(&run_id, &state, 0, 0, Some(&reason));
            let payload = terminal.content.clone().unwrap_or_default();
            super::sqlite_store::insert_transcript_entries(&tx, &id, Self::encoded(&[terminal])?)?;
            tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1", [&id])?;
            tx.commit()?;
            Ok(payload)
        })?;
        self.invalidate_display_projection(&session_id);
        Ok(result)
    }

    pub fn save(&self, session: &Session) -> Result<()> {
        #[cfg(test)]
        if self
            .fail_saves_remaining
            .load(std::sync::atomic::Ordering::Acquire)
            > 0
        {
            self.fail_saves_remaining
                .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            return Err(anyhow!("injected session save failure"));
        }
        self.storage()?
            .replace(&session.id, Self::encoded(&session.entries)?)?;
        self.invalidate_display_projection(&session.id);
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        self.delete_many(&[id.to_owned()])
    }

    pub(crate) fn delete_many(&self, ids: &[String]) -> Result<()> {
        self.storage()?.delete_many(ids.to_vec())?;
        for id in ids {
            self.invalidate_display_projection(id);
        }
        Ok(())
    }
}

mod display_cache;
mod fork;
mod loading;
mod metadata;
use display_cache::DisplayProjectionCacheEntry;
#[cfg(test)]
mod tests;
