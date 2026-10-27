//! Raw event persistence, cursors and snapshot-aware replay.
use super::SqliteStore;
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

/// An event this journal will never accept: the same value would be refused on
/// every retry. The writer must drop these rather than keep them, because a
/// single unappendable event left in a batch fails every later batch with it —
/// which used to wedge the whole session until the agent restarted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // read through `is_event_rejected`, never matched directly
pub(crate) struct EventRejected(pub(crate) String);

impl std::fmt::Display for EventRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EventRejected {}

/// Permanent refusal, as opposed to a storage error a retry may clear.
pub(crate) fn is_event_rejected(error: &anyhow::Error) -> bool {
    error.downcast_ref::<EventRejected>().is_some()
}

/// `bail!` for the deterministic refusals below, so the writer can tell them
/// apart from a transient SQLite failure without matching on message text.
fn reject(message: &str) -> anyhow::Error {
    anyhow::Error::new(EventRejected(message.to_owned()))
}

pub(crate) struct StoredEventRow {
    session_id: String,
    run_id: String,
    epoch: i64,
    idx: i64,
    event_id: Option<String>,
    pub(crate) payload: String,
    pub(crate) sequence: i64,
}

impl SqliteStore {
    /// The `usage` and `model_changed` journal payloads of one session, in
    /// journal order — the inputs for pricing its history request by request
    /// (see `RpcSession::replay_cost_split`). Every other event kind is skipped
    /// in SQL: a picture-heavy conversation journals far more text deltas than
    /// model calls, and decoding those here would multiply the cost of a
    /// backfill that only needs the two kinds.
    pub(crate) fn pricing_event_payloads(&self, session: &str) -> Result<Vec<String>> {
        let canonical = self.storage_format(session)? == crate::session::content::FORMAT_VERSION;
        let session = session.to_owned();
        if canonical {
            let mut after = 0i64;
            let mut result = Vec::new();
            let expected_identity = self.storage_identity()?;
            loop {
                let (scope, expected) = (session.clone(), expected_identity.clone());
                let (last,page)=self.db.read(move |db| {
                    let tx=db.transaction()?;
                    let scope=crate::session::identity::BoundIdentity::session(&tx,&scope)?;
                    anyhow::ensure!(crate::session::content::transport::storage_identity(&tx)?==expected,"storage identity changed during pricing replay");
                    let ids={
                        let mut query=tx.prepare_cached("SELECT c.id,length(CAST(c.metadata_json AS BLOB))+length(CAST(x.descriptor_json AS BLOB))+x.symbol_bytes FROM stream_runs r JOIN stream_changes c ON c.stream_id=r.id LEFT JOIN stream_contexts x ON x.id=c.context_id AND x.stream_id=r.id AND x.event_kind=c.event_kind WHERE r.session_id=?1 AND c.event_type IN ('usage','model_changed') AND c.id>?2 AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=r.id) ORDER BY c.id LIMIT 128")?;
                        let mut rows=query.query(params![scope,after])?;
                        let mut ids=Vec::new();let mut bytes=0usize;
                        while let Some(row)=rows.next()? {
                            let size=usize::try_from(row.get::<_,i64>(1)?)?;
                            if !ids.is_empty() && bytes.saturating_add(size)>crate::session::content::stream::CHANGE_METADATA_BYTES {break;}
                            bytes=bytes.saturating_add(size);ids.push(row.get::<_,i64>(0)?);
                        }
                        ids
                    };
                    let page=crate::session::content::compact::read_encoded(&tx,&ids)?;
                    let last=ids.last().copied();tx.commit()?;Ok((last,page))
                })?;
                let Some(last) = last else {
                    break;
                };
                after = last;
                for change in page.decode()? {
                    let mut metadata = change.metadata;
                    metadata["event_type"] = serde_json::json!(change.event_type);
                    result.push(metadata.to_string());
                }
            }
            return Ok(result);
        }
        self.db.read(move |db| {
            let mut stmt = db.prepare(

                "SELECT payload FROM (
                    SELECT sequence,payload FROM run_events WHERE session_id=?1
                      AND json_extract(payload,'$.event_type') IN ('usage','model_changed')
                      AND NOT EXISTS(SELECT 1 FROM run_snapshots s WHERE s.session_id=run_events.session_id AND s.run_id=run_events.run_id)
                    UNION ALL
                    SELECT json_extract(j.value,'$.sequence') AS sequence,
                           json_extract(j.value,'$.payload') AS payload
                      FROM run_snapshots s,json_each(s.pricing_payloads) j WHERE s.session_id=?1
                 ) ORDER BY sequence"
            )?;
            let rows = stmt
                .query_map([&session], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }

    pub fn events(&self, session: &str, run: &str) -> Result<Vec<Value>> {
        self.typed_events(session, run)
    }

    pub(crate) fn typed_events<T: serde::de::DeserializeOwned>(
        &self,
        session: &str,
        run: &str,
    ) -> Result<Vec<T>> {
        if self.storage_format(session)? == crate::session::content::FORMAT_VERSION {
            return self
                .segmented_replay(session, run, -1, false, None)?
                .events
                .into_iter()
                .map(|event| Ok(serde_json::from_value(serde_json::to_value(event)?)?))
                .collect();
        }
        let (session, run) = (session.to_owned(), run.to_owned());
        let expected_run = run.clone();
        let (rows,snapshot) = self.db.call(move |db| {
            let tx = db.transaction()?;
            let snapshot: Option<(i64,String)> = tx.query_row(
                "SELECT cursor,payload FROM run_snapshots WHERE session_id=?1 AND run_id=?2 AND version=1",
                params![session,run], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
            let rows = if snapshot.is_some() {Vec::new()} else {
                let sql = if run.is_empty() {
                    "SELECT session_id,run_id,epoch,idx,event_id,payload,sequence FROM run_events WHERE session_id=?1 AND run_id=?2 ORDER BY sequence"
                } else {
                    "SELECT session_id,run_id,epoch,idx,event_id,payload,sequence FROM run_events WHERE session_id=?1 AND run_id=?2 ORDER BY epoch,idx"
                };
                let mut statement = tx.prepare(sql)?;
                let rows = statement.query_map(params![session,run], stored_event)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            tx.commit()?;
            Ok((rows,snapshot))
        })?;
        if let Some((cursor, payload)) = snapshot {
            let snapshot: crate::session::journal_compaction::RunJournalSnapshot =
                serde_json::from_str(&payload)?;
            snapshot.validate(&expected_run, cursor)?;
            return snapshot
                .events
                .into_iter()
                .map(|event| Ok(serde_json::from_value(serde_json::to_value(event)?)?))
                .collect();
        }
        rows.into_iter().map(decode_event_row).collect()
    }

    /// Filter durable replay before fetching/decoding payloads. Existence is
    /// independent of the cursor: an exhausted known run is not an unknown run.
    #[cfg(test)]
    pub(crate) fn typed_events_since<T: serde::de::DeserializeOwned>(
        &self,
        session: &str,
        run: &str,
        since: i64,
        session_scope: bool,
    ) -> Result<(bool, Vec<T>)> {
        self.typed_events_page(session, run, since, session_scope, None)
    }

    #[cfg(test)]
    pub(crate) fn typed_events_page<T: serde::de::DeserializeOwned>(
        &self,
        session: &str,
        run: &str,
        since: i64,
        session_scope: bool,
        limit: Option<usize>,
    ) -> Result<(bool, Vec<T>)> {
        if self.storage_format(session)? == crate::session::content::FORMAT_VERSION {
            let page = self.segmented_replay(session, run, since, session_scope, limit)?;
            return Ok((
                page.known,
                page.events
                    .into_iter()
                    .map(|event| Ok(serde_json::from_value(serde_json::to_value(event)?)?))
                    .collect::<Result<_>>()?,
            ));
        }
        let (session, run) = (session.to_owned(), run.to_owned());
        let (known, rows) = self.db.call(move |db| {
            let tx = db.transaction()?;
            let known = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM run_events WHERE session_id=?1 AND run_id=?2)",
                params![session, run],
                |row| row.get::<_, bool>(0),
            )?;
            let rows = read_replay_event_rows(&tx, &session, &run, since, session_scope, limit)?;
            tx.commit()?;
            Ok((known, rows))
        })?;
        let events = rows
            .into_iter()
            .map(decode_event_row)
            .collect::<Result<Vec<_>>>()?;
        Ok((known, events))
    }

    /// Restore sequence counters without decoding the session event history.
    pub(crate) fn event_cursors(&self, session: &str) -> Result<(i64, i64)> {
        let session = session.to_owned();
        self.db.read(move |db| {
            let sql = if crate::session::content::format(db,&session)? == crate::session::content::FORMAT_VERSION {
                "SELECT COALESCE(MAX(CASE WHEN epoch=-1 THEN durable_cursor END),-1),COALESCE(MAX(CASE WHEN epoch>=0 THEN durable_cursor END),-1) FROM stream_runs WHERE session_id=?1 AND run_id='' AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=stream_runs.id)"
            } else {
                "SELECT COALESCE(MAX(CASE WHEN epoch=-1 THEN idx END),-1), COALESCE(MAX(CASE WHEN epoch>=0 THEN idx END),-1) FROM run_events WHERE session_id=?1 AND run_id=''"
            };
            let session=crate::session::identity::BoundIdentity::session(db,&session)?;
            let (session_idx, loose_idx): (i64, i64) = db.query_row(
                sql,
                [session], |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            Ok((session_idx.saturating_add(1), loose_idx.saturating_add(1)))
        })
    }

    pub(crate) fn run_replay_watermark(&self, session: &str, run: &str) -> Result<Option<i64>> {
        let (session, run) = (session.to_owned(), run.to_owned());
        self.db.read(move |db| {
            if crate::session::content::format(db, &session)? == crate::session::content::FORMAT_VERSION {
                let run=crate::session::identity::BoundIdentity::run(db,&session,&run)?;
                let session=crate::session::identity::BoundIdentity::session(db,&session)?;
                Ok(db.query_row("SELECT durable_cursor FROM stream_runs WHERE session_id=?1 AND run_id=?2 AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=stream_runs.id) ORDER BY epoch DESC LIMIT 1", params![session,run], |row| row.get(0)).optional()?)
            } else {
                Ok(db.query_row("SELECT max(cursor) FROM (SELECT max(idx) AS cursor FROM run_events WHERE session_id=?1 AND run_id=?2 UNION ALL SELECT cursor FROM run_snapshots WHERE session_id=?1 AND run_id=?2)", params![session,run], |row| row.get(0))?)
            }
        })
    }

    pub fn append_event(&self, session: &str, event: Value) -> Result<()> {
        self.append_events(session, vec![event])
    }

    /// Persist an ordered event batch with one WAL commit. Individual event
    /// coordinates remain addressable for replay and cursor compatibility.
    pub(crate) fn append_events(&self, session: &str, events: Vec<Value>) -> Result<()> {
        let session = session.to_owned();
        self.db.call(move |db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            for event in events {
                insert_event_record(&tx, &session, event)?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Reserve event storage before the first prompt, without listing an empty transcript.
    pub(crate) fn bind_events(&self, session: &str) -> Result<()> {
        self.bind_events_with_format(session, crate::session::content::NEW_SESSION_FORMAT)
    }

    /// Historical raw-journal fixtures must select their layout explicitly.
    /// Production/default-creation specifications continue to use bind_events.
    #[cfg(test)]
    pub(crate) fn bind_legacy_events(&self, session: &str) -> Result<()> {
        self.bind_events_with_format(session, 1)
    }

    fn bind_events_with_format(&self, session: &str, new_format: i64) -> Result<()> {
        let session = session.to_owned();
        self.db.call(move |db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            let mut session=crate::session::identity::BoundIdentity::session(&tx,&session)?;
            let (deleted, imported, skipped): (bool, bool, bool) = tx.query_row(
                "SELECT (EXISTS(SELECT 1 FROM session_tombstones WHERE session_id=?1)
                         OR EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='deleted')),
                        EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1),
                        EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='skipped')",
                [&session], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            if deleted { bail!("session identity was deleted"); }
            if skipped { bail!("session migration was skipped"); }
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",[&session],|row|row.get(0))?;
            if !exists && !imported && new_format==crate::session::content::FORMAT_VERSION {
                session=crate::session::identity::BoundIdentity::fresh_session(&tx,&session)?;
            }
            let created = tx.execute("INSERT OR IGNORE INTO sessions(id,revision) VALUES (?1,-1)", [&session])? != 0;
            if created && !imported && new_format == crate::session::content::FORMAT_VERSION {
                crate::session::content::enable(&tx, &session)?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub(crate) fn has_events(&self, session: &str, run: &str) -> Result<bool> {
        let (session, run) = (session.to_owned(), run.to_owned());
        self.db.call(move |db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM run_events WHERE session_id=?1 AND run_id=?2) OR EXISTS(SELECT 1 FROM run_snapshots WHERE session_id=?1 AND run_id=?2) OR EXISTS(SELECT 1 FROM stream_runs WHERE session_id=?1 AND run_id=?2 AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=stream_runs.id))",
                params![session, run],
                |r| r.get(0),
            )?)
        })
    }

    pub fn prune_events(&self, session: &str, run: &str) -> Result<()> {
        let (session, run) = (session.to_owned(), run.to_owned());
        let result = self.db.call(move |db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            tx.execute(
                "DELETE FROM run_events WHERE session_id=?1 AND run_id=?2",
                params![session, run],
            )?;
            tx.execute(
                "DELETE FROM run_snapshots WHERE session_id=?1 AND run_id=?2",
                params![session, run],
            )?;
            tx.execute("INSERT OR IGNORE INTO retired_streams(stream_id) SELECT id FROM stream_runs WHERE session_id=?1 AND run_id=?2",params![session,run])?;
            tx.commit()?;
            Ok(())
        });
        self.request_reclamation_after_success(result.is_ok());
        result
    }
}

