//! Original-text offsets over immutable canonical fields. Ordinary text/JSON
//! ranges are sliced before transfer; derived encodings are interpreted outside
//! SQLite workers. No compatibility view constructs an entire entry body.
use super::*;
use std::collections::BTreeSet;

struct Block {
    ordinal: i64,
    kind: Option<String>,
    id: Option<String>,
    name: Option<String>,
    error: Option<bool>,
    inline_length: i64,
    reference: Option<Reference>,
}
enum Source {
    Reference(Reference),
    Inline(Column),
    Derived {
        reference: Reference,
        body: std::sync::Arc<crate::session::history::fragment_cache::Fragment>,
    },
    Empty,
}
struct Field {
    block: Block,
    length: i64,
    source: Source,
}

impl Field {
    fn page(
        &self,
        scope: &Snapshot,
        store: &SqliteStore,
        entry: &Entry,
        start: i64,
        budget: usize,
    ) -> Result<String> {
        let budget = usize::try_from((self.length - start).max(0))?.min(budget);
        match &self.source {
            Source::Reference(reference) => Ok(scope
                .reference_page(
                    store,
                    entry,
                    reference,
                    reference.range.offset + start,
                    budget,
                )?
                .text),
            Source::Inline(column) => {
                let raw = scope
                    .column(store, entry, *column, None)?
                    .context("history field disappeared during read")?;
                ensure!(
                    i64::try_from(raw.len())? == self.length,
                    "history field length changed"
                );
                slice(&raw, start, i64::try_from(budget)?)
            }
            Source::Derived { reference, body } => {
                scope.verify_reference(store, entry, reference)?;
                if start == self.length {
                    Ok(String::new())
                } else {
                    body.read(usize::try_from(start)?, budget)
                }
            }
            Source::Empty => Ok(String::new()),
        }
    }

    /// Search one visible field cooperatively. Only the current read page,
    /// query overlap and snippet survive a page; no body aggregation or SQL
    /// lower()/instr() over an assembled historical message is necessary.
    fn find(
        &self,
        scope: &Snapshot,
        store: &SqliteStore,
        entry: &Entry,
        query: &str,
    ) -> Result<Option<(i64, String)>> {
        let id_hit = self.block.id.as_deref() == Some(query);
        let mut offset = 0i64;
        let mut carry = Vec::new();
        loop {
            let page = self.page(scope, store, entry, offset, READ_BYTES)?;
            let base = offset - i64::try_from(carry.len())?;
            offset += i64::try_from(page.len())?;
            carry.extend_from_slice(page.as_bytes());
            let hit = if id_hit {
                Some(0)
            } else {
                carry
                    .windows(query.len())
                    .position(|bytes| bytes.eq_ignore_ascii_case(query.as_bytes()))
                    .map(|at| base + at as i64)
            };
            if let Some(hit) = hit {
                let start = (hit - 120).max(0);
                let end = (start + 480).min(self.length);
                while offset < end {
                    let tail = self.page(
                        scope,
                        store,
                        entry,
                        offset,
                        usize::try_from(end - offset)? + 3,
                    )?;
                    ensure!(!tail.is_empty(), "history search snippet made no progress");
                    offset += i64::try_from(tail.len())?;
                    carry.extend_from_slice(tail.as_bytes());
                }
                let from = usize::try_from(start - base)?;
                let to = usize::try_from(end - base)?;
                let mut bytes = &carry[from..to];
                let skip = bytes
                    .iter()
                    .take_while(|byte| **byte & 0xc0 == 0x80)
                    .count();
                bytes = &bytes[skip..];
                let text = match std::str::from_utf8(bytes) {
                    Ok(text) => text,
                    Err(error) if error.error_len().is_none() => {
                        std::str::from_utf8(&bytes[..error.valid_up_to()])?
                    }
                    Err(error) => return Err(error.into()),
                };
                return Ok(Some((hit, text.to_owned())));
            }
            if offset == self.length {
                return Ok(None);
            }
            ensure!(!page.is_empty(), "history search made no progress");
            // A cross-page match can begin query.len()-1 bytes before the
            // page. Retain its 120-byte snippet prefix as well as that overlap.
            let keep = query
                .len()
                .saturating_sub(1)
                .saturating_add(120)
                .min(carry.len());
            carry.drain(..carry.len() - keep);
        }
    }
}

