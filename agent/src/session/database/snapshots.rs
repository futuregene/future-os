//! Bounded, warm read-only leases for one consistent history page. WAL lets
//! writers commit while a page reads older immutable bodies, even after their
//! current references are retired. No writer connection or worker is borrowed.
use anyhow::{ensure, Context, Result};
use rusqlite::Connection;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

const CONNECTIONS: usize = 4;
const WAIT: Duration = Duration::from_secs(5);
const LIFETIME: Duration = Duration::from_secs(10);

struct State {
    idle: Vec<Connection>,
    closed: bool,
}
struct Pool {
    state: parking_lot::Mutex<State>,
    available: parking_lot::Condvar,
    metrics: Arc<super::metrics::Metrics>,
}
pub(super) struct Readers(Arc<Pool>);

impl Readers {
    pub(super) fn open(path: &Path, metrics: Arc<super::metrics::Metrics>) -> Result<Self> {
        let idle = (0..CONNECTIONS)
            .map(|_| super::readers::open_connection(path))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self(Arc::new(Pool {
            state: parking_lot::Mutex::new(State {
                idle,
                closed: false,
            }),
            available: parking_lot::Condvar::new(),
            metrics,
        })))
    }

    pub(super) fn acquire(&self) -> Result<ReadSnapshot> {
        let deadline = Instant::now() + WAIT;
        let mut state = self.0.state.lock();
        loop {
            ensure!(!state.closed, "history snapshot readers are closed");
            if let Some(connection) = state.idle.pop() {
                drop(state);
                // The guard owns the connection before BEGIN, so a failed
                // BEGIN also returns/closes it without losing pool capacity.
                let snapshot = ReadSnapshot(Arc::new(Lease {
                    connection: parking_lot::Mutex::new(Some(connection)),
                    pool: self.0.clone(),
                    deadline: Instant::now() + LIFETIME,
                }));
                snapshot.read(|db| {
                    db.execute_batch("BEGIN DEFERRED")?;
                    Ok(())
                })?;
                return Ok(snapshot);
            }
            ensure!(
                Instant::now() < deadline,
                "history snapshot read admission timed out"
            );
            self.0.available.wait_until(&mut state, deadline);
        }
    }

    pub(super) fn shutdown(&self) {
        let mut state = self.0.state.lock();
        state.closed = true;
        state.idle.clear();
        self.0.available.notify_all();
    }
}
impl Drop for Readers {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Clone)]
pub(crate) struct ReadSnapshot(Arc<Lease>);
struct Lease {
    connection: parking_lot::Mutex<Option<Connection>>,
    pool: Arc<Pool>,
    deadline: Instant,
}
impl ReadSnapshot {
    pub(crate) fn read<T>(&self, read: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        ensure!(
            Instant::now() < self.0.deadline,
            "history read snapshot expired"
        );
        ensure!(
            !self.0.pool.state.lock().closed,
            "history snapshot readers are closed"
        );
        let queued = Instant::now();
        let mut connection = self.0.connection.lock();
        self.0
            .pool
            .metrics
            .reader_queue
            .record(queued.elapsed(), true);
        let started = Instant::now();
        let result = read(connection.as_mut().context("history snapshot is closed")?);
        self.0
            .pool
            .metrics
            .reader_job
            .record(started.elapsed(), result.is_ok());
        result
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.get_mut().take() {
            // Roll back before making it available to the next request. A
            // failed reset closes the handle rather than leaking its snapshot.
            let reset = connection.is_autocommit() || connection.execute_batch("ROLLBACK").is_ok();
            let mut state = self.pool.state.lock();
            if reset && !state.closed {
                state.idle.push(connection);
            }
            self.pool.available.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_is_consistent_query_only_and_does_not_hold_a_writer_or_normal_reader() {
        let root = tempfile::tempdir().unwrap();
        let database = super::super::Database::open(&root.path().join("agent.db")).unwrap();
        let snapshot = database.read_snapshot().unwrap();
        let before = snapshot
            .read(|db| {
                Ok(db.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?)
            })
            .unwrap();
        database
            .call(|db| {
                db.execute("INSERT INTO sessions(id,revision) VALUES ('new',0)", [])?;
                Ok(())
            })
            .unwrap();
        assert_eq!(
            snapshot
                .read(|db| Ok(
                    db.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?
                ))
                .unwrap(),
            before
        );
        assert_eq!(
            database
                .read(|db| Ok(
                    db.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?
                ))
                .unwrap(),
            before + 1
        );
        assert!(snapshot
            .read(|db| {
                db.execute("DELETE FROM sessions", [])?;
                Ok(())
            })
            .is_err());
        drop(snapshot);
        // Reusing every slot observes current state, not the prior transaction.
        let leases = (0..CONNECTIONS)
            .map(|_| database.read_snapshot().unwrap())
            .collect::<Vec<_>>();
        for lease in &leases {
            assert_eq!(
                lease
                    .read(|db| Ok(
                        db.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?
                    ))
                    .unwrap(),
                before + 1
            );
        }
        database.shutdown().unwrap();
        for lease in leases {
            assert!(lease.read(|_| Ok(())).is_err());
        }
    }
}
