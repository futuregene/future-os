//! Session transcript reads, replacement and incremental index updates.
use super::SqliteStore;
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

impl SqliteStore {
    /// Indexed ownership lookup for admission retries. A run-id collision must
    /// not hydrate every message merely to discover an earlier run marker.
    pub(crate) fn has_transcript_run(&self, session: &str, run: &str) -> Result<bool> {
        let (session, run) = (session.to_owned(), run.to_owned());
        self.db.read(move |db| {
            let run = crate::session::identity::BoundIdentity::run(db, &session, &run)?;
            let session = crate::session::identity::BoundIdentity::session(db, &session)?;
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM entries WHERE session_id=?1 AND run_id=?2)",
                params![session, run],
                |row| row.get(0),
            )?)
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
        if self.storage_format(session)? == crate::session::content::FORMAT_VERSION {
            let snapshot = crate::session::content::reader::Snapshot::capture(self, session)?;
            let mut after = -1;
            let mut entries = Vec::new();
            loop {
                let page = snapshot.page(self, after)?;
                if page.is_empty() {
                    break;
                }
                for entry in page {
                    after = entry.position;
                    entries.push(serde_json::from_value(snapshot.entry(self, &entry)?)?);
                }
            }
            if entries.is_empty() {
                bail!("session has no entries or was not imported");
            }
            return Ok(entries);
        }
        let session = session.to_owned();
        let rows = self.db.read(move |db| read_entry_rows(db, &session))?;
        rows.into_iter()
            .map(|row| Ok(serde_json::from_str(&row)?))
            .collect()
    }

    /// Summary inputs only: use the kind index to skip assistant/tool bodies.
    /// Read the tail timestamp separately so ordering includes all entry kinds.
    pub(crate) fn summary_rows(&self, session: &str) -> Result<(Vec<String>, String, usize)> {
        let session = session.to_owned();
        self.db.read(move |db| {
            let tx = db.transaction()?;
            let session=crate::session::identity::BoundIdentity::session(&tx,&session)?;
            let rows = {
                let mut stmt = tx.prepare("SELECT payload FROM entry_records WHERE session_id=?1 AND position IN (
                    SELECT min(position) FROM entries WHERE session_id=?1 AND entry_type='user'
                    UNION SELECT max(position) FROM entries WHERE session_id=?1 AND entry_type='model_change'
                    UNION SELECT position FROM entries WHERE session_id=?1 AND entry_type='session_info'
                ) ORDER BY position")?;
                let rows = stmt.query_map([&session], |row| row.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
                rows
            };
            let timestamp = tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ',updated_at_ms/1000.0,'unixepoch') FROM sessions WHERE id=?1", [&session], |row| row.get(0))?;
            let query_count = tx.query_row("SELECT count(*) FROM entries WHERE session_id=?1 AND entry_type='user'", [&session], |row| row.get::<_,i64>(0))?;
            let query_count = usize::try_from(query_count)?;
            tx.commit()?;
            Ok((rows, timestamp, query_count))
        })
    }

    pub fn replace(&self, session: &str, entries: Vec<Value>) -> Result<()> {
        let mut prepared = crate::session::content::entries::prepare(self, session, entries)?;
        let entries = prepared.take();
        let session = session.to_owned();
        let result = self.db.call(move |db| {
            // Reads `legacy_imports` before writing, so the transaction must own
            // the write lock from the start: a deferred read-then-upgrade fails
            // instantly (SQLITE_BUSY_SNAPSHOT) when another connection commits
            // in between, which is exactly the "database is locked" seen while
            // the skill registry wrote this same file.
            let tx = crate::session::database::begin_write_transaction(db)?;
            rewrite_transcript_rows(&tx, &session, entries)?;
            tx.commit()?;
            Ok(())
        });
        // A rewrite can drop a large amount of history (compaction), which would
        // otherwise stay in the file as free pages forever. Separate job: the
        // worker is one ordered thread, so this cannot nest inside the call above.
        self.request_reclamation_after_success(result.is_ok());
        result
    }

    /// Apply an in-place history reset: verify inside one write transaction that
    /// the session still holds the history the caller planned against, compute
    /// the plan from *that* snapshot, rewrite the transcript, and drop the stream
    /// journal of every run the rewrite touched.
    ///
    /// Read-verify-rewrite is one transaction because the alternative — plan
    /// outside, `replace` inside — can lose a concurrent metadata write (a rename
    /// landing between the two would be overwritten by the stale `session_info`
    /// the plan carries). `expected_digest` is the caller's pre-computed hash of
    /// the entry list it showed the user and backed up; a mismatch means the
    /// session moved since, and the caller must re-read rather than apply a plan
    /// nobody saw.
    pub(crate) fn reset_transcript(
        &self,
        session: &str,
        expected_digest: &str,
        selectors: &crate::session::reset::ResetSelectors,
    ) -> Result<crate::session::reset::ResetPlan> {
        let session = session.to_owned();
        let expected_digest = expected_digest.to_owned();
        let selectors = selectors.clone();
        let result = self.db.call(move |db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            let rows = read_entry_rows(&tx, &session)?;
            let raw: Vec<Value> = rows
                .iter()
                .map(|row| serde_json::from_str(row))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let digest =
                crate::session::compaction_ops::canonical_hash(&Value::Array(raw.clone()))?;
            if digest != expected_digest {
                bail!(
                    "{}: session history changed while the reset was prepared",
                    crate::session::reset::HISTORY_CHANGED
                );
            }
            let entries: Vec<crate::session::SessionEntry> = rows
                .iter()
                .map(|row| serde_json::from_str(row))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let planned = crate::session::reset::plan_reset(&entries, &selectors)?;
            let retained = planned
                .retained
                .iter()
                .map(serde_json::to_value)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rewrite_transcript_rows(&tx, &session, retained)?;
            // The streamed journal is not the history: a run whose entries changed
            // must not keep replaying the content the reset just dropped
            // (`get_events_since`/`list_run_events` read it directly).
            for run in &planned.affected_runs {
                tx.execute(
                    "DELETE FROM run_events WHERE session_id=?1 AND run_id=?2",
                    params![session, run],
                )?;
                tx.execute(
                    "DELETE FROM run_snapshots WHERE session_id=?1 AND run_id=?2",
                    params![session, run],
                )?;
                tx.execute(
                    "DELETE FROM stream_runs WHERE session_id=?1 AND run_id=?2",
                    params![session, run],
                )?;
            }
            tx.commit()?;
            Ok(planned.plan)
        });
        self.request_reclamation_after_success(result.is_ok());
        result
    }

    pub fn append(&self, session: &str, entries: Vec<Value>) -> Result<()> {
        self.commit(session, entries, Vec::new())
    }
}

