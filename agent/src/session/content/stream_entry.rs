//! Publish an assistant header from already-committed source identities.
//! This recipe never owns a copy of streamed text, arguments or provider state.
use super::{
    stream::{Encoding, StreamIdentity},
    ContentRange,
};
use crate::{
    session::{sqlite_store::SqliteStore, SessionEntry},
    types::{AgentMessage, ContentBlock},
};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

pub(crate) struct StreamEntry {
    pub header: SessionEntry,
    run: String,
    epoch: i64,
    fields: Vec<Field>,
}
enum Field {
    Text {
        pointer: String,
        key: String,
        bytes: usize,
    },
    ReasoningProvider {
        pointer: String,
        key: String,
    },
    Tool {
        ordinal: usize,
        id: String,
        string_args: bool,
        null_args: bool,
        provider: bool,
    },
}
struct Link {
    pointer: String,
    range: ContentRange,
    encoding: Encoding,
}
struct Source {
    cursor: i64,
    data: Value,
    links: Vec<(String, ContentRange, Encoding)>,
}

impl StreamEntry {
    pub(crate) fn from_assistant(message: &AgentMessage, run: &str, epoch: u64) -> Result<Self> {
        let epoch = i64::try_from(epoch)?;
        ensure!(
            message.role == "assistant",
            "only source assistant messages use this contract"
        );
        let id = message
            .journal_entry_id()
            .context("assistant has no source entry identity")?;
        let metadata = message
            .metadata
            .as_ref()
            .context("assistant has no source metadata")?;
        let keys = metadata
            .get(AgentMessage::STREAM_BLOCKS_KEY)
            .and_then(Value::as_array)
            .context("assistant has no ordered source block identities")?;
        ensure!(
            keys.len() == message.content.len() && keys.len() <= 1024,
            "invalid source block manifest"
        );
        let mut tools = std::collections::HashSet::new();
        let mut blocks = Vec::with_capacity(keys.len());
        let mut fields = Vec::new();
        for (ordinal, block) in message.content.iter().enumerate() {
            let key = keys[ordinal].as_str();
            match block {
                ContentBlock::Text { text } => {
                    let key = key.context("text has no source block identity")?;
                    blocks.push(json!({"type":"text","text":""}));
                    fields.push(Field::Text {
                        pointer: format!("/content/{ordinal}/text"),
                        key: key.into(),
                        bytes: text.len(),
                    });
                }
                ContentBlock::Reasoning {
                    text,
                    provider_metadata,
                } => {
                    let key = key.context("reasoning has no source block identity")?;
                    blocks.push(json!({"type":"reasoning","text":"","provider_metadata":{}}));
                    fields.push(Field::Text {
                        pointer: format!("/content/{ordinal}/text"),
                        key: key.into(),
                        bytes: text.len(),
                    });
                    if !provider_metadata.is_empty() {
                        fields.push(Field::ReasoningProvider {
                            pointer: format!("/content/{ordinal}/provider_metadata"),
                            key: key.into(),
                        });
                    }
                }
                ContentBlock::ToolCall {
                    id,
                    name,
                    args,
                    provider_metadata,
                } => {
                    ensure!(
                        !id.is_empty() && id.len() <= 1024,
                        "tool has no bounded source identity"
                    );
                    ensure!(
                        name.len() <= 512 && tools.insert(id.as_str()),
                        "ambiguous or oversized tool identity within one assistant message"
                    );
                    blocks.push(json!({"type":"tool_call","id":id,"name":name,"args":null,"provider_metadata":{}}));
                    fields.push(Field::Tool {
                        ordinal,
                        id: id.clone(),
                        string_args: args.is_string(),
                        null_args: args.is_null(),
                        provider: !provider_metadata.is_empty(),
                    });
                }
                _ => anyhow::bail!("assistant block has no canonical source contract"),
            }
        }
        let mut meta = serde_json::Map::new();
        for (key, value) in metadata {
            if matches!(
                key.as_str(),
                AgentMessage::JOURNAL_ENTRY_ID_KEY
                    | AgentMessage::STREAM_BLOCKS_KEY
                    | AgentMessage::STREAM_EPOCH_KEY
            ) {
                continue;
            }
            meta.insert(key.clone(), value.clone());
        }
        meta.insert("run_id".into(), json!(run));
        Ok(Self {
            header: SessionEntry {
                id: id.into(),
                entry_type: "assistant".into(),
                role: "assistant".into(),
                content: Some(Value::Array(blocks)),
                tool_calls: Vec::new(),
                timestamp: chrono::Local::now(),
                tool_call_id: String::new(),
                name: message.name.clone(),
                tool_args: String::new(),
                thinking: String::new(),
                meta: Some(Value::Object(meta)),
            },
            run: run.into(),
            epoch,
            fields,
        })
    }
}

