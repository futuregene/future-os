//! Persistence lifecycle and unlocked durable history reads.
#[cfg(test)]
use super::JournalFailPoint;
use super::{
    is_buffered_delta, AppendFailure, EventBatchWriter, EventJournalState, RunProjectionSnapshot,
    SseBroadcaster, SseEvent,
};

impl SseBroadcaster {
    /// Bind this session's broadcaster to the Agent-owned SQLite writer.
    /// Tests and short-lived utility broadcasters may intentionally remain
    /// memory-only by never calling this method.
    pub fn configure_journal(
        &self,
        session_id: impl Into<String>,
        manager: &crate::session::Manager,
    ) -> anyhow::Result<()> {
        let session_id = session_id.into();
        {
            let journal = self.journal.lock();
            if !journal.session_id.is_empty() && journal.session_id != session_id {
                anyhow::bail!(
                    "event journal is permanently owned by session {}; cannot rebind to {}",
                    journal.session_id,
                    session_id
                );
            }
        }
        let result = (|| {
            let store = manager.storage()?.clone();
            store.bind_events(&session_id)?;
            let (session_idx, loose_idx) = store.event_cursors(&session_id)?;
            let writer = EventBatchWriter::new(
                store.clone(),
                session_id.clone(),
                self.journal_health.clone(),
            )?;
            Ok::<_, anyhow::Error>((store, writer, session_idx, loose_idx))
        })();
        let mut run = self.run.lock();
        let mut journal = self.journal.lock();
        if !journal.session_id.is_empty() && journal.session_id != session_id {
            anyhow::bail!(
                "event journal is permanently owned by session {}; cannot rebind to {}",
                journal.session_id,
                session_id
            );
        }
        journal.session_id = session_id;
        match result {
            Ok((store, writer, session_idx, loose_idx)) => {
                if run.run_id.is_empty() {
                    run.idx = loose_idx;
                }
                journal.writer = Some(writer);
                journal.store = Some(store);
                journal.session_idx = session_idx;
                journal.closed = false;
                self.journal_health.clear();
                Ok(())
            }
            Err(error) => {
                self.journal_health
                    .fail(format!("event storage unavailable: {error}"));
                Err(error)
            }
        }
    }

    pub fn set_persistence_interrupt(
        &self,
        interrupt_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        self.journal_health.set_interrupt(interrupt_flag);
    }

    pub fn persistence_error(&self) -> Option<String> {
        self.journal_health.error()
    }

    pub(crate) fn journal_session_id(&self) -> String {
        self.journal.lock().session_id.clone()
    }

    /// Drain the writer's pending batch through a durable barrier. Run
    /// completion calls this before asking for journal compaction: the
    /// compactor publishes a snapshot and retires a run's raw rows, and an
    /// event for that run still sitting in the batch would then be refused.
    pub(crate) fn flush_journal(&self) -> anyhow::Result<()> {
        let journal = self.journal.lock();
        match journal.writer.as_ref() {
            Some(writer) => writer.flush(),
            None => Ok(()),
        }
    }

    /// Fence the event journal before session deletion. The journal mutex is
    /// also held by append for the complete transaction, so when this
    /// returns no append can recreate rows for the deleted session.
    pub(crate) fn shutdown_journal(&self) -> anyhow::Result<()> {
        let mut journal = self.journal.lock();
        journal.closed = true;
        let result = if let Some(writer) = journal.writer.as_mut() {
            writer.shutdown()
        } else {
            Ok(())
        };
        journal.writer.take();
        journal.store = None;
        self.journal_health.clear_interrupt();
        result
    }

    pub fn close_journal(&self) {
        let mut journal = self.journal.lock();
        journal.closed = true;
        journal.writer.take();
        journal.store = None;
        self.journal_health.clear_interrupt();
    }

