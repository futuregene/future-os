//! Journal maintenance: what the raw event journal costs, why settled runs are
//! still holding one, and the two ways they get reclaimed.
//!
//! The per-run algorithm is `journal_compaction.rs`; this module answers the
//! operational questions it cannot:
//!
//! * **is the journal growing?** [`SqliteStore::journal_stats`] reports rows,
//!   payload bytes, the largest run and the durable reclaim/skip counters.
//!   Before this existed, "no reclamation happened" was a silent `Ok(0)` and the
//!   only way to notice was to count rows in `run_events` by hand.
//! * **how do I explicitly reclaim an old raw journal?**
//!   [`SqliteStore::reclaim_settled_journals`] retains the operator-invoked,
//!   resumable legacy command. Normal startup/completion never schedules it.
//!   Canonical conversations are excluded; upgrades never sweep old bodies.

use super::journal_compaction::{
    bump_meta, delete_raw_rows, RETIRE_CHUNK_ROWS, SNAPSHOT_BODY_MIN_BYTES,
};
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
/// Folds encoded per sweep call. Each one is a read, a gzip and a two-statement
/// transaction, so the batch is smaller than the journal plan's: the point is a
/// pass that finishes its slices, not one that holds the worker.
const SNAPSHOT_BODY_BATCH: i64 = 32;
/// Runs folded per sweep call. One run is a read of its whole journal plus one
/// write transaction, so the batch is small enough to finish inside a slice.
const FOLD_BATCH_RUNS: i64 = 8;
/// Bodies repacked per sweep call. Each is a read of its fragments plus a delete
/// and an insert, so the batch stays small enough to finish inside a slice.
const RECHUNK_BATCH_OBJECTS: i64 = 64;
/// Bodies compressed per sweep call. Each is a read of one body plus a gzip and
/// a two-statement transaction, so a small batch keeps a slice responsive.
const COMPRESS_BATCH_OBJECTS: i64 = 16;

/// Runs one startup catch-up may fold. A completion notification already
/// handles the normal case, so this only mops up what a crash or a stop left
/// behind; a bound keeps a large backlog from turning startup into a long
/// maintenance job. Whatever it does not reach, the next start takes.
#[cfg(test)]
pub(crate) const CATCH_UP_MAX_RUNS: usize = 16;
/// How long one startup catch-up may hold the shared SQLite worker. Both bounds
/// apply: the run count caps the plan, this caps the time, and the smaller one
/// wins.
#[cfg(test)]
pub(crate) const CATCH_UP_BUDGET: Duration = Duration::from_secs(10);

/// The settled statuses whose journal can never be appended to again.
const SETTLED: &str = "('completed','failed','cancelled','interrupted')";

/// What one startup catch-up did. Returned so tests can assert the bounds, and
/// logged once because "it did nothing" and "it never ran" look identical in a
/// database that stops shrinking — that ambiguity is what let 135 settled runs
/// sit unreclaimed.
#[cfg(test)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CatchUpReport {
    /// Candidates actually attempted (folds plus refusals).
    pub examined: usize,
    pub folded: usize,
    pub events: usize,
    pub snapshot_bytes: usize,
    /// Published snapshots whose stream had no `agent_end`: the runs a crash left.
    pub truncated: usize,
    /// Folds that returned an error. Never retried in-process.
    pub errors: usize,
    /// Refusals by durable reason (`not_terminal`, `provider_delta`, …).
    pub refused: Vec<(String, usize)>,
    /// The plan or the budget ended this pass with work left for the next start.
    pub has_more: bool,
    pub elapsed: Duration,
}

