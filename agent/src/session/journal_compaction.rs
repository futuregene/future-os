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
use std::sync::{mpsc, Arc};

// Below the 8 MiB replay budget even after wire envelopes. Oversize runs keep
// their raw journal and its existing paged replay; they are never truncated.
const SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;
const PAGE_EVENTS: usize = 256;
// Deletion is atomic with publication, so bound the rows in that transaction.
// Larger runs keep raw paging instead of monopolizing the SQLite worker.
const MAX_RETIRE_EVENTS: i64 = 65_536;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CompactRunSnapshot {
    pub run_id: String,
    pub epoch: i64,
    pub run_sequence: i64,
    pub cursor: i64,
    pub events: Vec<SseEvent>,
}

impl CompactRunSnapshot {
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

pub(crate) struct JournalPage {
    pub known: bool,
    pub events: Vec<SseEvent>,
    pub snapshot: Option<CompactRunSnapshot>,
}

#[derive(Clone, PartialEq, Eq)]
struct Source {
    status: String,
    completed: Option<i64>,
    count: i64,
    last_sequence: i64,
}

fn source(db: &rusqlite::Connection, session: &str, run: &str) -> Result<Option<Source>> {
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
                Ok(Source {
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
    ) -> Result<JournalPage> {
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
            let rows=super::sqlite_store::event_page_rows(&tx,&session,&run,since,session_scope,limit)?;
            tx.commit()?;
            Ok((known,rows,None))
        })?;
        let snapshot = snapshot
            .and_then(|(cursor, payload)| payload.map(|payload| (cursor, payload)))
            .map(|(cursor, payload)| -> Result<_> {
                let snapshot: CompactRunSnapshot = serde_json::from_str(&payload)?;
                snapshot.validate(&decode_run, cursor)?;
                Ok(snapshot)
            })
            .transpose()?;
        Ok(JournalPage {
            known,
            events: rows
                .into_iter()
                .map(super::sqlite_store::decode_event)
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

    fn compact_run_cancellable(
        &self,
        session: &str,
        run: &str,
        stop: &AtomicBool,
    ) -> Result<usize> {
        let (owned_session, owned_run) = (session.to_owned(), run.to_owned());
        let original = self
            .db
            .call(move |db| source(db, &owned_session, &owned_run))?;
        let Some(original) = original.filter(|s| s.count > 0 && s.count <= MAX_RETIRE_EVENTS)
        else {
            return Ok(0);
        };
        let mut folder = DeltaFolder::bounded();
        let mut cursor = -1i64;
        let mut epoch = None;
        let mut last = None;
        let mut pricing = Vec::new();
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
            let rows = self.db.call(move |db| {
                super::sqlite_store::compaction_page_rows(
                    db,
                    &page_session,
                    &page_run,
                    cursor,
                    after_epoch,
                    PAGE_EVENTS,
                )
            })?;
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
                    let event: SseEvent = super::sqlite_store::decode_event(row)?;
                    if matches!(event.event_type.as_str(), "usage" | "model_changed") {
                        pricing.push(serde_json::json!({"sequence": sequence, "payload": payload}));
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
                if folder.estimated_bytes() > SNAPSHOT_BYTES {
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
        let snapshot = CompactRunSnapshot {
            run_id: run.to_owned(),
            epoch: last.epoch,
            run_sequence: last.run_sequence,
            cursor,
            events: strip_repeated_tool_arguments(folder.finish()),
        };
        snapshot.validate(run, cursor)?;
        let payload = serde_json::to_string(&snapshot)?;
        if payload.len() > SNAPSHOT_BYTES {
            return Ok(0);
        }
        let pricing = serde_json::to_string(&pricing)?;
        if pricing.len() > SNAPSHOT_BYTES {
            return Ok(0);
        }
        let (session, run) = (session.to_owned(), run.to_owned());
        let deleted=self.db.call(move |db| {
            let tx=super::database::begin_immediate(db)?;
            ensure!(source(&tx,&session,&run)?.as_ref()==Some(&original), "run changed before snapshot commit");
            // Pricing was extracted off the DB worker; source revalidation
            // fences the same immutable journal. No full scan under write lock.
            // Rowids can be reused after cleanup. Capture the global high
            // water here, before deletion, without another write per token.
            tx.execute("INSERT INTO storage_meta(key,value) VALUES ('event_sequence',CAST(coalesce((SELECT max(sequence) FROM run_events),0) AS TEXT))
                ON CONFLICT(key) DO UPDATE SET value=CAST(max(CAST(storage_meta.value AS INTEGER),CAST(excluded.value AS INTEGER)) AS TEXT)", [])?;
            tx.execute("INSERT INTO run_snapshots(session_id,run_id,version,cursor,payload,pricing_payloads,raw_pending) VALUES (?1,?2,1,?3,?4,?5,0)",
                params![session,run,cursor,payload,pricing])?;
            #[cfg(test)]
            crash_point("after_snapshot");
            // Retire this run completely in the same transaction. A failed
            // delete or commit rolls back publication and preserves every raw
            // row; there is no later cleanup job or automatic retry.
            let deleted=tx.execute("DELETE FROM run_events WHERE session_id=?1 AND run_id=?2",params![session,run])?;
            ensure!(deleted==usize::try_from(original.count)?, "raw journal changed before deletion");
            #[cfg(test)]
            crash_point("after_delete");
            tx.commit()?;
            #[cfg(test)]
            crash_point("after_commit");
            Ok(deleted)
        })?;
        self.db.request_reclaim();
        Ok(deleted)
    }
}

// Completion notifications are the only trigger. No startup/history sweep and
// no automatic retries: failures keep raw replay available. Drop cancels an
// uncommitted build between pages and joins before releasing the store.
enum MaintenanceRequest {
    Compact(String, String),
    Stop,
    #[cfg(test)]
    Barrier(mpsc::SyncSender<()>),
}

pub(crate) struct JournalMaintenance {
    sender: mpsc::SyncSender<MaintenanceRequest>,
    thread: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl JournalMaintenance {
    pub(crate) fn start(store: SqliteStore) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<MaintenanceRequest>(32);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("agent-journal-maintenance".into())
            .spawn(move || {
                while !stopping.load(Ordering::Acquire) {
                    match receiver.recv() {
                        Ok(MaintenanceRequest::Compact(session, run)) => {
                            if stopping.load(Ordering::Acquire) { break; }
                            if let Err(error) = store.compact_run_cancellable(&session, &run, &stopping) {
                                tracing::warn!(%error, %session, %run, "could not compact completed journal; raw rows retained without retry");
                            }
                        }
                        #[cfg(test)]
                        Ok(MaintenanceRequest::Barrier(sender)) => { let _ = sender.send(()); }
                        Ok(MaintenanceRequest::Stop) | Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            sender,
            thread: Some(thread),
            stop,
        })
    }

    #[cfg(test)]
    fn flush(&self) {
        let (sender, receiver) = mpsc::sync_channel(0);
        self.sender
            .send(MaintenanceRequest::Barrier(sender))
            .unwrap();
        receiver.recv().unwrap();
    }

    pub(crate) fn request(&self, session: &str, run: &str) {
        let _ = self.sender.try_send(MaintenanceRequest::Compact(
            session.to_owned(),
            run.to_owned(),
        ));
    }
}

impl Drop for JournalMaintenance {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.sender.send(MaintenanceRequest::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
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
