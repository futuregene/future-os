//! Mutable staging bounds over immutable segments. The persisted owner is a
//! stream/key integer pair, sharing the semantic dictionary; public content
//! keys remain unchanged. Published event/entry ranges remain independent.
use super::{
    stream::{Encoding, StreamIdentity},
    ContentRange,
};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

/// Compatibility for internal callers preparing one named field. This JSON is
/// transient input only; it is never written into an ownership table/index.
pub(super) fn decode(session: &str, owner: &str) -> Result<(StreamIdentity, String)> {
    let (run_id, epoch, key): (String, i64, String) =
        serde_json::from_str(owner).context("invalid stream body identity")?;
    ensure!(
        key.len() <= super::stream::CHANGE_METADATA_BYTES,
        "stream body key exceeds byte budget"
    );
    Ok((
        StreamIdentity {
            session_id: session.into(),
            run_id,
            epoch,
        },
        key,
    ))
}

pub(super) fn get(
    db: &Connection,
    stream: &StreamIdentity,
    key: &str,
) -> Result<Option<(ContentRange, Encoding)>> {
    let scope = stream.bind(db)?;
    let raw: Option<(i64, i64, i64, String)> = db
        .query_row(
            "SELECT b.content_id,b.byte_offset,b.byte_length,b.encoding
        FROM stream_runs r JOIN stream_symbols s ON s.stream_id=r.id
        JOIN stream_body_owners b ON b.stream_id=r.id AND b.key_id=s.id
        WHERE r.session_id=?1 AND r.run_id=?2 AND r.epoch=?3 AND s.value=?4 COLLATE BINARY
        AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=r.id)",
            params![
                scope.session_id,
                scope.run_id,
                scope.epoch,
                super::symbols::key(key)
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    raw.map(|(id, offset, length, encoding)| {
        Ok((
            ContentRange { id, offset, length },
            Encoding::parse(&encoding)?,
        ))
    })
    .transpose()
}

fn closed(db: &Connection, stream: i64) -> Result<bool> {
    Ok(db.query_row("SELECT coalesce((SELECT event_kind=2 FROM stream_changes WHERE stream_id=?1 ORDER BY last_cursor DESC LIMIT 1),0)",[stream],|r|r.get(0))?)
}

pub(super) fn bind(
    tx: &Transaction<'_>,
    stream: &StreamIdentity,
    key: &str,
    range: ContentRange,
    encoding: Encoding,
) -> Result<()> {
    let stream = super::stream::stream_id(tx, stream)?;
    let key = super::symbols::intern_value(tx, stream, key)?;
    let old:Option<(i64,i64,i64,String)>=tx.query_row("SELECT content_id,byte_offset,byte_length,encoding FROM stream_body_owners WHERE stream_id=?1 AND key_id=?2",params![stream,key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    if let Some(old) = old {
        ensure!(
            old == (
                range.id,
                range.offset,
                range.length,
                encoding.name().to_owned()
            ),
            "conflicting stream body reference"
        );
    } else {
        ensure!(
            !closed(tx, stream)?,
            "terminal stream cannot stage a new content owner"
        );
        tx.execute("INSERT INTO stream_body_owners(stream_id,key_id,content_id,byte_offset,byte_length,encoding) VALUES (?1,?2,?3,?4,?5,?6)",params![stream,key,range.id,range.offset,range.length,encoding.name()])?;
    }
    Ok(())
}

pub(super) fn extend(
    tx: &Transaction<'_>,
    stream: &StreamIdentity,
    key: &str,
    body: i64,
    next: i64,
) -> Result<()> {
    let scope = stream.bind(tx)?;
    let current:Option<(i64,i64,i64)>=tx.query_row("SELECT r.id,s.id,b.byte_length FROM stream_runs r
        JOIN stream_symbols s ON s.stream_id=r.id JOIN stream_body_owners b ON b.stream_id=r.id AND b.key_id=s.id
        WHERE r.session_id=?1 AND r.run_id=?2 AND r.epoch=?3 AND s.value=?4 COLLATE BINARY AND b.content_id=?5
        AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=r.id)",params![scope.session_id,scope.run_id,scope.epoch,super::symbols::key(key),body],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let (stream, key, length) = current.context("staged stream body owner changed")?;
    ensure!(
        next <= length || !closed(tx, stream)?,
        "terminal stream cannot extend content"
    );
    let changed=tx.execute("UPDATE stream_body_owners SET byte_length=max(byte_length,?4) WHERE stream_id=?1 AND key_id=?2 AND content_id=?3",params![stream,key,body,next])?;
    ensure!(changed == 1, "staged stream body owner changed");
    Ok(())
}