fn read_entry_rows(db: &Connection, session: &str) -> Result<Vec<String>> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    let mut statement =
        db.prepare("SELECT payload FROM entry_records WHERE session_id=?1 ORDER BY position")?;
    let values = statement
        .query_map([&stored_session], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if values.is_empty() {
        bail!("session has no entries or was not imported");
    }
    Ok(values)
}

pub(crate) fn read_run_marker_headers(db: &Connection, session: &str) -> Result<Vec<Value>> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    if crate::session::content::format(db, session)? != crate::session::content::FORMAT_VERSION {
        return run_marker_rows(db, session, "entry_records");
    }
    // The latest start supersedes earlier runs. Only its latest start, stop
    // and terminal can affect admission/resume; bodies and old runs are skipped.
    let mut query = db.prepare("SELECT payload FROM entry_header_records WHERE session_id=?1 AND position IN (
        SELECT max(position) FROM entries WHERE session_id=?1 AND run_id=(
            SELECT run_id FROM entries WHERE session_id=?1 AND entry_type='run_started' ORDER BY position DESC LIMIT 1)
        AND entry_type IN ('run_started','run_terminal','run_stop_requested') GROUP BY entry_type) ORDER BY position")?;
    let rows = query
        .query_map([&stored_session], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|raw| Ok(serde_json::from_str(&raw)?))
        .collect()
}

