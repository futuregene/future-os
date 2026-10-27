//! Canonical context-checkpoint transactions. Admission checks immutable prefix
//! identity; bodies stage off the writer and receipts retain only a scoped
//! checkpoint locator/digest, never a second copy of its summary.
use super::{
    operations::ActiveOperation,
    reader::{Entry, Snapshot},
    stream::Encoding,
    ContentDigest, ContentRange, READ_BYTES, WRITE_BYTES,
};
use crate::session::{
    compaction_ops::{canonical_hash, CompactionClaim},
    database,
    sqlite_store::SqliteStore,
    SessionEntry,
};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    storage_format: u8,
    checkpoint: Option<CheckpointRef>,
    receipt: Value,
}

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointRef {
    entry_id: String,
    sha256: String,
    cutoff_entry_id: Option<String>,
    covered_from_entry_id: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FailureReceipt {
    storage_format: u8,
    owner: String,
    sha256: String,
    bytes: i64,
}
enum Claim {
    Failed {
        receipt: FailureReceipt,
        key: String,
        scope: Snapshot,
    },
    New {
        key: String,
        input: String,
    },
    Cached {
        receipt: Receipt,
        operation: String,
        scope: Snapshot,
        entry: Option<Entry>,
    },
}

/// Every conversation entry is immutable after publication. Four indexed tail
/// lookups plus the explicit rewrite fence identify its entire append-only
/// prefix; metadata, run markers and checkpoints do not change that identity.
pub(crate) fn input_identity(db: &Connection, session: &str) -> Result<String> {
    let generation = super::checkpoint_retirement::generation(db, session)?;
    let stored_session = crate::session::identity::session_key(db, session)?;
    let position: Option<i64> = db.query_row(
        "SELECT max(position) FROM (
        SELECT max(position) AS position FROM entries WHERE session_id=?1 AND entry_type='user'
        UNION ALL SELECT max(position) FROM entries WHERE session_id=?1 AND entry_type='assistant'
        UNION ALL SELECT max(position) FROM entries WHERE session_id=?1 AND entry_type='tool'
        UNION ALL SELECT max(position) FROM entries WHERE session_id=?1 AND entry_type='system')",
        [&stored_session],
        |row| row.get(0),
    )?;
    let tail=position.map(|position|->Result<_> {
        let (id,digest):(crate::session::identity::BoundIdentity,Option<String>)=db.query_row("SELECT e.entry_id,f.digest FROM entries e LEFT JOIN entry_fingerprints f ON f.session_id=e.session_id AND f.entry_id=e.entry_id WHERE e.session_id=?1 AND e.position=?2",params![stored_session,position],|row|Ok((row.get(0)?,row.get(1)?)))?;
        Ok((position,id,digest.context("canonical input entry has no immutable identity")?))
    }).transpose()?;
    canonical_hash(&json!(["canonical-prefix-v1", session, generation, tail]))
}

fn receipt_entry(db: &Connection, session: &str, checkpoint: &CheckpointRef) -> Result<Entry> {
    let entry = crate::session::identity::BoundIdentity::entry(db, session, &checkpoint.entry_id)?;
    let stored_session = crate::session::identity::session_key(db, session)?;
    let cutoff_key = checkpoint
        .cutoff_entry_id
        .as_deref()
        .map(|id| crate::session::identity::BoundIdentity::entry(db, session, id))
        .transpose()?;
    let position:Option<i64>=db.query_row("SELECT position FROM entries WHERE session_id=?1 AND entry_id=?2 AND entry_type='compaction'",params![stored_session,entry],|row|row.get(0)).optional()?;
    let position = position.context(
        "compaction cached result is no longer in the journal; refusing to regenerate it silently",
    )?;
    let cutoff: Option<i64> = db
        .query_row(
            "SELECT position FROM entries WHERE session_id=?1 AND entry_id=?2",
            params![stored_session, cutoff_key],
            |row| row.get(0),
        )
        .optional()?;
    ensure!(
        cutoff.is_some_and(|cutoff| cutoff < position),
        "cached compaction checkpoint range is invalid"
    );
    if let Some(from) = &checkpoint.covered_from_entry_id {
        let from = crate::session::identity::BoundIdentity::entry(db, session, from)?;
        let first: Option<i64> = db
            .query_row(
                "SELECT position FROM entries WHERE session_id=?1 AND entry_id=?2",
                params![stored_session, from],
                |row| row.get(0),
            )
            .optional()?;
        ensure!(
            first.is_some_and(|first| Some(first) <= cutoff),
            "cached compaction checkpoint start is invalid"
        );
    }
    Ok(Entry {
        position,
        id: entry,
    })
}

