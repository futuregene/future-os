//! Sidebar metadata and one selected preview. Large configuration/provider
//! fields stay referenced; list requests never aggregate complete transcripts.
use super::*;
use crate::session::model::SessionSummary;
use crate::session::SessionEntry;

/// One row of a bulk listing. `needs_preview_fallback` is set only when the
/// first user text is body-referenced; the caller resolves those few sessions
/// through `Snapshot::summary` rather than hydrating a preview for every row.
pub(crate) struct BatchSummary {
    pub(crate) summary: SessionSummary,
    pub(crate) needs_preview_fallback: bool,
}

type SummaryRow = (
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

impl Snapshot {
    /// List every live session from one read job. Canonical-format sessions use
    /// one indexed query per id **inside the same snapshot**, which removes the
    /// per-session worker round trips while preserving the existing metadata,
    /// title and ordering rules. Legacy ids are returned for the caller's
    /// existing summary path.
    pub(crate) fn batch_summaries(
        store: &SqliteStore,
        sessions: Vec<String>,
    ) -> Result<(Vec<BatchSummary>, Vec<String>)> {
        store.db.read(move |db| {
            let tx = db.transaction()?;
            let mut canonical = Vec::new();
            let mut legacy = Vec::new();
            let mut summary = tx.prepare_cached(
                "SELECT s.current_metadata_json,s.updated_at_ms,s.created_at_ms,
                    (SELECT json_extract(content_json,'$.model') FROM entries WHERE session_id=s.id AND entry_type='model_change' AND json_type(content_json,'$.model')='text' AND json_extract(content_json,'$.model')<>'' ORDER BY position DESC LIMIT 1),
                    (SELECT count(*) FROM entries WHERE session_id=s.id AND entry_type='user'),
                    (SELECT content_json FROM entries WHERE session_id=s.id AND entry_type='user' ORDER BY position LIMIT 1),
                    (SELECT role FROM entries WHERE session_id=s.id AND entry_type='user' ORDER BY position LIMIT 1),
                    (SELECT b.text FROM entries e JOIN message_blocks b ON b.session_id=e.session_id AND b.entry_position=e.position WHERE e.session_id=s.id AND e.entry_type='user' AND b.text IS NOT NULL AND e.position=(SELECT min(position) FROM entries WHERE session_id=s.id AND entry_type='user') ORDER BY e.position,b.ordinal LIMIT 1),
                    (SELECT printf('%d|%d|%d|%s|%s',r.content_id,r.byte_offset,r.byte_length,r.encoding,r.field) FROM entries e JOIN content_references r ON r.session_id=e.session_id AND r.owner_id=e.entry_id WHERE e.session_id=s.id AND e.entry_type='user' AND r.owner_kind='entry' AND r.field LIKE '/content/%/text' AND e.position=(SELECT min(position) FROM entries WHERE session_id=s.id AND entry_type='user') ORDER BY length(r.field),r.field LIMIT 1)
                 FROM sessions s WHERE s.id=?1 AND s.revision>=0
                   AND NOT EXISTS(SELECT 1 FROM session_tombstones t WHERE t.session_id=s.id)",
            )?;
            for id in sessions {
                let bound = BoundIdentity::session(&tx, &id)?;
                if super::super::format_bound(&tx, &bound)? != super::super::FORMAT_VERSION {
                    legacy.push(id);
                    continue;
                }
                let (
                    raw,
                    time,
                    created,
                    model,
                    count,
                    first_content,
                    first_role,
                    first_block,
                    first_reference,
                ): SummaryRow = summary.query_row(
                    [&bound],
                    |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?)),
                )?;
                ensure!(raw.as_ref().is_none_or(|raw|raw.len()<=super::super::WRITE_BYTES),"session summary metadata exceeds its thin budget");
                let info: Value = raw
                    .map(|raw| serde_json::from_str(&raw))
                    .transpose()?
                    .unwrap_or(Value::Null);
                super::super::metadata::validate_thin(&info)?;

                let content: Option<Value> = first_content
                    .map(|raw| serde_json::from_str::<Value>(&raw))
                    .transpose()?;
                let inline_text = content.as_ref().and_then(|value| {
                    value.as_str().map(str::to_owned).or_else(|| {
                        value.as_array().and_then(|blocks| {
                            blocks
                                .iter()
                                .find_map(|block| block.get("text").and_then(Value::as_str))
                                .map(str::to_owned)
                        })
                    })
                });
                let has_inline_text_slot = inline_text.is_some();
                let first_is_user = first_role.as_deref() == Some("user");
                let mut preview = if first_is_user {
                    inline_text.as_ref().and_then(|text| {
                        crate::session::Manager::summary_first_message(&SessionEntry::new_user(
                            "user",
                            Value::String(text.clone()),
                        ))
                    })
                } else {
                    None
                };
                let reference = first_reference.as_deref().and_then(|encoded| {
                    let mut fields = encoded.splitn(5, '|');
                    Some((
                        fields.next()?.parse::<i64>().ok()?,
                        fields.next()?.parse::<i64>().ok()?,
                        fields.next()?.parse::<i64>().ok()?,
                        fields.next()?.to_owned(),
                        fields.next()?.to_owned(),
                    ))
                });
                if preview.is_none() && first_is_user {
                    if let Some((content_id, offset, length, encoding, _)) = reference.as_ref() {
                        if encoding == "text" {
                            let title = crate::session::title_preview::paged(
                                usize::try_from(*length)?,
                                |start, budget| {
                                    Ok(
                                        super::super::read(
                                            &tx,
                                            *content_id,
                                            offset + i64::try_from(start)?,
                                            budget,
                                        )?
                                        .text,
                                    )
                                },
                            )?;
                            preview = (!title.is_empty()).then_some(title);
                        }
                    }
                }
                if preview.is_none() && !has_inline_text_slot && first_is_user {
                    if let Some(block) = first_block.filter(|block| !block.is_empty()) {
                        preview = crate::session::Manager::summary_first_message(
                            &SessionEntry::new_user("user", Value::String(block)),
                        );
                    }
                }
                let needs_preview_fallback = reference
                    .as_ref()
                    .is_some_and(|(_, _, _, encoding, _)| encoding != "text")
                    && preview.is_none();

                canonical.push(BatchSummary {
                    summary: SessionSummary {
                        id: id.clone(),
                        cwd: info["cwd"].as_str().unwrap_or_default().to_owned(),
                        updated_at: chrono::DateTime::from_timestamp_millis(
                            time.context("published session has no update timestamp")?,
                        )
                        .context("session timestamp is out of range")?
                        .with_timezone(&chrono::Local),
                        created_at: chrono::DateTime::from_timestamp_millis(
                            created.context("published session has no creation timestamp")?,
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
                    },
                    needs_preview_fallback,
                });
            }
            drop(summary);
            tx.commit()?;
            Ok((canonical, legacy))
        })
    }

    pub(crate) fn summary(store: &SqliteStore, session: &str) -> Result<SessionSummary> {
        let session = session.to_owned();
        let (scope,raw,model,time,created,count,first)=store.db.read(move |db| {
            let tx=db.transaction()?;
            let scope=Self::capture_from(&tx,&session)?;
            let session=BoundIdentity::session(&tx,&session)?;
            let (raw,time,created):(Option<String>,Option<i64>,Option<i64>)=tx.query_row("SELECT current_metadata_json,updated_at_ms,created_at_ms FROM sessions WHERE id=?1",[&session],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
            ensure!(raw.as_ref().is_none_or(|raw|raw.len()<=super::super::WRITE_BYTES),"session summary metadata exceeds its thin budget");
            let model:Option<String>=tx.query_row("SELECT json_extract(content_json,'$.model') FROM entries WHERE session_id=?1 AND entry_type='model_change' AND json_type(content_json,'$.model')='text' AND json_extract(content_json,'$.model')<>'' ORDER BY position DESC LIMIT 1",[&session],|row|row.get(0)).optional()?;
            let count:i64=tx.query_row("SELECT count(*) FROM entries WHERE session_id=?1 AND entry_type='user'",[&session],|row|row.get(0))?;
            let first:Option<Entry>=tx.query_row("SELECT position,entry_id FROM entries WHERE session_id=?1 AND entry_type='user' ORDER BY position LIMIT 1",[&session],|row|Ok(Entry {position:row.get(0)?,id:row.get(1)?})).optional()?;
            let generation=super::super::metadata::generation(&tx,&session)?;
            tx.commit()?;Ok(((scope,generation),raw,model,time,created,count,first))
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
            created_at: chrono::DateTime::from_timestamp_millis(
                created.context("published session has no creation timestamp")?,
            )
            .context("session creation timestamp is out of range")?
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
        let reference=self.read(store, move |db| {
            let tx=db.savepoint()?;scope.check(&tx)?;
            let mut statement=tx.prepare_cached("SELECT r.content_id,r.byte_offset,r.byte_length,r.encoding FROM content_references r JOIN entries e ON e.session_id=r.session_id AND e.entry_id=r.owner_id WHERE e.session_id=?1 AND e.position=?2 AND e.entry_id=?3 AND r.owner_kind='entry' AND r.field=?4")?;
            let row:Option<(i64,i64,i64,String)>=statement.query_row(params![scope.session,locator.position,locator.id,pointer],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
            drop(statement);
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
