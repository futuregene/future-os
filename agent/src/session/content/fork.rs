//! Hidden fork ownership. Copy unchanged field ranges in bounded transactions;
//! changed checkpoint identities are prepared separately from shared bodies.
use super::{
    operations::ActiveOperation, reader::Snapshot, stream::Encoding, ContentRange, READ_BYTES,
};
use crate::session::{database, sqlite_store::SqliteStore, ForkRequest};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

pub(crate) fn active_key(child: &str) -> String {
    format!("fork:{child}")
}

impl SqliteStore {
    pub(crate) fn pin_fork_fields(
        &self,
        scope: &Snapshot,
        child: &str,
        source: &Value,
        target: &Value,
    ) -> Result<()> {
        let source_id = source["id"]
            .as_str()
            .context("fork source identity missing")?;
        let target_id = target["id"]
            .as_str()
            .context("fork target identity missing")?;
        let mut after = String::new();
        loop {
            let (check, owner, cursor) = (scope.clone(), source_id.to_owned(), after.clone());
            let page=self.db.read(move |db| {
                let tx=db.transaction()?;check.check(&tx)?;
                let session=crate::session::identity::BoundIdentity::session(&tx,check.session_id())?;
                let owner=crate::session::identity::BoundIdentity::entry(&tx,check.session_id(),&owner)?;
                let source:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM entries WHERE session_id=?1 AND entry_id=?2 AND position<=?3)",params![session,owner,check.end_position()],|row|row.get(0))?;
                ensure!(source,"fork source is outside its captured transcript");
                let mut query=tx.prepare("SELECT r.field,r.content_id,r.byte_offset,r.byte_length,r.encoding FROM content_references r JOIN entries e ON e.session_id=r.session_id AND e.entry_id=r.owner_id WHERE r.session_id=?1 AND r.owner_kind='entry' AND r.owner_id=?2 AND r.field>?3 ORDER BY r.field LIMIT 64")?;
                let rows=query.query_map(params![session,owner,cursor],|row|Ok((row.get::<_,String>(0)?,ContentRange{id:row.get(1)?,offset:row.get(2)?,length:row.get(3)?},row.get::<_,String>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                ensure!(rows.iter().map(|(field,_,encoding)|field.len()+encoding.len()+64).sum::<usize>()<=READ_BYTES,"fork field identity page exceeds budget");
                drop(query);tx.commit()?;Ok(rows)
            })?;
            if page.is_empty() {
                break;
            }
            after = page.last().expect("nonempty reference page").0.clone();
            let links = page
                .into_iter()
                .filter(|(field, _, _)| {
                    source
                        .pointer(field)
                        .is_some_and(|value| Some(value) == target.pointer(field))
                })
                .collect::<Vec<_>>();
            if links.is_empty() {
                continue;
            }
            let (scope, child, target) = (scope.clone(), child.to_owned(), target_id.to_owned());
            self.db.call(move |db| {
                let tx=database::begin_write_transaction(db)?;scope.check(&tx)?;
                let child=crate::session::identity::BoundIdentity::session(&tx,&child)?;
                let hidden:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM fork_operations f JOIN sessions s ON s.id=f.child_session_id WHERE f.child_session_id=?1 AND f.parent_session_id=?2 AND s.revision=-1)",params![child,scope.session_key()],|row|row.get(0))?;
                ensure!(hidden,"fork staging receipt changed");
                for (field,range,encoding) in links {
                    super::bind_encoded(&tx,&child,"entry",&target,&field,range,Encoding::parse(&encoding)?)?;
                }
                tx.commit()?;Ok(())
            })?;
        }
        Ok(())
    }

    pub(crate) fn discard_hidden_fork(&self, child: &str) -> Result<()> {
        self.discard_hidden_fork_cancellable(child, None)
    }

    fn discard_hidden_fork_cancellable(
        &self,
        child: &str,
        cancellation: Option<&crate::service::StartupCancellation>,
    ) -> Result<()> {
        let id = child.to_owned();
        let receipt=self.db.read(move |db| {
            let id=crate::session::identity::BoundIdentity::session(db,&id)?;
            let raw:Option<(String,crate::session::identity::BoundIdentity,String)>=db.query_row("SELECT f.request_id,f.parent_session_id,f.request_fingerprint FROM fork_operations f JOIN sessions s ON s.id=f.child_session_id JOIN session_formats v ON v.session_id=s.id WHERE f.child_session_id=?1 AND s.revision=-1 AND v.format=2",[id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
            Ok(raw)
        })?;
        let Some((request, parent, fingerprint)) = receipt else {
            return Ok(());
        };
        let parsed: ForkRequest = serde_json::from_str(&fingerprint)
            .context("invalid hidden fork receipt; preserve staging")?;
        ensure!(
            parsed.request_id == request && parent == parsed.parent_session_id,
            "invalid hidden fork ownership; preserve staging"
        );
        loop {
            if let Some(cancellation) = cancellation {
                cancellation.check()?;
            }
            let (id, request, fingerprint) =
                (child.to_owned(), request.clone(), fingerprint.clone());
            let done=self.db.call(move |db| {
                let tx=database::begin_write_transaction(db)?;
                let id=crate::session::identity::BoundIdentity::session(&tx,&id)?;
                let revision:Option<i64>=tx.query_row("SELECT revision FROM sessions WHERE id=?1",[&id],|row|row.get(0)).optional()?;
                if revision!=Some(-1) {return Ok(true);}
                let owned:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM fork_operations f JOIN session_formats v ON v.session_id=f.child_session_id WHERE f.child_session_id=?1 AND f.request_id=?2 AND f.request_fingerprint=?3 AND v.format=2)",params![id,request,fingerprint],|row|row.get(0))?;
                ensure!(owned,"hidden fork receipt changed during cleanup");
                let refs=tx.execute("DELETE FROM content_references WHERE (session_id,owner_kind,owner_id,field) IN (SELECT session_id,owner_kind,owner_id,field FROM content_references WHERE session_id=?1 LIMIT 64)",[&id])?;
                if refs>0 {tx.commit()?;return Ok(false);}
                let position:Option<i64>=tx.query_row("SELECT min(position) FROM entries WHERE session_id=?1",[&id],|row|row.get(0))?;
                if let Some(position)=position {
                    let blocks=tx.execute("DELETE FROM message_blocks WHERE session_id=?1 AND entry_position=?2 AND ordinal IN (SELECT ordinal FROM message_blocks WHERE session_id=?1 AND entry_position=?2 ORDER BY ordinal LIMIT 16)",params![id,position])?;
                    if blocks==0 {tx.execute("DELETE FROM entries WHERE session_id=?1 AND position=?2",params![id,position])?;}
                    tx.commit()?;return Ok(false);
                }
                let display=tx.execute("DELETE FROM history_display WHERE session_id=?1 AND ordinal IN (SELECT ordinal FROM history_display WHERE session_id=?1 LIMIT 64)",[&id])?;
                if display>0 {tx.commit()?;return Ok(false);}
                let runs=tx.execute("DELETE FROM runs WHERE session_id=?1 AND run_id IN (SELECT run_id FROM runs WHERE session_id=?1 LIMIT 64)",[&id])?;
                if runs>0 {tx.commit()?;return Ok(false);}
                let unexpected:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM stream_runs WHERE session_id=?1) OR EXISTS(SELECT 1 FROM run_events WHERE session_id=?1) OR EXISTS(SELECT 1 FROM run_snapshots WHERE session_id=?1) OR EXISTS(SELECT 1 FROM queued_run_receipts WHERE session_id=?1)",[&id],|row|row.get(0))?;
                ensure!(!unexpected,"hidden fork has unexpected execution ownership; preserve it");
                tx.execute("DELETE FROM sessions WHERE id=?1 AND revision=-1",[&id])?;
                tx.commit()?;Ok(true)
            })?;
            if let Some(cancellation) = cancellation {
                cancellation.record_progress();
            }
            if done {
                self.db.request_space_reclamation();
                return Ok(());
            }
        }
    }

    pub(crate) fn recover_hidden_forks(
        &self,
        cancellation: &crate::service::StartupCancellation,
    ) -> Result<()> {
        let mut after = crate::session::identity::BoundIdentity::from_stored(
            crate::session::identity::StoredIdentity::Legacy(String::new()),
        );
        loop {
            cancellation.check()?;
            let cursor = after.clone();
            let children=self.db.read(move |db| {
                let mut query=db.prepare("SELECT s.id FROM sessions s JOIN fork_operations f ON f.child_session_id=s.id JOIN session_formats v ON v.session_id=s.id WHERE s.revision=-1 AND v.format=2 AND s.id>?1 ORDER BY s.id LIMIT 32")?;
                let rows=query.query_map([cursor],|row|row.get::<_,crate::session::identity::BoundIdentity>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok(rows)
            })?;
            if children.is_empty() {
                return Ok(());
            }
            for child in children {
                after = child.clone();
                if !ActiveOperation::is_active(&active_key(&child)) {
                    self.discard_hidden_fork_cancellable(&child, Some(cancellation))
                        .with_context(|| format!("recover hidden fork session={child}"))?;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "fork_tests.rs"]
mod tests;