pub(crate) fn insert_event_record(db: &Connection, session: &str, mut value: Value) -> Result<()> {
    if crate::session::content::format(db, session)? == crate::session::content::FORMAT_VERSION {
        return Err(reject(
            "canonical conversation requires the semantic stream writer",
        ));
    }
    let run = value
        .get("run_id")
        .and_then(Value::as_str)
        .ok_or_else(|| reject("missing event run identity"))?
        .to_owned();
    let session_scoped = run.is_empty()
        && value
            .get("session_idx")
            .and_then(Value::as_i64)
            .is_some_and(|idx| idx >= 0);
    let epoch = if session_scoped {
        -1
    } else {
        value
            .get("epoch")
            .and_then(Value::as_i64)
            .filter(|epoch| *epoch >= 0)
            .ok_or_else(|| reject("missing event epoch"))?
    };
    let idx = value
        .get(if session_scoped { "session_idx" } else { "idx" })
        .and_then(Value::as_i64)
        .filter(|idx| *idx >= 0)
        .ok_or_else(|| reject("missing event sequence"))?;
    if value
        .get("session_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty() && id != session)
    {
        return Err(reject("event session identity mismatch"));
    }
    value["session_id"] = Value::String(session.to_owned());
    let identity = if session_scoped {
        format!("{session}:session:{idx}")
    } else {
        format!("{session}:{run}:{epoch}:{idx}")
    };
    let event_id = value
        .get("event_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .unwrap_or(&identity)
        .to_owned();
    value["event_id"] = Value::String(event_id.clone());
    let compacted: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM run_snapshots WHERE session_id=?1 AND run_id=?2)",
        params![session, run],
        |r| r.get(0),
    )?;
    if compacted {
        return Err(reject("cannot append to a compacted settled run"));
    }
    let existing: Option<StoredEventRow> = db.query_row(
        "SELECT session_id,run_id,epoch,idx,event_id,payload,sequence FROM run_events WHERE session_id=?1 AND run_id=?2 AND idx=?3 AND epoch=?4",
        params![session,run,idx,epoch],
        stored_event,
    ).optional()?;
    if let Some(existing) = existing {
        if decode_event_row::<Value>(existing)? == value {
            return Ok(());
        }
        return Err(reject("conflicting event identity"));
    }
    let stored_event_id = (event_id != identity).then_some(event_id);
    let object = value
        .as_object_mut()
        .ok_or_else(|| reject("event payload must be an object"))?;
    object.remove("session_id");
    object.remove("run_id");
    object.remove("event_id");
    db.execute(
        "INSERT INTO run_events(sequence,session_id,run_id,epoch,idx,event_id,payload)
         VALUES (max(coalesce((SELECT max(sequence) FROM run_events),0),
                     coalesce((SELECT CAST(value AS INTEGER) FROM storage_meta WHERE key='event_sequence'),0))+1,
                 ?1,?2,?3,?4,?5,?6)",
        params![session, run, epoch, idx, stored_event_id, value.to_string()],
    )?;
    Ok(())
}

