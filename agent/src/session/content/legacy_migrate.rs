//! Convert a legacy session into the canonical body store.
//!
//! A session's entries are the truth; how they are stored is a detail. This
//! conversion reads each entry the way the projections already present it
//! (`entry_records.payload`) and writes the session back through the same writer
//! a new session uses.
//!
//! The writer stages content before it rewrites entries, and staging refuses to
//! publish content for an entry that is still in `entries` ("published entry
//! content is immutable"). The conversion therefore does what an explicit history
//! reset does, in one transaction, before the writer runs: it records the entries
//! it is about to replace, marks the session canonical, and drops the session's
//! rows. `replace` then rebuilds them from the recorded payloads - one code path
//! for blocks, references and envelopes instead of moving bytes by hand.
//!
//! Crash safety: the recorded payloads live in `legacy_conversions` until the
//! rewrite commits, and the startup recovery re-runs any leftover row. A process
//! that dies mid-way leaves a session that the next start finishes, never one
//! with its entries gone.
//!
//! What changes for the user: a converted session replays its transcript, not its
//! token stream - the product decision already applied to canonical sessions.

use super::*;
use crate::session::sqlite_store::SqliteStore;
use serde_json::Value;

/// Legacy sessions still holding block bodies: `(sessions, bodies, bytes)`.
pub(crate) fn plan(db: &Connection) -> Result<(i64, i64, i64)> {
    Ok(db.query_row(
        "SELECT count(DISTINCT session_id),count(*),coalesce(sum(length(body)),0)
           FROM message_block_bodies",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?)
}

/// Convert up to `limit` sessions, largest first.
///
/// Returns `(sessions converted, entries rewritten, bodies cleared)`.
pub(crate) fn apply(store: &SqliteStore, limit: i64) -> Result<(i64, i64, i64)> {
    let sessions: Vec<String> = store.db.call(move |db| {
        let mut statement = db.prepare_cached(
            "SELECT session_id FROM message_block_bodies
              GROUP BY session_id ORDER BY sum(length(body)) DESC LIMIT ?1",
        )?;
        let rows = statement
            .query_map(params![limit], |row| {
                row.get::<_, crate::session::identity::BoundIdentity>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().map(|id| id.to_string()).collect())
    })?;
    let (mut converted, mut rewritten, mut cleared) = (0i64, 0i64, 0i64);
    for session in sessions {
        match convert_one(store, &session) {
            Ok(Some((entries, bodies))) => {
                converted += 1;
                rewritten += entries;
                cleared += bodies;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, %session, "legacy session conversion failed; recovery keeps it recoverable");
            }
        }
    }
    Ok((converted, rewritten, cleared))
}

/// Finish a conversion a previous process started: the payloads are on disk, the
/// entries are not. Re-running the writer is the whole recovery.
pub(crate) fn recover(store: &SqliteStore, limit: i64) -> Result<usize> {
    let pending: Vec<String> = store.db.call(move |db| {
        let mut statement = db.prepare_cached(
            "SELECT session_id FROM legacy_conversions ORDER BY started_at_ms LIMIT ?1",
        )?;
        let rows = statement
            .query_map(params![limit], |row| {
                row.get::<_, crate::session::identity::BoundIdentity>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().map(|id| id.to_string()).collect())
    })?;
    let mut finished = 0usize;
    for session in pending {
        if let Err(error) = resume(store, &session) {
            tracing::warn!(%error, %session, "legacy conversion recovery failed; will retry on the next start");
            continue;
        }
        finished += 1;
    }
    Ok(finished)
}

fn convert_one(store: &SqliteStore, session: &str) -> Result<Option<(i64, i64)>> {
    let payloads = read_payloads(store, session)?;
    if payloads.is_empty() {
        return Ok(None);
    }
    if !reserve(store, session, &payloads)? {
        return Ok(None);
    }
    finish(store, session, payloads)
}

fn resume(store: &SqliteStore, session: &str) -> Result<()> {
    let payloads = read_recorded_payloads(store, session)?;
    if payloads.is_empty() {
        clear(store, session)?;
        return Ok(());
    }
    finish(store, session, payloads).map(|_| ())
}

fn read_payloads(store: &SqliteStore, session: &str) -> Result<Vec<Value>> {
    let session = session.to_owned();
    store.db.call(move |db| {
        let key = crate::session::identity::session_key(db, &session)?;
        let mut statement =
            db.prepare_cached("SELECT payload FROM entry_records WHERE session_id=?1 ORDER BY position")?;
        let rows = statement
            .query_map([&key], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|raw| serde_json::from_str::<Value>(&raw))
            .collect::<std::result::Result<Vec<_>, _>>()?)
    })
}

fn read_recorded_payloads(store: &SqliteStore, session: &str) -> Result<Vec<Value>> {
    let session = session.to_owned();
    store.db.call(move |db| {
        let key = crate::session::identity::session_key(db, &session)?;
        let raw: Option<String> = db
            .query_row(
                "SELECT payloads FROM legacy_conversions WHERE session_id=?1",
                [&key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(match raw {
            Some(raw) => serde_json::from_str::<Vec<Value>>(&raw)?,
            None => Vec::new(),
        })
    })
}

/// Record the payloads, mark the session canonical and drop what the writer is
/// about to rebuild - all in one transaction, so a crash can only ever leave the
/// payloads behind, never a session without them.
fn reserve(store: &SqliteStore, session: &str, payloads: &[Value]) -> Result<bool> {
    let session = session.to_owned();
    let recorded = serde_json::to_string(payloads)?;
    store.db.call_as("legacy_convert", move |db| {
        let key = crate::session::identity::session_key(db, &session)?;
        let tx = crate::session::database::begin_write_transaction(db)?;
        tx.execute(
            "INSERT INTO legacy_conversions(session_id,payloads,started_at_ms)
             VALUES (?1,?2,CAST(strftime('%s','now') AS INTEGER)*1000)
             ON CONFLICT(session_id) DO UPDATE SET payloads=excluded.payloads",
            params![&key, recorded],
        )?;
        tx.execute(
            "INSERT INTO session_formats(session_id,format) VALUES (?1,2)
             ON CONFLICT(session_id) DO NOTHING",
            [&key],
        )?;
        for table in [
            "entries",
            "message_blocks",
            "message_block_bodies",
            "content_references",
            "history_shapes",
            "history_display",
        ] {
            let sql = format!("DELETE FROM {table} WHERE session_id=?1");
            tx.execute(&sql, [&key])?;
        }
        tx.execute("DELETE FROM history_index_state WHERE session_id=?1", [&key])?;
        tx.commit()?;
        Ok(true)
    })
}

fn finish(store: &SqliteStore, session: &str, payloads: Vec<Value>) -> Result<Option<(i64, i64)>> {
    let entries = payloads.len() as i64;
    store.replace(session, payloads)?;
    let bodies = clear(store, session)?;
    Ok(Some((entries, bodies)))
}

fn clear(store: &SqliteStore, session: &str) -> Result<i64> {
    let session = session.to_owned();
    store.db.call_as("legacy_convert", move |db| {
        let key = crate::session::identity::session_key(db, &session)?;
        let bodies: i64 = db.query_row(
            "SELECT count(*) FROM message_block_bodies WHERE session_id=?1",
            [&key],
            |row| row.get(0),
        )?;
        db.execute("DELETE FROM legacy_conversions WHERE session_id=?1", [&key])?;
        Ok(bodies)
    })
}
