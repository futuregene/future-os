//! Transactional, idempotent creation of a materialized session fork.
use super::Manager;
use crate::session::content::{operations::ActiveOperation, reader::Snapshot};
use crate::session::SessionEntry;
use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use std::collections::HashMap;

impl Manager {
    /// Resolve, materialize, and record a fork under one SQLite transaction.
    /// Replaying the same request returns the same child; reusing a request id
    /// for different semantics is rejected instead of silently duplicating or
    /// returning the wrong branch.
    pub fn create_fork(
        &self,
        request: crate::session::ForkRequest,
    ) -> Result<crate::session::ForkResult> {
        let outcome = self.create_fork_identity(request)?;
        Ok(crate::session::ForkResult {
            session: self.load(&outcome.session_id)?,
            created: outcome.created,
        })
    }

    pub(crate) fn create_fork_identity(
        &self,
        request: crate::session::ForkRequest,
    ) -> Result<crate::session::fork::ForkIdentity> {
        if request.request_id.trim().is_empty() {
            return Err(anyhow!("fork request id is required"));
        }
        if request.parent_session_id.trim().is_empty() {
            return Err(anyhow!("fork parent session id is required"));
        }
        let creation = fork_lock(self)?;
        let _creation = creation.lock();
        let operation = request.request_id.clone();
        let pending_canonical=self.storage()?.db.read(move|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM fork_operations f JOIN sessions s ON s.id=f.child_session_id JOIN session_formats v ON v.session_id=s.id WHERE f.request_id=?1 AND v.format=2)",[operation],|row|row.get::<_,bool>(0))?))?;
        if pending_canonical
            || self.storage()?.storage_format(&request.parent_session_id)?
                == crate::session::content::FORMAT_VERSION
        {
            return self.create_segmented_fork(request);
        }
        let fingerprint = serde_json::to_string(&request)?;
        let request_id = request.request_id.clone();
        let parent_id = request.parent_session_id.clone();
        let point = request.point.clone();
        let created_by = request.created_by.clone();
        let creator_id = request.creator_id.clone();
        let fingerprint_for_tx = fingerprint.clone();
        let outcome = self.storage()?.db.call(move |db| {
            // Reads the operation and the parent's entries before it writes:
            // take the write lock at BEGIN, or a concurrent connection's commit
            // invalidates this snapshot and the upgrade fails instantly.
            let tx = crate::session::database::begin_write_transaction(db)?;
            if let Some((stored_fingerprint, child_id)) = tx
                .query_row(
                    "SELECT request_fingerprint, child_session_id FROM fork_operations WHERE request_id=?1",
                    [&request_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?
            {
                if stored_fingerprint != fingerprint_for_tx {
                    return Err(anyhow!("fork request id was already used with different parameters"));
                }
                tx.commit()?;
                return Ok((child_id, false));
            }

            let rows = {
                let mut statement = tx.prepare(
                    "SELECT payload FROM entry_records WHERE session_id=?1 ORDER BY position",
                )?;
                let rows = statement
                    .query_map([&parent_id], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            if rows.is_empty() {
                return Err(anyhow!("Session not found on disk — it may have been deleted or moved."));
            }
            let entries = rows
                .into_iter()
                .map(|row| serde_json::from_str::<SessionEntry>(&row))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let parent = Self::session_from_entries(&parent_id, entries)?;
            let mut child = crate::session::build_fork_session(&parent, &point)?;
            crate::session::set_creation_provenance(&mut child, &created_by, &creator_id);
            let child_id = child.id.clone();
            let forked_at_ms = child.created_at.timestamp_millis();
            let values = Self::encoded(&child.entries)?;
            tx.execute("INSERT INTO sessions(id) VALUES (?1)", [&child_id])?;
            crate::session::sqlite_store::insert_transcript_entries(&tx, &child_id, values)?;
            // Copied entries intentionally retain their historical timestamps,
            // but the session itself was created now. `insert_transcript_entries` derives
            // `created_at_ms` as the minimum entry timestamp, so restore the
            // fork operation boundary after the snapshot is materialized.
            tx.execute(
                "UPDATE sessions SET created_at_ms=?2 WHERE id=?1",
                params![child_id, forked_at_ms],
            )?;
            tx.execute(
                "INSERT INTO fork_operations(
                    request_id, request_fingerprint, parent_session_id,
                    child_session_id, created_at_ms
                 ) VALUES (?1,?2,?3,?4,?5)",
                params![
                    request_id,
                    fingerprint_for_tx,
                    parent_id,
                    child_id,
                    forked_at_ms
                ],
            )?;
            tx.commit()?;
            Ok((child_id, true))
        })?;

        self.invalidate_display_projection(&outcome.0);
        Ok(crate::session::fork::ForkIdentity {
            session_id: outcome.0,
            created: outcome.1,
        })
    }
}

impl Manager {
    fn create_segmented_fork(
        &self,
        request: crate::session::ForkRequest,
    ) -> Result<crate::session::fork::ForkIdentity> {
        let store = self.storage()?;
        let fingerprint = serde_json::to_string(&request)?;
        let request_id = request.request_id.clone();
        let prior=store.db.read(move|db| {
            Ok(db.query_row("SELECT f.request_fingerprint,f.child_session_id,s.revision FROM fork_operations f JOIN sessions s ON s.id=f.child_session_id WHERE f.request_id=?1",[request_id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,crate::session::identity::BoundIdentity>(1)?.to_string(),row.get::<_,i64>(2)?))).optional()?)
        })?;
        if let Some((stored, child, revision)) = prior {
            anyhow::ensure!(
                stored == fingerprint,
                "fork request id was already used with different parameters"
            );
            if revision >= 0 {
                return Ok(crate::session::fork::ForkIdentity {
                    session_id: child,
                    created: false,
                });
            }
            // A previous process stopped before publication. The durable
            // operation receipt identifies exactly its hidden staging session.
            store.discard_hidden_fork(&child)?;
        }
        store.db.request_space_reclamation();
        let parent_id = request.parent_session_id.clone();
        anyhow::ensure!(
            self.contains(&parent_id)?,
            "Session not found on disk — it may have been deleted or moved."
        );
        let scope = Snapshot::capture(store, &parent_id)?;
        let (parent, sources) = self.fork_source_entries(&scope, &request.point)?;
        let (mut child, mapping) =
            crate::session::fork::build_fork_session_with_mapping(&parent, &request.point)?;
        crate::session::set_creation_provenance(
            &mut child,
            &request.created_by,
            &request.creator_id,
        );
        let origins = mapping
            .into_iter()
            .map(|(source, target)| (target, source))
            .collect::<HashMap<_, _>>();
        let child_id = child.id.clone();
        let _active = ActiveOperation::claim(&crate::session::content::fork::active_key(&child_id));
        let created = child.created_at.timestamp_millis();
        let values = Self::encoded(&std::mem::take(&mut child.entries))?;
        let (id, parent, operation, input, check) = (
            child_id.clone(),
            parent_id.clone(),
            request.request_id.clone(),
            fingerprint,
            scope.clone(),
        );
        anyhow::ensure!(
            input.len() <= crate::session::content::WRITE_BYTES,
            "fork request exceeds its metadata budget"
        );
        store.db.call(move |db| {
            let tx=crate::session::database::begin_write_transaction(db)?;
            check.check(&tx)?;
            let parent=crate::session::identity::BoundIdentity::session(&tx,&parent)?;
            let id=crate::session::identity::BoundIdentity::fresh_session(&tx,&id)?;
            tx.execute("INSERT INTO sessions(id,revision) VALUES (?1,-1)",[&id])?;
            crate::session::content::enable(&tx,&id)?;
            tx.execute("INSERT INTO fork_operations(request_id,request_fingerprint,parent_session_id,child_session_id,created_at_ms) VALUES (?1,?2,?3,?4,?5)",params![operation,input,parent,id,created])?;
            tx.commit()?;Ok(())
        })?;
        let result = (|| -> Result<()> {
            for mut entry in values {
                let target = entry["id"]
                    .as_str()
                    .ok_or_else(|| anyhow!("fork entry identity missing"))?;
                if let Some(source) = origins.get(target).and_then(|source| sources.get(source)) {
                    // SessionEntry owns the known envelope. Preserve opaque root
                    // extensions from the captured raw header as well.
                    for (key, value) in source
                        .as_object()
                        .ok_or_else(|| anyhow!("fork source is not an object"))?
                    {
                        if !matches!(
                            key.as_str(),
                            "id" | "type"
                                | "role"
                                | "timestamp"
                                | "content"
                                | "meta"
                                | "thinking"
                                | "tool_calls"
                                | "tool_call_id"
                                | "name"
                                | "tool_args"
                        ) {
                            entry
                                .as_object_mut()
                                .expect("encoded entry object")
                                .entry(key.clone())
                                .or_insert_with(|| value.clone());
                        }
                    }
                    store.pin_fork_fields(&scope, &child_id, source, &entry)?;
                }
                let prepared = crate::session::content::entries::prepare_shared_fork(
                    store,
                    &child_id,
                    vec![entry],
                )?;
                let (id, check) = (child_id.clone(), scope.clone());
                store.db.call(move |db| {
                    let tx = crate::session::database::begin_write_transaction(db)?;
                    check.check(&tx)?;
                    let id = crate::session::identity::BoundIdentity::session(&tx, &id)?;
                    let hidden: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND revision=-1)",
                        [&id],
                        |row| row.get(0),
                    )?;
                    anyhow::ensure!(hidden, "fork publication state changed");
                    crate::session::sqlite_store::insert_transcript_entries(&tx, &id, prepared)?;
                    tx.commit()?;
                    Ok(())
                })?;
            }
            let (id, check) = (child_id.clone(), scope.clone());
            store.db.call(move |db| {
                let tx=crate::session::database::begin_write_transaction(db)?;check.check(&tx)?;
                let id=crate::session::identity::BoundIdentity::session(&tx,&id)?;
                anyhow::ensure!(tx.execute("UPDATE sessions SET revision=0,created_at_ms=?2,updated_at_ms=?2 WHERE id=?1 AND revision=-1",params![id,created])?==1,"fork publication state changed");
                tx.commit()?;Ok(())
            })
        })();
        if let Err(error) = result {
            if let Err(cleanup) = store.discard_hidden_fork(&child_id) {
                tracing::warn!(%cleanup,"hidden fork staging retained for startup/request retry");
            }
            store.db.request_space_reclamation();
            return Err(error);
        }
        self.invalidate_display_projection(&child_id);
        Ok(crate::session::fork::ForkIdentity {
            session_id: child_id,
            created: true,
        })
    }