impl SqliteStore {
    pub(crate) fn fail_segmented_checkpoint(
        &self,
        session: &str,
        key: &str,
        error: &str,
    ) -> Result<()> {
        let operation = uuid::Uuid::new_v4().to_string();
        let prefix = format!("checkpoint:{operation}:");
        let _active = ActiveOperation::claim(&operation);
        let (s, k, op, p) = (
            session.to_owned(),
            key.to_owned(),
            operation.clone(),
            prefix.clone(),
        );
        let pending=self.db.call(move |db| {
            let tx=database::begin_write_transaction(db)?;
            let s=crate::session::identity::BoundIdentity::session(&tx,&s)?;
            let state:Option<String>=tx.query_row("SELECT state FROM compaction_operations WHERE session_id=?1 AND input_key=?2",params![s,k],|row|row.get(0)).optional()?;
            if state.as_deref()!=Some("started") {return Ok(false);}
            if !super::checkpoint_retirement::is_current(&tx,&s,&k)? {return Ok(false);}
            let revision:i64=tx.query_row("SELECT revision FROM sessions WHERE id=?1 AND revision>=0",[&s],|row|row.get(0))?;
            let reset:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM content_operations WHERE session_id=?1 AND kind='reset')",[&s],|row|row.get(0))?;
            ensure!(!reset,"session history reset is in progress");
            tx.execute("INSERT INTO content_operations(operation_id,session_id,owner_prefix,expected_revision,kind) VALUES (?1,?2,?3,?4,'checkpoint')",params![op,s,p,revision])?;
            tx.commit()?;Ok(true)
        })?;
        if !pending {
            return Ok(());
        }
        let result = (|| -> Result<()> {
            let range = super::entries::stage_named(
                self,
                session,
                "checkpoint",
                &format!("{prefix}failure"),
                "/error",
                error,
                false,
            )?;
            let mut digest = ContentDigest::default();
            digest.push(error)?;
            let owner = format!("compaction-error:{key}");
            let receipt = serde_json::to_string(&FailureReceipt {
                storage_format: 2,
                owner: owner.clone(),
                sha256: digest.finish(),
                bytes: range.length,
            })?;
            let (s, k, op) = (session.to_owned(), key.to_owned(), operation.clone());
            self.db.call(move |db| {
                let tx=database::begin_write_transaction(db)?;
            let s=crate::session::identity::BoundIdentity::session(&tx,&s)?;
                let state:Option<String>=tx.query_row("SELECT state FROM compaction_operations WHERE session_id=?1 AND input_key=?2",params![s,k],|row|row.get(0)).optional()?;
                if state.as_deref()!=Some("started") {return Ok(());}
                ensure!(super::checkpoint_retirement::is_current(&tx,&s,&k)?,"checkpoint history changed while recording its failure");
                let owned:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM content_operations WHERE operation_id=?1 AND session_id=?2 AND kind='checkpoint')",params![op,s],|row|row.get(0))?;
                ensure!(owned,"checkpoint failure staging receipt is missing");
                super::bind_encoded(&tx,&s,"tool",&owner,"/error",range,Encoding::Text)?;
                tx.execute("UPDATE compaction_operations SET state='failed',result_json=?3 WHERE session_id=?1 AND input_key=?2 AND state='started'",params![s,k,receipt])?;
                tx.commit()?;Ok(())
            })
        })();
        if let Err(error) = self.cleanup_content_operation(&operation) {
            tracing::warn!(%error,operation,"checkpoint failure cleanup deferred until startup");
        }
        self.db.request_space_reclamation();
        result
    }

    fn checkpoint_failure_text(
        &self,
        scope: &Snapshot,
        key: &str,
        receipt: &FailureReceipt,
    ) -> Result<String> {
        let mut offset = 0i64;
        let mut text = String::new();
        let mut digest = ContentDigest::default();
        loop {
            let (check, key, owner, bytes) = (
                scope.clone(),
                key.to_owned(),
                receipt.owner.clone(),
                receipt.bytes,
            );
            let page=self.db.read(move |db| {
                let tx=db.transaction()?;check.check(&tx)?;
                let live:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM compaction_operations WHERE session_id=?1 AND input_key=?2 AND state='failed')",params![check.session_key(),key],|row|row.get(0))?;
                ensure!(live,"checkpoint failure receipt changed during read");
                let range:Option<(i64,i64,i64,String)>=tx.query_row("SELECT content_id,byte_offset,byte_length,encoding FROM content_references WHERE session_id=?1 AND owner_kind='tool' AND owner_id=?2 AND field='/error'",params![check.session_key(),owner],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
                let (id,start,length,encoding)=range.context("checkpoint failure body is missing")?;
                ensure!(start==0 && length==bytes && encoding=="text" && offset<=length,"checkpoint failure range changed");
                let page=super::read(&tx,id,offset,usize::try_from(length-offset)?.min(READ_BYTES))?;
                tx.commit()?;Ok(page)
            })?;
            if offset == receipt.bytes {
                ensure!(page.text.is_empty(), "invalid empty failure range");
                break;
            }
            ensure!(
                page.next_offset > offset && page.next_offset <= receipt.bytes,
                "checkpoint failure read made invalid progress"
            );
            offset = page.next_offset;
            digest.push(&page.text)?;
            text.try_reserve(page.text.len())?;
            text.push_str(&page.text);
            if offset == receipt.bytes {
                break;
            }
        }
        ensure!(
            digest.bytes() == receipt.bytes && digest.finish() == receipt.sha256,
            "checkpoint failure body digest mismatch"
        );
        scope.verify(self)?;
        Ok(text)
    }

    pub(crate) fn claim_segmented_checkpoint(
        &self,
        session: &str,
        policy: String,
        operation: &str,
    ) -> Result<CompactionClaim> {
        ensure!(
            operation.len() <= WRITE_BYTES,
            "checkpoint operation identity exceeds its metadata budget"
        );
        let (session, operation) = (session.to_owned(), operation.to_owned());
        let claim=self.db.call(move |db| {
            let tx=database::begin_write_transaction(db)?;
            let session=crate::session::identity::BoundIdentity::session(&tx,&session)?;
            let scope=Snapshot::capture_from(&tx,&session)?;
            let input=input_identity(&tx,&session)?;
            let key=canonical_hash(&json!(["canonical-checkpoint-v1",input,policy]))?;
            let existing:Option<(String,String,Option<String>)>=tx.query_row("SELECT state,operation_id,result_json FROM compaction_operations WHERE session_id=?1 AND input_key=?2",params![session,key],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
            if let Some((state,original,raw))=existing {
                ensure!(super::checkpoint_retirement::is_current(&tx,&session,&key)?,"canonical checkpoint history changed");
                if state=="completed" {
                    let raw=raw.context("missing compaction receipt")?;
                    ensure!(raw.len()<=WRITE_BYTES,"canonical checkpoint receipt exceeds its metadata budget");
                    let receipt:Receipt=serde_json::from_str(&raw).context("invalid canonical checkpoint receipt; preserve it")?;
                    ensure!(receipt.storage_format==2,"unsupported checkpoint receipt format");
                    let entry=receipt.checkpoint.as_ref().map(|checkpoint|receipt_entry(&tx,&session,checkpoint)).transpose()?;
                    tx.commit()?;return Ok(Claim::Cached {receipt,operation:original,scope,entry});
                }
                if state=="failed" {
                    let raw=raw.context("missing checkpoint failure receipt")?;
                    ensure!(raw.len()<=WRITE_BYTES,"checkpoint failure receipt exceeds metadata budget");
                    let receipt:FailureReceipt=serde_json::from_str(&raw).context("invalid checkpoint failure receipt")?;
                    ensure!(receipt.storage_format==2 && receipt.owner==format!("compaction-error:{key}"),"checkpoint failure ownership mismatch");
                    tx.commit()?;return Ok(Claim::Failed {receipt,key,scope});
                }
                anyhow::bail!("compaction_indeterminate: operation {original} was started but has no durable result; it may still be running or may have been interrupted. No model call was repeated");
            }
            tx.execute("INSERT INTO compaction_operations(session_id,input_key,input_digest,operation_id,state) VALUES (?1,?2,?3,?4,'started')",params![session,key,input,operation])?;
            super::checkpoint_retirement::record(&tx,&session,&key)?;
            tx.commit()?;Ok(Claim::New {key,input})
        })?;
        match claim {
            Claim::Failed {
                receipt,
                key,
                scope,
            } => {
                let error = self.checkpoint_failure_text(&scope, &key, &receipt)?;
                anyhow::bail!("compaction_previous_failed: {error}");
            }
            Claim::New { key, input } => Ok(CompactionClaim::New { key, input }),
            Claim::Cached {
                mut receipt,
                operation,
                scope,
                entry,
            } => {
                let checkpoint =
                    if let (Some(reference), Some(entry)) = (&receipt.checkpoint, entry) {
                        let value = scope.entry(self, &entry)?;
                        ensure!(
                            canonical_hash(&value)? == reference.sha256,
                            "cached compaction checkpoint was modified"
                        );
                        let entry: SessionEntry = serde_json::from_value(value)?;
                        let checkpoint = crate::session::entry_to_checkpoint(&entry)
                            .context("cached checkpoint is not a supported context checkpoint")?;
                        serde_json::to_value(checkpoint)?
                    } else {
                        Value::Null
                    };
                receipt.receipt["checkpoint"] = checkpoint;
                scope.verify(self)?;
                Ok(CompactionClaim::Cached {
                    value: receipt.receipt,
                    operation_id: operation,
                })
            }
        }
    }

    pub(crate) fn finish_segmented_checkpoint(
        &self,
        session: &str,
        key: &str,
        input: &str,
        entry: Option<SessionEntry>,
        mut receipt: Value,
    ) -> Result<()> {
        let supplied = receipt
            .as_object_mut()
            .context("checkpoint receipt is not an object")?
            .remove("checkpoint")
            .unwrap_or(Value::Null);
        let (checkpoint, body) = if let Some(entry) = entry {
            let (id, cutoff, from) = {
                let parsed = crate::session::entry_to_checkpoint(&entry)
                    .context("checkpoint entry does not match the supported context schema")?;
                if !supplied.is_null() {
                    ensure!(
                        serde_json::to_value(&parsed)? == supplied,
                        "checkpoint receipt does not match its entry"
                    );
                }
                (
                    parsed.entry_id,
                    parsed.cutoff_entry_id,
                    parsed.covered_from_entry_id,
                )
            };
            let value = serde_json::to_value(entry)?;
            let reference = CheckpointRef {
                entry_id: id,
                sha256: canonical_hash(&value)?,
                cutoff_entry_id: cutoff,
                covered_from_entry_id: from,
            };
            (Some(reference), Some(value))
        } else {
            ensure!(
                supplied.is_null(),
                "checkpoint receipt has no durable entry"
            );
            (None, None)
        };
        let receipt = serde_json::to_string(&Receipt {
            storage_format: 2,
            checkpoint,
            receipt,
        })?;
        ensure!(
            receipt.len() <= WRITE_BYTES,
            "checkpoint receipt exceeds its metadata budget"
        );
        let operation = uuid::Uuid::new_v4().to_string();
        let _active = ActiveOperation::claim(&operation);
        let prefix = format!("checkpoint:{operation}:");
        let (s, k, i, op, p, expected) = (
            session.to_owned(),
            key.to_owned(),
            input.to_owned(),
            operation.clone(),
            prefix.clone(),
            receipt.clone(),
        );
        let pending=self.db.call(move |db| {
            let tx=database::begin_write_transaction(db)?;
            let s=crate::session::identity::BoundIdentity::session(&tx,&s)?;
            let (state,stored,raw):(String,String,Option<String>)=tx.query_row("SELECT state,input_digest,result_json FROM compaction_operations WHERE session_id=?1 AND input_key=?2",params![s,k],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
            ensure!(stored==i,"checkpoint input identity mismatch");
            if state=="completed" {ensure!(raw.as_deref()==Some(&expected),"conflicting checkpoint completion receipt");return Ok(false);}
            ensure!(state=="started","compaction receipt is not pending");
            ensure!(input_identity(&tx,&s)?==i,"compaction_input_changed: original history changed during preparation");
            let reset:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM content_operations WHERE session_id=?1 AND kind='reset')",[&s],|row|row.get(0))?;
            ensure!(!reset,"session history reset is in progress");
            let revision:i64=tx.query_row("SELECT revision FROM sessions WHERE id=?1 AND revision>=0",[&s],|row|row.get(0))?;
            tx.execute("INSERT INTO content_operations(operation_id,session_id,owner_prefix,expected_revision,kind) VALUES (?1,?2,?3,?4,'checkpoint')",params![op,s,p,revision])?;
            tx.commit()?;Ok(true)
        })?;
        if !pending {
            return Ok(());
        }
        let result = (|| -> Result<()> {
            let headers =
                super::entries::prepare_staged(self, session, body.into_iter().collect(), &prefix)?;
            ensure!(
                serde_json::to_vec(&headers)?.len() <= WRITE_BYTES,
                "checkpoint header exceeds its metadata budget"
            );
            let (s, k, i, op, p) = (
                session.to_owned(),
                key.to_owned(),
                input.to_owned(),
                operation.clone(),
                prefix.clone(),
            );
            self.db.call(move |db| {
                let tx=database::begin_write_transaction(db)?;
            let s=crate::session::identity::BoundIdentity::session(&tx,&s)?;
                let state:String=tx.query_row("SELECT state FROM compaction_operations WHERE session_id=?1 AND input_key=?2 AND input_digest=?3",params![s,k,i],|row|row.get(0))?;
                if state=="completed" {
                    let raw:Option<String>=tx.query_row("SELECT result_json FROM compaction_operations WHERE session_id=?1 AND input_key=?2",params![s,k],|row|row.get(0))?;
                    ensure!(raw.as_deref()==Some(&receipt),"conflicting checkpoint completion receipt");return Ok(());
                }
                ensure!(state=="started","compaction receipt is not pending");
                ensure!(input_identity(&tx,&s)?==i,"compaction_input_changed: original history changed during preparation");
                let owned:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM content_operations WHERE operation_id=?1 AND session_id=?2 AND kind='checkpoint')",params![op,s],|row|row.get(0))?;
                ensure!(owned,"checkpoint staging receipt is missing");
                for entry in &headers {
                    super::entries::verify_boundary_retry(&tx,&s,&p,entry)?;
                    let id=entry["id"].as_str().context("checkpoint entry identity missing")?;
                    let mut query=tx.prepare("SELECT field,content_id,byte_offset,byte_length,encoding FROM content_references WHERE session_id=?1 AND owner_kind='checkpoint' AND owner_id=?2 ORDER BY field LIMIT 129")?;
                    let rows=query.query_map(params![s,format!("{p}{id}")],|row|Ok((row.get::<_,String>(0)?,ContentRange{id:row.get(1)?,offset:row.get(2)?,length:row.get(3)?},row.get::<_,String>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                    ensure!(rows.len()<=128 && rows.iter().map(|(field,_,encoding)|field.len()+encoding.len()+64).sum::<usize>()<=WRITE_BYTES,"checkpoint references exceed publication budget");
                    for (field,range,encoding) in rows {super::bind_encoded(&tx,&s,"entry",id,&field,range,Encoding::parse(&encoding)?)?;}
                }
                if !headers.is_empty() {
                    crate::session::sqlite_store::insert_transcript_entries(&tx,&s,headers)?;
                    tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1",[&s])?;
                }
                // Validate the covered range in the same transaction as its
                // receipt; body/digest verification stays outside the writer.
                let decoded:Receipt=serde_json::from_str(&receipt)?;
                if let Some(checkpoint)=&decoded.checkpoint {receipt_entry(&tx,&s,checkpoint)?;}
                tx.execute("UPDATE compaction_operations SET state='completed',result_json=?3 WHERE session_id=?1 AND input_key=?2",params![s,k,receipt])?;
                tx.commit()?;Ok(())
            })
        })();
        if let Err(error) = self.cleanup_content_operation(&operation) {
            tracing::warn!(%error,operation,"checkpoint staging cleanup deferred until startup");
        }
        self.db.request_space_reclamation();
        result
    }
}
