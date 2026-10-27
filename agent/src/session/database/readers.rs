//! Fixed, bounded read workers. They never run schema changes or writer jobs.
use super::DatabaseJob;
use anyhow::{anyhow, Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::{
    path::Path,
    sync::{mpsc, Arc},
    thread::JoinHandle,
    time::Duration,
};

const READ_WORKERS: usize = 2;
const READ_QUEUE: usize = 64;

pub(super) struct Readers {
    sender: parking_lot::Mutex<Option<mpsc::SyncSender<DatabaseJob>>>,
    threads: parking_lot::Mutex<Vec<JoinHandle<()>>>,
}

impl Readers {
    pub(super) fn open(path: &Path) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<DatabaseJob>(READ_QUEUE);
        let receiver = Arc::new(parking_lot::Mutex::new(receiver));
        let pool = Self {
            sender: parking_lot::Mutex::new(Some(sender)),
            threads: parking_lot::Mutex::new(Vec::new()),
        };
        for index in 0..READ_WORKERS {
            let path = path.to_owned();
            let receiver = receiver.clone();
            let (ready, started) = mpsc::sync_channel(1);
            let worker = std::thread::Builder::new()
                .name(format!("agent-sqlite-read-{index}"))
                .spawn(move || {
                    let connection = (|| -> Result<Connection> {
                        let db = Connection::open_with_flags(
                            path,
                            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                        )?;
                        crate::session::content::install_functions(&db)?;
                        db.busy_timeout(Duration::from_millis(250))?;
                        db.pragma_update(None, "query_only", true)?;
                        db.pragma_update(None, "foreign_keys", true)?;
                        // Prepare the schema before advertising readiness. The
                        // first visible read must not pay lazy schema parsing
                        // separately on each worker. LIMIT 0 reads no history.
                        drop(db.prepare("SELECT id FROM sessions LIMIT 0")?);
                        Ok(db)
                    })();
                    let mut db = match connection {
                        Ok(db) => db,
                        Err(error) => {
                            let _ = ready.send(Err(error));
                            return;
                        }
                    };
                    if ready.send(Ok(())).is_err() {
                        return;
                    }
                    loop {
                        // Only receiving is serialized. Jobs use separate connections.
                        let message = receiver.lock().recv();
                        match message {
                            Ok(job) => job(&mut db),
                            Err(_) => break,
                        }
                    }
                })
                .context("start SQLite read worker")?;
            pool.threads.lock().push(worker);
            started
                .recv()
                .context("SQLite read worker initialization stopped")??;
        }
        Ok(pool)
    }

    pub(super) fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (reply, result) = mpsc::sync_channel(1);
        self.sender
            .lock()
            .as_ref()
            .ok_or_else(|| anyhow!("SQLite readers are closed"))?
            .try_send(Box::new(move |db| {
                let _ = reply.send(operation(db));
            }))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => anyhow!("SQLite read queue is full"),
                mpsc::TrySendError::Disconnected(_) => anyhow!("SQLite readers are unavailable"),
            })?;
        result.recv().context("SQLite read interrupted")?
    }

    pub(super) fn shutdown(&self) -> Result<()> {
        self.sender.lock().take();
        let threads = std::mem::take(&mut *self.threads.lock());
        let mut panicked = false;
        for thread in threads {
            if thread.thread().id() != std::thread::current().id() && thread.join().is_err() {
                panicked = true;
            }
        }
        anyhow::ensure!(!panicked, "SQLite read worker panicked");
        Ok(())
    }
}

impl Drop for Readers {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
