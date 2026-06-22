use rusqlite::params;

use super::db::*;
use super::markdown_refs::sync_message_markdown_references;
use super::records::*;
use super::util::*;

pub fn list_messages(thread_id: &str) -> Result<Vec<MessageRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(
        "SELECT id, thread_id, run_id, role, content_type, content, status, created_at, updated_at
             FROM messages
             WHERE thread_id = ?1
             ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![thread_id], message_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn append_message(input: AppendMessageInput) -> Result<MessageRecord, crate::AppError> {
    let id = create_id("msg");
    let now = now_millis();
    let content_type = input.content_type.unwrap_or_else(|| "markdown".to_string());
    let status = input.status.unwrap_or_else(|| "complete".to_string());
    let conn = connect()?;
    conn.execute(
        "INSERT INTO messages (
             id, thread_id, run_id, role, content_type, content, status, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            id,
            input.thread_id,
            input.run_id,
            input.role,
            content_type,
            input.content,
            status,
            now
        ],
    )?;
    conn.execute(
        "UPDATE threads
         SET last_message_at = ?1, last_opened_at = ?1, updated_at = ?1
         WHERE id = ?2",
        params![now, input.thread_id],
    )?;
    let _ = sync_message_markdown_references(&conn, &id, &input.thread_id, &input.content);

    get_message(&id)?.ok_or_else(|| "Created message could not be loaded.".to_string().into())
}
