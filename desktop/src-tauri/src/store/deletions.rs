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
    conn.execute(
        "INSERT INTO agent_delete_outbox(session_id, requested_at, attempts, last_error)
         VALUES (?1, ?2, 0, NULL)
         ON CONFLICT(session_id) DO NOTHING",
        params![session_id, now_millis()],
    )?;
    Ok(())
}

/// A tombstone is an admission fence for GUI import/discovery.  It remains
/// until the Agent acknowledges deletion, so a temporarily offline Agent can
/// never cause a locally deleted session to reappear in the sidebar.
pub fn is_agent_session_tombstoned(session_id: &str) -> Result<bool, crate::AppError> {
    let conn = connect()?;
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_delete_outbox WHERE session_id = ?1)",
        [session_id],
        |row| row.get(0),
    )?)
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
    // NOTE: `ON CONFLICT` after `INSERT ... SELECT` requires a `WHERE` clause —
    // without one SQLite parses `ON CONFLICT` as a table alias and fails with
    // "near DO: syntax error" (verified on 3.46 and 3.51).
    conn.execute(
        "INSERT INTO agent_delete_outbox(session_id, requested_at, attempts, last_error)
         SELECT DISTINCT COALESCE(NULLIF(TRIM(agent_session_id), ''), id), ?1, 0, NULL
         FROM threads
         WHERE TRUE
         ON CONFLICT(session_id) DO NOTHING",
        [now],
    )?;
    Ok(())
}
