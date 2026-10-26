//! Durable leases for ordinary entry preparation. Keep each lease until the
//! header or queued-input receipt commits; crashed preparation releases only
//! its exact unpublished owner, with bounded cleanup and no transcript scan.
use super::{operations::ActiveOperation, WRITE_BYTES};
use crate::session::{database, sqlite_store::SqliteStore};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection};
use serde_json::Value;

pub(crate) struct PreparedEntries {
    values: Vec<Value>,
    leases: Vec<Lease>,
}
struct Lease {
    store: SqliteStore,
    operation: String,
    _active: ActiveOperation,
}

impl PreparedEntries {
    pub(crate) fn legacy(values: Vec<Value>) -> Self {
        Self {
            values,
            leases: Vec::new(),
        }
    }

    pub(crate) fn stage(store: &SqliteStore, session: &str, values: Vec<Value>) -> Result<Self> {
        let mut prepared = Self::legacy(Vec::new());
        // Register before the first body owner is created. The guard already
        // exists when the durable receipt becomes visible to another Manager.
        for value in &values {
            let id = value["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .context("entry identity is required")?;
            let run = value
                .pointer("/meta/run_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            ensure!(
                id.len().saturating_add(run.as_ref().map_or(0, String::len)) <= WRITE_BYTES,
                "entry staging identity exceeds metadata budget"
            );
            let operation = uuid::Uuid::new_v4().to_string();
            let lease = Lease {
                store: store.clone(),
                _active: ActiveOperation::claim(&operation),
                operation: operation.clone(),
            };
            let (s, e, op) = (session.to_owned(), id.to_owned(), operation);
            store.db.call(move |db| {
                let tx=database::begin_write_transaction(db)?;
                let revision:i64=tx.query_row("SELECT max(revision,0) FROM sessions WHERE id=?1",[&s],|row|row.get(0))?;
                let reset:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM content_operations WHERE session_id=?1 AND kind='reset')",[&s],|row|row.get(0))?;
                ensure!(!reset,"session history reset is in progress");
                tx.execute("INSERT INTO content_operations(operation_id,session_id,owner_prefix,expected_revision,kind) VALUES (?1,?2,?3,?4,'entry')",params![op,s,format!("entry:{op}:"),revision])?;
                tx.execute("INSERT INTO content_entry_stages(operation_id,session_id,entry_id,run_id) VALUES (?1,?2,?3,?4)",params![op,s,e,run])?;
                tx.commit()?;Ok(())
            })?;
            prepared.leases.push(lease);
        }
        prepared.values = super::entries::prepare_unleased(store, session, values)?;
        Ok(prepared)
    }

    /// Keep the lease outside the writer closure. Dropping it on that worker
    /// would recursively call the worker while cleaning its durable receipt.
    pub(crate) fn take(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.values)
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Err(error) = self.store.cleanup_content_operation(&self.operation) {
            tracing::warn!(%error,operation=%self.operation,"entry staging cleanup deferred until startup");
        }
        self.store.db.request_space_reclamation();
    }
}

/// Called inside the cleanup transaction. A published header, a durable queued
/// input, or another preparation lease protects the owner. No body is read.
pub(super) fn cleanup(db: &Connection, operation: &str, session: &str) -> Result<bool> {
    let (entry, run): (String, Option<String>) = db.query_row(
        "SELECT entry_id,run_id FROM content_entry_stages WHERE operation_id=?1 AND session_id=?2",
        params![operation, session],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let protected:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM entries WHERE session_id=?1 AND entry_id=?2)
        OR EXISTS(SELECT 1 FROM content_entry_stages WHERE session_id=?1 AND entry_id=?2 AND operation_id<>?3)
        OR EXISTS(SELECT 1 FROM queued_run_receipts WHERE session_id=?1 AND run_id=?4 AND json_extract(user_entry_json,'$.id')=?2)",params![session,entry,operation,run],|row|row.get(0))?;
    if protected {
        return Ok(true);
    }
    let removed=db.execute("DELETE FROM content_references WHERE (session_id,owner_kind,owner_id,field) IN
        (SELECT session_id,owner_kind,owner_id,field FROM content_references WHERE session_id=?1 AND owner_kind='entry' AND owner_id=?2 LIMIT 64)",params![session,entry])?;
    Ok(removed == 0)
}

#[cfg(test)]
#[path = "entry_stages_tests.rs"]
mod tests;
