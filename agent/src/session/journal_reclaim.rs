//! Journal maintenance: what the raw event journal costs, why settled runs are
//! still holding one, and an explicit sweep that reclaims them.
//!
//! The per-run algorithm is `journal_compaction.rs`; this module answers the two
//! operational questions it cannot:
//!
//! * **is the journal growing?** [`SqliteStore::journal_stats`] reports rows,
//!   payload bytes, the largest run and the durable reclaim/skip counters.
//!   Before this existed, "no reclamation happened" was a silent `Ok(0)` and the
//!   only way to notice was to count rows in `run_events` by hand.
//! * **how do I reclaim a database that already grew?** A completion
//!   notification is the only automatic trigger, and it is in-process: a run
//!   that completed under an older build, or while the compactor was stopping,
//!   is never retried. [`SqliteStore::reclaim_settled_journals`] is the explicit,
//!   resumable sweep — off by default, bounded per call, idempotent.
use super::journal_compaction::{bump_meta, delete_raw_rows, RETIRE_CHUNK_ROWS};
use super::sqlite_store::SqliteStore;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// How much work one sweep call accepts before it answers `hasMore`. Callers
/// repeat until `hasMore` is false; bounding each call keeps the shared SQLite
/// worker responsive and keeps the RPC under its own deadline.
pub(crate) const SWEEP_WORK_BUDGET: Duration = Duration::from_secs(20);
/// Runs examined per call, so one call cannot queue an unbounded plan.
const SWEEP_BATCH_RUNS: i64 = 64;

/// The settled statuses whose journal can never be appended to again.
const SETTLED: &str = "('completed','failed','cancelled','interrupted')";

fn journal_bytes(connection: &Connection) -> Result<i64> {
    let page_size: i64 = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
    let page_count: i64 = connection.pragma_query_value(None, "page_count", |row| row.get(0))?;
    Ok(page_size.saturating_mul(page_count))
}

fn free_bytes(connection: &Connection) -> Result<i64> {
    let page_size: i64 = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
    let free: i64 = connection.pragma_query_value(None, "freelist_count", |row| row.get(0))?;
    Ok(page_size.saturating_mul(free))
}

