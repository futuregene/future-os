//! Patch-only session control metadata. Large changed values stage outside the
//! writer; publication merges the current thin header and moves scoped owners
//! in one transaction. Unchanged bodies are never read or serialized again.
use super::{operations::ActiveOperation, stream::Encoding, ContentRange};
use crate::session::{database, sqlite_store::SqliteStore, SessionEntry};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{Map, Value};
use std::collections::HashSet;

// Metadata admission budgets, independent from the size of referenced values.
const MAX_FIELDS: usize = 128;
const HEADER_BYTES: usize = super::WRITE_BYTES;
const CONTROL_VALUE_BYTES: usize = 4096;

pub(super) fn inline_control(key: &str) -> bool {
    matches!(
        key,
        "cwd"
            | "model"
            | "session_name"
            | "parent_session_id"
            | "thinking_level"
            | "created_by"
            | "creator_id"
    )
}

pub(super) fn validate_input(content: &Value) -> Result<()> {
    if content.is_null() {
        return Ok(());
    }
    let object = content
        .as_object()
        .context("session metadata must be an object")?;
    validate_object(object)
}

fn validate_object(object: &Map<String, Value>) -> Result<()> {
    ensure!(
        object.len() <= MAX_FIELDS,
        "session metadata exceeds the {MAX_FIELDS}-field header budget"
    );
    for (key, value) in object {
        if inline_control(key) {
            ensure!(
                value.is_null() || value.is_string(),
                "session control field {key} must be a string or null"
            );
            ensure!(
                value
                    .as_str()
                    .is_none_or(|value| value.len() <= CONTROL_VALUE_BYTES),
                "session control field {key} exceeds its inline budget"
            );
        }
    }
    Ok(())
}

pub(crate) fn validate_thin(content: &Value) -> Result<()> {
    validate_input(content)?;
    ensure!(
        serde_json::to_vec(content)?.len() <= HEADER_BYTES,
        "session metadata header exceeds its {HEADER_BYTES}-byte budget"
    );
    Ok(())
}

pub(crate) fn validate_model(model: &str) -> Result<()> {
    ensure!(
        model.len() <= CONTROL_VALUE_BYTES,
        "session model exceeds its inline budget"
    );
    Ok(())
}

pub(super) fn generation(db: &Connection, session: &str) -> Result<i64> {
    Ok(db.query_row("SELECT coalesce((SELECT metadata_generation FROM content_history_versions WHERE session_id=?1),0)",[session],|row|row.get(0))?)
}

pub(crate) fn advance_generation(db: &Connection, session: &str) -> Result<()> {
    db.execute("INSERT INTO content_history_versions(session_id,generation,metadata_generation) VALUES (?1,0,1) ON CONFLICT(session_id) DO UPDATE SET metadata_generation=metadata_generation+1",[session])?;
    Ok(())
}

