//! Foreground write retries and non-waiting maintenance transactions.
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::time::Duration;

pub(crate) struct DurableTransaction<'connection>(rusqlite::Transaction<'connection>);
impl<'connection> std::ops::Deref for DurableTransaction<'connection> {
    type Target = rusqlite::Transaction<'connection>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DurableTransaction<'_> {
    pub(crate) fn commit(self) -> rusqlite::Result<()> {
        let start = std::time::Instant::now();
        let result = self.0.commit();
        super::metrics::commit(start.elapsed(), result.is_ok());
        result
    }
}

fn is_lock_contention(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.code == rusqlite::ErrorCode::DatabaseBusy
                || inner.code == rusqlite::ErrorCode::DatabaseLocked
    )
}

/// Acquire the write lock before reading. Retrying BEGIN preserves ownership
/// of the write payload and avoids read-then-upgrade SQLITE_BUSY_SNAPSHOT.
pub(crate) fn begin_write_transaction(connection: &Connection) -> Result<DurableTransaction<'_>> {
    const ATTEMPTS: usize = 4;
    let mut delay = Duration::from_millis(20);
    let mut attempt = 0;
    loop {
        attempt += 1;
        // `new_unchecked` takes `&Connection` (rusqlite's own API for retry
        // loops); the checked constructor needs `&mut`, which cannot be
        // re-borrowed while the returned transaction borrows it.
        match rusqlite::Transaction::new_unchecked(
            connection,
            rusqlite::TransactionBehavior::Immediate,
        ) {
            Ok(transaction) => return Ok(DurableTransaction(transaction)),
            Err(error) if attempt < ATTEMPTS && is_lock_contention(&error) => {
                std::thread::sleep(delay);
                delay *= 2;
            }
            Err(error) => return Err(error).context("begin immediate transaction"),
        }
    }
}

/// Optional maintenance must never wait behind an external writer on the
/// shared worker. Rollback/commit finishes before restoring foreground waits.
pub(crate) fn try_maintenance_transaction<T>(
    connection: &mut Connection,
    work: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
) -> Result<T> {
    connection.busy_timeout(Duration::ZERO)?;
    let result = (|| {
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let value = work(&tx)?;
        tx.commit()?;
        Ok(value)
    })();
    connection.busy_timeout(Duration::from_secs(5))?;
    result
}
