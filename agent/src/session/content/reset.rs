//! Explicit reset staging: plan off the writer, pin retained immutable ranges,
//! atomically publish headers/references, then retire unreachable owners in slices.
use super::{operations::ActiveOperation, stream::Encoding, ContentRange};
use crate::session::{
    database,
    reset::{ResetPlan, ResetPlanResult, HISTORY_CHANGED},
    sqlite_store::SqliteStore,
};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::HashMap;

fn check_revision(db: &Connection, session: &str, expected: i64) -> Result<()> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    let current: Option<i64> = db
        .query_row(
            "SELECT revision FROM sessions WHERE id=?1",
            [&stored_session],
            |row| row.get(0),
        )
        .optional()?;
    ensure!(
        current == Some(expected),
        "{HISTORY_CHANGED}: session history changed while reset was prepared"
    );
    Ok(())
}

impl SqliteStore {
    pub(crate) fn apply_segmented_reset(
        &self,
        session: &str,
        revision: i64,
        original: &[Value],
        planned: ResetPlanResult,
    ) -> Result<ResetPlan> {
        let operation = uuid::Uuid::new_v4().to_string();
        let _active = ActiveOperation::claim(&operation);
        let prefix = format!("reset:{operation}:");
        let (s, op, p) = (session.to_owned(), operation.clone(), prefix.clone());
        self.db.call(move |db| {
            let tx = database::begin_write_transaction(db)?;
            let s=crate::session::identity::BoundIdentity::session(&tx,&s)?;
            check_revision(&tx, &s, revision)?;
            let busy: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE session_id=?1 AND status='running') OR EXISTS(SELECT 1 FROM queued_run_receipts WHERE session_id=?1 AND status IN ('queued','cancel_requested')) OR EXISTS(SELECT 1 FROM content_operations WHERE session_id=?1)",[&s],|row|row.get(0))?;
            ensure!(!busy,"session is busy; finish accepted work before resetting history");
            tx.execute("INSERT INTO content_operations(operation_id,session_id,owner_prefix,expected_revision,kind) VALUES (?1,?2,?3,?4,'reset')",params![op,s,p,revision])?;
            tx.commit()?; Ok(())
        })?;
        let ResetPlanResult {
            plan,
            retained,
            affected_runs,
        } = planned;
        let result = (|| -> Result<ResetPlan> {
            let sources: HashMap<&str, &Value> = original
                .iter()
                .filter_map(|value| value["id"].as_str().map(|id| (id, value)))
                .collect();
            let mut headers = Vec::with_capacity(retained.len());
            for retained in &retained {
                let value =
                    crate::session::entry_records::normalize_entry(serde_json::to_value(retained)?);
                let entry = value["id"]
                    .as_str()
                    .context("reset entry has no identity")?;
                if let Some(source) = sources.get(entry) {
                    let source = crate::session::entry_records::normalize_entry((**source).clone());
                    self.pin_reset_fields(session, revision, &prefix, &source, &value)?;
                }
                headers.extend(super::entries::prepare_staged(
                    self,
                    session,
                    vec![value],
                    &prefix,
                )?);
            }
            let (s, p) = (session.to_owned(), prefix.clone());
            self.db.call(move |db| {
                let tx = database::begin_write_transaction(db)?;
            let s=crate::session::identity::BoundIdentity::session(&tx,&s)?;
                check_revision(&tx, &s, revision)?;
                // Replacing ordinal 0 after dropping reasoning must not read
                // the old ordinal-0 owner. The independent checkpoint owners
                // pin all retained bytes while entry ownership is replaced.
                tx.execute("DELETE FROM content_references WHERE session_id=?1 AND owner_kind='entry'",[&s])?;
                tx.execute(&format!("INSERT INTO content_references(session_id,owner_kind,owner_id,field,encoding,content_id,byte_offset,byte_length) SELECT session_id,'entry',{},field,encoding,content_id,byte_offset,byte_length FROM content_references WHERE session_id=?1 AND owner_kind='checkpoint' AND owner_id>=?2 AND owner_id<?4",crate::session::identity::storage_expression("substr(owner_id,?3)")),params![s,p,i64::try_from(p.len()+1)?,format!("{p}\u{10ffff}")])?;
                crate::session::sqlite_store::rewrite_transcript_rows(&tx,&s,headers)?;
                for run in &affected_runs {
                    // Reader visibility changes here. Physical event/link
                    // retirement happens in bounded idle jobs, never cascades
                    // a large journal inside this publication transaction.
                    let run=crate::session::identity::BoundIdentity::run(&tx,&s,run)?;
                    tx.execute("INSERT OR IGNORE INTO retired_streams(stream_id) SELECT id FROM stream_runs WHERE session_id=?1 AND run_id=?2",params![s,run])?;
                }
                tx.commit()?;
                Ok(plan)
            })
        })();
        // This receipt remains recoverable if cleanup fails AFTER publication.
        // Removing unpublished owners cannot remove active entry references.
        if let Err(error) = self.cleanup_content_operation(&operation) {
            tracing::warn!(%error,operation,"reset staging cleanup deferred until startup");
        }
        self.db.request_space_reclamation();
        result
    }

    fn pin_reset_fields(
        &self,
        session: &str,
        revision: i64,
        prefix: &str,
        source: &Value,
        retained: &Value,
    ) -> Result<()> {
        let entry = retained["id"]
            .as_str()
            .context("reset entry identity missing")?
            .to_owned();
        let old = source["content"].as_array();
        let new = retained["content"].as_array();
        let mut ordinals = HashMap::new();
        if let (Some(old), Some(new)) = (old, new) {
            let mut start = 0;
            for (ordinal, block) in new.iter().enumerate() {
                if let Some(index) = old
                    .iter()
                    .enumerate()
                    .skip(start)
                    .find_map(|(index, known)| (known == block).then_some(index))
                {
                    ordinals.insert(index, ordinal);
                    start = index + 1;
                }
            }
        }
        let (s, e) = (session.to_owned(), entry.clone());
        let references = self.db.read(move |db| {
            let e=crate::session::identity::BoundIdentity::entry(db,&s,&e)?;
            let s=crate::session::identity::session_key(db,&s)?;
            let mut query = db.prepare("SELECT field,content_id,byte_offset,byte_length,encoding FROM content_references WHERE session_id=?1 AND owner_kind='entry' AND owner_id=?2")?;
            let rows = query.query_map(params![s,e],|row|Ok((row.get::<_,String>(0)?,ContentRange { id:row.get(1)?,offset:row.get(2)?,length:row.get(3)? },row.get::<_,String>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        let mut links = Vec::new();
        for (field, range, encoding) in references {
            let target = if source["content"].is_array() && field.starts_with("/content/") {
                let path = field.strip_prefix("/content/").expect("prefix checked");
                let (index, tail) = path
                    .split_once('/')
                    .map_or((path, None), |(index, tail)| (index, Some(tail)));
                let Some(ordinal) = index
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| ordinals.get(&index))
                else {
                    continue;
                };
                tail.map_or_else(
                    || format!("/content/{ordinal}"),
                    |tail| format!("/content/{ordinal}/{tail}"),
                )
            } else {
                field.clone()
            };
            if source
                .pointer(&field)
                .is_some_and(|value| Some(value) == retained.pointer(&target))
            {
                links.push((target, range, Encoding::parse(&encoding)?));
            }
        }
        for batch in links.chunks(64) {
            let (s, owner, links) = (
                session.to_owned(),
                format!("{prefix}{entry}"),
                batch.to_vec(),
            );
            self.db.call(move |db| {
                let tx = database::begin_write_transaction(db)?;
                let s = crate::session::identity::BoundIdentity::session(&tx, &s)?;
                check_revision(&tx, &s, revision)?;
                for (field, range, encoding) in links {
                    super::bind_encoded(&tx, &s, "checkpoint", &owner, &field, range, encoding)?;
                }
                tx.commit()?;
                Ok(())
            })?;
        }
        Ok(())
    }

    pub(crate) fn cleanup_content_operation(&self, operation: &str) -> Result<()> {
        self.cleanup_operation_cancellable(operation, None)
    }

    fn cleanup_operation_cancellable(
        &self,
        operation: &str,
        cancellation: Option<&crate::service::StartupCancellation>,
    ) -> Result<()> {
        loop {
            if let Some(cancellation) = cancellation {
                cancellation.check()?;
            }
            let operation = operation.to_owned();
            let complete = self.db.call(move |db| {
                let tx = database::begin_write_transaction(db)?;
                let scope: Option<(crate::session::identity::BoundIdentity,String,String)> = tx.query_row("SELECT session_id,owner_prefix,kind FROM content_operations WHERE operation_id=?1",[&operation],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
                let Some((session,prefix,kind)) = scope else { return Ok(true); };
                ensure!(uuid::Uuid::parse_str(&operation).is_ok() && matches!(kind.as_str(),"reset"|"metadata"|"checkpoint"|"entry") && prefix == format!("{kind}:{operation}:"),"invalid content staging receipt; preserve content ownership");
                if kind=="entry" {
                    let complete=super::entry_stages::cleanup(&tx,&operation,&session)?;
                    if complete {tx.execute("DELETE FROM content_operations WHERE operation_id=?1",[operation])?;}
                    tx.commit()?;return Ok(complete);
                }
                let changed = tx.execute("DELETE FROM content_references WHERE (session_id,owner_kind,owner_id,field) IN (SELECT session_id,owner_kind,owner_id,field FROM content_references WHERE session_id=?1 AND owner_kind='checkpoint' AND owner_id>=?2 AND owner_id<?3 LIMIT 64)",params![session,prefix,format!("{prefix}\u{10ffff}")])?;
                if changed == 0 { tx.execute("DELETE FROM content_operations WHERE operation_id=?1",[operation])?; }
                tx.commit()?; Ok(changed==0)
            })?;
            if complete {
                return Ok(());
            }
        }
    }

    pub(crate) fn recover_content_operations(
        &self,
        cancellation: &crate::service::StartupCancellation,
    ) -> Result<()> {
        let mut after = String::new();
        loop {
            cancellation.check()?;
            let cursor = after.clone();
            let operations = self.db.read(move |db| {
                let mut query = db.prepare("SELECT operation_id FROM content_operations WHERE operation_id>?1 ORDER BY operation_id LIMIT 32")?;
                let rows = query.query_map([cursor],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })?;
            if operations.is_empty() {
                return Ok(());
            }
            for operation in operations {
                cancellation.check()?;
                after = operation.clone();
                // Separate Managers may open this database in one process.
                // Only an abandoned stage may be reclaimed during their open.
                if ActiveOperation::is_active(&operation) {
                    continue;
                }
                self.cleanup_operation_cancellable(&operation, Some(cancellation))?;
                self.db.request_space_reclamation();
            }
        }
    }
}

/// One small idle slice, requested only by explicit reset/prune operations.
/// Readers exclude the retirement marker immediately; no historical scan.
pub(crate) fn reclaim_retired(db: &mut Connection) -> Result<bool> {
    let id: Option<i64> = db
        .query_row(
            "SELECT stream_id FROM retired_streams ORDER BY stream_id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(id) = id else {
        return Ok(true);
    };
    database::try_maintenance_transaction(db, |tx| {
        tx.execute("DELETE FROM stream_change_content WHERE (change_id,field) IN (SELECT b.change_id,b.field FROM stream_change_content b JOIN stream_changes c ON c.id=b.change_id WHERE c.stream_id=?1 LIMIT 64)",[id])?;
        tx.execute("DELETE FROM stream_changes WHERE id IN (SELECT c.id FROM stream_changes c WHERE c.stream_id=?1 AND NOT EXISTS(SELECT 1 FROM stream_change_content b WHERE b.change_id=c.id) LIMIT 64)",[id])?;
        tx.execute("DELETE FROM stream_contexts WHERE id IN (SELECT x.id FROM stream_contexts x WHERE x.stream_id=?1 AND NOT EXISTS(SELECT 1 FROM stream_changes c WHERE c.context_id=x.id) LIMIT 64)",[id])?;
        tx.execute("DELETE FROM stream_body_owners WHERE (stream_id,key_id) IN (SELECT stream_id,key_id FROM stream_body_owners WHERE stream_id=?1 LIMIT 64)",[id])?;
        tx.execute("DELETE FROM stream_symbols WHERE id IN (SELECT s.id FROM stream_symbols s WHERE s.stream_id=?1 AND NOT EXISTS(SELECT 1 FROM stream_contexts WHERE stream_id=?1) AND NOT EXISTS(SELECT 1 FROM stream_body_owners b WHERE b.stream_id=?1 AND b.key_id=s.id) LIMIT 64)",[id])?;
        let remaining:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM stream_changes WHERE stream_id=?1) OR EXISTS(SELECT 1 FROM stream_contexts WHERE stream_id=?1) OR EXISTS(SELECT 1 FROM stream_symbols WHERE stream_id=?1) OR EXISTS(SELECT 1 FROM stream_body_owners WHERE stream_id=?1)",[id],|row|row.get(0))?;
        if !remaining {
            tx.execute("DELETE FROM stream_runs WHERE id=?1", [id])?;
        }
        Ok(())
    })?;
    Ok(
        !db.query_row("SELECT EXISTS(SELECT 1 FROM retired_streams)", [], |row| {
            row.get::<_, bool>(0)
        })?,
    )
}

#[cfg(test)]
#[path = "reset_tests.rs"]
mod tests;
