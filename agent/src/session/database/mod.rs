//! Agent-owned SQLite worker. Every operation runs on one bounded, ordered
//! connection; a successful write returns only after its transaction commits.

use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 7;
// Fail compilation if the storage owner and the client handshake drift.
const _: () =
    assert!(CURRENT_SCHEMA_VERSION == future_rpc::connection::AGENT_STORAGE_SCHEMA_VERSION as i64);

type DatabaseJob = Box<dyn FnOnce(&mut Connection) + Send>;

#[derive(Clone)]
pub(crate) struct Database {
    inner: Arc<DatabaseWorker>,
}

struct DatabaseWorker {
    reclaim_requested: Arc<AtomicBool>,
    sender: parking_lot::Mutex<Option<mpsc::SyncSender<DatabaseJob>>>,
    thread: parking_lot::Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Drop for DatabaseWorker {
    fn drop(&mut self) {
        self.sender.get_mut().take();
        if let Some(thread) = self.thread.get_mut().take() {
            if thread.thread().id() != std::thread::current().id() {
                let _ = thread.join();
            }
        }
    }
}

impl Database {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        Self::open_cancellable(path, crate::service::StartupCancellation::default())
    }

    pub(crate) fn open_cancellable(
        path: &Path,
        cancellation: crate::service::StartupCancellation,
    ) -> Result<Self> {
        let path = path.to_path_buf();
        let (sender, receiver) = mpsc::sync_channel::<DatabaseJob>(256);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let reclaim_requested = Arc::new(AtomicBool::new(true));
        let worker_reclaim = reclaim_requested.clone();
        let thread = std::thread::Builder::new()
            .name("agent-sqlite".into())
            .spawn(move || {
                let mut connection = match connection::open_connection_cancellable(&path, &cancellation) {
                    Ok(connection) => connection,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                let mut reclaim_error_reported = false;
                loop {
                    match receiver.recv_timeout(Duration::from_millis(50)) {
                        Ok(job) => job(&mut connection),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if worker_reclaim.load(Ordering::Acquire) {
                                match reclaim_space_when_idle(&mut connection) {
                                    Ok(done) => {
                                        reclaim_error_reported = false;
                                        worker_reclaim.store(!done, Ordering::Release);
                                    }
                                    Err(error) => {
                                        if !reclaim_error_reported {
                                            tracing::warn!(%error, "background space reclamation failed; retrying on idle");
                                            reclaim_error_reported = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            })
            .context("start SQLite worker")?;
        ready_rx
            .recv()
            .context("SQLite worker initialization stopped")??;
        Ok(Self {
            inner: Arc::new(DatabaseWorker {
                sender: parking_lot::Mutex::new(Some(sender)),
                thread: parking_lot::Mutex::new(Some(thread)),
                reclaim_requested,
            }),
        })
    }

    pub(crate) fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.inner
            .sender
            .lock()
            .as_ref()
            .ok_or_else(|| anyhow!("SQLite worker is closed"))?
            .try_send(Box::new(move |connection| {
                let _ = reply_tx.send(operation(connection));
            }))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => anyhow!("SQLite operation queue is full"),
                mpsc::TrySendError::Disconnected(_) => anyhow!("SQLite worker is unavailable"),
            })?;
        reply_rx.recv().context("SQLite operation interrupted")?
    }

    /// Close admission and join the ordered worker after its accepted jobs.
    /// The service supervisor owns the deadline, so a stuck job cannot cause
    /// the host to incorrectly report Stopped or release instance ownership.
    pub(crate) fn shutdown(&self) -> Result<()> {
        self.inner.sender.lock().take();
        if let Some(thread) = self.inner.thread.lock().take() {
            thread
                .join()
                .map_err(|_| anyhow!("SQLite worker panicked during shutdown"))?;
        }
        Ok(())
    }

    /// Non-blocking hint. Idle worker slices give queued foreground writes
    /// priority; startup may resume freed-page reclamation, never compaction
    /// or deletion of journal records.
    pub(crate) fn request_space_reclamation(&self) {
        self.inner.reclaim_requested.store(true, Ordering::Release);
    }
}

mod connection;
mod reclamation;
mod schema;
mod transactions;
#[cfg(test)]
use connection::open_connection;
pub(crate) use reclamation::reclaim_space_when_idle;
pub(crate) use transactions::{begin_write_transaction, try_maintenance_transaction};

#[cfg(test)]
mod connection_tests;
#[cfg(test)]
mod reclamation_arm_tests;
#[cfg(test)]
mod tests;
