//! Durable GUI-side delivery queue for idempotent Agent session deletion.

use rusqlite::params;

use super::db::connect;
use super::util::now_millis;

pub fn enqueue_agent_session_delete(session_id: &str) -> Result<(), crate::AppError> {
    if session_id.trim().is_empty() {
        return Ok(());
    }
    let conn = connect()?;
    conn.execute(
        "INSERT INTO agent_delete_outbox(session_id, requested_at, attempts, last_error)
         VALUES (?1, ?2, 0, NULL)
         ON CONFLICT(session_id) DO NOTHING",
        params![session_id, now_millis()],
    )?;
    Ok(())
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