struct SourceSelector<'a> {
    kind: &'a str,
    key: Option<&'a str>,
    tool: Option<&'a str>,
    index: Option<u64>,
}

impl SqliteStore {
    pub(crate) fn append_stream_entry(&self, session: &str, recipe: StreamEntry) -> Result<()> {
        let scope = StreamIdentity {
            session_id: session.into(),
            run_id: recipe.run.clone(),
            epoch: recipe.epoch,
        };
        let id = recipe.header.id.clone();
        let (s, r, entry) = (scope.clone(), recipe.run.clone(), id.clone());
        let boundary=self.db.read(move |db| {
            let cursor:Option<i64>=db.query_row("SELECT c.last_cursor FROM stream_runs r JOIN stream_changes c ON c.stream_id=r.id WHERE r.session_id=?1 AND r.run_id=?2 AND r.epoch=?3 AND json_extract(c.metadata_json,'$.data.entry_id')=?4 AND c.event_type='message_end' AND c.event_type IN ('message_end','tool_start','tool_input_end') AND c.last_cursor<=r.durable_cursor AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=r.id) ORDER BY c.last_cursor DESC LIMIT 1",
                params![s.session_id,r,s.epoch,entry],|row|row.get(0)).optional()?;
            cursor.context("assistant message end is not committed")
        })?;
        let mut value = serde_json::to_value(&recipe.header)?;
        let mut links = Vec::<Link>::new();
        for field in recipe.fields {
            match field {
                Field::Text {
                    pointer,
                    key,
                    bytes,
                } => {
                    ensure!(
                        key.starts_with(&format!("{id}:")),
                        "source block belongs to another message"
                    );
                    if bytes > 0 {
                        let range = self.sealed_source_range(&scope, &key)?;
                        ensure!(
                            range.length == i64::try_from(bytes)?,
                            "source text length differs from final model state"
                        );
                        links.push(Link {
                            pointer,
                            range,
                            encoding: Encoding::Text,
                        });
                    }
                }
                Field::ReasoningProvider { pointer, key } => {
                    let source = self
                        .entry_source(
                            &scope,
                            &id,
                            boundary,
                            SourceSelector {
                                kind: "thinking_end",
                                key: Some(&key),
                                tool: None,
                                index: None,
                            },
                        )?
                        .context("reasoning provider state has no committed source end")?;
                    restore_field(
                        &mut value,
                        &mut links,
                        &pointer,
                        &source,
                        "provider_metadata",
                    )?;
                }
                Field::Tool {
                    ordinal,
                    id: tool,
                    string_args,
                    null_args,
                    provider,
                } => {
                    let end = self.entry_source(
                        &scope,
                        &id,
                        boundary,
                        SourceSelector {
                            kind: "tool_input_end",
                            key: None,
                            tool: Some(&tool),
                            index: None,
                        },
                    )?;
                    let pointer = format!("/content/{ordinal}/args");
                    let source = if let Some(end) = end {
                        restore_field(&mut value, &mut links, &pointer, &end, "tool_args")?;
                        if string_args {
                            if let Some(link) = links.last_mut().filter(|link| {
                                link.pointer == pointer && link.encoding == Encoding::Text
                            }) {
                                link.encoding = Encoding::ToolArgumentsStringV1;
                            }
                        }
                        end
                    } else {
                        let source = self
                            .entry_source(
                                &scope,
                                &id,
                                boundary,
                                SourceSelector {
                                    kind: "tool_input",
                                    key: None,
                                    tool: Some(&tool),
                                    index: None,
                                },
                            )?
                            .context("tool input has no committed source identity")?;
                        let key = source.data["content_key"]
                            .as_str()
                            .context("tool input source key is missing")?;
                        let range = self.sealed_source_range(&scope, key)?;
                        if !null_args || range.length > 0 {
                            links.push(Link {
                                pointer,
                                range,
                                encoding: if string_args {
                                    Encoding::ToolArgumentsStringV1
                                } else {
                                    Encoding::Json
                                },
                            });
                        }
                        source
                    };
                    if provider {
                        let pointer = format!("/content/{ordinal}/provider_metadata");
                        let has_provider = source
                            .links
                            .iter()
                            .any(|(field, _, _)| field == "/data/provider_metadata")
                            || source.data["provider_metadata"]
                                .as_object()
                                .is_some_and(|object| !object.is_empty());
                        let source = if has_provider {
                            source
                        } else {
                            self.entry_source(
                                &scope,
                                &id,
                                source.cursor,
                                SourceSelector {
                                    kind: "tool_start",
                                    key: None,
                                    tool: None,
                                    index: Some(source.data["tc_index"].as_u64().unwrap_or(0)),
                                },
                            )?
                            .context("tool provider state has no committed input start")?
                        };
                        restore_field(
                            &mut value,
                            &mut links,
                            &pointer,
                            &source,
                            "provider_metadata",
                        )?;
                    }
                }
            }
        }
        // Small reference jobs may interleave with other conversations. The
        // transcript header is not visible until every field has an owner.
        for batch in links.chunks(64) {
            let (s, entry, batch) = (
                session.to_owned(),
                id.clone(),
                batch
                    .iter()
                    .map(|link| (link.pointer.clone(), link.range, link.encoding))
                    .collect::<Vec<_>>(),
            );
            self.db.call(move |db| {
                let tx = crate::session::database::begin_write_transaction(db)?;
                for (field, range, encoding) in batch {
                    super::bind_encoded(&tx, &s, "entry", &entry, &field, range, encoding)?;
                }
                tx.commit()?;
                Ok(())
            })?;
        }
        let session = session.to_owned();
        self.db.call(move |db| {
            let tx=crate::session::database::begin_write_transaction(db)?;
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM stream_runs r JOIN stream_changes c ON c.stream_id=r.id WHERE r.session_id=?1 AND r.run_id=?2 AND r.epoch=?3 AND c.last_cursor=?4 AND c.event_type='message_end' AND json_extract(c.metadata_json,'$.data.entry_id')=?5 AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=r.id))",
                params![session,scope.run_id,scope.epoch,boundary,id],|row|row.get(0))?;
            ensure!(valid,"source changed before assistant header publication");
            crate::session::sqlite_store::insert_transcript_entries(&tx,&session,vec![value])?;
            tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1",[session])?;
            tx.commit()?;Ok(())
        })
    }

    fn sealed_source_range(&self, scope: &StreamIdentity, key: &str) -> Result<ContentRange> {
        let (scope, key) = (scope.clone(), key.to_owned());
        self.db.read(move |db| {
            let range = super::resolve(
                db,
                &scope.session_id,
                "stream",
                &super::stream::owner(&scope, &key),
                "",
            )?
            .context("source body is missing")?;
            let sealed: bool = db.query_row(
                "SELECT sealed=1 AND byte_length=?2 FROM content_objects WHERE id=?1",
                params![range.id, range.length],
                |row| row.get(0),
            )?;
            ensure!(sealed, "source body is not closed at the message boundary");
            Ok(range)
        })
    }

    fn entry_source(
        &self,
        scope: &StreamIdentity,
        entry: &str,
        until: i64,
        selector: SourceSelector<'_>,
    ) -> Result<Option<Source>> {
        let SourceSelector {
            kind,
            key,
            tool,
            index,
        } = selector;
        let (scope, entry, kind, key, tool) = (
            scope.clone(),
            entry.to_owned(),
            kind.to_owned(),
            key.map(str::to_owned),
            tool.map(str::to_owned),
        );
        let index = index.map(i64::try_from).transpose()?;
        self.db.read(move |db| {
            let tx=db.transaction()?;
            let kind_filter=match kind.as_str() {
                "thinking_end" => "c.event_type='thinking_end'",
                "tool_input_end" => "c.event_type='tool_input_end' AND c.event_type IN ('message_end','tool_start','tool_input_end')",
                "tool_start" => "c.event_type='tool_start' AND c.event_type IN ('message_end','tool_start','tool_input_end')",
                "tool_input" => "c.event_type IN ('tool_start','tool_delta') AND c.event_type IN ('tool_start','tool_delta','tool_input_end')",
                _ => anyhow::bail!("unknown source field selector"),
            };
            let key_filter=if key.is_some() {"json_extract(c.metadata_json,'$.data.content_key')=?7"} else {"?7 IS NULL"};
            let tool_filter=if tool.is_some() {"coalesce(json_extract(c.metadata_json,'$.data.tool_id'),'')=?8"} else {"?8 IS NULL"};
            let index_filter=if index.is_some() {"coalesce(json_extract(c.metadata_json,'$.data.tc_index'),0)=?9"} else {"?9 IS NULL"};
            // Only static clauses vary. Direct equality lets the expression
            // indexes find a single end/key/tool without scanning token rows.
            let sql=format!("SELECT c.id,c.last_cursor,c.metadata_json FROM stream_runs r JOIN stream_changes c ON c.stream_id=r.id WHERE r.session_id=?1 AND r.run_id=?2 AND r.epoch=?3 AND json_extract(c.metadata_json,'$.data.entry_id')=?4 AND c.last_cursor<=?5 AND c.last_cursor<=r.durable_cursor
                AND ?6 IS NOT NULL AND {kind_filter} AND {key_filter} AND {tool_filter} AND {index_filter}
                AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=r.id) ORDER BY c.last_cursor DESC LIMIT 1");
            let row:Option<(i64,i64,String)>=tx.query_row(&sql,
                params![scope.session_id,scope.run_id,scope.epoch,entry,until,kind,key,tool,index],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
            let Some((change,cursor,metadata))=row else {return Ok(None);};
            let metadata:Value=serde_json::from_str(&metadata)?;
            let mut query=tx.prepare("SELECT field,content_id,byte_offset,byte_length,encoding FROM stream_change_content WHERE change_id=?1")?;
            let links=query.query_map([change],|row|Ok((row.get::<_,String>(0)?,ContentRange{id:row.get(1)?,offset:row.get(2)?,length:row.get(3)?},row.get::<_,String>(4)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?.into_iter().map(|(field,range,encoding)|Ok((field,range,Encoding::parse(&encoding)?))).collect::<Result<Vec<_>>>()?;
            drop(query);tx.commit()?;
            Ok(Some(Source {cursor,data:metadata["data"].clone(),links}))
        })
    }
}

fn restore_field(
    header: &mut Value,
    links: &mut Vec<Link>,
    target: &str,
    source: &Source,
    name: &str,
) -> Result<()> {
    let field = format!("/data/{name}");
    let slot = header
        .pointer_mut(target)
        .context("thin header field is missing")?;
    if let Some((_, range, encoding)) = source.links.iter().find(|(known, _, _)| *known == field) {
        *slot = if *encoding == Encoding::Json {
            super::extensions::json_placeholder(slot)
        } else {
            json!("")
        };
        links.push(Link {
            pointer: target.into(),
            range: *range,
            encoding: *encoding,
        });
    } else {
        let value = source
            .data
            .get(name)
            .context("source semantic field is missing")?;
        ensure!(
            value.to_string().len() <= super::stream::CHANGE_METADATA_BYTES,
            "source inline field exceeds metadata budget"
        );
        *slot = value.clone();
    }
    Ok(())
}
