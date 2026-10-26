//! Persistence lifecycle and unlocked durable history reads.
#[cfg(test)]
use super::JournalFailPoint;
use super::{
    is_buffered_delta, AppendFailure, EventBatchWriter, EventJournalState, RunProjectionSnapshot,
    SseBroadcaster, SseEvent,
};

fn validate_memory_boundary(
    run: &super::RunState,
    journal: &super::EventJournalState,
) -> anyhow::Result<()> {
    if let Some((session_idx, loose_idx)) = journal.memory_boundary {
        anyhow::ensure!(
            run.run_id.is_empty() && journal.session_idx == session_idx && run.idx == loose_idx,
            "ephemeral events have been produced; create another session to enable persistence"
        );
    }
    Ok(())
}

impl SseBroadcaster {
    #[cfg(test)]
    pub(crate) fn configure_legacy_journal(
        &self,
        session_id: impl Into<String>,
        manager: &crate::session::Manager,
    ) -> anyhow::Result<()> {
        let session_id = session_id.into();
        manager.storage()?.bind_legacy_events(&session_id)?;
        self.configure_journal(session_id, manager)
    }

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
            let run = self.run.lock();
            let journal = self.journal.lock();
            validate_memory_boundary(&run, &journal)?;
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
            let identity = store.storage_identity()?;
            let (session_idx, loose_idx) = store.event_cursors(&session_id)?;
            let segmented =
                store.storage_format(&session_id)? == crate::session::content::FORMAT_VERSION;
            let writer = EventBatchWriter::new(
                store.clone(),
                session_id.clone(),
                self.journal_health.clone(),
            )?;
            Ok::<_, anyhow::Error>((store, writer, session_idx, loose_idx, segmented, identity))
        })();
        let mut run = self.run.lock();
        let mut journal = self.journal.lock();
        validate_memory_boundary(&run, &journal)?;
        if !journal.session_id.is_empty() && journal.session_id != session_id {
            anyhow::bail!(
                "event journal is permanently owned by session {}; cannot rebind to {}",
                journal.session_id,
                session_id
            );
        }
        journal.session_id = session_id;
        match result {
            Ok((store, writer, session_idx, loose_idx, segmented, identity)) => {
                journal.storage_identity = Some(identity);
                journal.memory_boundary = None;
                journal.segmented = segmented;
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

    pub(crate) fn take_tool_result_entry_id(&self, tool: &str) -> Option<String> {
        let mut journal = self.journal.lock();
        if !journal.segmented {
            return None;
        }
        journal.content_sequence.take_result_entry(tool)
    }

    pub(crate) fn uses_segmented_storage(&self) -> bool {
        self.journal.lock().segmented
    }

    pub(crate) fn has_started_run(&self) -> bool {
        !self.run.lock().run_id.is_empty()
    }

    /// Select memory delivery before a prompt is admitted. The old actor is
    /// drained and joined before exposing the new mode; it cannot persist a
    /// later ephemeral body. A failed drain leaves admission closed.
    pub(crate) fn use_memory_journal(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.has_started_run(),
            "persistence mode is fixed after a run starts"
        );
        let writer = {
            let mut journal = self.journal.lock();
            anyhow::ensure!(!journal.closed, "event persistence is closed");
            journal.closed = true;
            journal.writer.take()
        };
        if let Some(mut writer) = writer {
            if let Err(error) = writer.shutdown() {
                self.journal_health
                    .fail(format!("could not enter ephemeral mode: {error:#}"));
                return Err(error);
            }
        }
        let mut run = self.run.lock();
        let mut journal = self.journal.lock();
        journal.memory_boundary = Some((journal.session_idx, run.idx));
        run.projection_events.limit(8 * 1024 * 1024);
        journal.segmented = false;
        journal.store = None;
        journal.storage_identity = None;
        journal.closed = false;
        self.journal_health.clear();
        Ok(())
    }

    pub fn persistence_error(&self) -> Option<String> {
        self.journal_health.error()
    }

    pub(crate) fn fail_persistence(&self, error: impl std::fmt::Display) {
        self.journal_health
            .fail(format!("transcript persistence failed: {error}"));
    }

    pub(crate) fn journal_metrics(&self) -> serde_json::Value {
        let journal = self.journal.lock();
        serde_json::json!({"segmented":journal.segmented,"memoryOnly":journal.memory_boundary.is_some(),
            "pending":journal.writer.as_ref().map(|writer| writer.admission.snapshot()),
            "bodyQueueEvents":journal.writer.as_ref().map(|writer|writer.sender.len()),
            "controlQueueCommands":journal.writer.as_ref().map(|writer|writer.control.sender.len()),
            "writer":self.journal_health.metrics.snapshot()})
    }

    pub(crate) fn journal_session_id(&self) -> String {
        self.journal.lock().session_id.clone()
    }

    /// Fence all admitted writes without retaining broadcaster locks while the
    /// writer commits. A later producer may enqueue behind this FIFO barrier.
    pub(crate) fn flush_journal(&self) -> anyhow::Result<()> {
        let sender = {
            let mut journal = self.journal.lock();
            if cfg!(test) && std::mem::take(&mut journal.fail_flush) {
                return Err(anyhow::anyhow!("injected event journal drain failure"));
            }
            journal.writer.as_ref().map(|writer| writer.control.clone())
        };
        if let Some(sender) = sender {
            sender
                .queue_flush()?
                .recv()
                .map_err(|_| anyhow::anyhow!("event persistence worker is unavailable"))?
                .map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }

    /// Await a durable causal boundary without blocking a Tokio worker or
    /// holding broadcaster locks. Dropping the future does not cancel the
    /// already-admitted write or its actor-owned memory reservation.
    pub(crate) async fn flush_journal_async(&self) -> anyhow::Result<()> {
        let result = async {
            if let Some(error) = self.persistence_error() {
                anyhow::bail!(error);
            }
            let control = {
                let mut journal = self.journal.lock();
                if cfg!(test) && std::mem::take(&mut journal.fail_flush) {
                    anyhow::bail!("injected event journal drain failure");
                }
                if journal.closed {
                    anyhow::bail!("event persistence is closed");
                }
                journal.writer.as_ref().map(|writer| writer.control.clone())
            };
            if let Some(control) = control {
                control
                    .queue_async_flush()?
                    .await
                    .map_err(|_| anyhow::anyhow!("event commit acknowledgment lost"))?
                    .map_err(anyhow::Error::msg)?;
            }
            if let Some(error) = self.persistence_error() {
                anyhow::bail!(error);
            }
            Ok(())
        }
        .await;
        if let Err(error) = &result {
            self.journal_health
                .fail(format!("event durability barrier failed: {error:#}"));
        }
        result
    }

    /// The terminal event was committed with its transcript marker. Publish
    /// that exact record without enqueuing another copy or reserving a cursor.
    pub(crate) async fn publish_committed_terminal(&self) -> anyhow::Result<()> {
        let (scope, after) = {
            let run = self.run.lock();
            (
                crate::session::content::stream::StreamIdentity {
                    session_id: self.journal_session_id(),
                    run_id: run.run_id.clone(),
                    epoch: run.epoch,
                },
                run.idx - 1,
            )
        };
        let store = self
            .journal
            .lock()
            .store
            .clone()
            .ok_or_else(|| anyhow::anyhow!("canonical event storage is unavailable"))?;
        let query_scope = scope.clone();
        let page = tokio::task::spawn_blocking(move || {
            store.segmented_stream_page(&query_scope, after, 1)
        })
        .await??;
        let event = page
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("committed terminal replay record missing"))?;
        anyhow::ensure!(
            event.event_type == "agent_end" && event.idx == after + 1,
            "unexpected event at terminal publication boundary"
        );
        let mut run = self.run.lock();
        anyhow::ensure!(
            run.run_id == scope.run_id && run.epoch == scope.epoch && run.idx == event.idx,
            "run changed before terminal publication"
        );
        run.idx += 1;
        run.record_event(&event, true);
        let _ = self.tx.send(event);
        Ok(())
    }

    /// Close admission first, then drain and join outside the lifecycle lock.
    /// No producer can recreate a session after the deletion fence returns.
    pub(crate) fn shutdown_journal(&self) -> anyhow::Result<()> {
        let writer = {
            let mut journal = self.journal.lock();
            journal.closed = true;
            journal.store = None;
            journal.writer.take()
        };
        self.journal_health.clear_interrupt();
        match writer {
            Some(mut writer) => writer.shutdown(),
            None => Ok(()),
        }
    }

    pub fn close_journal(&self) {
        let writer = {
            let mut journal = self.journal.lock();
            journal.closed = true;
            journal.store = None;
            journal.writer.take()
        };
        self.journal_health.clear_interrupt();
        drop(writer);
    }

    pub fn recover_storage(&self) -> anyhow::Result<()> {
        let (store, sender) = {
            let journal = self.journal.lock();
            let store = journal
                .store
                .clone()
                .ok_or_else(|| anyhow::anyhow!("event storage is not configured"))?;
            let sender = journal
                .writer
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("event persistence worker is unavailable"))?
                .control
                .clone();
            (store, sender)
        };
        store.db.call(|db| {
            let tx = db.transaction()?;
            tx.execute("INSERT INTO storage_meta(key,value) VALUES ('health','ok') ON CONFLICT(key) DO UPDATE SET value='ok'", [])?;
            tx.commit()?;
            Ok(())
        })?;
        sender
            .queue_flush()?
            .recv()
            .map_err(|_| anyhow::anyhow!("event persistence worker is unavailable"))?
            .map_err(anyhow::Error::msg)?;
        self.journal_health.clear();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fail_next_append(&self) {
        self.journal.lock().fail_at = Some(JournalFailPoint::Append);
    }

    /// Arm the next completion-time drain to fail **without** marking the journal
    /// unhealthy, so `prompt_internal`'s drain guard sees a healthy journal and
    /// still finds the drain failing.
    ///
    /// The error a live drain reports is reachable in production — a worker that
    /// died before answering a durable append, or a store that can no longer
    /// commit the batch — but both make an *append* fail first, which marks the
    /// journal unhealthy and takes the guard's other branch. That is why this
    /// stage needs the error injected rather than provoked.
    #[cfg(test)]
    pub(crate) fn fail_next_flush(&self) {
        self.journal.lock().fail_flush = true;
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
        canonical_data: Option<serde_json::Value>,
        bytes: usize,
    ) -> Result<Option<super::journal_writer::AppendReceipt>, AppendFailure> {
        #[cfg(test)]
        if journal.fail_at.take().is_some() {
            return Err(AppendFailure::Unavailable(
                "injected event persistence failure".to_string(),
            ));
        }
        debug_assert!(!journal.closed);
        let Some(writer) = journal.writer.as_ref() else {
            return Ok(None);
        };
        let event_type = event.event_type.clone();
        let mut event = serde_json::to_value(event).map_err(|error| {
            AppendFailure::Unavailable(format!("event serialization failed: {error}"))
        })?;
        if journal.segmented {
            event["content_fields"] = event["data"]
                .as_str()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .and_then(|data| {
                    data.get("content_refs")
                        .and_then(serde_json::Value::as_object)
                        .map(|refs| serde_json::json!(refs.keys().collect::<Vec<_>>()))
                })
                .unwrap_or_else(|| serde_json::json!([]));
            event["data"] = canonical_data.expect("canonical event prepared");
            writer.enqueue(event, !is_buffered_delta(&event_type), bytes)
        } else {
            writer
                .append(event, !is_buffered_delta(&event_type))
                .map(|_| None)
        }
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
                writer.control.clone(),
            )
        };
        // The FIFO flush fences every append that preceded this read. Wait
        // outside both broadcaster locks; later appends may enqueue normally.
        let flushed = sender.queue_flush()?;
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