impl Snapshot {
    pub(crate) fn search(
        store: &SqliteStore,
        session: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<Value>> {
        ensure!(
            !query.is_empty() && query.len() <= 800 && (1..=20).contains(&limit),
            "invalid canonical history search budget"
        );
        let scope = Self::capture(store, session)?;
        let mut before = scope
            .until
            .checked_add(1)
            .context("history position overflow")?;
        let mut matches = Vec::new();
        loop {
            let check = scope.clone();
            let rows=store.db.read(move |db| {
                let tx=db.transaction()?;check.check(&tx)?;
                let mut statement=tx.prepare("SELECT position,entry_id,role,run_id,timestamp_ms FROM entries WHERE session_id=?1 AND position<?2 AND entry_type IN ('user','assistant','tool') ORDER BY position DESC LIMIT 64")?;
                let rows=statement.query_map(params![check.session,before],|row|Ok((Entry {position:row.get(0)?,id:row.get(1)?},row.get::<_,Option<String>>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,Option<i64>>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                ensure!(rows.iter().map(|(entry,role,run,_)|entry.id.len()+role.as_ref().map_or(0,String::len)+run.as_ref().map_or(0,String::len)+32).sum::<usize>()<=READ_BYTES,"history search identity page exceeds budget");
                drop(statement);tx.commit()?;Ok(rows)
            })?;
            if rows.is_empty() {
                break;
            }
            for (entry, role, run, time) in rows {
                before = entry.position;
                let (fields, _, _) = scope.readable_fields(store, &entry)?;
                let mut base = 0i64;
                for field in fields {
                    if let Some((hit, snippet)) = field.find(&scope, store, &entry, query)? {
                        matches.push(json!({"entryId":entry.id,"entryPosition":entry.position,"role":role,"runId":run,"timestampMs":time,
                            "blockIndex":field.block.ordinal,"kind":field.block.kind,"toolCallId":field.block.id,"toolName":field.block.name,
                            "snippet":snippet,"byteOffset":base+hit}));
                        if matches.len() > limit as usize {
                            scope.verify(store)?;
                            return Ok(matches);
                        }
                    }
                    base = base
                        .checked_add(field.length)
                        .context("history search offset overflow")?;
                }
            }
        }
        scope.verify(store)?;
        Ok(matches)
    }

    pub(crate) fn recall(
        store: &SqliteStore,
        session: &str,
        id: &str,
        offset: i64,
        limit: i64,
    ) -> Result<Value> {
        let (session, id) = (session.to_owned(), id.to_owned());
        let (scope,entry,kind,role,run,time)=store.db.read(move |db| {
            let tx=db.transaction()?;
            let scope=Self::capture_from(&tx,&session)?;
            let row=tx.query_row("SELECT position,entry_id,entry_type,role,run_id,timestamp_ms FROM entries WHERE session_id=?1 AND entry_id=?2 AND entry_type IN ('user','assistant','tool')",
                params![session,id],|row|Ok((Entry {position:row.get(0)?,id:row.get(1)?},row.get::<_,String>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,Option<String>>(4)?,row.get::<_,Option<i64>>(5)?))).optional()?.context("entry not found in this session's conversation history")?;
            tx.commit()?;Ok((scope,row.0,row.1,row.2,row.3,row.4))
        })?;
        let (fields, omitted, total) = scope.readable_fields(store, &entry)?;
        ensure!(
            offset <= total,
            "offset is beyond the entry's {total} readable bytes"
        );
        let mut chunks = Vec::new();
        let mut base = 0i64;
        let mut consumed = 0i64;
        for field in fields {
            let end = base
                .checked_add(field.length)
                .context("history field offset overflow")?;
            if end <= offset {
                base = end;
                continue;
            }
            let start = (offset - base).max(0);
            let remaining = limit - consumed;
            if remaining == 0 {
                break;
            }
            let budget = (field.length - start).min(remaining);
            let text = field.page(&scope, store, &entry, start, usize::try_from(budget)?)?;
            if text.is_empty() {
                break;
            }
            consumed += i64::try_from(text.len())?;
            chunks.push(json!({"blockIndex":field.block.ordinal,"kind":field.block.kind,
                "field":if field.block.kind.as_deref()==Some("tool_call") {"argumentsJson"} else {"text"},
                "toolCallId":field.block.id,"toolName":field.block.name,"isError":field.block.error,
                "blockByteOffset":start,"blockTotalBytes":field.length,"text":text}));
            if start + i64::try_from(text.len())? < field.length {
                break;
            }
            base = end;
        }
        let next = offset + consumed;
        ensure!(
            next == total || consumed > 0,
            "offset must be on a UTF-8 character boundary"
        );
        scope.verify(store)?;
        Ok(
            json!({"sessionId":scope.session,"entryId":entry.id,"entryPosition":entry.position,"entryType":kind,
            "role":role,"runId":run,"timestampMs":time,"offset":offset,"maxBytes":limit,
            "totalBytes":total,"chunks":chunks,"hasMore":next<total,
            "nextOffset":if next<total {Some(next)} else {None},"omittedKinds":omitted}),
        )
    }

    fn readable_fields(
        &self,
        store: &SqliteStore,
        entry: &Entry,
    ) -> Result<(Vec<Field>, BTreeSet<String>, i64)> {
        let mut after = -1;
        let mut fields = Vec::new();
        let mut omitted = BTreeSet::new();
        let mut total = 0i64;
        loop {
            let (scope, locator) = (self.clone(), entry.clone());
            let rows=store.db.read(move |db| {
                let tx=db.transaction()?;scope.check(&tx)?;
                let mut query=tx.prepare("SELECT b.ordinal,b.kind,b.tool_call_id,b.tool_name,b.is_error,
                    coalesce(length(CAST(CASE WHEN b.kind='tool_call' THEN b.arguments_json ELSE b.text END AS BLOB)),0),
                    r.field,r.content_id,r.byte_offset,r.byte_length,r.encoding
                    FROM entries e JOIN message_blocks b ON b.session_id=e.session_id AND b.entry_position=e.position
                    LEFT JOIN content_references r ON r.session_id=e.session_id AND r.owner_kind='entry' AND r.owner_id=e.entry_id AND r.field=printf('/content/%d/%s',b.ordinal,CASE WHEN b.kind='tool_call' THEN 'args' WHEN b.kind='tool_result' THEN 'content' ELSE 'text' END)
                    WHERE e.session_id=?1 AND e.position=?2 AND e.entry_id=?3 AND b.ordinal>?4 ORDER BY b.ordinal LIMIT 64")?;
                let mut cursor=query.query(params![scope.session,locator.position,locator.id,after])?;
                let mut rows=Vec::new();let mut bytes=0usize;
                while let Some(row)=cursor.next()? {
                    let reference=if let Some(field)=row.get::<_,Option<String>>(6)? {
                        Some(Reference {field,range:ContentRange {id:row.get(7)?,offset:row.get(8)?,length:row.get(9)?},encoding:Encoding::parse(&row.get::<_,String>(10)?)?})
                    } else {None};
                    let block=Block {ordinal:row.get(0)?,kind:row.get(1)?,id:row.get(2)?,name:row.get(3)?,error:row.get(4)?,inline_length:row.get(5)?,reference};
                    bytes+=block.kind.as_ref().map_or(0,String::len)+block.id.as_ref().map_or(0,String::len)+block.name.as_ref().map_or(0,String::len)+block.reference.as_ref().map_or(0,|reference|reference.field.len())+128;
                    ensure!(bytes<=READ_BYTES,"history field identities exceed read budget");
                    rows.push(block);
                }
                drop(cursor);drop(query);tx.commit()?;Ok(rows)
            })?;
            if rows.is_empty() {
                break;
            }
            for mut block in rows {
                after = block.ordinal;
                let Some(kind) = block.kind.as_deref() else {
                    continue;
                };
                if !matches!(kind, "text" | "tool_call" | "tool_result") {
                    omitted.insert(kind.to_owned());
                    continue;
                }
                let arguments = kind == "tool_call";
                let (length, source) = if block.inline_length > 0 {
                    (
                        block.inline_length,
                        Source::Inline(if arguments {
                            Column::BlockArguments(block.ordinal)
                        } else {
                            Column::BlockText(block.ordinal)
                        }),
                    )
                } else if let Some(reference) = block.reference.take() {
                    match reference.encoding {
                        Encoding::Text | Encoding::Json
                            if arguments || reference.encoding == Encoding::Text =>
                        {
                            (reference.range.length, Source::Reference(reference))
                        }
                        Encoding::ShellSummary | Encoding::ToolArgumentsStringV1 => {
                            let body = self.derived_text(store, entry, &reference, arguments)?;
                            (
                                i64::try_from(body.len)?,
                                Source::Derived { reference, body },
                            )
                        }
                        // A JSON-valued unknown text extension is preserved by
                        // full reads but was never part of visible string text.
                        _ => (0, Source::Empty),
                    }
                } else {
                    (0, Source::Empty)
                };
                total = total
                    .checked_add(length)
                    .context("history entry length overflow")?;
                fields.push(Field {
                    block,
                    length,
                    source,
                });
            }
        }
        Ok((fields, omitted, total))
    }
}

fn slice(bytes: &[u8], offset: i64, limit: i64) -> Result<String> {
    let start = usize::try_from(offset)?;
    let end = start
        .checked_add(usize::try_from(limit)?)
        .context("history fragment length overflow")?
        .min(bytes.len());
    let bytes = &bytes[start..end];
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()])?
        }
        Err(_) => anyhow::bail!("offset must be on a UTF-8 character boundary"),
    };
    Ok(text.to_owned())
}
