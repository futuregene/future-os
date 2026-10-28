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
    stopping: std::sync::atomic::AtomicBool,
    /// Bounded LRU for legacy whole-history reads, keyed by transcript revision.
    /// Paged reads use the persisted display index directly.
    display_projection_cache: parking_lot::Mutex<Vec<DisplayProjectionCacheEntry>>,
    pub(in crate::session) history_fragments:
        parking_lot::Mutex<super::history::fragment_cache::FragmentCache>,
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
            display_projection_cache: parking_lot::Mutex::new(Vec::new()),
            history_fragments: parking_lot::Mutex::new(Default::default()),
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
        anyhow::ensure!(
            !self.stopping.load(std::sync::atomic::Ordering::Acquire),
            "session manager is stopping"
        );
        Ok(())
    }

    /// Compatibility metric. Production no longer starts or schedules a
    /// journal compactor, for either historical or canonical conversations.
    pub(crate) fn journal_compaction_activity(&self) -> Option<usize> {
        Some(0)
    }

    pub(crate) fn shutdown(&self) -> Result<()> {
        self.stopping
            .store(true, std::sync::atomic::Ordering::Release);
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
            let database = self.database_path();
            // Emitted where the work actually happens, so the CLI's own
            // initialization paths report the same progress. `agent.db` and the
            // legacy history can both live on slow shared storage.
            cancellation.report(crate::service::StartupStage::OpeningStorage);
            tracing::info!(
                database = %database.display(),
                "session storage: opening"
            );
            let store = startup::step(&database, "open_database", || {
                SqliteStore::open_cancellable(&database, cancellation.clone())
            })?;
            cancellation.report(crate::service::StartupStage::RecoveringSessions);
            startup::step(&database, "import_legacy_sessions", || {
                store.import_legacy_cancellable(&self.dir, None, cancellation)
            })?;
            startup::step(&database, "recover_hidden_forks", || {
                store.recover_hidden_forks(cancellation)
            })?;
            startup::step(&database, "recover_content_operations", || {
                store.recover_content_operations(cancellation)
            })?;
            let receipts = startup::step(&database, "recover_queued_receipts", || {
                store.recover_queued_receipts(None, false)
            })?;
            tracing::info!(receipts, "session storage: queued receipts recovered");
            startup::step(&database, "recover_segmented_runs", || {
                store.recover_segmented_runs()
            })?;
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

    /// Explicit run commits publish canonical replay termination atomically.
    /// Ordinary append/import/reset operations must not synthesize new events.
    pub(crate) fn commit_run_entries_synced(
        &self,
        id: &str,
        entries: &[SessionEntry],
    ) -> Result<()> {
        let store = self.storage()?;
        let values = if store.storage_format(id)? == super::content::FORMAT_VERSION {
            entries
                .iter()
                .map(|entry| {
                    if entry.entry_type == super::ENTRY_TYPE_SESSION_INFO {
                        let fields = super::content::metadata::run_fields(
                            entry
                                .content
                                .as_ref()
                                .ok_or_else(|| anyhow::anyhow!("run metadata has no content"))?,
                        )?;
                        let mut thin = SessionEntry::session_info(
                            serde_json::Value::Object(fields),
                            String::new(),
                            String::new(),
                        );
                        thin.id = entry.id.clone();
                        thin.timestamp = entry.timestamp;
                        Ok(serde_json::from_str(&Self::serialize_entry(&thin)?)?)
                    } else {
                        Ok(serde_json::from_str(&Self::serialize_entry(entry)?)?)
                    }
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            Self::encoded(entries)?
        };
        store.commit_terminal_entries(id, values)?;
        self.invalidate_display_projection(id);
        Ok(())
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
        let mut prepared = super::content::entries::prepare(self.storage()?, &id, values)?;
        let values = prepared.take();
        let retired=self.storage()?.db.call(move |db| {
            // Reads the existing markers before it writes: take the write lock
            // at BEGIN, or a concurrent connection's commit invalidates this
            // snapshot and the upgrade fails instantly (SQLITE_BUSY_SNAPSHOT).
            let tx = crate::session::database::begin_write_transaction(db)?;
            let id=super::identity::BoundIdentity::session(&tx,&id)?;
            let existing: Vec<SessionEntry> = super::sqlite_store::read_run_marker_headers(&tx, &id)?
                .into_iter()
                .map(serde_json::from_value)
                .collect::<std::result::Result<_, _>>()?;
            if let Some(run) = super::find_unterminated_run(&existing) {
                let terminal =
                    SessionEntry::run_terminal(&run, RUN_STATE_INTERRUPTED_BY_RESTART, 0, 0, None);
                super::sqlite_store::insert_transcript_entries(
                    &tx,
                    &id,
                    Self::encoded(std::slice::from_ref(&terminal))?,
                )?;
                if super::content::format(&tx,&id)? == super::content::FORMAT_VERSION {
                    super::content::terminal::publish(&tx,&id,&Self::encoded(&[terminal])?[0])?;
                }
            }
            super::sqlite_store::insert_transcript_entries(&tx, &id, values)?;
            let mut retired=retire_unpublished_queued_entry(&tx,&id,&run_id)?;
            let run_id=super::identity::BoundIdentity::run(&tx,&id,&run_id)?;
            tx.execute("UPDATE queued_run_receipts SET status='started',payload_json=NULL,user_entry_json=NULL WHERE session_id=?1 AND run_id=?2",params![id,run_id])?;
            for merged_run in merged.iter().filter_map(serde_json::Value::as_str) {
                // A coalesced queued input may never publish its original
                // entry header. Release exactly that receipt's staged owner,
                // preserving any entry that was actually published.
                retired+=retire_unpublished_queued_entry(&tx,&id,merged_run)?;
                let merged_run=super::identity::BoundIdentity::run(&tx,&id,merged_run)?;
                tx.execute("UPDATE queued_run_receipts SET status='merged',payload_json=NULL,user_entry_json=NULL WHERE session_id=?1 AND run_id=?2",params![id,merged_run])?;
            }
            tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1", [&id])?;
            tx.commit()?;
            Ok(retired)
        })?;
        if retired > 0 {
            self.storage()?.db.request_space_reclamation();
        }
        Ok(())
    }

    pub fn unterminated_run_id(&self, id: &str) -> Result<Option<String>> {
        self.run_candidate(id, false)
    }

    pub(crate) fn resumable_run_id(&self, id: &str) -> Result<Option<String>> {
        self.run_candidate(id, true)
    }

    fn run_candidate(&self, id: &str, resume: bool) -> Result<Option<String>> {
        if !self.contains(id)? {
            return Ok(None);
        }
        let id = id.to_owned();
        let entries: Vec<SessionEntry> = self
            .storage()?
            .db
            .read(move |db| super::sqlite_store::read_run_marker_headers(db, &id))?
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?;
        Ok(if resume {
            super::find_resumable_run(&entries)
        } else {
            super::find_unterminated_run(&entries)
        })
    }

    pub(crate) fn run_terminal(&self, id: &str, run_id: &str) -> Result<Option<serde_json::Value>> {
        let store = self.storage()?;
        if store.storage_format(id)? == super::content::FORMAT_VERSION {
            let (session, run) = (id.to_owned(), run_id.to_owned());
            let (scope,entry)=store.db.read(move |db| {
                use rusqlite::OptionalExtension;
                let tx=db.transaction()?;
                let scope=super::content::reader::Snapshot::capture_from(&tx,&session)?;
                let run=super::identity::BoundIdentity::run(&tx,&session,&run)?;
                let session=super::identity::BoundIdentity::session(&tx,&session)?;
                let entry=tx.query_row("SELECT position,entry_id FROM entries WHERE session_id=?1 AND run_id=?2 AND entry_type='run_terminal' ORDER BY position DESC LIMIT 1",
                    params![session,run],|row|Ok(super::content::reader::Entry {position:row.get(0)?,id:row.get(1)?})).optional()?;
                tx.commit()?;Ok((scope,entry))
            })?;
            return entry
                .map(|entry| Ok(scope.entry(store, &entry)?["content"].clone()))
                .transpose();
        }
        let (id, run_id) = (id.to_owned(), run_id.to_owned());
        store.db.read(move |db| {
            use rusqlite::OptionalExtension;
            let raw:Option<String> = db.query_row("SELECT payload FROM entry_records WHERE session_id=?1 AND entry_type='run_terminal' AND entry_id IN (SELECT entry_id FROM entries WHERE session_id=?1 AND run_id=?2) ORDER BY position DESC LIMIT 1",
                params![id,run_id],|row|row.get(0)).optional()?;
            raw.map(|raw|Ok(serde_json::from_str::<serde_json::Value>(&raw)?["content"].clone())).transpose()
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
            use rusqlite::OptionalExtension;
            let source = if super::content::format(&tx,&id)? == super::content::FORMAT_VERSION {
                "entry_header_records"
            } else { "entry_records" };
            let run_id=super::identity::BoundIdentity::run(&tx,&id,&run_id)?;
            let id=super::identity::BoundIdentity::session(&tx,&id)?;
            let previous:Option<String> = tx.query_row(&format!("SELECT payload FROM {source} WHERE session_id=?1 AND position=(SELECT max(position) FROM entries WHERE session_id=?1 AND run_id=?2 AND entry_type='run_terminal')"),
                params![id,run_id],|row|row.get(0)).optional()?;
            if let Some(previous) = previous {
                return Ok(serde_json::from_str::<serde_json::Value>(&previous)?["content"].clone());
            }
            let terminal = SessionEntry::run_terminal(&run_id, &state, 0, 0, Some(&reason));
            let payload = terminal.content.clone().unwrap_or_default();
            let encoded = Self::encoded(&[terminal])?;
            super::sqlite_store::insert_transcript_entries(&tx, &id, encoded.clone())?;
            if super::content::format(&tx, &id)? == super::content::FORMAT_VERSION {
                super::content::terminal::publish(&tx, &id, &encoded[0])?;
            }
            tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1", [&id])?;
            tx.commit()?;
            Ok(payload)
        })?;
        self.invalidate_display_projection(&session_id);
        Ok(result)
    }

    /// Startup only, before any executor is admitted. Settle durable user stops
    /// whose process died before cancellation committed, without resuming them.
    pub(crate) fn reconcile_user_stops_on_start(&self) -> Result<()> {
        self.storage()?.db.call(|db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            let ids = {
                let mut statement = tx.prepare("SELECT DISTINCT e.session_id FROM entries e JOIN runs r ON r.session_id=e.session_id AND r.run_id=e.run_id WHERE e.entry_type='run_stop_requested' AND r.status IN ('running','interrupted') AND NOT EXISTS(SELECT 1 FROM session_formats f WHERE f.session_id=e.session_id AND f.format=2)")?;
                let ids = statement.query_map([], |row| row.get::<_, String>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                ids
            };
            for id in ids {
                let entries: Vec<SessionEntry> = super::sqlite_store::read_run_marker_headers(&tx, &id)?
                    .into_iter().map(serde_json::from_value).collect::<std::result::Result<_, _>>()?;
                let Some(run) = entries.iter().rev().find(|e| e.entry_type == super::ENTRY_TYPE_RUN_STARTED)
                    .and_then(|e| e.content.as_ref()).and_then(|v| v["run_id"].as_str()) else { continue; };
                if !entries.iter().any(|e| e.entry_type == super::entry::ENTRY_TYPE_RUN_STOP_REQUESTED
                    && e.content.as_ref().is_some_and(|v| v["run_id"] == run)) { continue; }
                let previous = super::find_run_terminal(&entries, run);
                if previous.as_ref().is_some_and(|v| v["state"] != super::RUN_STATE_INTERRUPTED_BY_RESTART) { continue; }
                let mut terminal = SessionEntry::run_terminal(run, super::RUN_STATE_CANCELLED, 0, 0, None);
                if let Some(previous) = previous { terminal.content = Some(previous); }
                let content = terminal.content.as_mut().unwrap();
                content["state"] = super::RUN_STATE_CANCELLED.into();
                content["error"] = "Stopped by user before recovery".into();
                content["interruption_cause"] = "user_stop".into();
                super::sqlite_store::insert_transcript_entries(&tx, &id, Self::encoded(&[terminal])?)?;
                tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1", [&id])?;
            }
            tx.commit()?;
            Ok(())
        })
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

/// JSON receipts carry the public spelling; published headers and body owners
/// carry the typed key. Bind each at its own boundary, including an input that
/// was coalesced without ever publishing its original entry header.
fn retire_unpublished_queued_entry(
    db: &rusqlite::Connection,
    session: &str,
    run: &str,
) -> Result<usize> {
    use rusqlite::OptionalExtension as _;
    let stored_session = super::identity::session_key(db, session)?;
    let run = super::identity::BoundIdentity::run(db, session, run)?;
    let entry:Option<String>=db.query_row("SELECT json_extract(user_entry_json,'$.id') FROM queued_run_receipts WHERE session_id=?1 AND run_id=?2",params![stored_session,run],|row|row.get(0)).optional()?.flatten();
    let Some(entry) = entry else { return Ok(0) };
    let entry = super::identity::BoundIdentity::entry(db, session, &entry)?;
    Ok(db.execute("DELETE FROM content_references WHERE session_id=?1 AND owner_kind='entry' AND owner_id=?2 AND NOT EXISTS(SELECT 1 FROM entries e WHERE e.session_id=?1 AND e.entry_id=content_references.owner_id)",params![stored_session,entry])?)
}

mod display_cache;
mod fork;
mod loading;
mod metadata;
mod startup;
use display_cache::DisplayProjectionCacheEntry;
#[cfg(test)]
mod tests;
