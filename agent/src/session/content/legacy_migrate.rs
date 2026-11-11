//! Lazy conversion: old headers remain readable while bodies stage under
//! separate owners. Format, headers, pricing and ownership publish atomically.
use super::*;
use crate::session::{database, sqlite_store::SqliteStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, LazyLock, Weak};

type ConversionLocks =
    std::collections::HashMap<(std::path::PathBuf, String), Weak<parking_lot::Mutex<()>>>;
static LOCKS: LazyLock<parking_lot::Mutex<ConversionLocks>> = LazyLock::new(Default::default);

#[derive(Serialize, Deserialize)]
struct Conversion {
    revision: i64,
    prefix: String,
    entries: Vec<Value>,
}

type SessionState = (i64, Option<i64>, Option<i64>, Option<String>);

impl SqliteStore {
    /// Runtime opens and mutation admission converge here. Listing/deletion do
    /// not: deleting historical data never needs to convert it first.
    pub(crate) fn ensure_canonical(&self, session: &str) -> Result<bool> {
        let requested = session.to_owned();
        let ready = self.db.fence(move |db| {
            let key = crate::session::identity::session_key(db, &requested)?;
            Ok(format_bound(db, &key)? == FORMAT_VERSION
                && !db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM legacy_conversions WHERE session_id=?1)",
                    [&key],
                    |r| r.get::<_, bool>(0),
                )?)
        })?;
        if ready {
            return Ok(false);
        }
        // Independent Managers can own handles to this same database. Share
        // only this conversion; unrelated conversations continue normally.
        let identity = (self.db.path().canonicalize()?, session.to_owned());
        let lock = {
            let mut locks = LOCKS.lock();
            locks.retain(|_, lock| lock.strong_count() != 0);
            if let Some(lock) = locks.get(&identity).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(parking_lot::Mutex::new(()));
                locks.insert(identity, Arc::downgrade(&lock));
                lock
            }
        };
        let _guard = lock.lock();
        let Some(conversion) = capture(self, session)? else {
            return Ok(false);
        };
        publish(self, session, conversion)?;
        self.db.request_space_reclamation();
        Ok(true)
    }
}

