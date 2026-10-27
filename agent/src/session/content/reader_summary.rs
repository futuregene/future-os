//! Sidebar metadata and one selected preview. Large configuration/provider
//! fields stay referenced; list requests never aggregate complete transcripts.
use super::*;
use crate::session::model::SessionSummary;

impl Snapshot {
    pub(crate) fn summary(store: &SqliteStore, session: &str) -> Result<SessionSummary> {
        let session = session.to_owned();
        let (scope,raw,model,time,count,first)=store.db.read(move |db| {
            let tx=db.transaction()?;
            let scope=Self::capture_from(&tx,&session)?;
            let session=BoundIdentity::session(&tx,&session)?;
            let (raw,time):(Option<String>,Option<i64>)=tx.query_row("SELECT current_metadata_json,updated_at_ms FROM sessions WHERE id=?1",[&session],|row|Ok((row.get(0)?,row.get(1)?)))?;
            ensure!(raw.as_ref().is_none_or(|raw|raw.len()<=super::super::WRITE_BYTES),"session summary metadata exceeds its thin budget");
            let model:Option<String>=tx.query_row("SELECT json_extract(content_json,'$.model') FROM entries WHERE session_id=?1 AND entry_type='model_change' AND json_type(content_json,'$.model')='text' AND json_extract(content_json,'$.model')<>'' ORDER BY position DESC LIMIT 1",[&session],|row|row.get(0)).optional()?;
            let count:i64=tx.query_row("SELECT count(*) FROM entries WHERE session_id=?1 AND entry_type='user'",[&session],|row|row.get(0))?;
            let first:Option<Entry>=tx.query_row("SELECT position,entry_id FROM entries WHERE session_id=?1 AND entry_type='user' ORDER BY position LIMIT 1",[&session],|row|Ok(Entry {position:row.get(0)?,id:row.get(1)?})).optional()?;
            let generation=super::super::metadata::generation(&tx,&session)?;
            tx.commit()?;Ok(((scope,generation),raw,model,time,count,first))
        })?;
        let (scope, metadata_generation) = scope;
        let info: Value = raw
            .map(|raw| serde_json::from_str(&raw))
            .transpose()?
            .unwrap_or(Value::Null);
        super::super::metadata::validate_thin(&info)?;
        let mut preview = None;
        if let Some(entry) = first {
            let header = scope.header(store, &entry)?;
            if header["role"] == "user" {
                let title = if let Some(blocks) = header["content"].as_array() {
                    if let Some(ordinal) = blocks.iter().position(|block| block["text"].is_string())
                    {
                        Some(scope.preview_field(
                            store,
                            &entry,
                            &format!("/content/{ordinal}/text"),
                            blocks[ordinal]["text"].as_str().expect("selected string"),
                        )?)
                    } else {
                        None
                    }
                } else {
                    header["content"]
                        .as_str()
                        .map(crate::session::title_preview::from_str)
                };
                if let Some(title) = title {
                    if !title.is_empty() {
                        preview = Some(title);
                    }
                }
            }
        }
        scope.verify(store)?;
        let check = scope.clone();
        store.db.read(move |db| {
            let tx = db.transaction()?;
            check.check(&tx)?;
            ensure!(
                super::super::metadata::generation(&tx, &check.session)? == metadata_generation,
                "session metadata changed while its summary was read"
            );
            tx.commit()?;
            Ok(())
        })?;
        Ok(SessionSummary {
            id: scope.session.to_string(),
            cwd: info["cwd"].as_str().unwrap_or_default().to_owned(),
            updated_at: chrono::DateTime::from_timestamp_millis(
                time.context("published session has no update timestamp")?,
            )
            .context("session timestamp is out of range")?
            .with_timezone(&chrono::Local),
            model: info["model"]
                .as_str()
                .filter(|model| !model.is_empty())
                .map(str::to_owned)
                .or(model)
                .unwrap_or_default(),
            name: info["session_name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
            parent_session_id: info["parent_session_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            first_message: preview,
            query_count: usize::try_from(count)?,
        })
    }

    fn preview_field(
        &self,
        store: &SqliteStore,
        entry: &Entry,
        field: &str,
        inline: &str,
    ) -> Result<String> {
        let (scope, locator, pointer) = (self.clone(), entry.clone(), field.to_owned());
        let reference=store.db.read(move |db| {
            let tx=db.transaction()?;scope.check(&tx)?;
            let row:Option<(i64,i64,i64,String)>=tx.query_row("SELECT r.content_id,r.byte_offset,r.byte_length,r.encoding FROM content_references r JOIN entries e ON e.session_id=r.session_id AND e.entry_id=r.owner_id WHERE e.session_id=?1 AND e.position=?2 AND e.entry_id=?3 AND r.owner_kind='entry' AND r.field=?4",params![scope.session,locator.position,locator.id,pointer],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
            tx.commit()?;Ok(row)
        })?;
        if let Some((id, offset, length, encoding)) = reference {
            ensure!(encoding == "text", "title field is not canonical text");
            let reference = Reference {
                field: field.to_owned(),
                range: ContentRange { id, offset, length },
                encoding: Encoding::Text,
            };
            crate::session::title_preview::paged(usize::try_from(length)?, |start, budget| {
                Ok(self
                    .reference_page(
                        store,
                        entry,
                        &reference,
                        offset + i64::try_from(start)?,
                        budget,
                    )?
                    .text)
            })
        } else {
            Ok(crate::session::title_preview::from_str(inline))
        }
    }
}
