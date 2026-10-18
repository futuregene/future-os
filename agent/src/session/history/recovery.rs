//! Disposable projections are rebuilt per conversation, never by rewriting
//! authoritative entries or replaying a run. Read bodies in bounded batches.
use anyhow::Result;
use rusqlite::{params, Connection};

pub(crate) fn mark_history_index_ready(db: &Connection, session: &str) -> Result<()> {
    db.execute(
        "INSERT INTO history_index_state(session_id,projection_version)
        SELECT id,1 FROM sessions WHERE id=?1
        ON CONFLICT(session_id) DO UPDATE SET projection_version=excluded.projection_version",
        [session],
    )?;
    Ok(())
}

/// Caller owns a transaction, so readers cannot observe a partial rebuild.
pub(crate) fn ensure_history_index(db: &Connection, session: &str) -> Result<()> {
    let ready: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM history_index_state WHERE session_id=?1 AND projection_version=1)",
        [session], |row| row.get(0),
    )?;
    if ready {
        return Ok(());
    }
    db.execute("DELETE FROM history_shapes WHERE session_id=?1", [session])?;
    db.execute("DELETE FROM history_display WHERE session_id=?1", [session])?;
    let mut position = -1;
    loop {
        let rows: Vec<(i64, String)> = {
            let mut query = db.prepare("SELECT position,payload FROM entry_records WHERE session_id=?1 AND position>?2 ORDER BY position LIMIT 64")?;
            let rows = query
                .query_map(params![session, position], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        if rows.is_empty() {
            break;
        }
        for (next, raw) in rows {
            super::insert_entry_shape(db, session, next, &serde_json::from_str(&raw)?)?;
            position = next;
        }
    }
    super::update_display_index_from(db, session, 0)?;
    mark_history_index_ready(db, session)
}

#[cfg(test)]
mod tests {
    use crate::session::{Manager, Session, SessionEntry};
    use serde_json::json;

    #[test]
    fn incomplete_projection_rebuilds_without_changing_authoritative_rows() {
        let root = tempfile::tempdir().unwrap();
        let manager = Manager::new(root.path().join("sessions"));
        let mut session = Session::new(".", "mock");
        session
            .entries
            .push(SessionEntry::new_user("user", json!("历史中文🙂")));
        session
            .entries
            .push(SessionEntry::new_assistant(json!("answer"), vec![]));
        manager.save(&session).unwrap();
        let before = manager.storage().unwrap().entries(&session.id).unwrap();
        let expected = manager
            .history_page(&session.id, None, None, Some(100))
            .unwrap();
        let id = session.id.clone();
        manager
            .storage()
            .unwrap()
            .db
            .call(move |db| {
                db.execute("DELETE FROM history_shapes WHERE session_id=?1", [&id])?;
                db.execute(
                    "DELETE FROM history_display WHERE session_id=?1 AND ordinal=0",
                    [&id],
                )?;
                Ok(())
            })
            .unwrap();
        assert_eq!(
            manager.storage_health().unwrap()["pendingHistoryIndexes"],
            1
        );
        assert_eq!(
            manager
                .history_page(&session.id, None, None, Some(100))
                .unwrap(),
            expected
        );
        assert_eq!(
            manager.storage().unwrap().entries(&session.id).unwrap(),
            before
        );
        assert_eq!(
            manager.storage_health().unwrap()["pendingHistoryIndexes"],
            0
        );
    }
}