fn run_marker_rows(db: &Connection, session: &str, source: &str) -> Result<Vec<Value>> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    let mut statement=db.prepare(&format!("SELECT payload FROM {source} WHERE session_id=?1 AND entry_type IN ('run_started','run_terminal','run_stop_requested') ORDER BY position"))?;
    let rows = statement
        .query_map([&stored_session], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|row| Ok(serde_json::from_str(&row)?))
        .collect()
}

/// Replace a session's whole transcript inside an existing write transaction,
/// rebuilding the derived indexes the transaction owns.
///
/// Shared by `SqliteStore::replace` (its own transaction) and the in-place
/// history reset (which must verify and rewrite in one transaction).
pub(crate) fn rewrite_transcript_rows(
    db: &Connection,
    session: &str,
    entries: Vec<Value>,
) -> Result<()> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    let deleted: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_tombstones WHERE session_id=?1)",
        [&stored_session],
        |row| row.get(0),
    )?;
    if deleted {
        bail!("session was explicitly deleted; restore its matching database instead of recreating it");
    }
    let skipped: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='skipped')",
        [&stored_session],
        |r| r.get(0),
    )?;
    if skipped {
        bail!("session migration was skipped; retry import before modifying it");
    }
    db.execute("INSERT INTO sessions(id) VALUES (?1) ON CONFLICT(id) DO UPDATE SET revision=MAX(revision+1,0)", [&stored_session])?;
    if crate::session::content::format(db, session)? == crate::session::content::FORMAT_VERSION {
        crate::session::content::advance_history(db, session)?;
    }
    db.execute("DELETE FROM entries WHERE session_id=?1", [&stored_session])?;
    db.execute(
        "DELETE FROM history_display WHERE session_id=?1",
        [&stored_session],
    )?;
    db.execute("DELETE FROM runs WHERE session_id=?1", [&stored_session])?;
    db.execute(
        "UPDATE sessions SET current_metadata_json=NULL WHERE id=?1",
        [&stored_session],
    )?;
    insert_transcript_entries(db, session, entries)?;
    db.execute("DELETE FROM content_references WHERE session_id=?1 AND owner_kind='entry' AND NOT EXISTS(SELECT 1 FROM entries e WHERE e.session_id=content_references.session_id AND e.entry_id=content_references.owner_id)",[&stored_session])?;
    db.execute("DELETE FROM run_events WHERE session_id=?1 AND run_id IN (SELECT run_id FROM run_snapshots WHERE session_id=?1 AND NOT EXISTS(SELECT 1 FROM runs r WHERE r.session_id=run_snapshots.session_id AND r.run_id=run_snapshots.run_id))", [&stored_session])?;
    db.execute("DELETE FROM run_snapshots WHERE session_id=?1 AND NOT EXISTS(SELECT 1 FROM runs r WHERE r.session_id=run_snapshots.session_id AND r.run_id=run_snapshots.run_id)", [&stored_session])?;
    Ok(())
}

