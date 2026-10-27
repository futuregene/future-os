//! Transactional semantic changes over canonical content, without token journals.
//! A producer may stage a large body's segments in several bounded commits;
//! only the commit publishing its change advances the durable stream cursor.
use super::{append, bind, create, resolve, seal, ContentDigest, ContentRange, WRITE_BYTES};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;

pub(crate) const CHANGE_METADATA_BYTES: usize = 16 * 1024;
pub(crate) const CHANGES_PER_BATCH: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StreamIdentity {
    pub session_id: String,
    pub run_id: String,
    pub epoch: i64,
}

pub(crate) struct BoundStreamIdentity {
    pub session_id: crate::session::identity::BoundIdentity,
    pub run_id: crate::session::identity::BoundIdentity,
    pub epoch: i64,
}

impl StreamIdentity {
    /// Resolve keys within the caller's transaction, never cache an allocation
    /// across commit/rollback. The producer's wire identity stays unchanged.
    pub(crate) fn bind(&self, db: &Connection) -> Result<BoundStreamIdentity> {
        Ok(BoundStreamIdentity {
            session_id: crate::session::identity::BoundIdentity::session(db, &self.session_id)?,
            run_id: crate::session::identity::BoundIdentity::run(
                db,
                &self.session_id,
                &self.run_id,
            )?,
            epoch: self.epoch,
        })
    }
}

