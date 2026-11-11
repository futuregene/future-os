//! Explicit developer maintenance. One conversation per request, no history
//! deletion; invalid/busy sessions never prevent the remaining list progressing.
use super::{content, database, sqlite_store::SqliteStore, Manager};
use anyhow::{ensure, Context, Result};
use rusqlite::params;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const SLICE: Duration = Duration::from_secs(20);

struct Lease {
    store: SqliteStore,
    operation: String,
    _active: content::operations::ActiveOperation,
    released: bool,
}
impl Lease {
    fn finish(mut self) -> Result<()> {
        self.store.cleanup_content_operation(&self.operation)?;
        self.released = true;
        Ok(())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if !self.released {
            if let Err(error) = self.store.cleanup_content_operation(&self.operation) {
                tracing::warn!(%error,"database organization lease cleanup deferred");
            }
        }
        self.store.db.request_space_reclamation();
    }
}

impl Manager {
    pub fn database_organization_plan(&self) -> Result<Value> {
        let store = self.storage()?;
        Ok(json!({"sessionIds":store.ids(false)?,"storage":storage_size(store)?}))
    }

    /// A failed/partial session is reported by the RPC caller and skipped. A
    /// continued slice re-enters this same idempotent path with fresh admission.
    pub fn organize_database_session(&self, session: &str) -> Result<Value> {
        let store = self.storage()?;
        let id = session.to_owned();
        let busy = store.db.fence(move |db| busy_session(db, &id))?;
        if busy {
            return Ok(json!({"status":"skipped","reason":"busy","hasMore":false}));
        }
        let migrated = store.ensure_canonical(session)?;
        if migrated {
            self.invalidate_display_projection(session);
        }
        let operation = uuid::Uuid::new_v4().to_string();
        let lease = Lease {
            store: store.clone(),
            operation: operation.clone(),
            _active: content::operations::ActiveOperation::claim(&operation),
            released: false,
        };
        let (id, op) = (session.to_owned(), operation);
        let admitted=store.db.call(move |db| {
            let tx=database::begin_write_transaction(db)?;
            if busy_session(&tx,&id)? {return Ok(false);}
            let key=super::identity::BoundIdentity::session(&tx,&id)?;
            let revision:i64=tx.query_row("SELECT revision FROM sessions WHERE id=?1 AND revision>=0",[&key],|r|r.get(0))?;
            tx.execute("INSERT INTO content_operations(operation_id,session_id,owner_prefix,expected_revision,kind) VALUES (?1,?2,?3,?4,'reset')",params![op,key,format!("reset:{op}:"),revision])?;
            tx.commit()?;Ok(true)
        })?;
        if !admitted {
            return Ok(
                json!({"status":"skipped","reason":"busy","migrated":migrated,"hasMore":false}),
            );
        }
        // Decode the authoritative transcript before changing its storage.
        // This catches malformed headers, missing bodies and invalid codecs.
        store.entries(session)?;
        let id = session.to_owned();
        let legacy_rows = store.db.call_as("organize_legacy", move |db| {
            let tx = database::begin_write_transaction(db)?;
            let key = super::identity::session_key(&tx, &id)?;
            content::legacy_migrate::preserve_pricing(&tx, &key)?;
            let removed = tx.execute("DELETE FROM run_events WHERE session_id=?1", [&key])?
                + tx.execute("DELETE FROM run_snapshots WHERE session_id=?1", [&key])?
                + tx.execute("DELETE FROM run_snapshot_cache WHERE session_id=?1", [&key])?;
            tx.commit()?;
            Ok(i64::try_from(removed)?)
        })?;
        let deadline = Instant::now() + SLICE;
        let (mut retired, mut removed, mut compressed, mut saved) = (0i64, legacy_rows, 0i64, 0i64);
        loop {
            let id = session.to_owned();
            let (n, rows) = store.db.call_as("organize_retire", move |db| {
                content::retire::apply_for_session(db, &id, 4, deadline)
            })?;
            retired += n;
            removed += rows;
            if n == 0 || Instant::now() >= deadline {
                break;
            }
        }
        while Instant::now() < deadline {
            let id = session.to_owned();
            let (n, before, after, examined) =
                store.db.call_as("organize_compress", move |db| {
                    content::compress::apply_for_session(db, &id, 32)
                })?;
            compressed += n;
            saved += before - after;
            if examined == 0 {
                break;
            }
        }
        let id = session.to_owned();
        let metadata_saved = store
            .db
            .call_as("organize_metadata", move |db| compress_metadata(db, &id))?;
        saved += metadata_saved;
        let id = session.to_owned();
        let more=store.db.read(move |db| {
            let key=super::identity::session_key(db,&id)?;
            let bodies:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM content_references r JOIN content_objects o ON o.id=r.content_id WHERE r.session_id=?1 AND o.sealed=1 AND o.byte_length>=512 AND NOT EXISTS(SELECT 1 FROM content_compressed c WHERE c.content_id=o.id))",[&key],|r|r.get(0))?;
            let streams:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM stream_runs s JOIN runs r ON r.session_id=s.session_id AND r.run_id=s.run_id WHERE s.session_id=?1 AND r.status IN ('completed','failed','cancelled','interrupted') AND coalesce(r.completed_at_ms,0)<=CAST(strftime('%s','now') AS INTEGER)*1000-600000 AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=s.id))",[&key],|r|r.get(0))?;
            Ok(bodies||streams)
        })?;
        lease.finish()?;
        Ok(
            json!({"status":"done","migrated":migrated,"retiredStreams":retired,"removedRows":removed,"compressedBodies":compressed,"payloadBytesSaved":saved,"hasMore":more}),
        )
    }

    /// Drain explicitly retired/unowned data, then compact the file once. Busy
    /// databases keep completed per-session work and report deferred compaction.
    pub fn finish_database_organization(&self) -> Result<Value> {
        let store = self.storage()?;
        let deadline = Instant::now() + SLICE;
        loop {
            let done = store.db.call_as("organize_collect", content::reclaim)?;
            if done {
                break;
            }
            if Instant::now() >= deadline {
                return Ok(json!({"hasMore":true,"storage":storage_size(store)?}));
            }
        }
        let outcome=store.db.call_as("organize_vacuum",move |db| {
            let busy:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE status='running') OR EXISTS(SELECT 1 FROM queued_run_receipts WHERE status IN ('queued','cancel_requested')) OR EXISTS(SELECT 1 FROM content_operations) OR EXISTS(SELECT 1 FROM compaction_operations WHERE state='started')",[],|r|r.get(0))?;
            if busy {return Ok((false,Some("busy".to_owned())));}
            db.busy_timeout(Duration::ZERO)?;
            let compact=(||->Result<()> {
                db.execute_batch("VACUUM")?;
                let (busy,_,_):(i64,i64,i64)=db.query_row("PRAGMA wal_checkpoint(TRUNCATE)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
                ensure!(busy==0,"checkpoint_busy");
                Ok(())
            })();
            db.busy_timeout(Duration::from_secs(5))?;
            Ok(match compact {Ok(())=>(true,None),Err(error)=>(false,Some(error.to_string()))})
        })?;
        let integrity = store
            .db
            .read(|db| Ok(db.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))?))?;
        ensure!(
            integrity == "ok",
            "database integrity check failed: {integrity}"
        );
        Ok(
            json!({"hasMore":false,"compacted":outcome.0,"deferredReason":outcome.1,"integrity":integrity,"storage":storage_size(store)?}),
        )
    }
}

