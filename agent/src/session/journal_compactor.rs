//! Bounded completion notifications for optional run-journal compaction.
//! No startup sweep and no retries after a failed snapshot publication.
use super::sqlite_store::SqliteStore;
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

// Completion notifications are the only trigger. No startup/history sweep and
// no automatic retries: failures keep raw replay available. Drop cancels an
// uncommitted build between pages and joins before releasing the store.
enum CompactionRequest {
    Compact(String, String),
    Stop,
    #[cfg(test)]
    Barrier(mpsc::SyncSender<()>),
}

pub(crate) struct CompletedRunCompactor {
    sender: mpsc::SyncSender<CompactionRequest>,
    thread: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
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
                        Ok(CompactionRequest::Compact(session, run)) => {
                            if stopping.load(Ordering::Acquire) { break; }
                            if let Err(error) = store.compact_run_cancellable(&session, &run, &stopping) {
                                tracing::warn!(%error, %session, %run, "could not compact completed journal; raw rows retained without retry");
                            }
                        }
                        #[cfg(test)]
                        Ok(CompactionRequest::Barrier(sender)) => { let _ = sender.send(()); }
                        Ok(CompactionRequest::Stop) | Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            sender,
            thread: Some(thread),
            stop,
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

    pub(crate) fn request(&self, session: &str, run: &str) {
        let _ = self.sender.try_send(CompactionRequest::Compact(
            session.to_owned(),
            run.to_owned(),
        ));
    }
}

impl Drop for CompletedRunCompactor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.sender.send(CompactionRequest::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
