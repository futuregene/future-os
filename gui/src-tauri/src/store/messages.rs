use rusqlite::params;
use serde::Serialize;

use super::db::*;
use super::markdown_refs::sync_message_markdown_references;
use super::records::*;
use super::util::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRecord {
    pub id: String,
    pub thread_id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub content_type: String,
    pub content: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Column list for `message_from_row`, in struct order.
pub(super) const MESSAGE_COLUMNS: &str =
    "id, thread_id, run_id, role, content_type, content, status, created_at, updated_at";

pub(super) fn message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRecord> {
    Ok(MessageRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        run_id: row.get(2)?,
        role: row.get(3)?,
        content_type: row.get(4)?,
        content: row.get(5)?,
        status: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

pub fn list_messages(thread_id: &str) -> Result<Vec<MessageRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {MESSAGE_COLUMNS}
             FROM messages
             WHERE thread_id = ?1
             ORDER BY created_at ASC"
    ))?;
    let rows = stmt.query_map(params![thread_id], message_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn append_message(input: AppendMessageInput) -> Result<MessageRecord, crate::AppError> {
    let id = create_id("msg");
    let now = now_millis();
    let content_type = input.content_type.unwrap_or_else(|| "markdown".to_string());
    let status = input.status.unwrap_or_else(|| "complete".to_string());
    let mut conn = connect()?;
    // The message insert and the thread bump are one logical write — commit them
    // atomically so a crash can't leave a message without its `last_message_at`.
    let tx = conn.transaction()?;
    tx.execute(
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
    tx.execute(
        "UPDATE threads
         SET last_message_at = ?1, last_opened_at = ?1, updated_at = ?1
         WHERE id = ?2",
        params![now, input.thread_id],
    )?;
    tx.commit()?;

    // The reference index is best-effort: a failure must not lose the message,
    // but log it (search / `futureos://` resolution can lag until the next edit)
    // rather than dropping it silently.
    if let Err(error) =
        sync_message_markdown_references(&conn, &id, &input.thread_id, &input.content)
    {
        eprintln!("FutureOS message reference sync failed for {id}: {error}");
    }

    loaded(get_message(&id)?, "Created message")
}
