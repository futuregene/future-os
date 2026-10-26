//! Crash-safe retirement of settled token journals. Snapshot publication and
//! the first slice of raw-row deletion share one FULL-synchronous SQLite
//! transaction; the remaining slices follow in bounded transactions, because a
//! long run's journal is routinely six figures of rows.
use super::sqlite_store::SqliteStore;
use crate::rpc::run_snapshot::{strip_repeated_tool_arguments, DeltaFolder};
use crate::rpc::SseEvent;
use anyhow::{ensure, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

// The size a *published* snapshot aims for: the delivered projection crosses
// the wire about three times (JSON `data` dual-write, typed event, typed
// payload), so this keeps an ordinary replay well inside the 32 MiB gRPC cap.
// It is a target, not a veto: a run whose folded projection is larger than this
// is still published (measured 4.5 MiB for the largest real run on a 1 GB
// database), because the alternative — refusing forever — is what let
// `run_events` grow without bound. Publishing also never makes a replay bigger
// than the raw journal it replaces; the raw path folds the same events anyway.
const SNAPSHOT_BYTE_BUDGET: usize = 4 * 1024 * 1024;
const COMPACTION_PAGE_EVENT_LIMIT: usize = 256;
// One transaction retires at most this many raw rows. A single 200k-row journal
// therefore commits in several slices instead of holding the shared SQLite
// worker for one unbounded delete. Publication and the first slice stay atomic
// so a crash can never leave a reader without a snapshot.
pub(super) const RETIRE_CHUNK_ROWS: i64 = 32_768;

/// Durable evidence that a settled journal *was* retired, read back by
/// `future session journal` (`journal_reclaim.rs`).
const META_RECLAIMED_RUNS: &str = "journal.reclaimed_runs";
const META_RECLAIMED_EVENTS: &str = "journal.reclaimed_events";
const META_RECLAIMED_AT_MS: &str = "journal.reclaimed_at_ms";
const META_OVERSIZE_PUBLISHES: &str = "journal.oversize_publishes";

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct RunJournalSnapshot {
    pub run_id: String,
    pub epoch: i64,
    pub run_sequence: i64,
    pub cursor: i64,
    pub events: Vec<SseEvent>,
}

impl RunJournalSnapshot {
    pub(super) fn validate(&self, run: &str, cursor: i64) -> Result<()> {
        ensure!(
            self.run_id == run && self.cursor == cursor && cursor >= 0,
            "invalid compact journal identity"
        );
        ensure!(
            !self.events.is_empty() && self.events.last().is_some_and(|e| e.idx == cursor),
            "incomplete compact journal"
        );
        ensure!(
            self.events
                .iter()
                .all(|e| e.run_id == run && e.epoch == self.epoch && e.idx <= cursor)
                && self.events.windows(2).all(|w| w[0].idx < w[1].idx),
            "invalid compact journal ordering"
        );
        Ok(())
    }
}

pub(crate) struct RunReplayPage {
    pub known: bool,
    pub events: Vec<SseEvent>,
    pub snapshot: Option<RunJournalSnapshot>,
}

/// A refusal that keeps the raw journal. Every variant is durable evidence
/// (`journal.skipped.<reason>`) rather than a silent `Ok(0)`, so a database
/// that stops shrinking can be explained by one command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SkipReason {
    /// Raw rows exist but no `runs` row does: nothing proves the run settled.
    MissingRun,
    /// The run is still `running`, so its journal is still being appended to.
    NotSettled,
    /// Already published; the raw journal is gone or is an inert leftover.
    AlreadyCompacted,
    /// A run with no raw rows: `bind_events` created the session only.
    Empty,
    /// One stored event is above the paged-read budget, so a bounded snapshot
    /// cannot be built from it without materializing it in one piece.
    OversizedEvent,
    /// Raw provider deltas that are not provably a duplicate of the visible
    /// text stream: dropping them would change what a client renders.
    ProviderDelta,
    /// The last event is not `agent_end`, so a producer may still append.
    NotTerminal,
    /// Agent shutdown interrupted the build. Not evidence: the next completion
    /// (or an explicit sweep) retries the run.
    Cancelled,
}