pub(crate) fn read_replay_event_rows(
    db: &Connection,
    session: &str,
    run: &str,
    since: i64,
    session_scope: bool,
    limit: Option<usize>,
) -> Result<Vec<StoredEventRow>> {
    let sql = if session_scope && since >= -1 {
        "SELECT session_id,run_id,epoch,idx,event_id,payload,sequence FROM run_events WHERE session_id=?1 AND run_id=?2 AND epoch=-1 AND idx>?3 ORDER BY sequence"
    } else if session_scope {
        "SELECT session_id,run_id,epoch,idx,event_id,payload,sequence FROM run_events WHERE session_id=?1 AND run_id=?2 AND ((epoch=-1 AND idx>?3) OR (epoch>=0 AND ?3 < -1)) ORDER BY sequence"
    } else {
        "SELECT session_id,run_id,epoch,idx,event_id,payload,sequence FROM run_events WHERE session_id=?1 AND run_id=?2 AND idx>?3 ORDER BY epoch,idx"
    };
    let mut statement = db.prepare(&format!("{sql} LIMIT ?4"))?;
    let limit = limit
        .map(|n| i64::try_from(n).unwrap_or(i64::MAX))
        .unwrap_or(-1);
    let rows = statement
        .query_map(params![session, run, since, limit], stored_event)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Index-ordered keyset pages for maintenance. Include epoch in the cursor so
/// duplicate indices across epochs cannot hide at a page boundary.
pub(crate) fn read_compaction_event_rows(
    db: &Connection,
    session: &str,
    run: &str,
    idx: i64,
    epoch: i64,
    limit: usize,
) -> Result<Option<Vec<StoredEventRow>>> {
    const PAGE_BYTES: usize = 4 * 1024 * 1024;
    let mut statement = db.prepare(
        "SELECT session_id,run_id,epoch,idx,event_id,
         CASE WHEN length(CAST(payload AS BLOB))<=?6 THEN payload ELSE NULL END,sequence
         FROM run_events WHERE session_id=?1 AND run_id=?2 AND (idx,epoch)>(?3,?4)
         ORDER BY idx,epoch LIMIT ?5",
    )?;
    let mut cursor = statement.query(params![
        session,
        run,
        idx,
        epoch,
        limit as i64,
        PAGE_BYTES as i64
    ])?;
    let mut rows = Vec::new();
    let mut bytes = 0usize;
    while let Some(row) = cursor.next()? {
        // A single oversized event cannot produce a bounded snapshot. A NULL
        // payload fails the optional build, leaving its entire raw run intact.
        if matches!(row.get_ref(5)?, rusqlite::types::ValueRef::Null) {
            return Ok(None);
        }
        let event = stored_event(row)?;
        if !rows.is_empty() && bytes + event.payload.len() > PAGE_BYTES {
            break;
        }
        bytes += event.payload.len();
        rows.push(event);
    }
    Ok(Some(rows))
}

fn stored_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredEventRow> {
    Ok(StoredEventRow {
        session_id: row.get(0)?,
        run_id: row.get(1)?,
        epoch: row.get(2)?,
        idx: row.get(3)?,
        event_id: row.get(4)?,
        payload: row.get(5)?,
        sequence: row.get(6)?,
    })
}

#[cfg(test)]
#[path = "reservation_tests.rs"]
mod reservation_tests;

pub(crate) fn decode_event_row<T: serde::de::DeserializeOwned>(row: StoredEventRow) -> Result<T> {
    let identity = if row.epoch == -1 {
        format!("{}:session:{}", row.session_id, row.idx)
    } else {
        format!(
            "{}:{}:{}:{}",
            row.session_id, row.run_id, row.epoch, row.idx
        )
    };
    let mut value: Value = serde_json::from_str(&row.payload)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("stored event payload is not an object"))?;
    object.insert("session_id".into(), Value::String(row.session_id));
    object.insert("run_id".into(), Value::String(row.run_id));
    object.insert(
        "event_id".into(),
        Value::String(row.event_id.unwrap_or(identity)),
    );
    Ok(serde_json::from_value(value)?)
}