#[derive(Clone)]
pub(crate) struct ContentAppend {
    pub key: String,
    pub offset: i64,
    pub text: String,
    pub seal: Option<ContentDigest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Encoding {
    Text,
    Json,
    ShellSummary,
    ToolArgumentsStringV1,
}
impl Encoding {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
            Self::ShellSummary => "shell_summary",
            Self::ToolArgumentsStringV1 => "tool_args_string_v1",
        }
    }
    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "shell_summary" => Ok(Self::ShellSummary),
            "tool_args_string_v1" => Ok(Self::ToolArgumentsStringV1),
            _ => anyhow::bail!("unknown canonical encoding"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BodyLink {
    pub field: String,
    pub encoding: Encoding,
    pub key: String,
    pub offset: i64,
    pub length: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct Change {
    pub first_cursor: i64,
    pub last_cursor: i64,
    pub event_type: String,
    pub metadata: Value,
    pub bodies: Vec<BodyLink>,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredChange {
    pub first_cursor: i64,
    pub last_cursor: i64,
    pub event_type: String,
    pub metadata: Value,
    pub bodies: Vec<(String, ContentRange, Encoding)>,
}

#[derive(Debug)]
pub(crate) struct ChangePage {
    pub changes: Vec<StoredChange>,
    pub durable_cursor: i64,
    pub has_more: bool,
}

pub(crate) fn owner(stream: &StreamIdentity, key: &str) -> String {
    serde_json::json!([stream.run_id, stream.epoch, key]).to_string()
}

fn stream_id(tx: &Transaction<'_>, stream: &StreamIdentity) -> Result<i64> {
    ensure!(
        super::format(tx, &stream.session_id)? == super::FORMAT_VERSION,
        "conversation does not use segmented storage"
    );
    let stream = stream.bind(tx)?;
    tx.execute(
        "INSERT INTO stream_runs(session_id,run_id,epoch) VALUES (?1,?2,?3) ON CONFLICT DO NOTHING",
        params![stream.session_id, stream.run_id, stream.epoch],
    )?;
    Ok(tx.query_row(
        "SELECT id FROM stream_runs WHERE session_id=?1 AND run_id=?2 AND epoch=?3 AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=stream_runs.id)",
        params![stream.session_id, stream.run_id, stream.epoch],
        |r| r.get(0),
    )?)
}

pub(crate) fn commit_batch(
    db: &mut Connection,
    stream: &StreamIdentity,
    appends: Vec<ContentAppend>,
    changes: Vec<Change>,
) -> Result<i64> {
    let bytes = appends.iter().try_fold(0usize, |sum, a| {
        sum.checked_add(a.text.len())
            .context("content batch size overflow")
    })?;
    ensure!(
        bytes <= WRITE_BYTES
            && changes.len() <= CHANGES_PER_BATCH
            && appends.len() <= CHANGES_PER_BATCH,
        "semantic write batch exceeds budget"
    );
    let metadata_bytes = changes.iter().try_fold(0usize, |sum, c| {
        sum.checked_add(c.metadata.to_string().len())
            .context("metadata size overflow")
    })?;
    ensure!(
        metadata_bytes <= CHANGE_METADATA_BYTES,
        "semantic metadata batch exceeds budget"
    );
    let tx = crate::session::database::begin_write_transaction(db)?;
    let id = stream_id(&tx, stream)?;
    let stored_session = crate::session::identity::session_key(&tx, &stream.session_id)?;
    let last: Option<(i64,String)> = tx.query_row(
        "SELECT last_cursor,event_type FROM stream_changes WHERE stream_id=?1 ORDER BY last_cursor DESC LIMIT 1",
        [id], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    let closed = last
        .filter(|(_, kind)| kind == "agent_end")
        .map(|(cursor, _)| cursor);
    if let Some(cursor) = closed {
        ensure!(
            changes.iter().all(|change| change.last_cursor <= cursor),
            "run stream is already terminal"
        );
        ensure!(
            !changes.is_empty() || appends.is_empty(),
            "terminal stream cannot stage new content"
        );
    }
    for piece in appends {
        let key = owner(stream, &piece.key);
        let prior = resolve(&tx, &stream.session_id, "stream", &key, "")?;
        if closed.is_some() {
            let end = piece
                .offset
                .checked_add(i64::try_from(piece.text.len())?)
                .context("content range overflow")?;
            ensure!(
                prior.is_some_and(|range| end <= range.length),
                "terminal stream cannot extend content"
            );
        }
        let body = match prior {
            Some(range) => range.id,
            None => create(&tx)?,
        };
        let range = append(&tx, body, piece.offset, &piece.text)?;
        let end = range
            .offset
            .checked_add(range.length)
            .context("content range overflow")?;
        if let Some(prior) = prior {
            // The stream owner grows with its durable body. Published change
            // and transcript references remain immutable independent ranges.
            let length = prior.length.max(end);
            tx.execute("UPDATE content_references SET byte_length=?5 WHERE session_id=?1 AND owner_kind='stream' AND owner_id=?2 AND field='' AND content_id=?3 AND byte_length=?4",params![stored_session,key,body,prior.length,length])?;
        } else {
            bind(
                &tx,
                &stream.session_id,
                "stream",
                &key,
                "",
                ContentRange {
                    id: body,
                    offset: 0,
                    length: end,
                },
            )?;
        }
        if let Some(digest) = piece.seal {
            seal(&tx, body, digest)?;
        }
    }
    let mut cursor: i64 = tx.query_row(
        "SELECT durable_cursor FROM stream_runs WHERE id=?1",
        [id],
        |r| r.get(0),
    )?;
    for change in changes {
        ensure!(
            change.first_cursor >= 0 && change.last_cursor >= change.first_cursor,
            "invalid change cursor range"
        );
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM stream_changes WHERE stream_id=?1 AND last_cursor=?2)",
            params![id, change.last_cursor],
            |r| r.get(0),
        )?;
        ensure!(
            exists || change.first_cursor == cursor + 1,
            "semantic change has a gap or conflicting overlap"
        );
        let mut bodies = Vec::new();
        for body in &change.bodies {
            ensure!(
                change.metadata.pointer(&body.field).is_some(),
                "content link does not name a metadata field"
            );
            let range = resolve(
                &tx,
                &stream.session_id,
                "stream",
                &owner(stream, &body.key),
                "",
            )?
            .context("semantic change references missing content")?;
            let end = body
                .offset
                .checked_add(body.length)
                .context("content link overflow")?;
            ensure!(
                body.offset >= 0 && body.length >= 0 && end <= range.length,
                "semantic content link exceeds committed body"
            );
            bodies.push((
                body.field.clone(),
                ContentRange {
                    id: range.id,
                    offset: body.offset,
                    length: body.length,
                },
                body.encoding,
            ));
        }
        super::compact::publish(&tx, id, &change, bodies)?;
        cursor = cursor.max(change.last_cursor);
    }
    tx.execute(
        "UPDATE stream_runs SET durable_cursor=?2 WHERE id=?1",
        params![id, cursor],
    )?;
    tx.commit()?;
    Ok(cursor)
}

#[derive(Debug)]
pub(crate) struct EncodedChangePage {
    changes: super::compact::EncodedPage,
    durable_cursor: i64,
    has_more: bool,
}
impl EncodedChangePage {
    pub(crate) fn decode(self) -> Result<ChangePage> {
        Ok(ChangePage {
            changes: self.changes.decode()?,
            durable_cursor: self.durable_cursor,
            has_more: self.has_more,
        })
    }
}

#[cfg(test)]
pub(crate) fn read_changes(
    db: &Connection,
    stream: &StreamIdentity,
    since: i64,
    limit: usize,
) -> Result<ChangePage> {
    read_encoded_changes(db, stream, since, limit)?.decode()
}

/// Metadata-only page, inside the caller's read transaction. Body transport
/// uses explicit bounded reads rather than hydrating a whole tool result here.
pub(crate) fn read_encoded_changes(
    db: &Connection,
    stream: &StreamIdentity,
    since: i64,
    limit: usize,
) -> Result<EncodedChangePage> {
    ensure!((1..=256).contains(&limit), "invalid change page limit");
    let stream = stream.bind(db)?;
    let row: Option<(i64,i64)>=db.query_row("SELECT id,durable_cursor FROM stream_runs WHERE session_id=?1 AND run_id=?2 AND epoch=?3 AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=stream_runs.id)",params![stream.session_id,stream.run_id,stream.epoch],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((id, cursor)) = row else {
        return Ok(EncodedChangePage {
            changes: super::compact::EncodedPage::default(),
            durable_cursor: -1,
            has_more: false,
        });
    };
    let mut stmt=db.prepare("SELECT c.id,length(CAST(c.metadata_json AS BLOB))+length(CAST(x.descriptor_json AS BLOB))+x.symbol_bytes FROM stream_changes c LEFT JOIN stream_contexts x ON x.id=c.context_id AND x.stream_id=c.stream_id AND x.event_kind=c.event_kind WHERE c.stream_id=?1 AND c.last_cursor>?2 ORDER BY c.last_cursor LIMIT ?3")?;
    let mut rows = stmt.query(params![id, since, i64::try_from(limit + 1)?])?;
    let mut ids = Vec::new();
    let mut bytes = 0usize;
    let mut more = false;
    while let Some(row) = rows.next()? {
        let size = usize::try_from(row.get::<_, i64>(1)?)?;
        if ids.len() == limit
            || (!ids.is_empty() && bytes.saturating_add(size) > CHANGE_METADATA_BYTES)
        {
            more = true;
            break;
        }
        bytes = bytes.saturating_add(size);
        ids.push(row.get::<_, i64>(0)?);
    }
    let changes = super::compact::read_encoded(db, &ids)?;
    Ok(EncodedChangePage {
        changes,
        durable_cursor: cursor,
        has_more: more,
    })
}

/// Recovery/control boundary published in the transcript owner's transaction.
/// There is no body payload and no independent cursor reservation.
pub(crate) fn append_boundary(
    tx: &Transaction<'_>,
    stream: &StreamIdentity,
    event_type: &str,
    metadata: Value,
) -> Result<i64> {
    ensure!(
        metadata.to_string().len() <= CHANGE_METADATA_BYTES,
        "boundary metadata exceeds budget"
    );
    let id = stream_id(tx, stream)?;
    let cursor: i64 = tx.query_row(
        "SELECT durable_cursor+1 FROM stream_runs WHERE id=?1",
        [id],
        |row| row.get(0),
    )?;
    super::compact::publish(
        tx,
        id,
        &Change {
            first_cursor: cursor,
            last_cursor: cursor,
            event_type: event_type.into(),
            metadata,
            bodies: Vec::new(),
        },
        Vec::new(),
    )?;
    tx.execute(
        "UPDATE stream_runs SET durable_cursor=?2 WHERE id=?1",
        params![id, cursor],
    )?;
    Ok(cursor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::content::{enable, read, ContentDigest};

    fn db() -> Connection {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE sessions(id TEXT PRIMARY KEY,revision INTEGER NOT NULL DEFAULT 0); CREATE TABLE entries(session_id TEXT,entry_id TEXT,UNIQUE(session_id,entry_id)); CREATE TABLE compaction_operations(session_id TEXT,input_key TEXT,PRIMARY KEY(session_id,input_key)); CREATE TABLE runs(session_id TEXT,run_id TEXT,PRIMARY KEY(session_id,run_id)); CREATE TABLE queued_run_receipts(session_id TEXT,run_id TEXT,PRIMARY KEY(session_id,run_id)); INSERT INTO sessions(id) VALUES('s');").unwrap();
        db.execute_batch(include_str!("schema.sql")).unwrap();
        let tx = db.transaction().unwrap();
        enable(&tx, "s").unwrap();
        tx.commit().unwrap();
        db
    }
    fn identity() -> StreamIdentity {
        StreamIdentity {
            session_id: "s".into(),
            run_id: "r".into(),
            epoch: 1,
        }
    }
    fn change(first: i64, last: i64, length: i64) -> Change {
        Change {
            first_cursor: first,
            last_cursor: last,
            event_type: "text_chunk".into(),
            metadata: serde_json::json!({"text":""}),
            bodies: vec![BodyLink {
                field: "/text".into(),
                encoding: Encoding::Text,
                key: "block".into(),
                offset: 0,
                length,
            }],
        }
    }

    #[test]
    fn canonical_ranges_commit_with_watermark_and_retry_without_duplication() {
        let mut db = db();
        let mut hash = ContentDigest::default();
        hash.push("你好").unwrap();
        let append = || ContentAppend {
            key: "block".into(),
            offset: 0,
            text: "你好".into(),
            seal: Some(hash.clone()),
        };
        assert_eq!(
            commit_batch(&mut db, &identity(), vec![append()], vec![change(0, 31, 6)]).unwrap(),
            31
        );
        assert_eq!(
            commit_batch(&mut db, &identity(), vec![append()], vec![change(0, 31, 6)]).unwrap(),
            31
        );
        let page = read_changes(&db, &identity(), -1, 10).unwrap();
        assert_eq!(page.durable_cursor, 31);
        assert_eq!(page.changes.len(), 1);
        let range = page.changes[0].bodies[0].1;
        assert_eq!(
            read(&db, range.id, range.offset, range.length as usize)
                .unwrap()
                .text,
            "你好"
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM content_segments", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(commit_batch(&mut db, &identity(), vec![], vec![change(33, 40, 6)]).is_err());
        assert_eq!(
            read_changes(&db, &identity(), 31, 10)
                .unwrap()
                .durable_cursor,
            31
        );
    }

    #[test]
    fn staging_large_bodies_cannot_publish_a_cursor_before_their_change() {
        let mut db = db();
        commit_batch(
            &mut db,
            &identity(),
            vec![ContentAppend {
                key: "block".into(),
                offset: 0,
                text: "first".into(),
                seal: None,
            }],
            vec![],
        )
        .unwrap();
        assert_eq!(
            read_changes(&db, &identity(), -1, 10)
                .unwrap()
                .durable_cursor,
            -1
        );
        // The invalid reference rolls back both the attempted append and change.
        assert!(commit_batch(
            &mut db,
            &identity(),
            vec![ContentAppend {
                key: "block".into(),
                offset: 5,
                text: " tail".into(),
                seal: None
            }],
            vec![change(0, 1, 99)]
        )
        .is_err());
        let range = resolve(&db, "s", "stream", &owner(&identity(), "block"), "")
            .unwrap()
            .unwrap();
        assert_eq!(range.length, 5);
        let mut other = identity();
        other.session_id = "someone-else".into();
        assert!(read_changes(&db, &other, -1, 10)
            .unwrap()
            .changes
            .is_empty());
    }
}
