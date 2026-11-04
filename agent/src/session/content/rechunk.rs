//! Maintenance: one row per body instead of one row per fragment.
//!
//! `content_segments` is written one append at a time, so a body that arrived as
//! a token stream keeps a row per fragment: on a real store 2,396,179 rows held
//! 179 MiB of text, and the row overhead (every row carries its `content_id`,
//! its `byte_offset` and a btree slot) was another 107 MiB.
//!
//! Nothing addresses those rows. Every reader asks for a byte range of a body
//! (`mod.rs::read`, `transport::load`) and walks the segments that start inside
//! it, so replacing N contiguous segments with one row that starts at the same
//! offset and holds the same bytes changes no address and no byte. That is the
//! whole operation: it is a repack, not a rewrite, and it only runs when the
//! operator asks for it.

use super::*;

/// One row may hold exactly one segment's worth of body bytes; the schema caps a
/// segment at 16 KiB, so an object whose body is larger keeps its fragments.
pub(crate) const RECHUNK_MAX_BODY_BYTES: i64 = 16_384;

/// Bodies stored as more than one segment that a repack can collapse:
/// `(objects, rows)`.
pub(crate) fn plan(db: &Connection) -> Result<(i64, i64)> {
    Ok(db.query_row(
        &format!(
            "SELECT count(*),coalesce(sum(n),0) FROM (
                 SELECT content_id,count(*) n
                   FROM content_segments GROUP BY content_id
                  HAVING count(*)>1 AND sum(length(CAST(body AS BLOB)))<={RECHUNK_MAX_BODY_BYTES})"
        ),
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}

/// Repack up to `limit` bodies, largest fragment count first.
///
/// Returns `(objects, rows before, rows after)`. Each batch commits on its own,
/// so an interruption leaves the remaining bodies exactly as they were and the
/// next pass continues; running it twice repacks nothing.
pub(crate) fn apply(db: &mut Connection, limit: i64) -> Result<(i64, i64, i64)> {
    let candidates: Vec<i64> = {
        let mut statement = db.prepare_cached(&format!(
            "SELECT content_id FROM content_segments GROUP BY content_id
              HAVING count(*)>1 AND sum(length(CAST(body AS BLOB)))<={RECHUNK_MAX_BODY_BYTES}
              ORDER BY count(*) DESC LIMIT ?1"
        ))?;
        let rows = statement
            .query_map(params![limit], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let (mut objects, mut before, mut after) = (0i64, 0i64, 0i64);
    for id in candidates {
        let repacked = crate::session::database::try_maintenance_transaction(db, |tx| {
            let mut statement = tx.prepare_cached(
                "SELECT CAST(body AS BLOB) FROM content_segments
                  WHERE content_id=?1 ORDER BY byte_offset",
            )?;
            let segments = statement
                .query_map([id], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if segments.len() < 2 {
                return Ok(None);
            }
            let mut body = Vec::new();
            for segment in &segments {
                body.extend_from_slice(segment);
            }
            tx.execute("DELETE FROM content_segments WHERE content_id=?1", [id])?;
            tx.execute(
                "INSERT INTO content_segments(content_id,byte_offset,body) VALUES (?1,0,CAST(?2 AS TEXT))",
                params![id, body],
            )?;
            Ok(Some((segments.len() as i64, 1i64)))
        })?;
        if let Some((rows_before, rows_after)) = repacked {
            objects += 1;
            before += rows_before;
            after += rows_after;
        }
    }
    Ok((objects, before, after))
}
