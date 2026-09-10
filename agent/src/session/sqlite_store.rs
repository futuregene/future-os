//! Transactional storage primitives, independent of RPC and display repair.

use super::database::Database;
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::path::Path;

struct StoredEvent {
    session_id: String,
    run_id: String,
    epoch: i64,
    idx: i64,
    event_id: Option<String>,
    payload: String,
}

#[derive(Clone)]
pub struct SqliteStore {
    pub(crate) db: Database,
}

impl SqliteStore {
    /// A transcript can be absent while its session-scoped events exist.
    pub fn contains(&self, session: &str) -> Result<bool> {
        let session = session.to_owned();
        self.db.call(move |db| Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND revision>=0) OR EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='skipped')", [session], |r| r.get(0))?))
    }

    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            db: Database::open(path)?,
        })
    }

    pub fn entries(&self, session: &str) -> Result<Vec<Value>> {
        self.typed_entries(session)
    }

    /// Transfer raw rows off the database worker before decoding. Large history
    /// parsing must not hold up unrelated durable writes on that worker.
    pub(crate) fn typed_entries<T: serde::de::DeserializeOwned>(
        &self,
        session: &str,
    ) -> Result<Vec<T>> {
        let session = session.to_owned();
        let rows = self.db.call(move |db| read_entry_rows(db, &session))?;
        rows.into_iter()
            .map(|row| Ok(serde_json::from_str(&row)?))
            .collect()
    }

    /// Summary inputs only: use the kind index to skip assistant/tool bodies.
    /// Read the tail timestamp separately so ordering includes all entry kinds.
    pub(crate) fn summary_rows(&self, session: &str) -> Result<(Vec<String>, String)> {
        let session = session.to_owned();
        self.db.call(move |db| {
            let tx = db.transaction()?;
            let rows = {
                let mut stmt = tx.prepare("SELECT payload FROM entry_records WHERE session_id=?1 AND entry_type IN ('session_info','model_change','user') ORDER BY position")?;
                let rows = stmt.query_map([&session], |row| row.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
                rows
            };
            let timestamp = tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ',updated_at_ms/1000.0,'unixepoch') FROM sessions WHERE id=?1", [&session], |row| row.get(0))?;
            tx.commit()?;
            Ok((rows, timestamp))
        })
    }

    pub fn replace(&self, session: &str, entries: Vec<Value>) -> Result<()> {
        let session = session.to_owned();
        self.db.call(move |db| {
            let tx = db.transaction()?;
            let skipped: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='skipped')", [&session], |r| r.get(0))?;
            if skipped { bail!("session migration was skipped; retry import before modifying it"); }
            tx.execute("INSERT INTO sessions(id) VALUES (?1) ON CONFLICT(id) DO UPDATE SET revision=MAX(revision+1,0)", [&session])?;
            tx.execute("DELETE FROM entries WHERE session_id=?1", [&session])?;
            tx.execute("DELETE FROM runs WHERE session_id=?1", [&session])?;
            tx.execute("UPDATE sessions SET current_metadata_json=NULL WHERE id=?1", [&session])?;
            insert_entries(&tx, &session, entries)?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn append(&self, session: &str, entries: Vec<Value>) -> Result<()> {
        self.commit(session, entries, Vec::new())
    }

    /// Commit transcript changes, derived run state and terminal events as a
    /// single durability boundary. All values are owned by the writer job.
    pub fn commit(&self, session: &str, entries: Vec<Value>, events: Vec<Value>) -> Result<()> {
        let session = session.to_owned();
        self.db.call(move |db| {
            let tx = db.transaction()?;
            if tx.execute(
                "UPDATE sessions SET revision=revision+1 WHERE id=?1 AND revision>=0",
                [&session],
            )? == 0
            {
                bail!("session does not exist");
            }
            insert_entries(&tx, &session, entries)?;
            for event in events {
                insert_event(&tx, &session, event)?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn revision(&self, session: &str) -> Result<Option<i64>> {
        let session = session.to_owned();
        self.db.call(move |db| {
            Ok(db
                .query_row(
                    "SELECT revision FROM sessions WHERE id=?1 AND revision>=0",
                    [session],
                    |r| r.get(0),
                )
                .optional()?)
        })
    }

    pub fn ids(&self, include_skipped: bool) -> Result<Vec<String>> {
        self.db.call(move |db| {
            let mut statement = db.prepare(if include_skipped {
                "SELECT id FROM sessions WHERE revision>=0 UNION SELECT session_id FROM legacy_imports WHERE status='skipped' ORDER BY 1"
            } else { "SELECT id FROM sessions WHERE revision>=0 ORDER BY id" })?;
            let ids = statement.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(ids)
        })
    }

    pub fn delete(&self, session: &str) -> Result<()> {
        let session = session.to_owned();
        self.db.call(move |db| {
            let tx = db.transaction()?;
            tx.execute("DELETE FROM sessions WHERE id=?1", [&session])?;
            tx.execute(
                "UPDATE legacy_imports SET status='deleted' WHERE session_id=?1",
                [&session],
            )?;
            tx.commit()?;
            Ok(())
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
        let rows = self.db.call(move |db| {
            let sql = if run.is_empty() {
                "SELECT session_id,run_id,epoch,idx,event_id,payload FROM run_events WHERE session_id=?1 AND run_id=?2 ORDER BY sequence"
            } else {
                "SELECT session_id,run_id,epoch,idx,event_id,payload FROM run_events WHERE session_id=?1 AND run_id=?2 ORDER BY epoch,idx"
            };
            let mut statement = db.prepare(sql)?;
            let rows = statement
                .query_map(params![session,run], stored_event)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        rows.into_iter().map(decode_event).collect()
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
                params![session,run], |row| row.get::<_, bool>(0),
            )?;
            let sql = if session_scope && since >= -1 {
                "SELECT session_id,run_id,epoch,idx,event_id,payload FROM run_events WHERE session_id=?1 AND run_id=?2 AND epoch=-1 AND idx>?3 ORDER BY sequence"
            } else if session_scope {
                // Non-session events have session_idx=-1; preserve the legacy
                // behavior for callers supplying a cursor below -1.
                "SELECT session_id,run_id,epoch,idx,event_id,payload FROM run_events WHERE session_id=?1 AND run_id=?2 AND ((epoch=-1 AND idx>?3) OR (epoch>=0 AND ?3 < -1)) ORDER BY sequence"
            } else {
                "SELECT session_id,run_id,epoch,idx,event_id,payload FROM run_events WHERE session_id=?1 AND run_id=?2 AND idx>?3 ORDER BY epoch,idx"
            };
            let rows = {
                let mut statement = tx.prepare(&format!("{sql} LIMIT ?4"))?;
                let limit = limit.map(|n| i64::try_from(n).unwrap_or(i64::MAX)).unwrap_or(-1);
                let rows = statement.query_map(params![session,run,since,limit], stored_event)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            tx.commit()?;
            Ok((known, rows))
        })?;
        let events = rows
            .into_iter()
            .map(decode_event)
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
            let tx = db.transaction()?;
            for event in events {
                insert_event(&tx, &session, event)?;
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
                "SELECT EXISTS(SELECT 1 FROM run_events WHERE session_id=?1 AND run_id=?2)",
                params![session, run],
                |r| r.get(0),
            )?)
        })
    }

    pub fn prune_events(&self, session: &str, run: &str) -> Result<()> {
        let (session, run) = (session.to_owned(), run.to_owned());
        self.db.call(move |db| {
            db.execute(
                "DELETE FROM run_events WHERE session_id=?1 AND run_id=?2",
                params![session, run],
            )?;
            Ok(())
        })
    }
}

fn read_entry_rows(db: &Connection, session: &str) -> Result<Vec<String>> {
    let mut statement =
        db.prepare("SELECT payload FROM entry_records WHERE session_id=?1 ORDER BY position")?;
    let values = statement
        .query_map([session], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if values.is_empty() {
        bail!("session has no entries or was not imported");
    }
    Ok(values)
}

pub(crate) fn read_run_markers(db: &Connection, session: &str) -> Result<Vec<Value>> {
    let mut statement=db.prepare("SELECT payload FROM entry_records WHERE session_id=?1 AND entry_type IN ('run_started','run_terminal') ORDER BY position")?;
    let rows = statement
        .query_map([session], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|row| Ok(serde_json::from_str(&row)?))
        .collect()
}

pub(crate) fn insert_entries(
    db: &Connection,
    session: &str,
    mut entries: Vec<Value>,
) -> Result<()> {
    // Resolve legacy message ownership once, before discarding repeated
    // snapshots. A terminal/start marker supplies identity, not a display-time
    // positional guess. Explicit identities are never overwritten.
    let mut boundary_run: Option<String> = None;
    for entry in entries.iter_mut().rev() {
        let kind = entry["type"].as_str().unwrap_or("").to_owned();
        if matches!(kind.as_str(), "run_started" | "run_terminal") {
            boundary_run = entry["content"]["run_id"].as_str().map(str::to_owned);
        } else if matches!(kind.as_str(), "user" | "assistant" | "tool") {
            if entry
                .get("meta")
                .and_then(|m| m.get("run_id"))
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .is_none()
            {
                if let Some(run) = &boundary_run {
                    if !entry["meta"].is_object() {
                        entry["meta"] = serde_json::json!({});
                    }
                    entry["meta"]["run_id"] = Value::String(run.clone());
                }
            }
            if kind == "user" {
                boundary_run = None;
            }
        }
    }
    let mut position: i64 = db.query_row(
        "SELECT COALESCE(MAX(position)+1,0) FROM entries WHERE session_id=?1",
        [session],
        |r| r.get(0),
    )?;
    for value in entries {
        let mut value = super::records::canonical_entry(value);
        if value["type"] == "user"
            && value
                .get("meta")
                .and_then(|m| m.get("run_id"))
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .is_none()
        {
            let id = value["id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing entry identity"))?;
            let run_id = format!("history:{session}:{id}");
            if !value["meta"].is_object() {
                value["meta"] = serde_json::json!({});
            }
            value["meta"]["run_id"] = Value::String(run_id);
        }
        if matches!(value["type"].as_str(), Some("assistant" | "tool"))
            && value
                .get("meta")
                .and_then(|m| m.get("run_id"))
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .is_none()
        {
            let active:Option<String> = db.query_row("SELECT run_id FROM entries WHERE session_id=?1 AND run_id IS NOT NULL ORDER BY (entry_id=?2) DESC,position DESC LIMIT 1",params![session,value["id"].as_str()],|r|r.get(0)).optional()?
                .or_else(||value["id"].as_str().map(|id|format!("history:{session}:{id}")));
            if let Some(run_id) = active {
                if !value["meta"].is_object() {
                    value["meta"] = serde_json::json!({});
                }
                value["meta"]["run_id"] = Value::String(run_id);
            }
        }
        if matches!(value["type"].as_str(), Some("user" | "assistant" | "tool")) {
            db.execute("INSERT INTO runs(session_id,run_id,status) VALUES (?1,?2,'unknown') ON CONFLICT(session_id,run_id) DO NOTHING",params![session,value["meta"]["run_id"].as_str()])?;
        }
        if value["type"] == "run_terminal" {
            let baseline: Option<(i64,i64)> = db.query_row(
                "SELECT input_baseline,cache_read_baseline FROM runs WHERE session_id=?1 AND run_id=?2 AND input_baseline IS NOT NULL",
                params![session,value["content"]["run_id"].as_str()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            let (input,cache)=match baseline {Some(value)=>value,None=>db.query_row("SELECT COALESCE(SUM(input_tokens),0),COALESCE(SUM(cache_read_tokens),0) FROM runs WHERE session_id=?1 AND run_id<>?2",params![session,value["content"]["run_id"].as_str()],|r|Ok((r.get(0)?,r.get(1)?)))?};
            {
                let (total_input,total_cache):(Option<i64>,Option<i64>)=db.query_row(
                    "SELECT json_extract(current_metadata_json,'$.tokens_in'),json_extract(current_metadata_json,'$.tokens_cache_r') FROM sessions WHERE id=?1",
                    [session],|r|Ok((r.get(0)?,r.get(1)?)))?;
                let content = value["content"]
                    .as_object_mut()
                    .ok_or_else(|| anyhow::anyhow!("invalid run terminal"))?;
                if let Some(total) = total_input {
                    content
                        .entry("input_tokens")
                        .or_insert_with(|| (total - input).max(0).into());
                }
                if let Some(total) = total_cache {
                    content
                        .entry("cache_read_tokens")
                        .or_insert_with(|| (total - cache).max(0).into());
                }
            }
        }
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| anyhow::anyhow!("missing entry identity"))?;
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing entry type"))?;
        let previous: Option<String> = db
            .query_row(
                "SELECT payload FROM entry_records WHERE session_id=?1 AND entry_id=?2 LIMIT 1",
                params![session, id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            if serde_json::from_str::<Value>(&previous)? != value {
                bail!("conflicting entry identity");
            }
            continue;
        }
        // A session has one current metadata record, not a snapshot for each
        // settings change. Keep its original position so history cursors do not
        // move when metadata is refreshed.
        let metadata_position: Option<i64> = if kind == "session_info" {
            db.query_row("SELECT position FROM entries WHERE session_id=?1 AND entry_type='session_info' LIMIT 1",[session],|r|r.get(0)).optional()?
        } else {
            None
        };
        let write_position = metadata_position.unwrap_or(position);
        if metadata_position.is_some() {
            db.execute(
                "DELETE FROM entries WHERE session_id=?1 AND position=?2",
                params![session, write_position],
            )?;
        }
        super::records::insert(db, session, write_position, &value)?;
        super::history_index::insert_shape(db, session, write_position, &value)?;
        if matches!(kind, "run_started" | "run_terminal") {
            let content = &value["content"];
            let run = content
                .get("run_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing run identity"))?;
            let state = if kind == "run_started" {
                "running"
            } else {
                content
                    .get("state")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing terminal state"))?
            };
            let timestamp = value["timestamp"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.timestamp_millis());
            let state = match state {
                "error" => "failed",
                "incomplete" | "interrupted_by_restart" => "interrupted",
                state => state,
            };
            db.execute("INSERT INTO runs(session_id,run_id,status,run_sequence,epoch,started_at_ms,completed_at_ms,error,cache_write_tokens,input_baseline,cache_read_baseline,input_tokens,cache_read_tokens,output_tokens,duration_ms)
                SELECT ?1,?2,?3,?8,?9,?10,?11,?12,?13,COALESCE(json_extract(current_metadata_json,'$.tokens_in'),0),COALESCE(json_extract(current_metadata_json,'$.tokens_cache_r'),0),?4,?5,?6,?7 FROM sessions WHERE id=?1
                ON CONFLICT(session_id,run_id) DO UPDATE SET status=excluded.status,run_sequence=COALESCE(excluded.run_sequence,runs.run_sequence),epoch=COALESCE(excluded.epoch,runs.epoch),started_at_ms=COALESCE(runs.started_at_ms,excluded.started_at_ms),completed_at_ms=excluded.completed_at_ms,error=excluded.error,cache_write_tokens=excluded.cache_write_tokens,input_baseline=COALESCE(runs.input_baseline,excluded.input_baseline),cache_read_baseline=COALESCE(runs.cache_read_baseline,excluded.cache_read_baseline),input_tokens=excluded.input_tokens,cache_read_tokens=excluded.cache_read_tokens,output_tokens=excluded.output_tokens,duration_ms=excluded.duration_ms",
                params![session,run,state,content["input_tokens"].as_i64(),content["cache_read_tokens"].as_i64(),content["run_tokens"].as_i64(),content["run_duration_ms"].as_i64(),content["run_sequence"].as_i64(),content["epoch"].as_i64(),if kind=="run_started" {timestamp} else {None},if kind=="run_terminal" {timestamp} else {None},content["error"].as_str().filter(|s|!s.is_empty()),content["cache_write_tokens"].as_i64()])?;
        }
        if metadata_position.is_none() {
            position += 1;
        }
    }
    super::history_index::reconcile(db, session)?;
    Ok(())
}

pub(crate) fn insert_event(db: &Connection, session: &str, mut value: Value) -> Result<()> {
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
    let existing: Option<StoredEvent> = db.query_row(
        "SELECT session_id,run_id,epoch,idx,event_id,payload FROM run_events WHERE session_id=?1 AND run_id=?2 AND idx=?3 AND epoch=?4",
        params![session,run,idx,epoch],
        stored_event,
    ).optional()?;
    if let Some(existing) = existing {
        if decode_event::<Value>(existing)? == value {
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
        "INSERT INTO run_events(session_id,run_id,epoch,idx,event_id,payload) VALUES (?1,?2,?3,?4,?5,?6)",
        params![session, run, epoch, idx, stored_event_id, value.to_string()],
    )?;
    Ok(())
}

fn stored_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredEvent> {
    Ok(StoredEvent {
        session_id: row.get(0)?,
        run_id: row.get(1)?,
        epoch: row.get(2)?,
        idx: row.get(3)?,
        event_id: row.get(4)?,
        payload: row.get(5)?,
    })
}

fn decode_event<T: serde::de::DeserializeOwned>(row: StoredEvent) -> Result<T> {
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

#[cfg(test)]
mod replay_cursor_tests {
    use super::*;

    #[test]
    fn event_page_bounds_decoding_before_a_corrupt_tail() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&directory.path().join("agent.db")).unwrap();
        store.bind_events("synthetic").unwrap();
        for idx in 0..4 {
            store
                .append_event(
                    "synthetic",
                    serde_json::json!({
                        "run_id": "run", "epoch": 1, "idx": idx,
                    }),
                )
                .unwrap();
        }
        store
            .db
            .call(|db| {
                db.execute(
                    "UPDATE run_events SET payload='{\"idx\":\"invalid\"}' WHERE idx=3",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        #[derive(serde::Deserialize)]
        struct Event {
            idx: i64,
        }
        let (known, page) = store
            .typed_events_page::<Event>("synthetic", "run", -1, false, Some(2))
            .unwrap();
        assert!(known);
        assert_eq!(
            page.iter().map(|event| event.idx).collect::<Vec<_>>(),
            [0, 1]
        );
        let (_, next) = store
            .typed_events_page::<Event>("synthetic", "run", 1, false, Some(1))
            .unwrap();
        assert_eq!(next[0].idx, 2);
        assert!(store
            .typed_events_page::<Event>("synthetic", "run", 2, false, Some(1),)
            .is_err());
    }

    #[test]
    fn event_batch_conflict_rolls_back_every_row() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&directory.path().join("agent.db")).unwrap();
        store.bind_events("synthetic").unwrap();
        let first = serde_json::json!({
            "session_id": "synthetic", "run_id": "run", "epoch": 1, "idx": 0,
            "event_type": "text_chunk", "data": "{\"text\":\"first\"}"
        });
        let mut conflicting = first.clone();
        conflicting["data"] = serde_json::json!("different");

        assert!(store
            .append_events("synthetic", vec![first, conflicting])
            .is_err());
        assert!(store.events("synthetic", "run").unwrap().is_empty());
    }

    #[test]
    fn cursor_query_does_not_decode_old_payloads_and_uses_range_index() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&directory.path().join("sessions.sqlite")).unwrap();
        store.bind_events("synthetic").unwrap();
        for idx in 0..3 {
            store
                .append_event(
                    "synthetic",
                    serde_json::json!({
                        "run_id": "run", "epoch": 1, "idx": idx, "event_id": format!("e{idx}")
                    }),
                )
                .unwrap();
        }
        store.db.call(|db| {
            db.execute("UPDATE run_events SET payload='{\"idx\":\"not-an-integer\"}' WHERE idx=0", [])?;
            let mut statement = db.prepare("EXPLAIN QUERY PLAN SELECT payload FROM run_events WHERE session_id='synthetic' AND run_id='run' AND idx>1 ORDER BY epoch,idx")?;
            let plans = statement.query_map([], |row| row.get::<_,String>(3))?.collect::<rusqlite::Result<Vec<_>>>()?;
            assert!(plans.iter().any(|plan| plan.contains("sqlite_autoindex_run_events_1") && plan.contains("idx>?")));
            assert!(!plans.iter().any(|plan| plan.contains("run_events_cursor")));
            Ok(())
        }).unwrap();
        #[derive(serde::Deserialize)]
        struct Event {
            idx: i64,
        }
        let (known, tail) = store
            .typed_events_since::<Event>("synthetic", "run", 1, false)
            .unwrap();
        assert!(known);
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].idx, 2);
        assert!(store.typed_events::<Event>("synthetic", "run").is_err());
        assert!(store
            .typed_events_since::<Event>("synthetic", "run", 2, false)
            .unwrap()
            .1
            .is_empty());
        assert!(
            !store
                .typed_events_since::<Event>("synthetic", "unknown", -1, false)
                .unwrap()
                .0
        );
    }

    #[test]
    fn compact_event_storage_reconstructs_identity_and_unknown_fields() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&directory.path().join("agent.db")).unwrap();
        store.bind_events("synthetic").unwrap();
        let event = serde_json::json!({
            "event_type": "text_chunk",
            "data": "{\"text\":\"safe\"}",
            "session_id": "synthetic",
            "run_id": "run",
            "epoch": 1,
            "idx": 0,
            "session_idx": -1,
            "run_sequence": 3,
            "timestamp": "2026-01-01T00:00:00Z",
            "future_field": [1,2,3]
        });
        store.append_event("synthetic", event.clone()).unwrap();
        let mut expected = event;
        expected["event_id"] = serde_json::json!("synthetic:run:1:0");
        assert_eq!(store.events("synthetic", "run").unwrap(), [expected]);
        store
            .db
            .call(|db| {
                let (sequence, event_id, payload): (i64, Option<String>, String) = db.query_row(
                    "SELECT sequence,event_id,payload FROM run_events",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
                assert!(sequence > 0);
                assert!(event_id.is_none());
                let compact: Value = serde_json::from_str(&payload)?;
                assert!(compact.get("session_id").is_none());
                assert!(compact.get("run_id").is_none());
                assert!(compact.get("event_id").is_none());
                assert_eq!(compact["future_field"], serde_json::json!([1, 2, 3]));
                Ok(())
            })
            .unwrap();
    }
}
