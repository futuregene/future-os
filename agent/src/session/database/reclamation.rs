//! Bounded idle space reclamation; never compacts or deletes journals.
#[cfg(test)]
use super::Database;
use anyhow::Result;
use rusqlite::Connection;
use std::time::Duration;

/// Enable incremental reclamation only when no existing data needs rewriting.
/// Existing NONE-mode databases keep freed pages available for SQLite reuse;
/// startup never performs a full VACUUM or changes their storage layout.
pub(super) fn configure_incremental_auto_vacuum(connection: &Connection) -> Result<()> {
    let mode: i64 = connection.pragma_query_value(None, "auto_vacuum", |row| row.get(0))?;
    let empty: bool = connection.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%')",
        [],
        |row| row.get(0),
    )?;
    // FULL and INCREMENTAL share the pointer-map layout. Switching between
    // them needs no rebuild; a NONE-mode populated database is left alone.
    if empty || mode == 1 {
        connection.pragma_update(None, "auto_vacuum", 2)?;
    }
    Ok(())
}

/// Short idle slices, not a long job placed on the same ordered worker.
/// Busy checkpoints are retried without the foreground 5-second lock wait.
pub(crate) fn reclaim_space_when_idle(connection: &mut Connection) -> Result<bool> {
    let started = std::time::Instant::now();
    connection.busy_timeout(Duration::ZERO)?;
    let result = (|| -> Result<bool> {
        // Space reclamation never changes journals. Snapshot construction
        // and raw deletion happen only in the run-completion transaction.
        let free = |connection: &Connection| -> Result<i64> {
            Ok(connection.pragma_query_value(None, "freelist_count", |r| r.get(0))?)
        };
        let mode: i64 = connection.pragma_query_value(None, "auto_vacuum", |r| r.get(0))?;
        let mut remaining = if mode == 2 { free(connection)? } else { 0 };
        for _ in 0..128 {
            if remaining == 0 || started.elapsed() >= Duration::from_millis(10) {
                break;
            }
            connection.execute_batch("PRAGMA incremental_vacuum(1)")?;
            let now = free(connection)?;
            if now >= remaining {
                break;
            }
            remaining = now;
        }
        // Reuse/truncate the WAL only when no reader needs it; never force a
        // long reader to end. Failure leaves the retry hint set.
        let (blocked, _, _): (i64, i64, i64) =
            connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
        Ok(remaining == 0 && blocked == 0)
    })();
    connection.busy_timeout(Duration::from_secs(5))?;
    result
}

#[cfg(test)]
impl Database {
    /// Test helper: reclaim at most `max_pages` when the freelist reaches
    /// `min_free_pages`, stopping if SQLite can make no further progress.
    pub(crate) fn reclaim(&self, min_free_pages: i64, max_pages: i64) -> Result<()> {
        self.call(move |connection| {
            let free = |connection: &Connection| -> Result<i64> {
                Ok(connection.pragma_query_value(None, "freelist_count", |row| row.get(0))?)
            };
            let mut remaining = free(connection)?;
            if remaining < min_free_pages {
                return Ok(());
            }
            let mut reclaimed = 0;
            while remaining > 0 && reclaimed < max_pages {
                connection.execute_batch("PRAGMA incremental_vacuum")?;
                let now = free(connection)?;
                if now >= remaining {
                    break; // the last page is in use: no further progress
                }
                reclaimed += remaining - now;
                remaining = now;
            }
            Ok(())
        })
    }
}
