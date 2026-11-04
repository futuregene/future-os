//! Drop the derived display projections.
//!
//! `history_display` and `history_shapes` are not data: they are the rendered
//! shape of each entry, rebuilt from `entries` by `ensure_history_index`
//! whenever `history_index_state` says they are not ready - which is exactly what
//! the delete triggers make true. Loading a session goes through
//! `with_history_index`, so the rebuild happens on the next open, not on a
//! background timer and not at seed time.
//!
//! On a real store the two tables are 168 MiB of duplicates. Purging them is a
//! cache eviction with a first-open cost, not a loss; this pass only runs when an
//! operator asks for it, and it never touches `entries`.

use anyhow::Result;
use rusqlite::{params, Connection};

/// Rows removed per table per transaction.
const PURGE_CHUNK: i64 = 4_096;

/// What the projections hold right now: `(rows, payload bytes)`.
pub(crate) fn plan(db: &Connection) -> Result<(i64, i64)> {
    Ok(db.query_row(
        "SELECT (SELECT count(*) FROM history_display)+(SELECT count(*) FROM history_shapes),
                (SELECT coalesce(sum(length(CAST(payload AS BLOB))),0) FROM history_display)
              + (SELECT coalesce(sum(length(CAST(payload AS BLOB))),0) FROM history_shapes)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}

/// Delete the projections of up to `limit` sessions, largest first.
///
/// Returns `(rows deleted, bytes deleted)`. The triggers on both tables clear
/// `history_index_state` for every session touched, so a partially purged session
/// is rebuilt from its entries rather than read as a gap.
pub(crate) fn apply(db: &mut Connection, limit: i64) -> Result<(i64, i64)> {
    // The stored session key is bound verbatim: older sessions store text, the
    // ones the desktop created store raw bytes, and both must reach their rows.
    // `String` refuses the second kind, which is how this pass once failed 473
    // times on a real store while reporting nothing.
    let measured: Vec<(rusqlite::types::Value, i64)> = {
        let mut statement = db.prepare_cached(
            "SELECT session_id,sum(b) FROM (
                 SELECT session_id,coalesce(sum(length(CAST(payload AS BLOB))),0) b
                   FROM history_shapes GROUP BY session_id
                 UNION ALL
                 SELECT session_id,coalesce(sum(length(CAST(payload AS BLOB))),0) b
                   FROM history_display GROUP BY session_id)
              GROUP BY session_id ORDER BY 2 DESC LIMIT ?1",
        )?;
        let rows = statement
            .query_map(params![limit], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let (mut deleted, mut bytes) = (0i64, 0i64);
    for (session, size) in measured {
        let removed = crate::session::database::try_maintenance_transaction(db, |tx| {
            let mut removed = 0i64;
            loop {
                let slice = tx.execute(
                    "DELETE FROM history_display WHERE (session_id,ordinal) IN
                       (SELECT session_id,ordinal FROM history_display WHERE session_id=?1 LIMIT ?2)",
                    params![session, PURGE_CHUNK],
                )?;
                removed += slice as i64;
                if slice == 0 {
                    break;
                }
            }
            loop {
                let slice = tx.execute(
                    "DELETE FROM history_shapes WHERE (session_id,position) IN
                       (SELECT session_id,position FROM history_shapes WHERE session_id=?1 LIMIT ?2)",
                    params![session, PURGE_CHUNK],
                )?;
                removed += slice as i64;
                if slice == 0 {
                    break;
                }
            }
            Ok(removed)
        })?;
        if removed > 0 {
            deleted += removed;
            bytes += size;
        }
    }
    Ok((deleted, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::sqlite_store::SqliteStore;
    use serde_json::json;

    /// Dropping the projection is an eviction: the next load rebuilds it from the
    /// entries and lands on exactly what was there before.
    #[test]
    fn purging_the_projection_rebuilds_it_from_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
        store.reserve_segmented("s").unwrap();
        store
            .replace(
                "s",
                vec![
                    json!({"id":"u1","type":"user","timestamp":"2026-01-01T00:00:00Z",
                           "content":[{"type":"text","text":"hello"}]}),
                    json!({"id":"a1","type":"assistant","timestamp":"2026-01-01T00:00:01Z",
                           "content":[{"type":"text","text":"hi there"}]}),
                ],
            )
            .unwrap();
        let snapshot = |db: &mut Connection| -> anyhow::Result<(i64, String)> {
            Ok((
                db.query_row("SELECT count(*) FROM history_display", [], |r| r.get(0))?,
                db.query_row(
                    "SELECT payload FROM history_display WHERE ordinal=0",
                    [],
                    |r| r.get(0),
                )?,
            ))
        };
        let before = store.db.call(snapshot).unwrap();
        assert!(before.0 >= 2, "the projection exists to drop");

        let (rows, bytes) = store.db.call(|db| apply(db, 8)).unwrap();
        assert!(rows >= 2);
        assert!(bytes > 0);
        assert_eq!(
            store
                .db
                .call(|db| Ok(db
                    .query_row("SELECT count(*) FROM history_display", [], |r| r
                        .get::<_, i64>(0))?))
                .unwrap(),
            0
        );

        store
            .db
            .call(|db| {
                let tx = db.transaction()?;
                crate::session::history::ensure_history_index(&tx, "s")?;
                tx.commit()?;
                Ok(())
            })
            .unwrap();
        assert_eq!(
            store.db.call(snapshot).unwrap(),
            before,
            "the rebuild reproduces the projection"
        );
    }
}

#[cfg(test)]
mod blob_key_tests {
    use super::*;
    use crate::session::sqlite_store::SqliteStore;
    use rusqlite::params;

    /// A desktop-created session stores its id as raw bytes. Binding that key as
    /// text is refused by `String`, which made this pass report success while
    /// deleting nothing on a real store.
    #[test]
    fn the_purge_reaches_a_session_whose_key_is_stored_as_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
        let key: Vec<u8> = (0u8..16).collect();
        store
            .db
            .call(move |db| {
                db.execute("INSERT INTO sessions(id) VALUES (?1)", params![key])?;
                db.execute(
                    "INSERT INTO history_display(session_id,ordinal,source_position,is_user,payload)
                     VALUES (?1,0,0,0,'{}')",
                    params![key],
                )?;
                Ok(())
            })
            .unwrap();
        let (rows, _bytes) = store.db.call(|db| apply(db, 4)).unwrap();
        assert_eq!(rows, 1, "the row belongs to the byte-keyed session");
        let left: i64 = store
            .db
            .call(|db| {
                Ok(db.query_row(
                    "SELECT (SELECT count(*) FROM history_display)+(SELECT count(*) FROM history_shapes)",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(left, 0);
    }
}