#[cfg(test)]
impl CatchUpReport {
    /// One line, whatever the outcome: how many were folded, how many were kept
    /// and why, and how long it took. `folded: 0` with a non-empty `refused`
    /// means the journals were looked at and legitimately kept; no line at all
    /// means the catch-up never ran.
    pub(crate) fn log(&self) {
        if self.examined == 0 {
            tracing::debug!("run-journal catch-up: nothing settled was left to fold");
            return;
        }
        let refused = if self.refused.is_empty() {
            "none".to_string()
        } else {
            self.refused
                .iter()
                .map(|(reason, count)| format!("{reason}={count}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        tracing::info!(
            examined = self.examined,
            folded = self.folded,
            events = self.events,
            snapshot_bytes = self.snapshot_bytes,
            truncated = self.truncated,
            errors = self.errors,
            refused = %refused,
            has_more = self.has_more,
            elapsed_ms = self.elapsed.as_millis() as u64,
            "run-journal catch-up: folded settled journals, kept the rest"
        );
    }
}

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

/// Published folds still stored as plain JSON: how many, and the bytes they hold.
///
/// Read-only and cheap (one index probe per candidate row's size), so the
/// report can print it on every run without touching a payload.
pub(crate) fn plan_snapshot_bodies(db: &Connection) -> Result<(i64, i64)> {
    Ok(db.query_row(
        "SELECT count(*),coalesce(sum(length(CAST(s.payload AS BLOB))),0)
           FROM run_snapshots s
          WHERE length(CAST(s.payload AS BLOB)) >= ?1
            AND NOT EXISTS(SELECT 1 FROM run_snapshot_bodies b
                            WHERE b.session_id=s.session_id AND b.run_id=s.run_id)",
        params![SNAPSHOT_BODY_MIN_BYTES as i64],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}

/// Encode up to `limit` published folds, biggest first.
///
/// Returns `(runs, bytes before, bytes after)`. Each fold is rewritten in its
/// own maintenance transaction, so an interruption leaves the remaining rows
/// exactly as they were and the next pass continues from what is left.
pub(crate) fn compress_snapshot_bodies(db: &mut Connection, limit: i64) -> Result<(i64, i64, i64)> {
    let candidates: Vec<(String, String, i64, String)> = {
        let mut statement = db.prepare_cached(
            "SELECT s.session_id,s.run_id,s.cursor,s.payload
               FROM run_snapshots s
              WHERE length(CAST(s.payload AS BLOB)) >= ?1
                AND NOT EXISTS(SELECT 1 FROM run_snapshot_bodies b
                                WHERE b.session_id=s.session_id AND b.run_id=s.run_id)
              ORDER BY length(CAST(s.payload AS BLOB)) DESC LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![SNAPSHOT_BODY_MIN_BYTES as i64, limit], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let (mut runs, mut before, mut after) = (0i64, 0i64, 0i64);
    for (session, run, cursor, payload) in candidates {
        let row_before = payload.len() as i64;
        let written = super::database::try_maintenance_transaction(db, |tx| {
            super::journal_compaction::store_run_snapshot_body(tx, &session, &run, cursor, &payload)
        });
        match written {
            Ok(()) => {}
            Err(error) => {
                // One unencodable fold must not end the pass; it stays as it is
                // and the next sweep tries again.
                tracing::debug!(%session, %run, %error, "snapshot body left as plain JSON");
                continue;
            }
        }
        let now: i64 = db.query_row(
            "SELECT coalesce((SELECT length(body) FROM run_snapshot_bodies
                               WHERE session_id=?1 AND run_id=?2),0)
                    + length(CAST((SELECT payload FROM run_snapshots
                                    WHERE session_id=?1 AND run_id=?2) AS BLOB))",
            params![session, run],
            |row| row.get(0),
        )?;
        runs += 1;
        before += row_before;
        after += now;
    }
    Ok((runs, before, after))
}

/// Settled runs whose canonical journal still holds one row per delta fragment:
/// `(runs, rows, bytes)` those rows occupy.
pub(crate) fn plan_settled_folds(db: &Connection) -> Result<(i64, i64, i64)> {
    Ok(db.query_row(
        &format!(
            "SELECT count(*),coalesce(sum(n),0),coalesce(sum(b),0) FROM (
                 SELECT r.session_id,r.run_id,count(*) n,
                        coalesce(sum(length(c.metadata_json)+coalesce(c.byte_length,0)),0) b
                   FROM stream_changes c
                   JOIN stream_runs r ON r.id=c.stream_id
                   JOIN runs u ON u.session_id=r.session_id AND u.run_id=r.run_id
                  WHERE u.status IN {SETTLED}
                  GROUP BY 1,2)"
        ),
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?)
}

/// Coalesce the delta rows of up to `limit` settled runs, largest first.
///
/// Returns `(runs, rows before, rows after)`. Each run is its own maintenance
/// transaction: an interruption leaves the remaining runs exactly as they were,
/// and folding is idempotent, so the next pass simply continues.
pub(crate) fn fold_settled_journals(db: &mut Connection, limit: i64) -> Result<(i64, i64, i64)> {
    let candidates: Vec<(String, String)> = {
        let mut statement = db.prepare_cached(&format!(
            "SELECT r.session_id,r.run_id
               FROM stream_changes c
               JOIN stream_runs r ON r.id=c.stream_id
               JOIN runs u ON u.session_id=r.session_id AND u.run_id=r.run_id
              WHERE u.status IN {SETTLED}
              GROUP BY 1,2 ORDER BY count(*) DESC LIMIT ?1"
        ))?;
        let rows = statement
            .query_map(params![limit], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let (mut runs, mut before, mut after) = (0i64, 0i64, 0i64);
    for (session, run) in candidates {
        let folded = super::database::try_maintenance_transaction(db, |tx| {
            super::content::journal::fold_settled_run(tx, &session, &run)
        });
        match folded {
            Ok((b, a)) if a < b => {
                runs += 1;
                before += b;
                after += a;
            }
            Ok(_) => {}
            Err(error) => {
                // One unfoldable run must not end the pass; its rows stay as they
                // are and a later sweep tries again.
                tracing::debug!(%session, %run, %error, "settled journal left unfolded");
            }
        }
    }
    Ok((runs, before, after))
}

impl SqliteStore {
    /// Everything `future session journal` prints. Read-only, cheap enough to
    /// run whenever someone suspects the journal, and never a mutation.
    pub(crate) fn journal_stats(&self) -> Result<Value> {
        self.db.call(|db| {
            let (raw_rows, raw_bytes) = table_journal(db, "run_events")?;
            let (snapshot_rows, snapshot_bytes) = table_journal(db, "run_snapshots")?;
            // A compressed fold's bytes live in its body, not in `payload`; the
            // report's whole job is the real cost, so both are counted.
            let body_bytes: i64 = db.query_row(
                "SELECT coalesce(sum(length(body)),0) FROM run_snapshot_bodies",
                [],
                |row| row.get(0),
            )?;
            let snapshot_bytes = snapshot_bytes + body_bytes;
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
                "truncatedPublishes": meta_int(db, "journal.truncated_publishes")?,
                "skipped": skip_counters(db)?,
            }))
        })
    }

    /// The bounded startup catch-up: fold what a crash, a stop or an older build
    /// left behind, without anybody remembering to ask.
    ///
    /// A completion notification already folds the normal case, so this exists
    /// for the runs it structurally cannot cover — the store where 135 of 1,166
    /// settled runs still held a raw journal. Both bounds apply (`max_runs`
    /// candidates planned, `budget` elapsed, whichever ends first); whatever is
    /// left is the next start's work, and the explicit sweep remains for a
    /// backlog too large to grind through a few starts.
    ///
    /// Compatibility-only helper for the legacy maintenance tests. Production
    /// startup never queues this pass or scans historical journals.
    #[cfg(test)]
    pub(crate) fn catch_up_settled_journals(
        &self,
        max_runs: usize,
        budget: Duration,
    ) -> Result<CatchUpReport> {
        let planned = (max_runs.saturating_mul(4)).max(1) as i64;
        // Index-driven on purpose: `runs` is small, and each `EXISTS` is a probe
        // on the journal's own index. Grouping the whole journal here — the way
        // the explicit sweep does to rank runs by size — is exactly the cost this
        // entry cannot afford at startup.
        let candidates = self.db.call_as("journal_reclaim", move |db| {
            let mut statement = db.prepare_cached(&format!(
                "SELECT r.session_id,r.run_id
                 FROM runs r
                 WHERE r.status IN {SETTLED}
                   AND r.completed_at_ms IS NOT NULL
                   AND NOT EXISTS(SELECT 1 FROM run_snapshots s
                                  WHERE s.session_id=r.session_id AND s.run_id=r.run_id)
                   AND EXISTS(SELECT 1 FROM run_events e
                              WHERE e.session_id=r.session_id AND e.run_id=r.run_id)
                 ORDER BY r.started_at_ms IS NULL, r.started_at_ms, r.run_id
                 LIMIT ?1"
            ))?;
            let rows = statement
                .query_map(params![planned], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        let planned_truncated = candidates.len() as i64 == planned;
        // The budget starts when the plan has been read, not when the request was
        // queued: at startup this job waits behind the storage worker's own
        // migration and reconciliation work, and billing that wait would spend
        // the whole budget before the first fold. What the bound promises is that
        // *folding* stops, not that the store is idle when it begins.
        let started = Instant::now();
        let mut report = CatchUpReport {
            examined: 0,
            ..CatchUpReport::default()
        };
        let mut folded = 0usize;
        for (session, run) in candidates {
            if folded >= max_runs || started.elapsed() >= budget {
                report.has_more = true;
                break;
            }
            report.examined += 1;
            match self.compact_run_cancellable(&session, &run, &AtomicBool::new(false)) {
                Ok(outcome) => {
                    if let Some(reason) = outcome.skip {
                        let reason = reason.as_str();
                        match report.refused.iter_mut().find(|(name, _)| name == reason) {
                            Some((_, count)) => *count += 1,
                            None => report.refused.push((reason.to_owned(), 1)),
                        }
                    } else {
                        folded += 1;
                        report.folded += 1;
                        report.events += outcome.deleted;
                        report.snapshot_bytes += outcome.snapshot_bytes;
                        if outcome.truncated {
                            report.truncated += 1;
                        }
                    }
                }
                Err(_) => report.errors += 1,
            }
        }
        report.has_more = report.has_more || planned_truncated;
        report.elapsed = started.elapsed();
        report.log();
        Ok(report)
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
            let mut statement = db.prepare_cached(
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
                    let chunk = self.db.call_as("journal_reclaim", move |db| {
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
        let candidates = self.db.call_as("journal_reclaim", move |db| {
            let mut statement = db.prepare_cached(&format!(
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
                    let _ = self
                        .db
                        .call_as("journal_reclaim", move |db| bump_meta(db, key, 1));
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

        // 3. Folds already published as plain JSON: same bytes, encoded. This is
        // the only step that touches rows a previous build wrote, so it stays
        // inside the operator-invoked sweep and never runs on open or idle.
        let pending_bodies = self.db.call(|db| plan_snapshot_bodies(db))?;
        let (encoded_bodies, bodies_before, bodies_after) =
            if dry_run || pending_bodies.0 == 0 || started.elapsed() >= budget {
                (0i64, 0i64, 0i64)
            } else {
                self.db
                    .call_as("journal_reclaim", move |db| {
                        compress_snapshot_bodies(db, SNAPSHOT_BODY_BATCH)
                    })
                    .unwrap_or((0, 0, 0))
            };
        let bodies_pending = self
            .db
            .call(|db| plan_snapshot_bodies(db))
            .map(|plan| plan.0)
            .unwrap_or(0);

        // 4. Settled canonical runs: the same range the writer would have written
        // had the fragments arrived in one batch. Readers see no difference, so
        // this is bytes, not behaviour.
        let pending_folds = self.db.call(|db| plan_settled_folds(db))?;
        let (folded_runs, folds_before, folds_after) =
            if dry_run || pending_folds.0 == 0 || started.elapsed() >= budget {
                (0i64, 0i64, 0i64)
            } else {
                self.db
                    .call_as("journal_reclaim", move |db| {
                        fold_settled_journals(db, FOLD_BATCH_RUNS)
                    })
                    .unwrap_or((0, 0, 0))
            };
        let folds_pending = self
            .db
            .call(|db| plan_settled_folds(db))
            .map(|plan| plan.1)
            .unwrap_or(0);

        // 5. Bodies written one fragment per row: the same bytes, one row. Every
        // reader addresses a byte range, so this changes no address and no byte.
        let pending_segments = self.db.call(|db| super::content::rechunk::plan(db))?;
        let (repacked, segments_before, segments_after) =
            if dry_run || pending_segments.1 == 0 || started.elapsed() >= budget {
                (0i64, 0i64, 0i64)
            } else {
                self.db
                    .call_as("journal_reclaim", move |db| {
                        super::content::rechunk::apply(db, RECHUNK_BATCH_OBJECTS)
                    })
                    .unwrap_or((0, 0, 0))
            };
        let segments_pending = self
            .db
            .call(|db| super::content::rechunk::plan(db))
            .map(|plan| plan.1)
            .unwrap_or(0);

        // 6. Sealed bodies: same bytes, gzip. The marker is written with the
        // chunks, so a reader never sees half an encoded body.
        let pending_compress = self.db.call(|db| super::content::compress::plan(db))?;
        let (compressed_objects, plain_bytes, stored_bytes) =
            if dry_run || pending_compress.1 == 0 || started.elapsed() >= budget {
                (0i64, 0i64, 0i64)
            } else {
                self.db
                    .call_as("journal_reclaim", move |db| {
                        super::content::compress::apply(db, COMPRESS_BATCH_OBJECTS)
                    })
                    .unwrap_or((0, 0, 0))
            };
        let compress_pending = self
            .db
            .call(|db| super::content::compress::plan(db))
            .map(|plan| plan.1)
            .unwrap_or(0);

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
            stopped_early
                || candidates_truncated
                || leftovers_truncated
                || bodies_pending > 0
                || folds_pending > 0
                || segments_pending > 0
                || compress_pending > 0
        };

        // Fold freed pages back into the file where this database can do it
        // incrementally, so the reported size reflects the retirement.
        let (bytes_after, free_after) = self.db.call_as("journal_reclaim", |db| {
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
            "snapshotBodies": {
                "pendingRuns": bodies_pending,
                "pendingBytes": pending_bodies.1,
                "encodedRuns": encoded_bodies,
                "bytesBefore": bodies_before,
                "bytesAfter": bodies_after,
            },
            "settledFolds": {
                "pendingRows": folds_pending,
                "foldedRuns": folded_runs,
                "rowsBefore": folds_before,
                "rowsAfter": folds_after,
            },
            "bodySegments": {
                "pendingRows": segments_pending,
                "repackedObjects": repacked,
                "rowsBefore": segments_before,
                "rowsAfter": segments_after,
            },
            "bodyEncoding": {
                "pendingBytes": compress_pending,
                "compressedObjects": compressed_objects,
                "bytesBefore": plain_bytes,
                "bytesAfter": stored_bytes,
            },
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
