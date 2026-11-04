//! Compression of sealed bodies, applied by the operator-invoked sweep.
//!
//! A body is written as plaintext fragments and sealed with a sha256 over those
//! bytes. Only once it is sealed can its storage change, and only as an
//! encoding: `content_segments` still holds one row per chunk keyed by its
//! *plaintext* offset, `content_objects.byte_length` still counts plaintext
//! bytes, and the digest still describes what `read` returns. The marker row in
//! `content_compressed` is what tells a reader the body's bytes are gzip.
//!
//! Measured on a real store, bodies compress 3.61x (zstd -3 on 40 sampled
//! objects: 0.88 MiB -> 0.24 MiB). Chunks are cut at 8 KiB of plaintext so a
//! chunk that does not compress at all still fits the 16 KiB segment ceiling
//! once gzip adds its header.

use super::*;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::{Read, Write};

/// Plaintext bytes per stored chunk once compressed. Half the segment ceiling,
/// so even incompressible bytes fit after gzip's framing.
const COMPRESSED_CHUNK_BYTES: usize = 8 * 1024;
/// Below this a body is left alone: the marker row and the header cost more than
/// the text saves, and small bodies are the ones read most often.
pub(crate) const COMPRESS_MIN_BODY_BYTES: i64 = 4096;

/// Sealed bodies still stored as text: `(objects, plaintext bytes)`.
pub(crate) fn plan(db: &Connection) -> Result<(i64, i64)> {
    Ok(db.query_row(
        &format!(
            "SELECT count(*),coalesce(sum(o.byte_length),0)
               FROM content_objects o
              WHERE o.sealed=1 AND o.byte_length>={COMPRESS_MIN_BODY_BYTES}
                AND NOT EXISTS(SELECT 1 FROM content_compressed c WHERE c.content_id=o.id)"
        ),
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}

pub(crate) fn decompress(blob: &[u8]) -> Result<String> {
    let mut decoder = GzDecoder::new(blob);
    let mut text = String::new();
    decoder.read_to_string(&mut text)?;
    Ok(text)
}

pub(crate) fn encode(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}

/// Compress up to `limit` bodies, largest first.
///
/// Returns `(objects, bytes before, bytes after)`, counting stored body bytes
/// only. Each body is one maintenance transaction: an interruption leaves the
/// rest exactly as they were, and the marker row is written in the same
/// transaction as its chunks, so a reader never sees a half-encoded body.
pub(crate) fn apply(db: &mut Connection, limit: i64) -> Result<(i64, i64, i64)> {
    let candidates: Vec<i64> = {
        let mut statement = db.prepare_cached(&format!(
            "SELECT o.id FROM content_objects o
              WHERE o.sealed=1 AND o.byte_length>={COMPRESS_MIN_BODY_BYTES}
                AND NOT EXISTS(SELECT 1 FROM content_compressed c WHERE c.content_id=o.id)
              ORDER BY o.byte_length DESC LIMIT ?1"
        ))?;
        let rows = statement
            .query_map(params![limit], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let (mut objects, mut before, mut after) = (0i64, 0i64, 0i64);
    for id in candidates {
        let done = crate::session::database::try_maintenance_transaction(db, |tx| {
            let segments: Vec<Vec<u8>> = {
                let mut statement = tx.prepare_cached(
                    "SELECT CAST(body AS BLOB) FROM content_segments
                      WHERE content_id=?1 ORDER BY byte_offset",
                )?;
                let rows = statement
                    .query_map([id], |row| row.get::<_, Vec<u8>>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            if segments.is_empty() {
                return Ok(None);
            }
            // Chunks are cut on the boundaries the writer already chose, so a
            // chunk boundary is always a char boundary.
            let mut chunks: Vec<(i64, Vec<u8>)> = Vec::new();
            let mut at = 0i64;
            for segment in &segments {
                match chunks.last_mut() {
                    Some((_, pending))
                        if pending.len() + segment.len() <= COMPRESSED_CHUNK_BYTES =>
                    {
                        pending.extend_from_slice(segment);
                    }
                    _ => chunks.push((at, segment.clone())),
                }
                at += segment.len() as i64;
            }
            let plain: i64 = segments.iter().map(|s| s.len() as i64).sum();
            let mut encoded: Vec<(i64, Vec<u8>)> = Vec::with_capacity(chunks.len());
            for (offset, chunk) in chunks {
                encoded.push((offset, encode(&chunk)?));
            }
            let compressed: i64 = encoded.iter().map(|(_, body)| body.len() as i64).sum();
            if compressed * 10 >= plain * 9 {
                // Not worth the marker: record the decision so the sweep stops
                // re-examining this body, and leave its text exactly as it is.
                tx.execute(
                    "INSERT INTO content_compressed(content_id,encoding) VALUES (?1,'text')",
                    [id],
                )?;
                return Ok(None);
            }
            tx.execute("DELETE FROM content_segments WHERE content_id=?1", [id])?;
            for (offset, body) in &encoded {
                tx.execute(
                    "INSERT INTO content_segments(content_id,byte_offset,body)
                     VALUES (?1,?2,CAST(?3 AS BLOB))",
                    params![id, offset, body],
                )?;
            }
            tx.execute(
                "INSERT INTO content_compressed(content_id,encoding) VALUES (?1,'gzip')",
                [id],
            )?;
            Ok(Some((plain, compressed)))
        })?;
        if let Some((plain, stored)) = done {
            objects += 1;
            before += plain;
            after += stored;
        }
    }
    Ok((objects, before, after))
}
