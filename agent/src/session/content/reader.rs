//! Cooperative canonical transcript reads. SQL jobs return bounded raw pages;
//! JSON assembly and derived representations run outside the read workers.
use super::{stream::Encoding, ContentRange, READ_BYTES};
use crate::session::{identity::BoundIdentity, sqlite_store::SqliteStore};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

#[derive(Clone)]
pub(crate) struct Snapshot {
    session: BoundIdentity,
    identity: future_rpc::content_range::StorageIdentity,
    generation: i64,
    until: i64,
    read_snapshot: Option<crate::session::database::ReadSnapshot>,
}
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub position: i64,
    pub id: BoundIdentity,
}
struct Header {
    kind: String,
    role: Option<String>,
    run: Option<BoundIdentity>,
    metadata_generation: i64,
}
#[derive(Clone)]
struct Reference {
    field: String,
    range: ContentRange,
    encoding: Encoding,
}
#[derive(Clone, Copy)]
enum Column {
    Metadata,
    Content,
    SessionMetadata,
    BlockMetadata(i64),
    BlockText(i64),
    BlockArguments(i64),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hydration {
    Full,
    Header,
    Display,
}

impl Snapshot {
    pub(crate) fn session_id(&self) -> &str {
        &self.session
    }
    pub(crate) fn session_key(&self) -> &BoundIdentity {
        &self.session
    }
    pub(crate) fn end_position(&self) -> i64 {
        self.until
    }
    pub(crate) fn capture(store: &SqliteStore, session: &str) -> Result<Self> {
        let session = session.to_owned();
        store.db.read(move |db| {
            let tx = db.transaction()?;
            let scope = Self::capture_from(&tx, &session)?;
            tx.commit()?;
            Ok(scope)
        })
    }

    /// Capture the page's identity in the same read transaction as its index
    /// rows. A history rewrite between two independently captured snapshots
    /// must never combine old ordinals with new entry bodies.
    pub(crate) fn capture_from(db: &Connection, session: &str) -> Result<Self> {
        let session = BoundIdentity::session(db, session)?;
        ensure!(
            super::format_bound(db, &session)? == super::FORMAT_VERSION,
            "wrong transcript reader format"
        );
        let live:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND revision>=0) AND NOT EXISTS(SELECT 1 FROM session_tombstones WHERE session_id=?1)",[&session],|row|row.get(0))?;
        ensure!(live, "session is missing or not published");
        Ok(Self {
            session: session.clone(),
            read_snapshot: None,
            identity: super::transport::storage_identity(db)?,
            generation: generation(db, &session)?,
            until: db.query_row(
                "SELECT coalesce(max(position),-1) FROM entries WHERE session_id=?1",
                [&session],
                |row| row.get(0),
            )?,
        })
    }

    pub(crate) fn use_read_snapshot(&mut self, snapshot: crate::session::database::ReadSnapshot) {
        self.read_snapshot = Some(snapshot);
    }