impl SkipReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            SkipReason::MissingRun => "missing_run",
            SkipReason::NotSettled => "not_settled",
            SkipReason::AlreadyCompacted => "already_compacted",
            SkipReason::Empty => "empty",
            SkipReason::OversizedEvent => "oversized_event",
            SkipReason::ProviderDelta => "provider_delta",
            SkipReason::NotTerminal => "not_terminal",
            SkipReason::Cancelled => "cancelled",
        }
    }

    /// Reasons that mean "this journal cannot be reclaimed as it stands" rather
    /// than "there was nothing to do". Only these are counted, so the counters
    /// stay a list of real leftovers.
    fn is_leftover(self) -> bool {
        !matches!(
            self,
            SkipReason::AlreadyCompacted | SkipReason::Empty | SkipReason::Cancelled
        )
    }
}

impl SkipReason {
    fn key(self) -> String {
        format!("journal.skipped.{}", self.as_str())
    }
}

/// What one attempt achieved. `deleted` is the number of raw rows this call
/// removed; `skip` is set exactly when nothing was published.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CompactionOutcome {
    pub deleted: usize,
    pub snapshot_bytes: usize,
    /// Published above [`SNAPSHOT_BYTE_BUDGET`]; visible because it is the
    /// signal that a run's visible content alone is large.
    pub oversize: bool,
    pub skip: Option<SkipReason>,
}

#[derive(Clone, PartialEq, Eq)]
struct CompactionSource {
    status: String,
    completed: Option<i64>,
    count: i64,
    last_sequence: i64,
}

/// The one gate every attempt reads, both before building and again under the
/// write lock before publishing. Equality of two reads is how a concurrent
/// append, a second compactor or a manual prune invalidates a build.
#[derive(Clone, PartialEq, Eq)]
enum CompactionRead {
    Source(CompactionSource),
    AlreadyCompacted,
    NotSettled,
    Missing,
}