/// `(rows, payload bytes)` for one table's journal columns.
fn table_journal(connection: &Connection, table: &str) -> Result<(i64, i64)> {
    Ok(connection.query_row(
        &format!("SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))),0) FROM {table}"),
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}

/// The largest single run in the raw journal: the number that decides how many
/// slices its retirement needs.
fn largest_raw_run(connection: &Connection) -> Result<Option<Value>> {
    let mut statement = connection.prepare(
        "SELECT session_id,run_id,count(*) n,coalesce(sum(length(CAST(payload AS BLOB))),0) b
         FROM run_events GROUP BY session_id,run_id ORDER BY n DESC LIMIT 1",
    )?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    Ok(Some(json!({
        "sessionId": row.get::<_, String>(0)?,
        "runId": row.get::<_, String>(1)?,
        "events": row.get::<_, i64>(2)?,
        "bytes": row.get::<_, i64>(3)?,
    })))
}

/// Settled runs still holding a raw journal: the work the sweep has left, and
/// the number that should stay near zero on a healthy database.
fn settled_uncompacted(connection: &Connection) -> Result<(i64, i64, i64)> {
    Ok(connection.query_row(
        &format!(
            "SELECT count(*),coalesce(sum(n),0),coalesce(sum(b),0) FROM (
                 SELECT count(*) n,
                        coalesce(sum(length(CAST(e.payload AS BLOB))),0) b
                 FROM runs r JOIN run_events e
                   ON e.session_id=r.session_id AND e.run_id=r.run_id
                 WHERE r.status IN {SETTLED}
                   AND NOT EXISTS(SELECT 1 FROM run_snapshots s
                                  WHERE s.session_id=r.session_id AND s.run_id=r.run_id)
                 GROUP BY r.session_id, r.run_id)"
        ),
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?)
}

fn meta_int(connection: &Connection, key: &str) -> Result<i64> {
    let stored: Option<String> = connection
        .query_row(
            "SELECT value FROM storage_meta WHERE key=?1",
            [key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(stored
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0))
}

/// `journal.skipped.<reason>` counters, keyed by reason.
fn skip_counters(connection: &Connection) -> Result<Value> {
    let mut counters = serde_json::Map::new();
    let mut statement = connection
        .prepare("SELECT key,value FROM storage_meta WHERE key LIKE 'journal.skipped.%'")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        let value: String = row.get(1)?;
        let reason = key.trim_start_matches("journal.skipped.").to_owned();
        counters.insert(reason, value.parse::<i64>().unwrap_or(0).into());
    }
    Ok(Value::Object(counters))
}

impl SqliteStore {
    /// Everything `future session journal` prints. Read-only, cheap enough to
    /// run whenever someone suspects the journal, and never a mutation.
    pub(crate) fn journal_stats(&self) -> Result<Value> {
        self.db.call(|db| {
            let (raw_rows, raw_bytes) = table_journal(db, "run_events")?;
            let (snapshot_rows, snapshot_bytes) = table_journal(db, "run_snapshots")?;
            let (pending_runs, pending_events, pending_bytes) = settled_uncompacted(db)?;
            let leftovers: (i64, i64) = db.query_row(
                "SELECT count(*),coalesce(sum(n),0) FROM (
                     SELECT count(*) n FROM run_events e
                     JOIN run_snapshots s ON s.session_id=e.session_id AND s.run_id=e.run_id
                     GROUP BY e.session_id, e.run_id)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            Ok(json!({
                "runEvents": {"rows": raw_rows, "bytes": raw_bytes},
                "runSnapshots": {"rows": snapshot_rows, "bytes": snapshot_bytes},
                "largestRun": largest_raw_run(db)?,
                "settledUncompacted": {
                    "runs": pending_runs,
                    "events": pending_events,
                    "bytes": pending_bytes,
                },
                "publishedWithRawLeftovers": {"runs": leftovers.0, "events": leftovers.1},
                "database": {"bytes": journal_bytes(db)?, "freeBytes": free_bytes(db)?},
                "reclaimed": {
                    "runs": meta_int(db, "journal.reclaimed_runs")?,
                    "events": meta_int(db, "journal.reclaimed_events")?,
                    "atMs": meta_int(db, "journal.reclaimed_at_ms")?,
                },
                "oversizePublishes": meta_int(db, "journal.oversize_publishes")?,
                "skipped": skip_counters(db)?,
            }))
        })
    }

    /// One bounded step of the explicit reclaim sweep.
    ///
    /// `dry_run` reports the same plan without writing. Each call:
    /// 1. retires raw rows left behind by an interrupted earlier retirement (a
    ///    published run's rows are already unreachable, so they are pure waste),
    /// 2. then compacts settled runs, largest first, until the work budget ends.
    ///
    /// Interrupting at any point is safe: a published run is never rebuilt, and
    /// the remaining rows are picked up by the next call. `hasMore` says whether
    /// another call has work; `quickCheck` (only on the final call) is the
    /// integrity gate the caller must see pass.
    pub(crate) fn reclaim_settled_journals(
        &self,
        budget: Duration,
        dry_run: bool,
    ) -> Result<Value> {
        let started = Instant::now();
        let bytes_before = self.db.call(|db| journal_bytes(db))?;
        let mut entries = Vec::new();
        let mut reclaimed_runs = 0i64;
        let mut reclaimed_events = 0i64;
        let mut leftover_runs = 0i64;
        let mut leftover_events = 0i64;

        // 1. Inert leftovers of an earlier, interrupted retirement.
        let leftovers = self.db.call(|db| {
            let mut statement = db.prepare(
                "SELECT e.session_id,e.run_id,count(*) n,
                        coalesce(sum(length(CAST(e.payload AS BLOB))),0) b
                 FROM run_events e
                 JOIN run_snapshots s ON s.session_id=e.session_id AND s.run_id=e.run_id
                 GROUP BY e.session_id, e.run_id ORDER BY n DESC LIMIT ?1",
            )?;
            let rows = statement
                .query_map(params![SWEEP_BATCH_RUNS], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        let leftovers_truncated = leftovers.len() as i64 == SWEEP_BATCH_RUNS;
        for (session, run, rows, bytes) in leftovers {
            let mut deleted = 0i64;
            if !dry_run {
                loop {
                    if started.elapsed() >= budget {
                        break;
                    }
                    let chunk_session = session.clone();
                    let chunk_run = run.clone();
                    let chunk = self.db.call(move |db| {
                        super::database::try_maintenance_transaction(db, |tx| {
                            let deleted =
                                delete_raw_rows(tx, &chunk_session, &chunk_run, RETIRE_CHUNK_ROWS)?;
                            if deleted > 0 {
                                bump_meta(tx, "journal.reclaimed_events", deleted as i64)?;
                            }
                            Ok(deleted)
                        })
                    });
                    let chunk = match chunk {
                        Ok(chunk) => chunk,
                        Err(error) => {
                            // The rows are already unreachable; report and move on
                            // rather than failing the whole sweep on a locked
                            // database.
                            tracing::debug!(%error, "leftover raw rows remain for a later sweep");
                            break;
                        }
                    };
                    if chunk == 0 {
                        break;
                    }
                    deleted += chunk as i64;
                }
            }
            if dry_run || deleted > 0 {
                leftover_runs += 1;
                leftover_events += if dry_run { rows } else { deleted };
                reclaimed_events += if dry_run { 0 } else { deleted };
                entries.push(json!({
                    "sessionId": session,
                    "runId": run,
                    "rawEvents": rows,
                    "rawBytes": bytes,
                    "kind": "leftover",
                    "deleted": deleted,
                    "snapshotBytes": 0,
                    "skip": Value::Null,
                }));
            }
        }

        // 2. Settled runs whose raw journal was never retired. Ordering by row
        // count keeps this index-only; the payload size is only summed for a plan
        // (where it is printed), because reading 350 MB of payload per batch is
        // the expensive half and the applied pass reports what it deleted anyway.
        let byte_expr = if dry_run {
            "coalesce(sum(length(CAST(e.payload AS BLOB))),0)"
        } else {
            "0"
        };
        let candidates = self.db.call(move |db| {
            let mut statement = db.prepare(&format!(
                "SELECT r.session_id,r.run_id,count(*) n,{byte_expr} b
                 FROM runs r JOIN run_events e
                   ON e.session_id=r.session_id AND e.run_id=r.run_id
                 WHERE r.status IN {SETTLED}
                   AND NOT EXISTS(SELECT 1 FROM run_snapshots s
                                  WHERE s.session_id=r.session_id AND s.run_id=r.run_id)
                 GROUP BY r.session_id, r.run_id ORDER BY n DESC LIMIT ?1"
            ))?;
            let rows = statement
                .query_map(params![SWEEP_BATCH_RUNS], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        let mut stopped_early = false;
        let candidates_truncated = candidates.len() as i64 == SWEEP_BATCH_RUNS;
        for (index, (session, run, rows, bytes)) in candidates.into_iter().enumerate() {
            // The first candidate always runs: a budget that expired during the
            // candidate scan must not turn into a call that reports `hasMore`
            // without doing anything.
            if index > 0 && started.elapsed() >= budget {
                stopped_early = true;
                break;
            }
            if dry_run {
                reclaimed_runs += 1;
                reclaimed_events += rows;
                entries.push(json!({
                    "sessionId": session,
                    "runId": run,
                    "rawEvents": rows,
                    "rawBytes": bytes,
                    "kind": "journal",
                    "deleted": 0,
                    "snapshotBytes": 0,
                    "skip": Value::Null,
                }));
                continue;
            }
            let outcome = self.compact_run_cancellable(&session, &run, &AtomicBool::new(false));
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(error) => {
                    // One unrepresentable journal must not end the sweep: it is
                    // recorded, skipped, and the next candidate is attempted.
                    let key = "journal.skipped.error";
                    let _ = self.db.call(move |db| bump_meta(db, key, 1));
                    entries.push(json!({
                        "sessionId": session,
                        "runId": run,
                        "rawEvents": rows,
                        "rawBytes": bytes,
                        "kind": "journal",
                        "deleted": 0,
                        "snapshotBytes": 0,
                        "skip": "error",
                        "error": error.to_string(),
                    }));
                    continue;
                }
            };
            if let Some(reason) = outcome.skip {
                entries.push(json!({
                    "sessionId": session,
                    "runId": run,
                    "rawEvents": rows,
                    "rawBytes": bytes,
                    "kind": "journal",
                    "deleted": 0,
                    "snapshotBytes": 0,
                    "skip": reason.as_str(),
                }));
                continue;
            }
            reclaimed_runs += 1;
            reclaimed_events += outcome.deleted as i64;
            entries.push(json!({
                "sessionId": session,
                "runId": run,
                "rawEvents": rows,
                "rawBytes": bytes,
                "kind": "journal",
                "deleted": outcome.deleted,
                "snapshotBytes": outcome.snapshot_bytes,
                "oversize": outcome.oversize,
                "skip": Value::Null,
            }));
        }

        // `hasMore` promises *more work this sweep can do*, not "some settled run
        // still keeps a journal": a run that cannot be representationally
        // reclaimed (its last event is not `agent_end`, its envelope is unknown)
        // stays a candidate forever, and answering `true` for it made a caller
        // loop on two unretirable runs — measured as 1,042 repeat refusals of the
        // same two runs on a real database. A full batch may have a tail, so that
        // counts as more; a short batch that was fully processed does not.
        let has_more = if dry_run {
            // A plan is one answer, not a loop: the same call would plan the same
            // (still undeleted) runs again.
            false
        } else {
            stopped_early || candidates_truncated || leftovers_truncated
        };

        // Fold freed pages back into the file where this database can do it
        // incrementally, so the reported size reflects the retirement.
        let (bytes_after, free_after) = self.db.call(|db| {
            let _ = super::database::reclaim_space_when_idle(db);
            Ok((journal_bytes(db)?, free_bytes(db)?))
        })?;
        let quick_check = if has_more || dry_run {
            Value::Null
        } else {
            let result: String = self.db.call(|db| {
                Ok(db.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))?)
            })?;
            Value::String(result)
        };

        Ok(json!({
            "dryRun": dry_run,
            "entries": entries,
            "reclaimedRuns": reclaimed_runs,
            "reclaimedEvents": reclaimed_events,
            "leftoverRuns": leftover_runs,
            "leftoverEvents": leftover_events,
            "hasMore": has_more,
            "quickCheck": quick_check,
            "database": {
                "bytesBefore": bytes_before,
                "bytesAfter": bytes_after,
                "freeBytesAfter": free_after,
            },
        }))
    }
}

#[cfg(test)]
#[path = "journal_reclaim_tests.rs"]
mod tests;