pub(crate) fn insert_transcript_entries(
    db: &Connection,
    session: &str,
    mut entries: Vec<Value>,
) -> Result<()> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    crate::session::history::ensure_history_index(db, session)?;
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
        [&stored_session],
        |r| r.get(0),
    )?;
    let segmented =
        crate::session::content::format(db, session)? == crate::session::content::FORMAT_VERSION;
    let mut changed_position: Option<i64> = None;
    let mut metadata_changed = false;
    for value in entries {
        let mut value = crate::session::entry_records::normalize_entry(value);
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
            let run_id = implicit_run_id(db, session, id, segmented, false)?;
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
            let active = value["id"]
                .as_str()
                .map(|id| implicit_run_id(db, session, id, segmented, true))
                .transpose()?;
            if let Some(run_id) = active {
                if !value["meta"].is_object() {
                    value["meta"] = serde_json::json!({});
                }
                value["meta"]["run_id"] = Value::String(run_id);
            }
        }
        if matches!(value["type"].as_str(), Some("user" | "assistant" | "tool")) {
            let run = value["meta"]["run_id"]
                .as_str()
                .map(|id| crate::session::identity::BoundIdentity::run(db, session, id))
                .transpose()?;
            db.execute("INSERT INTO runs(session_id,run_id,status) VALUES (?1,?2,'unknown') ON CONFLICT(session_id,run_id) DO NOTHING",params![&stored_session,run])?;
        }
        if value["type"] == "run_terminal" {
            let run = value["content"]["run_id"]
                .as_str()
                .map(|id| crate::session::identity::BoundIdentity::run(db, session, id))
                .transpose()?;
            let baseline: Option<(i64,i64)> = db.query_row(
                "SELECT input_baseline,cache_read_baseline FROM runs WHERE session_id=?1 AND run_id=?2 AND input_baseline IS NOT NULL",
                params![&stored_session,run],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            let (input,cache)=match baseline {Some(value)=>value,None=>db.query_row("SELECT COALESCE(SUM(input_tokens),0),COALESCE(SUM(cache_read_tokens),0) FROM runs WHERE session_id=?1 AND run_id<>?2",params![&stored_session,run],|r|Ok((r.get(0)?,r.get(1)?)))?};
            {
                let (total_input,total_cache):(Option<i64>,Option<i64>)=db.query_row(
                    "SELECT json_extract(current_metadata_json,'$.tokens_in'),json_extract(current_metadata_json,'$.tokens_cache_r') FROM sessions WHERE id=?1",
                    [&stored_session],|r|Ok((r.get(0)?,r.get(1)?)))?;
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
        let stored_id = crate::session::identity::BoundIdentity::entry(db, session, id)?;
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing entry type"))?;
        let fingerprint = if segmented {
            use sha2::{Digest, Sha256};
            Some(format!(
                "{:x}",
                Sha256::digest(value.to_string().as_bytes())
            ))
        } else {
            None
        };
        if let Some(fingerprint) = &fingerprint {
            let previous: Option<String> = db
                .query_row(
                    "SELECT digest FROM entry_fingerprints WHERE session_id=?1 AND entry_id=?2",
                    params![&stored_session, &stored_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(previous) = previous {
                if previous != *fingerprint {
                    bail!("conflicting entry identity");
                }
                continue;
            }
        } else {
            let previous: Option<String> = db
                .query_row(
                    "SELECT payload FROM entry_records WHERE session_id=?1 AND entry_id=?2 LIMIT 1",
                    params![&stored_session, &stored_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(previous) = previous {
                if serde_json::from_str::<Value>(&previous)? != value {
                    bail!("conflicting entry identity");
                }
                continue;
            }
        }
        // A session has one current metadata record, not a snapshot for each
        // settings change. Keep its original position so history cursors do not
        // move when metadata is refreshed.
        let metadata_position: Option<i64> = if kind == "session_info" {
            db.query_row("SELECT position FROM entries WHERE session_id=?1 AND entry_type='session_info' LIMIT 1",[&stored_session],|r|r.get(0)).optional()?
        } else {
            None
        };
        let write_position = metadata_position.unwrap_or(position);
        if metadata_position.is_some() {
            if segmented {
                // The new metadata header already owns staged/copied ranges.
                // Retire the previous owner atomically rather than leaking all
                // of its externalized fields after every rename/usage update.
                db.execute("DELETE FROM content_references WHERE session_id=?1 AND owner_kind='entry' AND owner_id=(SELECT entry_id FROM entries WHERE session_id=?1 AND position=?2)",params![&stored_session,write_position])?;
            }
            db.execute(
                "DELETE FROM entries WHERE session_id=?1 AND position=?2",
                params![&stored_session, write_position],
            )?;
        }
        crate::session::entry_records::insert_entry_record(db, session, write_position, &value)?;
        if let Some(fingerprint) = fingerprint {
            db.execute(
                "INSERT INTO entry_fingerprints(session_id,entry_id,digest) VALUES (?1,?2,?3)",
                params![&stored_session, &stored_id, fingerprint],
            )?;
        }
        crate::session::history::insert_entry_shape(db, session, write_position, &value)?;
        metadata_changed |= matches!(kind, "session_info" | "model_change");
        if metadata_position.is_none() && kind != "model_change" {
            changed_position =
                Some(changed_position.map_or(write_position, |p| p.min(write_position)));
        }
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
            let run = crate::session::identity::BoundIdentity::run(db, session, run)?;
            db.execute("INSERT INTO runs(session_id,run_id,status,run_sequence,epoch,started_at_ms,completed_at_ms,error,cache_write_tokens,input_baseline,cache_read_baseline,input_tokens,cache_read_tokens,output_tokens,duration_ms)
                SELECT ?1,?2,?3,?8,?9,?10,?11,?12,?13,COALESCE(json_extract(current_metadata_json,'$.tokens_in'),0),COALESCE(json_extract(current_metadata_json,'$.tokens_cache_r'),0),?4,?5,?6,?7 FROM sessions WHERE id=?1
                ON CONFLICT(session_id,run_id) DO UPDATE SET status=excluded.status,run_sequence=COALESCE(excluded.run_sequence,runs.run_sequence),epoch=COALESCE(excluded.epoch,runs.epoch),started_at_ms=COALESCE(runs.started_at_ms,excluded.started_at_ms),completed_at_ms=excluded.completed_at_ms,error=excluded.error,cache_write_tokens=excluded.cache_write_tokens,input_baseline=COALESCE(runs.input_baseline,excluded.input_baseline),cache_read_baseline=COALESCE(runs.cache_read_baseline,excluded.cache_read_baseline),input_tokens=excluded.input_tokens,cache_read_tokens=excluded.cache_read_tokens,output_tokens=excluded.output_tokens,duration_ms=excluded.duration_ms",
                params![&stored_session,run,state,content["input_tokens"].as_i64(),content["cache_read_tokens"].as_i64(),content["run_tokens"].as_i64(),content["run_duration_ms"].as_i64(),content["run_sequence"].as_i64(),content["epoch"].as_i64(),if kind=="run_started" {timestamp} else {None},if kind=="run_terminal" {timestamp} else {None},content["error"].as_str().filter(|s|!s.is_empty()),content["cache_write_tokens"].as_i64()])?;
        }
        if metadata_position.is_none() {
            position += 1;
        }
    }
    if let Some(position) = changed_position {
        crate::session::history::update_display_index_from(db, session, position)?;
    }
    if metadata_changed {
        crate::session::history::refresh_session_metadata(db, session)?;
    }
    crate::session::history::mark_history_index_ready(db, session)?;
    Ok(())
}

/// Allocate only when an ordinary entry has no producer-owned run. Retries
/// reuse its committed owner; assistant/tool entries inherit the latest owner.
/// New-format IDs never repeat session/message identity in a synthetic string.
fn implicit_run_id(
    db: &Connection,
    session: &str,
    entry: &str,
    segmented: bool,
    inherit: bool,
) -> Result<String> {
    if !segmented && !inherit {
        // Keep the released user's derived-identity rule byte-for-byte. Only
        // the new format allocates/reuses UUID ownership for an unscoped user.
        return Ok(format!("history:{session}:{entry}"));
    }
    let stored_entry = crate::session::identity::BoundIdentity::entry(db, session, entry)?;
    let stored_session = crate::session::identity::session_key(db, session)?;
    let existing:Option<crate::session::identity::BoundIdentity>=db.query_row(
        "SELECT coalesce((SELECT run_id FROM entries WHERE session_id=?1 AND entry_id=?2),
        (SELECT run_id FROM entries WHERE ?3 AND session_id=?1 AND run_id IS NOT NULL ORDER BY position DESC LIMIT 1))",
        params![stored_session,stored_entry,inherit],|row|row.get(0))?;
    Ok(existing.map(|id| id.to_string()).unwrap_or_else(|| {
        if segmented {
            crate::utils::generate_entry_id()
        } else {
            format!("history:{session}:{entry}")
        }
    }))
}
