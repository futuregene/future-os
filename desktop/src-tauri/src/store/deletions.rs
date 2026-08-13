//! Durable GUI-side delivery queue for idempotent Agent session deletion.

use rusqlite::{params, Connection};

use super::db::connect;
use super::util::now_millis;

pub(super) fn enqueue_agent_session_delete_in(
    conn: &Connection,
    session_id: &str,
) -> rusqlite::Result<()> {
    if session_id.trim().is_empty() {
        return Ok(());
    }
    const SQL: &str =
        "INSERT INTO agent_delete_outbox(session_id, requested_at, attempts, last_error)
         VALUES (?1, ?2, 0, NULL)
         ON CONFLICT(session_id) DO NOTHING";
    conn.execute(SQL, params![session_id, now_millis()])?;
    Ok(())
}

/// A tombstone is an admission fence for GUI import/discovery.  It remains
/// until the Agent acknowledges deletion, so a temporarily offline Agent can
/// never cause a locally deleted session to reappear in the sidebar.
pub fn is_agent_session_tombstoned(session_id: &str) -> Result<bool, crate::AppError> {
    const SQL: &str = "SELECT EXISTS(SELECT 1 FROM agent_delete_outbox WHERE session_id = ?1)";
    let conn = connect()?;
    Ok(conn.query_row(SQL, [session_id], |row| row.get(0))?)
}

pub fn pending_agent_session_deletes() -> Result<Vec<String>, crate::AppError> {
    let conn = connect()?;
    let mut stmt =
        conn.prepare("SELECT session_id FROM agent_delete_outbox ORDER BY requested_at")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    let session_ids = rows.collect::<Result<Vec<String>, _>>()?;
    Ok(session_ids)
}

pub fn acknowledge_agent_session_delete(session_id: &str) -> Result<(), crate::AppError> {
    let conn = connect()?;
    conn.execute(
        "DELETE FROM agent_delete_outbox WHERE session_id = ?1",
        [session_id],
    )?;
    Ok(())
}

pub fn note_agent_session_delete_failure(
    session_id: &str,
    error: &str,
) -> Result<(), crate::AppError> {
    let conn = connect()?;
    conn.execute(
        "UPDATE agent_delete_outbox SET attempts = attempts + 1, last_error = ?2 WHERE session_id = ?1",
        params![session_id, error],
    )?;
    Ok(())
}

/// Preserve deletion intent across Debug ▸ Reset.  The GUI database may be
/// cleared, but Agent-owned canonical sessions must still be reclaimed.
pub(super) fn enqueue_all_agent_session_deletes_in(conn: &Connection) -> rusqlite::Result<()> {
    let now = now_millis();
    // INSERT OR IGNORE, not `ON CONFLICT DO NOTHING`: the upsert clause is a
    // syntax error on INSERT…SELECT in SQLite's grammar (the ON reads as a
    // join constraint), and this path failing would break Debug ▸ Reset.
    const SQL: &str =
        "INSERT OR IGNORE INTO agent_delete_outbox(session_id, requested_at, attempts, last_error)
         SELECT DISTINCT COALESCE(NULLIF(TRIM(agent_session_id), ''), id), ?1, 0, NULL
         FROM threads";
    conn.execute(SQL, [now])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::test_support::{guarded_conn, memory_conn};

    #[test]
    fn enqueue_ignores_blank_and_duplicate_session_ids() {
        let conn = memory_conn();
        enqueue_agent_session_delete_in(&conn, "   ").expect("blank is a no-op");
        enqueue_agent_session_delete_in(&conn, "sess_1").expect("enqueue");
        enqueue_agent_session_delete_in(&conn, "sess_1").expect("conflict tolerated");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM agent_delete_outbox", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(count, 1);
    }

    #[test]
    fn outbox_lifecycle_against_the_real_db() {
        let (_home, conn) = guarded_conn("deletions_lifecycle");
        enqueue_agent_session_delete_in(&conn, "sess_b").expect("enqueue b");
        std::thread::sleep(std::time::Duration::from_millis(2));
        enqueue_agent_session_delete_in(&conn, "sess_a").expect("enqueue a");
        drop(conn);

        assert!(is_agent_session_tombstoned("sess_a").expect("tombstoned"));
        assert!(!is_agent_session_tombstoned("sess_z").expect("not tombstoned"));

        // Pending deletes come out in request order.
        assert_eq!(
            pending_agent_session_deletes().expect("pending"),
            vec!["sess_b".to_string(), "sess_a".to_string()]
        );

        note_agent_session_delete_failure("sess_a", "offline").expect("note failure");
        assert!(
            is_agent_session_tombstoned("sess_a").expect("still tombstoned"),
            "a failed delivery keeps the tombstone"
        );

        acknowledge_agent_session_delete("sess_a").expect("ack");
        assert!(!is_agent_session_tombstoned("sess_a").expect("acknowledged"));
        assert_eq!(
            pending_agent_session_deletes().expect("pending"),
            vec!["sess_b".to_string()]
        );
    }

    #[test]
    fn failure_notes_accumulate_attempts_and_the_error() {
        let (_home, conn) = guarded_conn("deletions_failure");
        enqueue_agent_session_delete_in(&conn, "sess_f").expect("enqueue");
        note_agent_session_delete_failure("sess_f", "first").expect("note 1");
        note_agent_session_delete_failure("sess_f", "second").expect("note 2");

        let (attempts, last_error): (i64, String) = conn
            .query_row(
                "SELECT attempts, last_error FROM agent_delete_outbox WHERE session_id = 'sess_f'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("load outbox row");
        assert_eq!(attempts, 2);
        assert_eq!(last_error, "second");
    }

    #[test]
    fn enqueue_all_collects_distinct_session_ids_with_thread_id_fallback() {
        let conn = memory_conn();
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, agent_session_id, created_at, updated_at
             ) VALUES
                 ('t1', 'ws1', 'chat', 'T1', 'sess_1', 1, 1),
                 ('t2', 'ws1', 'chat', 'T2', 'sess_1', 1, 1),
                 ('t3', 'ws1', 'chat', 'T3', '  ', 1, 1),
                 ('t4', 'ws1', 'chat', 'T4', NULL, 1, 1);",
        )
        .expect("seed threads");

        enqueue_all_agent_session_deletes_in(&conn).expect("enqueue all");

        let mut stmt = conn
            .prepare("SELECT session_id FROM agent_delete_outbox ORDER BY session_id")
            .expect("prepare");
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect");
        // sess_1 deduped across t1/t2; blank and NULL session ids fall back to
        // the thread id.
        assert_eq!(ids, vec!["sess_1", "t3", "t4"]);
    }
}