pub(crate) fn run_fields(value: &Value) -> Result<Map<String, Value>> {
    let fields = value.as_object().context("run metadata is not an object")?;
    Ok(fields
        .iter()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                "tokens_in"
                    | "tokens_out"
                    | "tokens_cache_r"
                    | "tokens_cache_w"
                    | "last_prompt_tokens"
                    | "total_cost"
                    | "cost_split"
                    | "cost_split_complete"
                    | "session_name"
            )
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

impl SqliteStore {
    pub(crate) fn patch_segmented_metadata(
        &self,
        session: &str,
        fields: Map<String, Value>,
    ) -> Result<()> {
        self.publish_segmented_metadata(session, fields, Vec::new(), false)
    }

    pub(crate) fn commit_segmented_run_metadata(
        &self,
        session: &str,
        fields: Map<String, Value>,
        terminals: Vec<Value>,
    ) -> Result<()> {
        self.publish_segmented_metadata(session, fields, terminals, true)
    }

    fn publish_segmented_metadata(
        &self,
        session: &str,
        fields: Map<String, Value>,
        following: Vec<Value>,
        run_boundary: bool,
    ) -> Result<()> {
        validate_object(&fields)?;
        ensure!(
            following.len() <= 8
                && following
                    .iter()
                    .all(|entry| entry["type"] != "session_info"),
            "run boundary exceeds its header budget"
        );
        let operation = uuid::Uuid::new_v4().to_string();
        let _active = ActiveOperation::claim(&operation);
        let prefix = format!("metadata:{operation}:");
        let (s, op, p) = (session.to_owned(), operation.clone(), prefix.clone());
        self.db.call(move |db| {
            let tx=database::begin_write_transaction(db)?;
            let revision:Option<i64>=tx.query_row("SELECT revision FROM sessions WHERE id=?1 AND revision>=0",[&s],|row|row.get(0)).optional()?;
            let revision=revision.context("session not found: metadata update requires a live session")?;
            let resetting:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM content_operations WHERE session_id=?1 AND kind='reset')",[&s],|row|row.get(0))?;
            ensure!(!resetting,"session history reset is in progress");
            tx.execute("INSERT INTO content_operations(operation_id,session_id,owner_prefix,expected_revision,kind) VALUES (?1,?2,?3,?4,'metadata')",params![op,s,p,revision])?;
            tx.commit()?;Ok(())
        })?;
        let result = (|| -> Result<()> {
            let entry =
                SessionEntry::session_info(Value::Object(fields), String::new(), String::new());
            let mut prepared = super::entries::prepare_staged(
                self,
                session,
                vec![serde_json::to_value(entry)?],
                &prefix,
            )?;
            let mut entry = prepared.pop().context("metadata stage has no entry")?;
            let following = super::entries::prepare_staged(self, session, following, &prefix)?;
            ensure!(
                serde_json::to_vec(&following)?.len() <= HEADER_BYTES,
                "run boundary exceeds its header byte budget"
            );
            let (s, op, p) = (session.to_owned(), operation.clone(), prefix.clone());
            self.db.call(move |db| {
                let tx=database::begin_write_transaction(db)?;
                let receipt:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM content_operations o JOIN sessions s ON s.id=o.session_id WHERE o.operation_id=?1 AND o.session_id=?2 AND o.kind='metadata' AND s.revision>=0)",params![op,s],|row|row.get(0))?;
                ensure!(receipt,"metadata staging owner is no longer live");
                if run_boundary && !following.is_empty() && following.iter().all(|entry|entry["type"]=="run_terminal") {
                    let mut complete=true;
                    for terminal in &following {
                        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM entries WHERE session_id=?1 AND entry_id=?2 AND entry_type='run_terminal')",params![s,terminal["id"].as_str()],|row|row.get(0))?;
                        complete &= exists;
                    }
                    if complete {
                        for terminal in &following {super::entries::verify_boundary_retry(&tx,&s,&p,terminal)?;}
                        // Reuse the immutable-header identity validation but do
                        // not apply an older run's counters over a newer run.
                        crate::session::sqlite_store::insert_transcript_entries(&tx,&s,following.clone())?;
                        for terminal in &following {super::terminal::publish(&tx,&s,terminal)?;}
                        tx.commit()?;return Ok(());
                    }
                }
                let bytes:Option<i64>=tx.query_row("SELECT length(CAST(current_metadata_json AS BLOB)) FROM sessions WHERE id=?1",[&s],|row|row.get(0))?;
                ensure!(bytes.is_none_or(|bytes|bytes<=HEADER_BYTES as i64),"current session metadata exceeds its thin-header budget");
                let raw:Option<String>=tx.query_row("SELECT current_metadata_json FROM sessions WHERE id=?1",[&s],|row|row.get(0))?;
                let mut current=match raw {
                    None=>Map::new(),
                    Some(raw)=>serde_json::from_str::<Value>(&raw)?.as_object().cloned().context("session has no session_info object")?,
                };
                let patch=entry["content"].as_object_mut().context("metadata patch is not an object")?;
                if run_boundary && current.get("session_name").and_then(Value::as_str).is_some_and(|name|!name.is_empty()) {patch.remove("session_name");}
                let updated:HashSet<String>=patch.keys().cloned().collect();
                current.append(patch);
                entry["content"]=Value::Object(current);
                validate_thin(&entry["content"])?;
                let id=entry["id"].as_str().context("metadata entry has no identity")?;
                // Merge against the CURRENT metadata owner under the write
                // lock, so two independent concurrent patches preserve each
                // other's committed fields, including referenced extensions.
                let old= {
                    let mut query=tx.prepare("SELECT r.field,r.content_id,r.byte_offset,r.byte_length,r.encoding FROM content_references r WHERE r.session_id=?1 AND r.owner_kind='entry' AND r.owner_id=(SELECT entry_id FROM entries WHERE session_id=?1 AND entry_type='session_info' LIMIT 1) AND r.field LIKE '/content/%' LIMIT ?2")?;
                    let rows=query.query_map(params![s,(MAX_FIELDS+1) as i64],|row|Ok((row.get::<_,String>(0)?,ContentRange {id:row.get(1)?,offset:row.get(2)?,length:row.get(3)?},row.get::<_,String>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                    rows
                };
                ensure!(old.len()<=MAX_FIELDS,"metadata references exceed the field budget");
                for (field,range,encoding) in old {
                    let key=root_field(&field).context("invalid session metadata reference field")?;
                    if !updated.contains(&key) {
                        ensure!(entry.pointer(&field).is_some(),"retained metadata reference has no owning field");
                        super::bind_encoded(&tx,&s,"entry",id,&field,range,Encoding::parse(&encoding)?)?;
                    }
                }
                let mut headers=vec![entry];headers.extend(following);
                let ids:HashSet<&str>=headers.iter().filter_map(|entry|entry["id"].as_str()).collect();
                let staged= {
                    let mut query=tx.prepare("SELECT owner_id,field,content_id,byte_offset,byte_length,encoding FROM content_references WHERE session_id=?1 AND owner_kind='checkpoint' AND owner_id>=?2 AND owner_id<?3 LIMIT ?4")?;
                    let rows=query.query_map(params![s,p,format!("{p}\u{10ffff}"),(MAX_FIELDS+1) as i64],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,ContentRange {id:row.get(2)?,offset:row.get(3)?,length:row.get(4)?},row.get::<_,String>(5)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;rows
                };
                ensure!(staged.len()<=MAX_FIELDS,"run metadata references exceed their publication budget");
                for (owner,field,range,encoding) in staged {
                    let entry=owner.strip_prefix(&p).context("invalid metadata staging owner")?;
                    ensure!(ids.contains(entry),"staged range has no published header");
                    // A concurrent explicit rename wins over the automatic
                    // run title; no staged value may resurrect that field.
                    if run_boundary && entry==headers[0]["id"].as_str().unwrap_or_default() && root_field(&field).as_deref()==Some("session_name") && !updated.contains("session_name") {continue;}
                    super::bind_encoded(&tx,&s,"entry",entry,&field,range,Encoding::parse(&encoding)?)?;
                }
                let terminals=headers.iter().filter(|entry|entry["type"]=="run_terminal").cloned().collect::<Vec<_>>();
                crate::session::sqlite_store::insert_transcript_entries(&tx,&s,headers)?;
                for terminal in terminals {super::terminal::publish(&tx,&s,&terminal)?;}
                tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1",[&s])?;
                tx.commit()?;Ok(())
            })
        })();
        if let Err(error) = self.cleanup_content_operation(&operation) {
            tracing::warn!(%error,operation,"metadata staging cleanup deferred until startup");
        }
        self.db.request_space_reclamation();
        result
    }
}

fn root_field(pointer: &str) -> Option<String> {
    pointer
        .strip_prefix("/content/")
        .and_then(|field| field.split('/').next())
        .map(|key| key.replace("~1", "/").replace("~0", "~"))
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