fn read_compaction_source(
    db: &rusqlite::Connection,
    session: &str,
    run: &str,
) -> Result<CompactionRead> {
    let row: Option<(String, Option<i64>, bool)> = db
        .query_row(
            "SELECT r.status,r.completed_at_ms,
                EXISTS(SELECT 1 FROM run_snapshots s
                       WHERE s.session_id=r.session_id AND s.run_id=r.run_id)
         FROM runs r WHERE r.session_id=?1 AND r.run_id=?2",
            params![session, run],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((status, completed, compacted)) = row else {
        return Ok(CompactionRead::Missing);
    };
    if compacted {
        return Ok(CompactionRead::AlreadyCompacted);
    }
    if !matches!(
        status.as_str(),
        "completed" | "failed" | "cancelled" | "interrupted"
    ) {
        return Ok(CompactionRead::NotSettled);
    }
    let (count, last_sequence) = db.query_row(
        "SELECT coalesce((SELECT idx+1 FROM run_events WHERE session_id=?1 AND run_id=?2 ORDER BY idx DESC LIMIT 1),0),
                coalesce((SELECT sequence FROM run_events WHERE session_id=?1 AND run_id=?2 ORDER BY idx DESC LIMIT 1),0)",
        params![session, run],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(CompactionRead::Source(CompactionSource {
        status,
        completed,
        count,
        last_sequence,
    }))
}

/// Add `delta` to an integer counter in `storage_meta`. Used for the durable
/// reclamation evidence; a counter is never the reason a reclaim fails.
pub(crate) fn bump_meta(connection: &rusqlite::Connection, key: &str, delta: i64) -> Result<()> {
    connection.execute(
        "INSERT INTO storage_meta(key,value) VALUES (?1,cast(?2 AS TEXT))
         ON CONFLICT(key) DO UPDATE SET
             value=cast(cast(storage_meta.value AS INTEGER)+?2 AS TEXT)",
        params![key, delta],
    )?;
    Ok(())
}

impl SqliteStore {
    /// One read transaction chooses the representation. A racing compaction
    /// cannot make a reader observe an empty raw journal without its snapshot.
    pub(crate) fn replay_page(
        &self,
        session: &str,
        run: &str,
        since: i64,
        session_scope: bool,
        limit: Option<usize>,
    ) -> Result<RunReplayPage> {
        let (session, run) = (session.to_owned(), run.to_owned());
        let decode_run = run.clone();
        let (known,rows,snapshot)=self.db.call(move |db| {
            let tx=db.transaction()?;
            let snapshot: Option<(i64,Option<String>)> = if run.is_empty() { None } else {
                tx.query_row("SELECT cursor,CASE WHEN cursor>?3 THEN payload END FROM run_snapshots WHERE session_id=?1 AND run_id=?2 AND version=1",
                    params![session,run,since], |r| Ok((r.get(0)?,r.get(1)?))).optional()?
            };
            if let Some(snapshot)=snapshot {
                tx.commit()?;
                return Ok((true,Vec::new(),Some(snapshot)));
            }
            let known=tx.query_row("SELECT EXISTS(SELECT 1 FROM run_events WHERE session_id=?1 AND run_id=?2)",
                params![session,run], |r| r.get::<_,bool>(0))?;
            let rows=super::sqlite_store::read_replay_event_rows(&tx,&session,&run,since,session_scope,limit)?;
            tx.commit()?;
            Ok((known,rows,None))
        })?;
        let snapshot = snapshot
            .and_then(|(cursor, payload)| payload.map(|payload| (cursor, payload)))
            .map(|(cursor, payload)| -> Result<_> {
                let snapshot: RunJournalSnapshot = serde_json::from_str(&payload)?;
                snapshot.validate(&decode_run, cursor)?;
                Ok(snapshot)
            })
            .transpose()?;
        Ok(RunReplayPage {
            known,
            events: rows
                .into_iter()
                .map(super::sqlite_store::decode_event_row)
                .collect::<Result<_>>()?,
            snapshot,
        })
    }

    /// Build off the ordered DB worker from bounded pages, then revalidate the
    /// source under the write lock before atomically publishing + deleting.
    #[cfg(test)]
    pub(crate) fn compact_run(&self, session: &str, run: &str) -> Result<usize> {
        Ok(self
            .compact_run_cancellable(session, run, &AtomicBool::new(false))?
            .deleted)
    }

    pub(super) fn compact_run_cancellable(
        &self,
        session: &str,
        run: &str,
        stop: &AtomicBool,
    ) -> Result<CompactionOutcome> {
        let (owned_session, owned_run) = (session.to_owned(), run.to_owned());
        let read = self
            .db
            .call(move |db| read_compaction_source(db, &owned_session, &owned_run))?;
        let original = match read {
            CompactionRead::Source(source) if source.count > 0 => source,
            CompactionRead::Source(_) => return Ok(self.skipped(session, SkipReason::Empty)),
            CompactionRead::AlreadyCompacted => {
                return Ok(self.skipped(session, SkipReason::AlreadyCompacted))
            }
            CompactionRead::NotSettled => return Ok(self.skipped(session, SkipReason::NotSettled)),
            CompactionRead::Missing => return Ok(self.skipped(session, SkipReason::MissingRun)),
        };
        let mut folder = DeltaFolder::bounded();
        let mut cursor = -1i64;
        let mut epoch = None;
        let mut last = None;
        let mut pricing = Vec::new();
        let mut pricing_bytes = 2usize;
        let mut provider_text = Sha256::new();
        let mut visible_text = Sha256::new();
        let mut provider_seen = false;
        let mut provider_plain = true;
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(self.skipped(session, SkipReason::Cancelled));
            }
            let (page_session, page_run) = (session.to_owned(), run.to_owned());
            let after_epoch = epoch.unwrap_or(i64::MIN);
            let Some(rows) = self.db.call(move |db| {
                super::sqlite_store::read_compaction_event_rows(
                    db,
                    &page_session,
                    &page_run,
                    cursor,
                    after_epoch,
                    COMPACTION_PAGE_EVENT_LIMIT,
                )
            })?
            else {
                // A single stored event above the page budget: a bounded read
                // cannot see it, so no byte-faithful snapshot can be built.
                return Ok(self.skipped(session, SkipReason::OversizedEvent));
            };
            let events = rows
                .into_iter()
                .map(|row| -> Result<SseEvent> {
                    let raw: serde_json::Value = serde_json::from_str(&row.payload)?;
                    ensure!(
                        raw.as_object()
                            .is_some_and(|object| object.keys().all(|key| matches!(
                                key.as_str(),
                                "event_type"
                                    | "data"
                                    | "run_id"
                                    | "session_id"
                                    | "event_id"
                                    | "epoch"
                                    | "idx"
                                    | "timestamp"
                                    | "session_idx"
                                    | "run_sequence"
                            ))),
                        "unknown event envelope fields; kept raw journal"
                    );
                    let sequence = row.sequence;
                    let payload = row.payload.clone();
                    let event: SseEvent = super::sqlite_store::decode_event_row(row)?;
                    if matches!(event.event_type.as_str(), "usage" | "model_changed") {
                        let item = serde_json::json!({"sequence": sequence, "payload": payload});
                        pricing_bytes =
                            pricing_bytes.saturating_add(serde_json::to_vec(&item)?.len() + 1);
                        pricing.push(item);
                    }
                    Ok(event)
                })
                .collect::<Result<Vec<_>>>()?;
            if events.is_empty() {
                break;
            }
            for event in events {
                ensure!(
                    event.run_id == run && event.idx == cursor + 1,
                    "raw journal has a gap; kept intact"
                );
                ensure!(
                    *epoch.get_or_insert(event.epoch) == event.epoch,
                    "raw journal epoch changed; kept intact"
                );
                let data: serde_json::Value = serde_json::from_str(&event.data)?;
                if event.event_type == "text_delta" {
                    provider_seen = true;
                    provider_plain &= data.as_object().is_some_and(|object| {
                        object
                            .keys()
                            .all(|key| matches!(key.as_str(), "text" | "type"))
                    });
                    if let Some(text) = data["text"].as_str() {
                        provider_text.update(text.as_bytes());
                    } else {
                        provider_plain = false;
                    }
                } else if event.event_type == "text_chunk" {
                    if let Some(text) = data["text"].as_str() {
                        visible_text.update(text.as_bytes());
                    }
                }
                cursor = event.idx;
                last = Some(event.clone());
                // A fold above the budget is published, not abandoned: the
                // bytes saved are the whole point, and `oversize` records it.
                folder.push(event);
            }
        }
        ensure!(
            cursor + 1 == original.count,
            "raw journal changed during compaction"
        );
        // Older journals may contain raw provider deltas. Drop them only
        // when they are plain text and duplicate the authoritative text stream.
        if provider_seen && (!provider_plain || provider_text.finalize() != visible_text.finalize())
        {
            return Ok(self.skipped(session, SkipReason::ProviderDelta));
        }
        let Some(last) = last else {
            return Ok(self.skipped(session, SkipReason::Empty));
        };
        // A persisted transcript terminal precedes the final event broadcast.
        // Never retire that journal while its producer may still append.
        if last.event_type != "agent_end" {
            return Ok(self.skipped(session, SkipReason::NotTerminal));
        }
        let snapshot = RunJournalSnapshot {
            run_id: run.to_owned(),
            epoch: last.epoch,
            run_sequence: last.run_sequence,
            cursor,
            events: strip_repeated_tool_arguments(folder.finish()),
        };
        snapshot.validate(run, cursor)?;
        let payload = serde_json::to_string(&snapshot)?;
        let pricing = serde_json::to_string(&pricing)?;
        let snapshot_bytes = payload.len();
        let oversize =
            snapshot_bytes > SNAPSHOT_BYTE_BUDGET || pricing.len() > SNAPSHOT_BYTE_BUDGET;
        if oversize {
            // Rare and worth a line: it means this run's *visible* content alone
            // exceeds the target replay size, which no fold can fix.
            tracing::info!(
                %session,
                %run,
                snapshot_bytes,
                budget = SNAPSHOT_BYTE_BUDGET,
                "run journal published above the snapshot budget; the raw journal was larger still"
            );
        }
        let (session, run) = (session.to_owned(), run.to_owned());
        // Publish and retire the first slice together. Every later slice is a
        // bounded transaction; between them the raw rows are already unreachable
        // (replay prefers the snapshot, and appends to a published run are
        // refused), so an interruption there wastes space but cannot corrupt a
        // replay. The reclaim sweep deletes those inert leftovers.
        let (commit_session, commit_run) = (session.clone(), run.clone());
        let expected = original.clone();
        let first = self.db.call(move |db| {
            let deleted = super::database::try_maintenance_transaction(db, |tx| {
                ensure!(
                    read_compaction_source(tx, &commit_session, &commit_run)?
                        == CompactionRead::Source(expected),
                    "run changed before snapshot commit"
                );
                // Pricing was extracted off the DB worker; revalidation fences
                // the immutable source. Preserve the global rowid high water
                // before retiring raw events, without a write per token.
                tx.execute(
                    "INSERT INTO storage_meta(key,value)
                     VALUES ('event_sequence',CAST(coalesce((SELECT max(sequence) FROM run_events),0) AS TEXT))
                     ON CONFLICT(key) DO UPDATE SET value=CAST(max(
                         CAST(storage_meta.value AS INTEGER),CAST(excluded.value AS INTEGER)) AS TEXT)",
                    [],
                )?;
                tx.execute(
                    "INSERT INTO run_snapshots(session_id,run_id,version,cursor,payload,pricing_payloads)
                     VALUES (?1,?2,1,?3,?4,?5)",
                    params![commit_session, commit_run, cursor, payload, pricing],
                )?;
                #[cfg(test)]
                crash_point("after_snapshot");
                let deleted = delete_raw_rows(tx, &commit_session, &commit_run, RETIRE_CHUNK_ROWS)?;
                record_reclamation(tx, deleted, true, oversize)?;
                #[cfg(test)]
                crash_point("after_delete");
                Ok(deleted)
            })?;
            #[cfg(test)]
            crash_point("after_commit");
            Ok(deleted)
        })?;
        let mut deleted = first;
        while (deleted as i64) < original.count && !stop.load(Ordering::Acquire) {
            let (chunk_session, chunk_run) = (session.clone(), run.clone());
            let chunk = self.db.call(move |db| {
                super::database::try_maintenance_transaction(db, |tx| {
                    let deleted =
                        delete_raw_rows(tx, &chunk_session, &chunk_run, RETIRE_CHUNK_ROWS)?;
                    record_reclamation(tx, deleted, false, false)?;
                    Ok(deleted)
                })
            });
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    // The snapshot is already published, so these rows are inert:
                    // stopping here keeps a locked database from turning a
                    // completed retirement into a failure. The sweep collects
                    // the leftovers.
                    tracing::debug!(%error, %session, %run, "raw journal slices remain; `future session journal --reclaim` collects them");
                    break;
                }
            };
            if chunk == 0 {
                break;
            }
            deleted += chunk;
        }
        self.db.request_space_reclamation();
        Ok(CompactionOutcome {
            deleted,
            snapshot_bytes,
            oversize,
            skip: None,
        })
    }

    /// Record a refusal, then hand back an outcome that publishes nothing.
    fn skipped(&self, session: &str, reason: SkipReason) -> CompactionOutcome {
        if reason.is_leftover() {
            let key = reason.key();
            if let Err(error) = self.db.call(move |db| bump_meta(db, &key, 1)) {
                tracing::debug!(%error, reason = reason.as_str(), "could not record journal skip");
            }
            tracing::debug!(
                session,
                reason = reason.as_str(),
                "settled journal kept; see `future session journal`"
            );
        }
        CompactionOutcome {
            skip: Some(reason),
            ..CompactionOutcome::default()
        }
    }
}