fn busy_session(db: &rusqlite::Connection, session: &str) -> Result<bool> {
    let key = super::identity::session_key(db, session)?;
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE session_id=?1 AND status IN ('running','recovery_failed')) OR EXISTS(SELECT 1 FROM queued_run_receipts WHERE session_id=?1 AND status IN ('queued','cancel_requested','recovery_failed')) OR EXISTS(SELECT 1 FROM content_operations WHERE session_id=?1) OR EXISTS(SELECT 1 FROM compaction_operations WHERE session_id=?1 AND state='started')",[key],|r|r.get(0))?)
}

fn storage_size(store: &SqliteStore) -> Result<Value> {
    let path = store.db.path();
    let database = std::fs::metadata(path)?.len();
    let wal = match std::fs::metadata(path.with_file_name(
        format!("{}-wal",path.file_name().context("database filename missing")?.to_string_lossy()),
    )) {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    Ok(json!({"databaseBytes":database,"walBytes":wal,"totalBytes":database.saturating_add(wal)}))
}

fn compress_metadata(db: &mut rusqlite::Connection, session: &str) -> Result<i64> {
    let key = super::identity::session_key(db, session)?;
    let rows = {
        let mut query=db.prepare("SELECT position,CAST(metadata_json AS BLOB) FROM entries e WHERE session_id=?1 AND length(CAST(metadata_json AS BLOB))>=512 AND NOT EXISTS(SELECT 1 FROM entry_bodies b WHERE b.session_id=e.session_id AND b.position=e.position AND b.field='metadata')")?;
        let rows = query
            .query_map([&key], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut saved = 0;
    for (position, plain) in rows {
        let encoded = content::compress::encode(&plain)?;
        if encoded.len() * 10 >= plain.len() * 9 {
            continue;
        }
        let tx = database::begin_write_transaction(db)?;
        tx.execute("INSERT INTO entry_bodies(session_id,position,field,encoding,body) VALUES (?1,?2,'metadata','gzip',?3)",params![key,position,encoded])?;
        tx.execute(
            "UPDATE entries SET metadata_json='{}' WHERE session_id=?1 AND position=?2",
            params![key, position],
        )?;
        tx.commit()?;
        saved += i64::try_from(plain.len() - encoded.len())?;
    }
    Ok(saved)
}

#[cfg(test)]
#[path = "maintenance_tests.rs"]
mod tests;
