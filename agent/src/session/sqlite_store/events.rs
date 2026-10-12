//! Raw event persistence, cursors and snapshot-aware replay.
use super::SqliteStore;
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

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
        let session = session.to_owned();
        self.db.call(move |db| {
            let mut stmt = db.prepare(
                "SELECT payload FROM (
                    SELECT sequence,payload FROM run_events WHERE session_id=?1
                      AND json_extract(payload,'$.event_type') IN ('usage','model_changed')
                      AND NOT EXISTS(SELECT 1 FROM run_snapshots s WHERE s.session_id=run_events.session_id AND s.run_id=run_events.run_id)
                    UNION ALL
                    SELECT json_extract(j.value,'$.sequence') AS sequence,
                           json_extract(j.value,'$.payload') AS payload
                      FROM run_snapshots s,json_each(s.pricing_payloads) j WHERE s.session_id=?1
                 ) ORDER BY sequence",
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
        self.db.call(move |db| {
            let (session_idx, loose_idx): (i64, i64) = db.query_row(
                "SELECT COALESCE(MAX(CASE WHEN epoch=-1 THEN idx END),-1), COALESCE(MAX(CASE WHEN epoch>=0 THEN idx END),-1) FROM run_events WHERE session_id=?1 AND run_id=''",
                [session], |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            Ok((session_idx.saturating_add(1), loose_idx.saturating_add(1)))
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
        let session = session.to_owned();
        self.db.call(move |db| {
            let skipped: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='skipped')", [&session], |r| r.get(0))?;
            if skipped { bail!("session migration was skipped"); }
            db.execute("INSERT OR IGNORE INTO sessions(id,revision) VALUES (?1,-1)", [&session])?;
            Ok(())
        })
    }

    pub(crate) fn has_events(&self, session: &str, run: &str) -> Result<bool> {
        let (session, run) = (session.to_owned(), run.to_owned());
        self.db.call(move |db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM run_events WHERE session_id=?1 AND run_id=?2) OR EXISTS(SELECT 1 FROM run_snapshots WHERE session_id=?1 AND run_id=?2)",
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
            tx.commit()?;
            Ok(())
        });
        self.request_reclamation_after_success(result.is_ok());
        result
    }
}

pub(crate) fn insert_event_record(db: &Connection, session: &str, mut value: Value) -> Result<()> {
    let run = value
        .get("run_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing event run identity"))?
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
            .ok_or_else(|| anyhow::anyhow!("missing event epoch"))?
    };
    let idx = value
        .get(if session_scoped { "session_idx" } else { "idx" })
        .and_then(Value::as_i64)
        .filter(|idx| *idx >= 0)
        .ok_or_else(|| anyhow::anyhow!("missing event sequence"))?;
    if value
        .get("session_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty() && id != session)
    {
        bail!("event session identity mismatch");
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
        bail!("cannot append to a compacted settled run");
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
        bail!("conflicting event identity");
    }
    let stored_event_id = (event_id != identity).then_some(event_id);
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("event payload must be an object"))?;
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
