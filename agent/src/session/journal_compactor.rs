//! Bounded completion notifications for optional run-journal compaction.
//! No startup sweep and no retries after a failed snapshot publication.
use super::sqlite_store::SqliteStore;
use anyhow::Result;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};

// Completion notifications are the only trigger. No startup/history sweep and
// no automatic retries: failures keep raw replay available. Drop cancels an
// uncommitted build between pages and joins before releasing the store.
enum CompactionRequest {
    Compact(String, String, PendingCompaction),
    /// The one bounded startup pass: the runs no completion notification covered.
    CatchUp,
    Stop,
    #[cfg(test)]
    Barrier(mpsc::SyncSender<()>),
}

struct PendingCompaction(Arc<AtomicUsize>);
impl Drop for PendingCompaction {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct CompletedRunCompactor {
    sender: mpsc::SyncSender<CompactionRequest>,
    thread: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    pending: Arc<AtomicUsize>,
}

impl CompletedRunCompactor {
    pub(crate) fn start(store: SqliteStore) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<CompactionRequest>(32);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("agent-journal-compactor".into())
            .spawn(move || {
                while !stopping.load(Ordering::Acquire) {
                    match receiver.recv() {
                        Ok(CompactionRequest::Compact(session, run, _pending)) => {
                            if stopping.load(Ordering::Acquire) { break; }
                            if let Err(error) = store.compact_run_cancellable(&session, &run, &stopping) {
                                tracing::warn!(%error, %session, %run, "could not compact completed journal; raw rows retained without retry");
                            }
                        }
                        Ok(CompactionRequest::CatchUp) => {
                            if stopping.load(Ordering::Acquire) { break; }
                            // Bounded on both axes and logged either way: a
                            // catch-up that finds nothing and one that never ran
                            // must not look the same in a store that stops
                            // shrinking.
                            if let Err(error) = store.catch_up_settled_journals(
                                super::journal_reclaim::CATCH_UP_MAX_RUNS,
                                super::journal_reclaim::CATCH_UP_BUDGET,
                            ) {
                                tracing::warn!(%error, "run-journal catch-up could not run; the next start retries it");
                            }
                        }
                        #[cfg(test)]
                        Ok(CompactionRequest::Barrier(sender)) => { let _ = sender.send(()); }
                        Ok(CompactionRequest::Stop) | Err(_) => break,
                    }
                }
            })?;
        // First request on the queue, so a start folds what the last stop left
        // behind before it handles new completions. Queued rather than run here:
        // starting the Agent must not wait for maintenance.
        let _ = sender.try_send(CompactionRequest::CatchUp);
        Ok(Self {
            sender,
            thread: Some(thread),
            stop,
            pending: Arc::new(AtomicUsize::new(0)),
        })
    }

    #[cfg(test)]
    pub(super) fn flush(&self) {
        let (sender, receiver) = mpsc::sync_channel(0);
        self.sender
            .send(CompactionRequest::Barrier(sender))
            .unwrap();
        receiver.recv().unwrap();
    }

    pub(crate) fn shutdown(mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        let _ = self.sender.send(CompactionRequest::Stop);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("journal compactor panicked during shutdown"))?;
        }
        Ok(())
    }

    pub(crate) fn request(&self, session: &str, run: &str) {
        self.pending.fetch_add(1, Ordering::AcqRel);
        let _ = self.sender.try_send(CompactionRequest::Compact(
            session.to_owned(),
            run.to_owned(),
            PendingCompaction(self.pending.clone()),
        ));
    }

    pub(crate) fn pending_count(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }
}

impl Drop for CompletedRunCompactor {
    fn drop(&mut self) {
        if self.thread.is_none() {
            return;
        }
        self.stop.store(true, Ordering::Release);
        let _ = self.sender.send(CompactionRequest::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
