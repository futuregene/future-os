//! Content-addressed compaction receipts. Checkpoint and successful receipt commit
//! in one SQLite transaction; an abandoned started row is never retried blindly.
use super::{Manager, SessionEntry};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) enum CompactionClaim {
    New { key: String, input: String },
    Cached { value: Value, operation_id: String },
}

pub(crate) fn canonical_hash(value: &Value) -> Result<String> {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let sorted_map = map.iter().collect::<std::collections::BTreeMap<_, _>>();
                Value::Object(
                    sorted_map
                        .into_iter()
                        .map(|(k, v)| (k.clone(), sorted(v)))
                        .collect(),
                )
            }
            Value::Array(values) => Value::Array(values.iter().map(sorted).collect()),
            value => value.clone(),
        }
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&sorted(value))?)
    ))
}

fn input_hash(db: &Connection, session: &str) -> Result<String> {
    let mut statement = db.prepare("SELECT payload FROM entry_records WHERE session_id=?1 AND entry_type IN ('user','assistant','tool','system') ORDER BY position")?;
    let mut rows = statement.query([session])?;
    let mut hash = Sha256::new();
    while let Some(row) = rows.next()? {
        let value: Value = serde_json::from_str(&row.get::<_, String>(0)?)?;
        // Record identity, contents and stable metadata matter; checkpoint,
        // usage, run markers and session-info updates do not change this key.
        hash.update(canonical_hash(&value)?.as_bytes());
    }
    Ok(format!("{:x}", hash.finalize()))
}

impl Manager {
    pub(crate) fn claim_compaction(
        &self,
        session: &str,
        policy: Value,
        operation_id: &str,
    ) -> Result<CompactionClaim> {
        let session = session.to_owned();
        let operation_id = operation_id.to_owned();
        self.storage()?.db.call(move |db| {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND revision>=0)", [&session], |r|r.get(0))?;
            if !exists { bail!("session does not exist"); }
            let input = input_hash(&tx,&session)?;
            let key = canonical_hash(&json!({"input":input,"policy":policy}))?;
            let existing: Option<(String,String,Option<String>)> = tx.query_row(
                "SELECT state,operation_id,result_json FROM compaction_operations WHERE session_id=?1 AND input_key=?2",
                params![session,key], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            ).optional()?;
            if let Some((state,original_operation,value))=existing {
                if state == "completed" {
                    let value:Value=serde_json::from_str(&value.context("missing compaction receipt")?)?;
                    if let Some(id)=value["checkpoint"]["entry_id"].as_str() {
                        let stored:Option<(i64,Option<String>)>=tx.query_row("SELECT position,content_json FROM entries WHERE session_id=?1 AND entry_id=?2 AND entry_type='compaction'",params![session,id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                        let (position,content)=stored.context("compaction cached result is no longer in the journal; refusing to regenerate it silently")?;
                        let cp:crate::compaction::ContextCheckpoint=serde_json::from_value(value["checkpoint"].clone())?;
                        let expected=super::checkpoint_to_entry(&cp).content;
                        let actual:Value=serde_json::from_str(&content.context("missing cached checkpoint body")?)?;
                        if expected.as_ref()!=Some(&actual) { bail!("cached compaction checkpoint was modified"); }
                        let cutoff:Option<i64>=tx.query_row("SELECT position FROM entries WHERE session_id=?1 AND entry_id=?2",params![session,cp.cutoff_entry_id],|r|r.get(0)).optional()?;
                        if !cutoff.is_some_and(|cutoff|cutoff<position) { bail!("cached compaction checkpoint range is invalid"); }
                    }
                    return Ok(CompactionClaim::Cached { value, operation_id:original_operation });
                }
                if state == "failed" {
                    bail!("compaction_previous_failed: {}",value.unwrap_or_default());
                }
                bail!("compaction_indeterminate: operation {original_operation} was started but has no durable result; it may still be running or may have been interrupted. No model call was repeated");
            }
            tx.execute("INSERT INTO compaction_operations(session_id,input_key,input_digest,operation_id,state) VALUES (?1,?2,?3,?4,'started')",params![session,key,input,operation_id])?;
            tx.commit()?;
            Ok(CompactionClaim::New {key,input})
        })
    }

    pub(crate) fn finish_compaction(
        &self,
        session: &str,
        key: &str,
        input: &str,
        entry: Option<SessionEntry>,
        receipt: Value,
    ) -> Result<()> {
        let (session, key, input) = (session.to_owned(), key.to_owned(), input.to_owned());
        self.storage()?.db.call(move |db| {
            let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let state:String=tx.query_row("SELECT state FROM compaction_operations WHERE session_id=?1 AND input_key=?2",params![session,key],|r|r.get(0))?;
            if state == "completed" { return Ok(()); }
            if state != "started" { bail!("compaction receipt is not pending"); }
            if input_hash(&tx,&session)? != input { bail!("compaction_input_changed: original history changed during preparation"); }
            if let Some(entry)=entry {
                tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1",[&session])?;
                super::sqlite_store::insert_entries(&tx,&session,vec![serde_json::to_value(entry)?])?;
            }
            tx.execute("UPDATE compaction_operations SET state='completed',result_json=?3 WHERE session_id=?1 AND input_key=?2",params![session,key,serde_json::to_string(&receipt)?])?;
            tx.commit()?;
            Ok(())
        })
    }

    pub(crate) fn fail_compaction(&self, session: &str, key: &str, error: &str) -> Result<()> {
        let (session, key, error) = (session.to_owned(), key.to_owned(), error.to_owned());
        self.storage()?.db.call(move |db| {
            db.execute("UPDATE compaction_operations SET state='failed',result_json=?3 WHERE session_id=?1 AND input_key=?2 AND state='started'",params![session,key,serde_json::to_string(&json!({"error":error}))?])?;
            Ok(())
        })
    }
}