pub(crate) fn plan(db: &Connection) -> Result<(i64, i64, i64)> {
    Ok(db.query_row("SELECT count(DISTINCT session_id),count(*),coalesce(sum(length(body)),0) FROM message_block_bodies", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?)
}

pub(crate) fn apply(store: &SqliteStore, limit: i64) -> Result<(i64, i64, i64)> {
    let sessions = store.db.call(move |db| {
        let mut q=db.prepare("SELECT session_id FROM message_block_bodies GROUP BY session_id ORDER BY sum(length(body)) DESC LIMIT ?1")?;
        let rows=q.query_map([limit], |r| r.get::<_,crate::session::identity::BoundIdentity>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().map(|id|id.to_string()).collect::<Vec<_>>())
    })?;
    let (mut converted, mut entries, mut bodies) = (0, 0, 0);
    for session in sessions {
        let id = session.clone();
        let (n,b)=store.db.read(move |db| {
            let key=crate::session::identity::session_key(db,&id)?;
            Ok(db.query_row("SELECT (SELECT count(*) FROM entries WHERE session_id=?1),(SELECT count(*) FROM message_block_bodies WHERE session_id=?1)",[key],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?)
        })?;
        if store.ensure_canonical(&session)? {
            converted += 1;
            entries += n;
            bodies += b;
        }
    }
    Ok((converted, entries, bodies))
}

#[cfg(test)]
pub(crate) fn recover(store: &SqliteStore, limit: i64) -> Result<usize> {
    let sessions = store.db.call(move |db| {
        let mut q = db
            .prepare("SELECT session_id FROM legacy_conversions ORDER BY started_at_ms LIMIT ?1")?;
        let rows = q
            .query_map([limit], |r| {
                r.get::<_, crate::session::identity::BoundIdentity>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>())
    })?;
    let mut count = 0;
    for session in sessions {
        store.ensure_canonical(&session)?;
        count += 1;
    }
    Ok(count)
}

fn capture(store: &SqliteStore, session: &str) -> Result<Option<Conversion>> {
    let session = session.to_owned();
    store.db.call_as("legacy_conversion_capture",move |db| {
        let tx=database::begin_write_transaction(db)?;
        let key=crate::session::identity::BoundIdentity::session(&tx,&session)?;
        let revision:Option<i64>=tx.query_row("SELECT revision FROM sessions WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM session_tombstones WHERE session_id=?1)",[&key],|r|r.get(0)).optional()?;
        let revision=revision.context("session is missing or deleted")?;
        let pending:Option<String>=tx.query_row("SELECT payloads FROM legacy_conversions WHERE session_id=?1",[&key],|r|r.get(0)).optional()?;
        if pending.is_none() && format_bound(&tx,&key)?==FORMAT_VERSION {return Ok(None);}
        let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE session_id=?1 AND status IN ('running','recovery_failed')) OR EXISTS(SELECT 1 FROM queued_run_receipts WHERE session_id=?1 AND status IN ('queued','cancel_requested','recovery_failed')) OR EXISTS(SELECT 1 FROM compaction_operations WHERE session_id=?1 AND state='started')",[&key],|r|r.get(0))?;
        ensure!(!busy,"session is busy or requires recovery; finish accepted work before migrating history");
        let conversion=if let Some(raw)=pending {
            let value:Value=serde_json::from_str(&raw)?;
            if value.is_object() {
                let recorded:Conversion=serde_json::from_value(value)?;
                ensure!(recorded.revision==revision,"session history changed during migration; original history is preserved");
                return Ok(Some(recorded));
            }
            // Older receipts cleared headers before recording format 2. Their
            // saved payloads remain authoritative and must still be recoverable.
            Conversion {revision,prefix:format!("legacy:{}:",uuid::Uuid::new_v4()),entries:serde_json::from_value(value)?}
        } else {
            let entries={
                let mut q=tx.prepare_cached("SELECT payload FROM entry_records WHERE session_id=?1 ORDER BY position")?;
                let rows=q.query_map([&key],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                rows.into_iter().map(|raw|serde_json::from_str(&raw)).collect::<serde_json::Result<Vec<Value>>>()?
            };
            Conversion {revision,prefix:format!("legacy:{}:",uuid::Uuid::new_v4()),entries}
        };
        tx.execute("INSERT INTO legacy_conversions(session_id,payloads,started_at_ms) VALUES (?1,?2,?3) ON CONFLICT(session_id) DO UPDATE SET payloads=excluded.payloads",params![key,serde_json::to_string(&conversion)?,chrono::Utc::now().timestamp_millis()])?;
        tx.commit()?;
        Ok(Some(conversion))
    })
}

fn publish(store: &SqliteStore, session: &str, conversion: Conversion) -> Result<()> {
    let headers =
        super::entries::prepare_staged(store, session, conversion.entries, &conversion.prefix)?;
    let session = session.to_owned();
    store.db.call_as("legacy_conversion_publish",move |db| {
        let tx=database::begin_write_transaction(db)?;
        let key=crate::session::identity::BoundIdentity::session(&tx,&session)?;
        let current:Option<SessionState>=tx.query_row("SELECT revision,created_at_ms,updated_at_ms,current_metadata_json FROM sessions WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM session_tombstones WHERE session_id=?1)",[&key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let (revision,created,updated,metadata)=current.context("session was deleted during migration")?;
        ensure!(revision==conversion.revision,"session history changed during migration; original history is preserved");
        let pending:String=tx.query_row("SELECT payloads FROM legacy_conversions WHERE session_id=?1",[&key],|r|r.get(0))?;
        let pending:Conversion=serde_json::from_str(&pending)?;
        ensure!(pending.prefix==conversion.prefix,"session migration ownership changed");
        preserve_pricing(&tx,&key)?;
        let runs={
            let mut q=tx.prepare("SELECT * FROM runs WHERE session_id=?1")?;
            let columns=q.column_count();
            let rows=q.query_map([&key],|r|(0..columns).map(|i|r.get::<_,rusqlite::types::Value>(i)).collect::<rusqlite::Result<Vec<_>>>())?.collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        tx.execute("INSERT INTO session_formats(session_id,format) VALUES (?1,2) ON CONFLICT(session_id) DO NOTHING",[&key])?;
        tx.execute("DELETE FROM content_references WHERE session_id=?1 AND owner_kind='entry'",[&key])?;
        tx.execute("INSERT INTO content_references(session_id,owner_kind,owner_id,field,encoding,content_id,byte_offset,byte_length) SELECT session_id,'entry',substr(owner_id,?3),field,encoding,content_id,byte_offset,byte_length FROM content_references WHERE session_id=?1 AND owner_kind='checkpoint' AND owner_id>=?2 AND owner_id<?4",params![key,conversion.prefix,i64::try_from(conversion.prefix.len()+1)?,format!("{}\u{10ffff}",conversion.prefix)])?;
        let has_metadata=headers.iter().any(|e|e["type"]=="session_info");
        tx.execute("DELETE FROM entries WHERE session_id=?1",[&key])?;
        tx.execute("DELETE FROM history_display WHERE session_id=?1",[&key])?;
        tx.execute("DELETE FROM history_index_state WHERE session_id=?1",[&key])?;
        if has_metadata {tx.execute("UPDATE sessions SET current_metadata_json=NULL WHERE id=?1",[&key])?;}
        crate::session::sqlite_store::insert_transcript_entries(&tx,&session,headers)?;
        tx.execute("UPDATE sessions SET revision=max(revision+1,0) WHERE id=?1",[&key])?;
        tx.execute("INSERT INTO checkpoint_generations(session_id,input_key,history_generation) SELECT session_id,input_key,coalesce((SELECT generation FROM content_history_versions WHERE session_id=?1),0) FROM compaction_operations WHERE session_id=?1 ON CONFLICT DO NOTHING",[&key])?;
        // Run-marker projections do not retain every token baseline or counter.
        if let Some(first)=runs.first() {
            let placeholders=(1..=first.len()).map(|i|format!("?{i}")).collect::<Vec<_>>().join(",");
            let mut insert=tx.prepare(&format!("INSERT OR REPLACE INTO runs VALUES ({placeholders})"))?;
            for row in &runs {insert.execute(rusqlite::params_from_iter(row))?;}
        }
        tx.execute("UPDATE sessions SET created_at_ms=?2,updated_at_ms=?3 WHERE id=?1",params![key,created,updated])?;
        if !has_metadata {tx.execute("UPDATE sessions SET current_metadata_json=?2 WHERE id=?1",params![key,metadata])?;}
        tx.execute("DELETE FROM content_references WHERE session_id=?1 AND owner_kind='checkpoint' AND owner_id>=?2 AND owner_id<?3",params![key,conversion.prefix,format!("{}\u{10ffff}",conversion.prefix)])?;
        tx.execute("DELETE FROM run_events WHERE session_id=?1",[&key])?;
        tx.execute("DELETE FROM run_snapshots WHERE session_id=?1",[&key])?;
        tx.execute("DELETE FROM run_snapshot_cache WHERE session_id=?1",[&key])?;
        tx.execute("DELETE FROM legacy_conversions WHERE session_id=?1",[&key])?;
        tx.commit()?;
        Ok(())
    })
}

pub(crate) fn preserve_pricing(db: &Connection, session: &impl rusqlite::ToSql) -> Result<()> {
    db.execute("INSERT OR IGNORE INTO run_pricing(session_id,run_id,sequence,payload)
        SELECT session_id,run_id,sequence,payload FROM run_events e WHERE session_id=?1
        AND json_extract(payload,'$.event_type') IN ('usage','model_changed')
        AND NOT EXISTS(SELECT 1 FROM run_snapshots s WHERE s.session_id=e.session_id AND s.run_id=e.run_id)",[session])?;
    db.execute("INSERT OR IGNORE INTO run_pricing(session_id,run_id,sequence,payload)
        SELECT s.session_id,s.run_id,json_extract(j.value,'$.sequence'),json_extract(j.value,'$.payload')
        FROM run_snapshots s,json_each(s.pricing_payloads) j WHERE s.session_id=?1",[session])?;
    Ok(())
}

#[cfg(test)]
#[path = "legacy_migrate_tests.rs"]
mod tests;
