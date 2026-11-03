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
        let content_done = crate::session::content::reclaim(connection)?;
        connection.busy_timeout(Duration::ZERO)?;
        // Explicit ownership retirement is separate from page reuse. Neither
        // step compacts historical events or builds a run-completion snapshot.
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
        // WAL checkpointing is independent, on its own PASSIVE worker. A
        // TRUNCATE here could hold write locks across a large fsync even with
        // busy_timeout(0), delaying a newly arriving foreground transaction.
        Ok(content_done && remaining == 0)
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
