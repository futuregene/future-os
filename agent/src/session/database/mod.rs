//! One ordered SQLite writer and independent, bounded read connections.
//! Successful writes return only after commit; readers never enter its queue.

use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 8;

/// Shared admission boundary for every entry opening agent.db. Structure and
/// bootstrap identity are validated separately before any schema mutation.
pub(crate) fn supports_schema_version(version: i64) -> bool {
    matches!(version, 0 | 2..=CURRENT_SCHEMA_VERSION)
}
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
    readers: readers::Readers,
    /// The fence lane, opened on first use. Most processes never ask for one
    /// (a CLI run, a test that only writes), and an idle connection is a file
    /// descriptor every future `Database::open` in a test binary would pay for.
    fence: parking_lot::Mutex<Option<Arc<readers::Readers>>>,
    fence_path: std::path::PathBuf,
    snapshots: snapshots::Readers,
    wal: wal::Checkpointer,
    metrics: Arc<metrics::Metrics>,
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
        let _ = self.wal.shutdown();
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
        let read_path = path.to_path_buf();
        let path = path.to_path_buf();
        let (sender, receiver) = mpsc::sync_channel::<DatabaseJob>(256);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let reclaim_requested = Arc::new(AtomicBool::new(true));
        let worker_reclaim = reclaim_requested.clone();
        let metrics = Arc::new(metrics::Metrics::default());
        let worker_metrics = metrics.clone();
        let thread = std::thread::Builder::new()
            .name("agent-sqlite".into())
            .spawn(move || {
                let _metrics=metrics::install(worker_metrics);
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
                let mut next_reclaim = std::time::Instant::now();
                loop {
                    match receiver.recv_timeout(Duration::from_millis(50)) {
                        Ok(job) => job(&mut connection),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if worker_reclaim.load(Ordering::Acquire)
                                && std::time::Instant::now() >= next_reclaim
                            {
                                // Timed like a job: this slice runs on the same
                                // thread as every write, so it is one of the
                                // things a stalled caller can be waiting for.
                                let started = std::time::Instant::now();
                                let slice = reclaim_space_when_idle(&mut connection);
                                slow::report("reclaim", started.elapsed(), Duration::ZERO);
                                match slice {
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
                                // A slice can issue many FULL-sync per-page commits.
                                // Give foreground jobs a quiet window before the
                                // next slice instead of retrying on every 50ms tick.
                                next_reclaim =
                                    std::time::Instant::now() + Duration::from_millis(250);
                            }
                        }
                    }
                }
            })
            .context("start SQLite worker")?;
        ready_rx
            .recv()
            .context("SQLite worker initialization stopped")??;
        let readers = match readers::Readers::open(&read_path) {
            Ok(readers) => readers,
            Err(error) => {
                drop(sender);
                let _ = thread.join();
                return Err(error);
            }
        };
        let snapshots = match snapshots::Readers::open(&read_path, metrics.clone()) {
            Ok(snapshots) => snapshots,
            Err(error) => {
                drop(readers);
                drop(sender);
                let _ = thread.join();
                return Err(error);
            }
        };
        let wal = match wal::Checkpointer::open(&read_path, metrics.clone()) {
            Ok(wal) => wal,
            Err(error) => {
                drop(snapshots);
                drop(readers);
                drop(sender);
                let _ = thread.join();
                return Err(error);
            }
        };
        Ok(Self {
            inner: Arc::new(DatabaseWorker {
                readers,
                fence: parking_lot::Mutex::new(None),
                fence_path: read_path,
                snapshots,
                wal,
                metrics,
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
        self.call_as("other", operation)
    }

    /// A writer job that names itself, so a job holding the thread long enough
    /// to stall its callers can be pointed at. Every production caller that can
    /// plausibly own seconds of work names itself; the rest keep `call`'s
    /// `job=other`, which is itself the answer when such a row appears.
    pub(crate) fn call_as<T: Send + 'static>(
        &self,
        job: &'static str,
        operation: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let queued = std::time::Instant::now();
        let metrics = self.inner.metrics.clone();
        self.inner
            .sender
            .lock()
            .as_ref()
            .ok_or_else(|| anyhow!("SQLite worker is closed"))?
            .try_send(Box::new(move |connection| {
                let waited = queued.elapsed();
                metrics.writer_queue.record(waited, true);
                let started = std::time::Instant::now();
                let result = operation(connection);
                let elapsed = started.elapsed();
                metrics.writer_job.record(elapsed, result.is_ok());
                slow::report(job, elapsed, waited);
                let _ = reply_tx.send(result);
            }))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => anyhow!("SQLite operation queue is full"),
                mpsc::TrySendError::Disconnected(_) => anyhow!("SQLite worker is unavailable"),
            })?;
        reply_rx.recv().context("SQLite operation interrupted")?
    }

    /// Read jobs never queue behind transcript writes or background maintenance.
    pub(crate) fn read<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let queued = std::time::Instant::now();
        let metrics = self.inner.metrics.clone();
        self.inner.readers.call(move |connection| {
            metrics.reader_queue.record(queued.elapsed(), true);
            let started = std::time::Instant::now();
            let result = operation(connection);
            metrics.reader_job.record(started.elapsed(), result.is_ok());
            result
        })
    }

    pub(crate) fn metrics(&self) -> serde_json::Value {
        self.inner.metrics.snapshot()
    }

    pub(crate) fn read_snapshot(&self) -> Result<ReadSnapshot> {
        self.inner.snapshots.acquire()
    }

    /// Close admission and join the ordered worker after its accepted jobs.
    /// The service supervisor owns the deadline, so a stuck job cannot cause
    /// the host to incorrectly report Stopped or release instance ownership.
    pub(crate) fn shutdown(&self) -> Result<()> {
        self.inner.snapshots.shutdown();
        if let Some(fence) = self.inner.fence.lock().take() {
            fence.shutdown()?;
        }
        self.inner.readers.shutdown()?;
        self.inner.sender.lock().take();
        if let Some(thread) = self.inner.thread.lock().take() {
            thread
                .join()
                .map_err(|_| anyhow!("SQLite worker panicked during shutdown"))?;
        }
        self.inner.wal.shutdown()?;
        Ok(())
    }

    /// A fence read: small, current, and never behind the ordered writer.
    ///
    /// Session presence and the source identity decide whether a caller may
    /// proceed, and they were read on the writer's lane — the only connection
    /// that existed when they were written. Every credential rotation and every
    /// run terminal then put them behind whatever job held that lane: measured
    /// 2026-10-29, four remote commands released together after 18–21 s because
    /// one writer job was running, and two of them had already hit their 30 s
    /// deadline.
    ///
    /// The trade is ordering, not freshness: a reader sees every *committed*
    /// write, but not one still queued. Callers that mutate and then read in the
    /// same sequence keep `call`; a fence gets a second look instead (see
    /// `agent_bridge::presence`).
    pub(crate) fn fence<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        // Opened once, on demand: a process that never asks for a fence read
        // (every CLI run, most tests) must not carry the connection.
        let lane = {
            let mut slot = self.inner.fence.lock();
            match slot.as_ref() {
                Some(lane) => lane.clone(),
                None => {
                    let lane = Arc::new(readers::Readers::open_fence(&self.inner.fence_path)?);
                    *slot = Some(lane.clone());
                    lane
                }
            }
        };
        lane.call(operation)
    }

    /// Non-blocking hint. Idle worker slices give queued foreground writes
    /// priority; startup may resume freed-page reclamation, never compaction
    /// or deletion of journal records.
    pub(crate) fn request_space_reclamation(&self) {
        self.inner.reclaim_requested.store(true, Ordering::Release);
    }
}

mod connection;
mod metrics;
mod readers;
mod reclamation;
mod schema;
mod slow;
mod snapshots;
pub(crate) use snapshots::ReadSnapshot;
mod transactions;
mod wal;
#[cfg(test)]
use connection::open_connection;
pub(crate) use reclamation::reclaim_space_when_idle;
pub(crate) use transactions::{begin_write_transaction, try_maintenance_transaction};

#[cfg(test)]
mod connection_tests;
#[cfg(test)]
mod historical_upgrade_tests;
#[cfg(test)]
mod reader_tests;
#[cfg(test)]
mod reclamation_arm_tests;
#[cfg(test)]
mod tests;