    fn read<T: Send + 'static>(
        &self,
        store: &SqliteStore,
        read: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        if let Some(snapshot) = &self.read_snapshot {
            snapshot.read(read)
        } else {
            store.db.read(read)
        }
    }

    pub(crate) fn verify(&self, store: &SqliteStore) -> Result<()> {
        if let Some(snapshot) = &self.read_snapshot {
            snapshot.read(|_| Ok(()))?;
        }
        // The final authorization uses current data, not the leased old view:
        // deletion/reset/restore must still invalidate the response.
        let scope = self.clone();
        store.db.read(move |db| scope.check(db))
    }

    /// Immutable entry identity plus the page's current derived annotations.
    /// Appending another run leaves old bodies reusable; reset/restore/delete
    /// still require the normal scope verification before serving cached bytes.
    pub(crate) fn render_key(&self, entry: &Entry, overlay: &str) -> String {
        use sha2::{Digest, Sha256};
        let key = json!([
            self.identity.database_id,
            self.identity.restore_generation,
            self.session,
            self.generation,
            entry.position,
            entry.id,
            overlay
        ]);
        format!("{:x}", Sha256::digest(key.to_string().as_bytes()))
    }

    pub(crate) fn check(&self, db: &Connection) -> Result<()> {
        ensure!(
            super::transport::storage_identity(db)? == self.identity,
            "transcript database generation changed"
        );
        ensure!(
            generation(db, &self.session)? == self.generation,
            "transcript changed during paged read"
        );
        let live:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND revision>=0) AND NOT EXISTS(SELECT 1 FROM session_tombstones WHERE session_id=?1)",[&self.session],|row|row.get(0))?;
        ensure!(live, "session was deleted during paged read");
        Ok(())
    }

    pub(crate) fn page(&self, store: &SqliteStore, after: i64) -> Result<Vec<Entry>> {
        let scope = self.clone();
        self.read(store, move |db| {
            let tx=db.savepoint()?;scope.check(&tx)?;
            let mut query=tx.prepare("SELECT position,entry_id FROM entries WHERE session_id=?1 AND position>?2 AND position<=?3 ORDER BY position LIMIT 64")?;
            let rows=query.query_map(params![scope.session,after,scope.until],|row|Ok(Entry {position:row.get(0)?,id:row.get(1)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
            ensure!(rows.iter().map(|entry|entry.id.len()+16).sum::<usize>()<=READ_BYTES,"entry identity page exceeds budget");
            drop(query);tx.commit()?;Ok(rows)
        })
    }

    pub(crate) fn entry(&self, store: &SqliteStore, entry: &Entry) -> Result<Value> {
        self.entry_impl(store, entry, Hydration::Full)
    }

    pub(crate) fn header(&self, store: &SqliteStore, entry: &Entry) -> Result<Value> {
        self.entry_impl(store, entry, Hydration::Header)
    }

    /// Hydrate exactly the fields consumed by the existing public projection.
    /// Unknown blocks and public metadata stay lossless; discarded root/known-
    /// block extensions remain available through full/context/export reads.
    pub(crate) fn display(&self, store: &SqliteStore, entry: &Entry) -> Result<Value> {
        self.entry_impl(store, entry, Hydration::Display)
    }

    fn entry_impl(
        &self,
        store: &SqliteStore,
        entry: &Entry,
        hydration: Hydration,
    ) -> Result<Value> {
        ensure!(
            entry.position <= self.until,
            "entry is beyond the captured transcript"
        );
        let (scope, locator) = (self.clone(), entry.clone());
        let header=self.read(store, move |db| {
            let tx=db.savepoint()?;scope.check(&tx)?;
            let header=tx.query_row("SELECT e.entry_type,e.role,e.run_id,coalesce(v.metadata_generation,0) FROM entries e LEFT JOIN content_history_versions v ON v.session_id=e.session_id WHERE e.session_id=?1 AND e.position=?2 AND e.entry_id=?3",
                params![scope.session,locator.position,locator.id],|row|Ok(Header {kind:row.get(0)?,role:row.get(1)?,run:row.get(2)?,metadata_generation:row.get(3)?}))?;
            tx.commit()?;Ok(header)
        })?;
        let metadata = self
            .column(store, entry, Column::Metadata, None)?
            .context("entry metadata is missing")?;
        let mut value: Value = serde_json::from_slice(&metadata)?;
        ensure!(value.is_object(), "entry metadata is not an object");
        value["id"] = json!(entry.id);
        value["type"] = json!(header.kind);
        if let Some(role) = header.role {
            value["role"] = json!(role);
        }
        if let Some(run) = header
            .run
            .filter(|_| !matches!(header.kind.as_str(), "run_started" | "run_terminal"))
        {
            ensure!(
                value["meta"].is_null() || value["meta"].is_object(),
                "canonical entry has invalid run metadata; refusing to overwrite it"
            );
            if !value["meta"].is_object() {
                value["meta"] = json!({});
            }
            value["meta"]["run_id"] = json!(run);
        }
        if header.kind == "session_info" {
            if let Some(content) = self.column(
                store,
                entry,
                Column::SessionMetadata,
                Some(header.metadata_generation),
            )? {
                value["content"] = serde_json::from_slice(&content)?;
            } else {
                value["content"] = Value::Null;
            }
        } else if let Some(content) = self.column(store, entry, Column::Content, None)? {
            if content == b"[]" {
                value["content"] = Value::Array(self.blocks(store, entry)?);
            } else {
                value["content"] = serde_json::from_slice(&content)?;
            }
        }
        let mut after = String::new();
        loop {
            let references = self.references(store, entry, &after)?;
            if references.is_empty() {
                break;
            }
            for reference in references {
                after = reference.field.clone();
                if hydration == Hydration::Header {
                    if value.pointer(&reference.field).is_none() {
                        let empty = if reference.encoding == Encoding::Json {
                            Value::Null
                        } else {
                            json!("")
                        };
                        set_field(&mut value, &reference.field, empty)?;
                    }
                    continue;
                }
                if hydration == Hydration::Display && !display_field(&value, &reference.field) {
                    continue;
                }
                let raw = self.body(store, entry, &reference)?;
                let field = decode(reference.encoding, raw)?;
                set_field(&mut value, &reference.field, field)?;
            }
        }
        self.verify(store)?;
        if header.kind == "session_info" {
            let (scope, expected) = (self.clone(), header.metadata_generation);
            self.read(store, move |db| {
                let tx = db.savepoint()?;
                scope.check(&tx)?;
                ensure!(
                    super::metadata::generation(&tx, &scope.session)? == expected,
                    "session metadata changed during paged read"
                );
                tx.commit()?;
                Ok(())
            })?;
        }
        Ok(value)
    }

    /// Targeted tool/detail fields: do not load the other blocks or invisible
    /// protocol extensions in the same assistant entry just to read one call.
    pub(crate) fn field(
        &self,
        store: &SqliteStore,
        entry: &Entry,
        pointer: &str,
    ) -> Result<Option<Value>> {
        ensure!(
            entry.position <= self.until,
            "entry is beyond the captured transcript"
        );
        let (scope, locator, field) = (self.clone(), entry.clone(), pointer.to_owned());
        let reference=self.read(store, move |db| {
            let tx=db.savepoint()?;scope.check(&tx)?;
            let reference=tx.query_row("SELECT r.content_id,r.byte_offset,r.byte_length,r.encoding FROM content_references r JOIN entries e ON e.session_id=r.session_id AND e.entry_id=r.owner_id WHERE e.session_id=?1 AND e.position=?2 AND e.entry_id=?3 AND r.owner_kind='entry' AND r.field=?4",
                params![scope.session,locator.position,locator.id,field],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get::<_,String>(3)?))).optional()?;
            tx.commit()?;Ok(reference)
        })?;
        let value = if let Some((id, offset, length, encoding)) = reference {
            let reference = Reference {
                field: pointer.into(),
                range: ContentRange { id, offset, length },
                encoding: Encoding::parse(&encoding)?,
            };
            Some(decode(
                reference.encoding,
                self.body(store, entry, &reference)?,
            )?)
        } else if pointer == "/meta/shell_result" {
            let metadata = self
                .column(store, entry, Column::Metadata, None)?
                .context("entry metadata is missing")?;
            serde_json::from_slice::<Value>(&metadata)?
                .pointer(pointer)
                .cloned()
        } else if let Some(field) = pointer
            .strip_prefix("/content/")
            .filter(|field| !field.contains('/'))
        {
            self.column(store, entry, Column::Content, None)?
                .map(|raw| -> Result<Option<Value>> {
                    Ok(serde_json::from_slice::<Value>(&raw)?
                        .pointer(&format!("/{field}"))
                        .cloned())
                })
                .transpose()?
                .flatten()
        } else {
            let components = pointer.split('/').collect::<Vec<_>>();
            ensure!(
                components.len() == 4 && components[0].is_empty() && components[1] == "content",
                "unsupported targeted content field"
            );
            let ordinal: i64 = components[2].parse()?;
            ensure!(ordinal >= 0, "invalid block ordinal");
            let column = match components[3] {
                "args" => Column::BlockArguments(ordinal),
                "text" | "content" => Column::BlockText(ordinal),
                _ => anyhow::bail!("unsupported targeted block field"),
            };
            self.column(store, entry, column, None)?
                .map(|raw| -> Result<Value> {
                    if components[3] == "args" {
                        Ok(serde_json::from_slice(&raw)?)
                    } else {
                        Ok(json!(String::from_utf8(raw)?))
                    }
                })
                .transpose()?
        };
        self.verify(store)?;
        Ok(value)
    }

    fn column(
        &self,
        store: &SqliteStore,
        entry: &Entry,
        column: Column,
        metadata_generation: Option<i64>,
    ) -> Result<Option<Vec<u8>>> {
        let mut bytes = Vec::new();
        let mut expected = None;
        loop {
            let (scope, entry, offset) = (self.clone(), entry.clone(), i64::try_from(bytes.len())?);
            let (length,piece)=self.read(store, move |db| {
                let tx=db.savepoint()?;scope.check(&tx)?;
                let (expression,join,ordinal)=match column {
                    Column::Metadata=>("e.metadata_json","",None),
                    Column::Content=>("e.content_json","",None),
                    Column::SessionMetadata=>("s.current_metadata_json","JOIN sessions s ON s.id=e.session_id",None),
                    Column::BlockMetadata(ordinal)=>("b.metadata_json","JOIN message_blocks b ON b.session_id=e.session_id AND b.entry_position=e.position AND b.ordinal=?6",Some(ordinal)),
                    Column::BlockText(ordinal)=>("b.text","JOIN message_blocks b ON b.session_id=e.session_id AND b.entry_position=e.position AND b.ordinal=?6",Some(ordinal)),
                    Column::BlockArguments(ordinal)=>("b.arguments_json","JOIN message_blocks b ON b.session_id=e.session_id AND b.entry_position=e.position AND b.ordinal=?6",Some(ordinal)),
                };
                if let Some(expected)=metadata_generation {
                    ensure!(super::metadata::generation(&tx,&scope.session)?==expected,"session metadata changed during paged read");
                }
                let extra=if ordinal.is_some() {""} else {" AND ?6 IS NULL"};
                let sql=format!("SELECT length(CAST({expression} AS BLOB)),substr(CAST({expression} AS BLOB),?4+1,?5) FROM entries e {join} WHERE e.session_id=?1 AND e.position=?2 AND e.entry_id=?3 {extra}");
                let result:Option<(Option<i64>,Option<Vec<u8>>)>=tx.query_row(&sql,params![scope.session,entry.position,entry.id,offset,READ_BYTES as i64,ordinal],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
                let result=result.context("entry changed during metadata read")?;
                tx.commit()?;Ok(result)
            })?;
            let Some(length) = length else {
                ensure!(bytes.is_empty(), "metadata became null during read");
                return Ok(None);
            };
            ensure!(
                expected.is_none_or(|expected| expected == length),
                "metadata length changed during read"
            );
            expected = Some(length);
            if length == 0 {
                ensure!(bytes.is_empty(), "metadata became empty during read");
                return Ok(Some(bytes));
            }
            let piece = piece.context("metadata page is missing")?;
            ensure!(piece.len() <= READ_BYTES, "metadata page exceeds budget");
            if offset < length {
                ensure!(!piece.is_empty(), "metadata read made no progress");
            }
            bytes.try_reserve(piece.len())?;
            bytes.extend(piece);
            if i64::try_from(bytes.len())? == length {
                return Ok(Some(bytes));
            }
            ensure!(
                i64::try_from(bytes.len())? < length,
                "metadata page crossed its boundary"
            );
        }
    }

    fn blocks(&self, store: &SqliteStore, entry: &Entry) -> Result<Vec<Value>> {
        let mut after = -1;
        let mut blocks = Vec::new();
        loop {
            let (scope, locator) = (self.clone(), entry.clone());
            let rows=self.read(store, move |db| {
                let tx=db.savepoint()?;scope.check(&tx)?;
                let mut query=tx.prepare("SELECT b.ordinal,b.kind,b.tool_call_id,b.tool_name,b.is_error FROM message_blocks b JOIN entries e ON e.session_id=b.session_id AND e.position=b.entry_position WHERE e.session_id=?1 AND e.entry_id=?2 AND e.position=?3 AND b.ordinal>?4 ORDER BY b.ordinal LIMIT 64")?;
                let rows=query.query_map(params![scope.session,locator.id,locator.position,after],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,Option<String>>(1)?,row.get::<_,Option<String>>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,Option<bool>>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                ensure!(rows.iter().map(|(_,kind,id,name,_)|kind.as_ref().map_or(0,String::len)+id.as_ref().map_or(0,String::len)+name.as_ref().map_or(0,String::len)+40).sum::<usize>()<=READ_BYTES,"block identity page exceeds budget");
                drop(query);tx.commit()?;Ok(rows)
            })?;
            if rows.is_empty() {
                break;
            }
            for (ordinal, kind, id, name, error) in rows {
                ensure!(
                    ordinal == i64::try_from(blocks.len())?,
                    "message block order has a gap"
                );
                let raw = self
                    .column(store, entry, Column::BlockMetadata(ordinal), None)?
                    .context("block metadata is missing")?;
                let mut block: Value = serde_json::from_slice(&raw)?;
                if let Some(kind) = &kind {
                    block["type"] = json!(kind);
                }
                if let Some(text) = self.column(store, entry, Column::BlockText(ordinal), None)? {
                    block[if kind.as_deref() == Some("tool_result") {
                        "content"
                    } else {
                        "text"
                    }] = json!(String::from_utf8(text)?);
                }
                if let Some(id) = id {
                    block[if kind.as_deref() == Some("tool_result") {
                        "tool_call_id"
                    } else {
                        "id"
                    }] = json!(id);
                }
                if let Some(name) = name {
                    block["name"] = json!(name);
                }
                if let Some(args) =
                    self.column(store, entry, Column::BlockArguments(ordinal), None)?
                {
                    block["args"] = serde_json::from_slice(&args)?;
                }
                if let Some(error) = error {
                    block["is_error"] = json!(error);
                }
                blocks.push(block);
                after = ordinal;
            }
        }
        Ok(blocks)
    }

    fn references(
        &self,
        store: &SqliteStore,
        entry: &Entry,
        after: &str,
    ) -> Result<Vec<Reference>> {
        let (scope, entry, after) = (self.clone(), entry.clone(), after.to_owned());
        self.read(store, move |db| {
            let tx=db.savepoint()?;scope.check(&tx)?;
            let mut query=tx.prepare("SELECT r.field,r.content_id,r.byte_offset,r.byte_length,r.encoding FROM content_references r JOIN entries e ON e.session_id=r.session_id AND e.entry_id=r.owner_id WHERE r.session_id=?1 AND e.position=?2 AND e.entry_id=?3 AND r.owner_kind='entry' AND r.field>?4 ORDER BY r.field LIMIT 64")?;
            let mut cursor=query.query(params![scope.session,entry.position,entry.id,after])?;
            let mut rows=Vec::new();let mut bytes=0usize;
            while let Some(row)=cursor.next()? {
                let field:String=row.get(0)?;let size=field.len()+96;
                if !rows.is_empty() && bytes+size>READ_BYTES {break;}
                ensure!(size<=READ_BYTES,"content field identity exceeds page budget");
                let encoding:String=row.get(4)?;
                rows.push(Reference {field,range:ContentRange{id:row.get(1)?,offset:row.get(2)?,length:row.get(3)?},encoding:Encoding::parse(&encoding)?});bytes+=size;
            }
            drop(cursor);drop(query);tx.commit()?;Ok(rows)
        })
    }

    fn body(&self, store: &SqliteStore, entry: &Entry, reference: &Reference) -> Result<String> {
        let mut offset = reference.range.offset;
        let end = offset
            .checked_add(reference.range.length)
            .context("content range overflow")?;
        let mut body = String::new();
        loop {
            let piece = self.reference_page(
                store,
                entry,
                reference,
                offset,
                (end - offset).min(READ_BYTES as i64) as usize,
            )?;
            if offset == end {
                ensure!(
                    piece.text.is_empty() && piece.next_offset == offset,
                    "invalid empty content range"
                );
                break;
            }
            ensure!(
                piece.next_offset > offset && piece.next_offset <= end,
                "content read made invalid progress"
            );
            offset = piece.next_offset;
            body.try_reserve(piece.text.len())?;
            body.push_str(&piece.text);
            if offset == end {
                break;
            }
        }
        Ok(body)
    }
    fn reference_page(
        &self,
        store: &SqliteStore,
        entry: &Entry,
        reference: &Reference,
        offset: i64,
        limit: usize,
    ) -> Result<super::ContentPage> {
        let end = reference
            .range
            .offset
            .checked_add(reference.range.length)
            .context("content range overflow")?;
        ensure!(
            offset >= reference.range.offset
                && offset <= end
                && limit <= READ_BYTES
                && i64::try_from(limit)? <= end - offset,
            "content read exceeds its field range"
        );
        let (scope, entry, reference) = (self.clone(), entry.clone(), reference.clone());
        self.read(store, move |db| {
            let tx = db.savepoint()?;
            scope.check_reference(&tx, &entry, &reference)?;
            let piece = super::read(&tx, reference.range.id, offset, limit)?;
            tx.commit()?;
            Ok(piece)
        })
    }

    fn check_reference(&self, db: &Connection, entry: &Entry, reference: &Reference) -> Result<()> {
        self.check(db)?;
        let current:Option<(i64,i64,i64,String)>=db.query_row("SELECT r.content_id,r.byte_offset,r.byte_length,r.encoding FROM content_references r JOIN entries e ON e.session_id=r.session_id AND e.entry_id=r.owner_id WHERE r.session_id=?1 AND e.position=?2 AND e.entry_id=?3 AND r.owner_kind='entry' AND r.field=?4",
            params![self.session,entry.position,entry.id,reference.field],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
        ensure!(
            current
                == Some((
                    reference.range.id,
                    reference.range.offset,
                    reference.range.length,
                    reference.encoding.name().into()
                )),
            "entry content reference changed during read"
        );
        Ok(())
    }

    fn verify_reference(
        &self,
        store: &SqliteStore,
        entry: &Entry,
        reference: &Reference,
    ) -> Result<()> {
        let (scope, entry, reference) = (self.clone(), entry.clone(), reference.clone());
        self.read(store, move |db| {
            let tx = db.savepoint()?;
            scope.check_reference(&tx, &entry, &reference)?;
            tx.commit()?;
            Ok(())
        })
    }
}

fn display_field(value: &Value, pointer: &str) -> bool {
    if pointer == "/meta" || pointer.starts_with("/meta/") {
        return true;
    }
    if pointer == "/content" {
        return true;
    }
    let Some(path) = pointer.strip_prefix("/content/") else {
        return false;
    };
    if !value["content"].is_array() {
        return true;
    }
    let Some((ordinal, field)) = path.split_once('/') else {
        return true;
    };
    let Some(block) = ordinal
        .parse::<usize>()
        .ok()
        .and_then(|ordinal| value["content"].get(ordinal))
    else {
        return true;
    };
    match block["type"].as_str() {
        Some("text" | "reasoning") => matches!(field, "text" | "provider_metadata"),
        Some("tool_call") => matches!(field, "args" | "provider_metadata"),
        Some("tool_result") => matches!(field, "content" | "provider_metadata"),
        // An unrecognized/invalid image shape is itself the public data field.
        // Unknown future blocks also retain their complete opaque contents.
        _ => true,
    }
}

fn decode(encoding: Encoding, raw: String) -> Result<Value> {
    Ok(match encoding {
        Encoding::Text => json!(raw),
        Encoding::Json => serde_json::from_str(&raw)?,
        Encoding::ShellSummary => json!(future_rpc::shell_result::model_summary(
            &serde_json::from_str(&raw)?
        )),
        Encoding::ToolArgumentsStringV1 => {
            let mut value = json!(raw);
            future_rpc::tool_arguments::repair_string_v1(&mut value);
            value
        }
    })
}

fn generation(db: &Connection, session: &BoundIdentity) -> Result<i64> {
    Ok(db.query_row(
        "SELECT coalesce((SELECT generation FROM content_history_versions WHERE session_id=?1),0)",
        [session],
        |row| row.get(0),
    )?)
}

fn set_field(entry: &mut Value, pointer: &str, value: Value) -> Result<()> {
    if let Some(slot) = entry.pointer_mut(pointer) {
        *slot = value;
        return Ok(());
    }
    let (parent, leaf) = pointer
        .rsplit_once('/')
        .context("invalid entry field pointer")?;
    let object = entry
        .pointer_mut(parent)
        .and_then(Value::as_object_mut)
        .context("entry field has no owning object")?;
    object.insert(leaf.replace("~1", "/").replace("~0", "~"), value);
    Ok(())
}

#[cfg(test)]
#[path = "reader_tests.rs"]
mod tests;

#[path = "reader_recall.rs"]
mod recall;

#[path = "reader_summary.rs"]
mod summary;

#[path = "reader_intent.rs"]
mod intent;

#[path = "reader_derived.rs"]
mod derived;