    pub fn recover_storage(&self) -> anyhow::Result<()> {
        let journal = self.journal.lock();
        let store = journal
            .store
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("event storage is not configured"))?;
        store.db.call(|db| {
            let tx = db.transaction()?;
            tx.execute("INSERT INTO storage_meta(key,value) VALUES ('health','ok') ON CONFLICT(key) DO UPDATE SET value='ok'", [])?;
            tx.commit()?;
            Ok(())
        })?;
        journal
            .writer
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("event persistence worker is unavailable"))?
            .flush()?;
        self.journal_health.clear();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fail_next_append(&self) {
        self.journal.lock().fail_at = Some(JournalFailPoint::Append);
    }

    #[cfg(test)]
    pub(super) fn fail_at(&self, point: JournalFailPoint) {
        self.journal.lock().fail_at = Some(point);
    }

    #[cfg(test)]
    pub(super) fn journal_commit_count(&self) -> u64 {
        self.journal
            .lock()
            .writer
            .as_ref()
            .map_or(0, EventBatchWriter::commit_count)
    }

    pub fn session_events_since(&self, since_idx: i64) -> anyhow::Result<Vec<SseEvent>> {
        Ok(self.read_journal_since("", since_idx, true)?.1)
    }

    pub(super) fn append_journal(
        journal: &mut EventJournalState,
        event: &SseEvent,
    ) -> Result<(), AppendFailure> {
        #[cfg(test)]
        if journal.fail_at.take().is_some() {
            return Err(AppendFailure::Unavailable(
                "injected event persistence failure".to_string(),
            ));
        }
        debug_assert!(!journal.closed);
        let Some(writer) = journal.writer.as_ref() else {
            return Ok(());
        };
        let event_type = event.event_type.clone();
        let event = serde_json::to_value(event).map_err(|error| {
            AppendFailure::Unavailable(format!("event serialization failed: {error}"))
        })?;
        writer.append(event, !is_buffered_delta(&event_type))
    }

    pub(super) fn read_journal_since(
        &self,
        run_id: &str,
        since_idx: i64,
        session_scope: bool,
    ) -> anyhow::Result<(bool, Vec<SseEvent>)> {
        let (known, events, _) = self.read_journal_page(run_id, since_idx, session_scope, None)?;
        Ok((known, events))
    }

    #[cfg(test)]
    pub(super) fn read_journal(&self, run_id: &str) -> anyhow::Result<Vec<SseEvent>> {
        let (_, events, snapshot) = self.read_journal_page(run_id, -1, false, None)?;
        Ok(snapshot.map_or(events, |s| s.events))
    }

    pub(super) fn read_journal_page(
        &self,
        run_id: &str,
        since_idx: i64,
        session_scope: bool,
        limit: Option<usize>,
    ) -> anyhow::Result<(bool, Vec<SseEvent>, Option<RunProjectionSnapshot>)> {
        let (store, session_id, sender) = {
            let journal = self.journal.lock();
            let Some(store) = journal.store.as_ref() else {
                return Ok((false, Vec::new(), None));
            };
            let writer = journal
                .writer
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("event persistence worker is unavailable"))?;
            (
                store.clone(),
                journal.session_id.clone(),
                writer.sender.clone(),
            )
        };
        // The FIFO flush fences every append that preceded this read. Wait
        // outside both broadcaster locks; later appends may enqueue normally.
        let flushed = EventBatchWriter::queue_flush(&sender)?;
        #[cfg(test)]
        let notify = self.journal.lock().read_flush_queued.clone();
        #[cfg(test)]
        if let Some(notify) = notify {
            let _ = notify.send(());
        }
        flushed
            .recv()
            .map_err(|_| anyhow::anyhow!("event persistence worker is unavailable"))?
            .map_err(anyhow::Error::msg)?;
        drop(sender);
        let page = store.replay_page(&session_id, run_id, since_idx, session_scope, limit)?;
        let snapshot = page.snapshot.map(|snapshot| RunProjectionSnapshot {
            run_id: snapshot.run_id,
            epoch: snapshot.epoch,
            run_sequence: snapshot.run_sequence,
            cursor: snapshot.cursor,
            events: snapshot.events,
        });
        Ok((page.known, page.events, snapshot))
    }
}
