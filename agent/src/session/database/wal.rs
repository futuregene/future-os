//! PASSIVE checkpointing never runs inside an application's commit. The worker
//! owns no session bodies. TRUNCATE is an explicit startup import command only;
//! normal service and shutdown use PASSIVE.
use super::metrics::Metrics;
use anyhow::{ensure, Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::Ordering, mpsc, Arc},
    time::{Duration, Instant},
};

const PERIOD: Duration = Duration::from_millis(250);
type Observation = mpsc::SyncSender<std::result::Result<(i64, i64, i64), String>>;
enum Command {
    Stop,
    Truncate(Observation),
    #[cfg(test)]
    Observe(Observation),
}
pub(super) struct Checkpointer {
    stop: parking_lot::Mutex<Option<mpsc::Sender<Command>>>,
    thread: parking_lot::Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl Checkpointer {
    pub(super) fn open(path: &Path, metrics: Arc<Metrics>) -> Result<Self> {
        let path = path.to_owned();
        let (stop, stopping) = mpsc::channel();
        let (ready, started) = mpsc::sync_channel(1);
        let thread=std::thread::Builder::new().name("agent-sqlite-checkpoint".into()).spawn(move || {
            let opened=(||->Result<Connection> {
                let connection=Connection::open_with_flags(&path,OpenFlags::SQLITE_OPEN_READ_WRITE|OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
                let (application,version):(i64,i64)=(connection.pragma_query_value(None,"application_id",|row|row.get(0))?,connection.pragma_query_value(None,"user_version",|row|row.get(0))?);
                ensure!(application==0x46555452 && version==super::CURRENT_SCHEMA_VERSION,"checkpoint connection has an unexpected database identity");
                connection.busy_timeout(Duration::ZERO)?;
                connection.pragma_update(None,"synchronous","FULL")?;
                connection.pragma_update(None,"wal_autocheckpoint",0)?;
                Ok(connection)
            })();
            let connection=match opened {Ok(connection)=>connection,Err(error)=>{let _=ready.send(Err(error));return;}};
            if ready.send(Ok(())).is_err() {return;}
            let mut reported=false;
            loop {
                let (finishing,truncate,observation):(bool,bool,Option<Observation>)=match stopping.recv_timeout(PERIOD) {
                    Ok(Command::Stop)|Err(mpsc::RecvTimeoutError::Disconnected)=>(true,false,None),
                    Err(mpsc::RecvTimeoutError::Timeout)=>(false,false,None),
                    Ok(Command::Truncate(ack))=>(false,true,Some(ack)),
                    #[cfg(test)]
                    Ok(Command::Observe(ack))=>(false,false,Some(ack)),
                };
                let start=Instant::now();
                let sql=if truncate {"PRAGMA wal_checkpoint(TRUNCATE)"} else {"PRAGMA wal_checkpoint(PASSIVE)"};
                let result=connection.query_row(sql,[],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?)));
                metrics.checkpoint.record(start.elapsed(),result.is_ok());
                let observed=result.as_ref().copied().map_err(ToString::to_string);
                match result {
                    Ok((busy,frames,copied))=> {
                        if busy!=0 {metrics.checkpoint_busy.fetch_add(1,Ordering::Relaxed);}
                        metrics.wal_frames.store(frames,Ordering::Relaxed);
                        metrics.checkpointed_frames.store(copied,Ordering::Relaxed);
                        reported=false;
                    },
                    Err(error)=> {if !reported {tracing::warn!(%error,"WAL checkpoint failed; durable commits remain in WAL");reported=true;}},
                }
                observe_files(&path,&metrics);
                if let Some(observation)=observation {let _=observation.send(observed);}
                if finishing {break;}
            }
        }).context("start SQLite checkpoint worker")?;
        let worker = Self {
            stop: parking_lot::Mutex::new(Some(stop)),
            thread: parking_lot::Mutex::new(Some(thread)),
        };
        started
            .recv()
            .context("SQLite checkpoint worker initialization stopped")??;
        Ok(worker)
    }
    pub(super) fn shutdown(&self) -> Result<()> {
        if let Some(stop) = self.stop.lock().take() {
            let _ = stop.send(Command::Stop);
        }
        if let Some(thread) = self.thread.lock().take() {
            if thread.thread().id() != std::thread::current().id() {
                thread
                    .join()
                    .map_err(|_| anyhow::anyhow!("SQLite checkpoint worker panicked"))?;
            }
        }
        Ok(())
    }
    fn truncate_after_import(&self) -> Result<(i64, i64, i64)> {
        let (ack, result) = mpsc::sync_channel(1);
        self.stop
            .lock()
            .as_ref()
            .context("checkpoint worker is closed")?
            .send(Command::Truncate(ack))
            .context("checkpoint worker stopped")?;
        result
            .recv()
            .context("import checkpoint observation stopped")?
            .map_err(anyhow::Error::msg)
    }
    #[cfg(test)]
    fn observe(&self) -> Result<(i64, i64, i64)> {
        let (ack, result) = mpsc::sync_channel(1);
        self.stop
            .lock()
            .as_ref()
            .context("checkpoint worker is closed")?
            .send(Command::Observe(ack))
            .context("checkpoint worker stopped")?;
        result
            .recv()
            .context("checkpoint observation stopped")?
            .map_err(anyhow::Error::msg)
    }
}

impl super::Database {
    /// Startup-only reclamation, serialized with the periodic checkpoint so
    /// SQLite's checkpoint lock cannot be held by our own PASSIVE worker.
    pub(crate) fn checkpoint_after_import(&self) -> Result<()> {
        let (busy, _, _) = self.inner.wal.truncate_after_import()?;
        if busy != 0 {
            tracing::warn!(
                "legacy import checkpoint is busy; committed data remains durable in WAL"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
impl super::Database {
    pub(crate) fn checkpoint_for_test(&self) -> Result<(i64, i64, i64)> {
        self.inner.wal.observe()
    }
}

#[cfg(test)]
#[path = "wal_tests.rs"]
mod tests;
impl Drop for Checkpointer {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn observe_files(path: &Path, metrics: &Metrics) {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let wal_bytes = std::fs::metadata(PathBuf::from(wal))
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    metrics.database_bytes.store(
        std::fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(0),
        Ordering::Relaxed,
    );
    metrics.wal_bytes.store(wal_bytes, Ordering::Relaxed);
    metrics
        .max_wal_bytes
        .fetch_max(wal_bytes, Ordering::Relaxed);
}