/// Delete at most `limit` raw rows of one run, in sequence order. Shared with
/// the sweep, which uses it to retire rows an interrupted retirement left behind.
pub(super) fn delete_raw_rows(
    tx: &rusqlite::Transaction<'_>,
    session: &str,
    run: &str,
    limit: i64,
) -> Result<usize> {
    Ok(tx.execute(
        "DELETE FROM run_events WHERE sequence IN (
             SELECT sequence FROM run_events
             WHERE session_id=?1 AND run_id=?2 ORDER BY sequence LIMIT ?3)",
        params![session, run, limit],
    )?)
}

/// Written inside the same transaction as the deletes, so the counters can
/// never describe a retirement that rolled back.
fn record_reclamation(
    tx: &rusqlite::Transaction<'_>,
    deleted: usize,
    published: bool,
    oversize: bool,
) -> Result<()> {
    if deleted > 0 {
        bump_meta(tx, META_RECLAIMED_EVENTS, deleted as i64)?;
    }
    if published {
        bump_meta(tx, META_RECLAIMED_RUNS, 1)?;
        if oversize {
            bump_meta(tx, META_OVERSIZE_PUBLISHES, 1)?;
        }
        tx.execute(
            "INSERT INTO storage_meta(key,value) VALUES (?1,cast(?2 AS TEXT))
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![META_RECLAIMED_AT_MS, chrono::Utc::now().timestamp_millis()],
        )?;
    }
    Ok(())
}

#[cfg(test)]
fn crash_point(phase: &str) {
    if std::env::var("FUTURE_COMPACTION_CRASH_PHASE").as_deref() == Ok(phase) {
        std::process::exit(73);
    }
}

#[cfg(test)]
#[path = "journal_compaction_tests.rs"]
mod tests;