    /// Resolve on thin headers, then hydrate only identity-bearing checkpoint
    /// fields inside the selected range. Normal message bodies remain shared.
    fn fork_source_entries(
        &self,
        scope: &Snapshot,
        point: &crate::session::ForkPoint,
    ) -> Result<(crate::session::Session, HashMap<String, Value>)> {
        let store = self.storage()?;
        let mut raw = Vec::new();
        let mut typed = Vec::new();
        let mut locators = Vec::new();
        let mut after = -1;
        loop {
            let page = scope.page(store, after)?;
            if page.is_empty() {
                break;
            }
            for locator in page {
                after = locator.position;
                let header = scope.header(store, &locator)?;
                typed.push(serde_json::from_value::<SessionEntry>(header.clone())?);
                raw.push(header);
                locators.push(locator);
            }
        }
        anyhow::ensure!(!typed.is_empty(), "fork parent has no history");
        let cutoff = crate::session::fork::resolve_cutoff(&typed, point)?;
        for index in 0..=cutoff {
            if typed[index].entry_type == crate::session::ENTRY_TYPE_COMPACTION
                && raw[index]["content"]["schema_version"] == 3
            {
                for name in [
                    "covered_from_entry_id",
                    "cutoff_entry_id",
                    "protected_entry_ids",
                    "summary",
                ] {
                    if raw[index]["content"].get(name).is_some() {
                        if let Some(value) =
                            scope.field(store, &locators[index], &format!("/content/{name}"))?
                        {
                            raw[index]["content"][name] = value;
                        }
                    }
                }
                typed[index] = serde_json::from_value(raw[index].clone())?;
            }
        }
        scope.verify(store)?;
        let parent = Self::session_from_entries(scope.session_id(), typed)?;
        let sources = raw
            .into_iter()
            .filter_map(|value| {
                value["id"]
                    .as_str()
                    .map(str::to_owned)
                    .map(|id| (id, value))
            })
            .collect();
        Ok((parent, sources))
    }
}

/// Forks serialize only with other fork requests against the same database,
/// including separate Manager handles; normal writers/readers keep running.
fn fork_lock(manager: &Manager) -> Result<std::sync::Arc<parking_lot::Mutex<()>>> {
    use std::{
        collections::HashMap,
        sync::{Arc, LazyLock, Weak},
    };
    static LOCKS: LazyLock<
        parking_lot::Mutex<HashMap<std::path::PathBuf, Weak<parking_lot::Mutex<()>>>>,
    > = LazyLock::new(Default::default);
    manager.storage()?;
    let path = manager.database_path().canonicalize()?;
    let mut locks = LOCKS.lock();
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&path).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(parking_lot::Mutex::new(()));
    locks.insert(path, Arc::downgrade(&lock));
    Ok(lock)
}
