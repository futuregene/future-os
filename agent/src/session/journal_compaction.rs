//! Crash-safe retirement of settled token journals. Snapshot publication and
//! raw-row deletion share one FULL-synchronous SQLite transaction.
use super::sqlite_store::SqliteStore;
use crate::rpc::run_snapshot::{strip_repeated_tool_arguments, DeltaFolder};
use crate::rpc::SseEvent;
use anyhow::{ensure, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

// Below the 8 MiB replay budget even after wire envelopes. Oversize runs keep
// their raw journal and its existing paged replay; they are never truncated.
const SNAPSHOT_BYTE_BUDGET: usize = 4 * 1024 * 1024;
const COMPACTION_PAGE_EVENT_LIMIT: usize = 256;
// Deletion is atomic with publication, so bound the rows in that transaction.
// Larger runs keep raw paging instead of monopolizing the SQLite worker.
const MAX_ATOMIC_RETIRE_EVENTS: i64 = 65_536;

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

#[derive(Clone, PartialEq, Eq)]
struct CompactionSource {
    status: String,
    completed: Option<i64>,
    count: i64,
    last_sequence: i64,
}

fn read_compaction_source(
    db: &rusqlite::Connection,
    session: &str,
    run: &str,
) -> Result<Option<CompactionSource>> {
    Ok(db
        .query_row(
            "SELECT status,completed_at_ms,
                coalesce((SELECT idx+1 FROM run_events WHERE session_id=?1 AND run_id=?2 ORDER BY idx DESC LIMIT 1),0),
                coalesce((SELECT sequence FROM run_events WHERE session_id=?1 AND run_id=?2 ORDER BY idx DESC LIMIT 1),0)
         FROM runs WHERE session_id=?1 AND run_id=?2
           AND status IN ('completed','failed','cancelled','interrupted')
           AND NOT EXISTS(SELECT 1 FROM run_snapshots WHERE session_id=?1 AND run_id=?2)",
            params![session, run],
            |r| {
                Ok(CompactionSource {
                    status: r.get(0)?,
                    completed: r.get(1)?,
                    count: r.get(2)?,
                    last_sequence: r.get(3)?,
                })
            },
        )
        .optional()?)
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
        self.compact_run_cancellable(session, run, &AtomicBool::new(false))
    }

    pub(super) fn compact_run_cancellable(
        &self,
        session: &str,
        run: &str,
        stop: &AtomicBool,
    ) -> Result<usize> {
        let (owned_session, owned_run) = (session.to_owned(), run.to_owned());
        let original = self
            .db
            .call(move |db| read_compaction_source(db, &owned_session, &owned_run))?;
        let Some(original) =
            original.filter(|s| s.count > 0 && s.count <= MAX_ATOMIC_RETIRE_EVENTS)
        else {
            return Ok(0);
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
                return Ok(0);
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
                return Ok(0);
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
                        ensure!(
                            pricing_bytes <= SNAPSHOT_BYTE_BUDGET,
                            "pricing snapshot too large; kept raw journal"
                        );
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
                folder.push(event);
                if folder.estimated_bytes() > SNAPSHOT_BYTE_BUDGET {
                    return Ok(0);
                }
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
            return Ok(0);
        }
        let Some(last) = last else { return Ok(0) };
        // A persisted transcript terminal precedes the final event broadcast.
        // Never retire that journal while its producer may still append.
        if last.event_type != "agent_end" {
            return Ok(0);
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
        if payload.len() > SNAPSHOT_BYTE_BUDGET {
            return Ok(0);
        }
        let pricing = serde_json::to_string(&pricing)?;
        if pricing.len() > SNAPSHOT_BYTE_BUDGET {
            return Ok(0);
        }
        let (session, run) = (session.to_owned(), run.to_owned());
        let deleted = self.db.call(move |db| {
            let deleted = super::database::try_maintenance_transaction(db, |tx| {
                ensure!(
                    read_compaction_source(tx, &session, &run)?.as_ref() == Some(&original),
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
                    params![session, run, cursor, payload, pricing],
                )?;
                #[cfg(test)]
                crash_point("after_snapshot");
                // A failed delete/commit rolls back publication and preserves
                // every raw row. There is no later cleanup or automatic retry.
                let deleted = tx.execute(
                    "DELETE FROM run_events WHERE session_id=?1 AND run_id=?2",
                    params![session, run],
                )?;
                ensure!(
                    deleted == usize::try_from(original.count)?,
                    "raw journal changed before deletion"
                );
                #[cfg(test)]
                crash_point("after_delete");
                Ok(deleted)
            })?;
            #[cfg(test)]
            crash_point("after_commit");
            Ok(deleted)
        })?;
        self.db.request_space_reclamation();
        Ok(deleted)
    }
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
