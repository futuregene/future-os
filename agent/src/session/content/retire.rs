//! Retire the stream of a settled run.
//!
//! A finished run keeps its journal for exactly one thing: replaying that run
//! event by event. The product decision is that a settled run replays its
//! *transcript*, not its token stream - `entries` and the content ranges behind
//! them already carry the text, the thinking and the tool output, and the run's
//! token totals live in `runs`. So the journal of a settled run is dead weight:
//! 99.5% of the rows on a real store.
//!
//! The retirement is the same two-phase shape an explicit reset uses: the marker
//! in `retired_streams` hides the stream from every reader in one transaction,
//! then the rows drain in bounded slices. A run that is still running, or whose
//! recovery is pending, is never a candidate.

use super::*;
use crate::session::sqlite_store::SqliteStore;
use std::time::Instant;

/// Rows deleted per transaction while draining. Larger than the reset path's
/// slice because this is a bulk retire with no reader waiting on it.
const DRAIN_CHUNK: i64 = 2_048;

/// Wall-clock predicate for the grace, in the same unit as `completed_at_ms`.
///
/// Ten minutes: long enough that a client which just watched the run finish can
/// still replay it, short enough that a store left running does not accumulate
/// weeks of journals.
const SETTLED_LONG_AGO: &str =
    "coalesce(u.completed_at_ms,0) <= CAST(strftime('%s','now') AS INTEGER)*1000 - 600000";

/// Settled runs whose stream is still held: `(streams, rows)`.
pub(crate) fn plan(db: &Connection) -> Result<(i64, i64)> {
    Ok(db.query_row(
        &format!(
            "SELECT count(*),coalesce(sum(n),0) FROM (
             SELECT r.id,count(c.id) n
               FROM stream_runs r
               JOIN runs u ON u.session_id=r.session_id AND u.run_id=r.run_id
               LEFT JOIN stream_changes c ON c.stream_id=r.id
              WHERE u.status IN ('completed','failed','cancelled','interrupted')
                AND {SETTLED_LONG_AGO}
                AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=r.id)
              GROUP BY r.id)"
        ),
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}

/// One bounded pass for the start of the process: the sweep retires the bulk,
/// this keeps a long-running store from accumulating journals between sweeps.
impl SqliteStore {
    pub(crate) fn auto_retire_settled_streams(&self) -> Result<usize> {
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        let (streams, rows) = self
            .db
            .call_as("content_retire", move |db| apply(db, 64, deadline))?;
        if streams > 0 {
            tracing::info!(streams, rows, "retired settled-run journals at startup");
        }
        Ok(streams as usize)
    }
}

/// Retire up to `limit` settled streams, oldest run first.
///
/// Returns `(streams retired, rows deleted)`. The marker is committed before any
/// row is touched, so an interruption leaves a stream nobody can read and the
/// next pass finishes it.
pub(crate) fn apply(db: &mut Connection, limit: i64, deadline: Instant) -> Result<(i64, i64)> {
    let candidates: Vec<i64> = {
        let mut statement = db.prepare_cached(&format!(
            "SELECT r.id FROM stream_runs r
               JOIN runs u ON u.session_id=r.session_id AND u.run_id=r.run_id
              WHERE u.status IN ('completed','failed','cancelled','interrupted')
                AND {SETTLED_LONG_AGO}
                AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=r.id)
              ORDER BY coalesce(u.started_at_ms,0), r.id LIMIT ?1",
        ))?;
        let rows = statement
            .query_map(params![limit], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let (mut streams, mut deleted) = (0i64, 0i64);
    for id in candidates {
        if Instant::now() >= deadline {
            break;
        }
        // Hide it first: an interrupted drain must never be visible.
        crate::session::database::try_maintenance_transaction(db, |tx| {
            tx.execute(
                "INSERT OR IGNORE INTO retired_streams(stream_id) VALUES (?1)",
                [id],
            )?;
            Ok(())
        })?;
        streams += 1;
        loop {
            if Instant::now() >= deadline {
                return Ok((streams, deleted));
            }
            let drained = crate::session::database::try_maintenance_transaction(db, |tx| {
                drain_slice(tx, id)
            })?;
            deleted += drained;
            if drained == 0 {
                break;
            }
        }
    }
    Ok((streams, deleted))
}

/// One bounded slice: the change rows first, then what only they were keeping.
fn drain_slice(tx: &Transaction<'_>, stream: i64) -> Result<i64> {
    let mut deleted = 0i64;
    deleted += tx.execute(
        "DELETE FROM stream_change_content WHERE (change_id,field) IN
           (SELECT b.change_id,b.field FROM stream_change_content b
              JOIN stream_changes c ON c.id=b.change_id WHERE c.stream_id=?1 LIMIT ?2)",
        params![stream, DRAIN_CHUNK],
    )? as i64;
    deleted += tx.execute(
        "DELETE FROM stream_changes WHERE id IN (SELECT c.id FROM stream_changes c
           WHERE c.stream_id=?1 AND NOT EXISTS(SELECT 1 FROM stream_change_content b WHERE b.change_id=c.id)
           LIMIT ?2)",
        params![stream, DRAIN_CHUNK],
    )? as i64;
    tx.execute(
        "DELETE FROM stream_contexts WHERE id IN (SELECT x.id FROM stream_contexts x
           WHERE x.stream_id=?1 AND NOT EXISTS(SELECT 1 FROM stream_changes c WHERE c.context_id=x.id)
           LIMIT ?2)",
        params![stream, DRAIN_CHUNK],
    )?;
    tx.execute(
        "DELETE FROM stream_body_owners WHERE (stream_id,key_id) IN
           (SELECT stream_id,key_id FROM stream_body_owners WHERE stream_id=?1 LIMIT ?2)",
        params![stream, DRAIN_CHUNK],
    )?;
    let remaining: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM stream_changes WHERE stream_id=?1)
             OR EXISTS(SELECT 1 FROM stream_contexts WHERE stream_id=?1)",
        [stream],
        |row| row.get(0),
    )?;
    if !remaining {
        tx.execute(
            "DELETE FROM stream_symbols WHERE stream_id=?1
               AND NOT EXISTS(SELECT 1 FROM stream_body_owners b WHERE b.stream_id=?1 AND b.key_id=stream_symbols.id)",
            [stream],
        )?;
        tx.execute("DELETE FROM stream_runs WHERE id=?1", [stream])?;
        tx.execute("DELETE FROM retired_streams WHERE stream_id=?1", [stream])?;
    }
    Ok(deleted)
}
