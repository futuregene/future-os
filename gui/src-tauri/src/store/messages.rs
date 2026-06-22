use rusqlite::params;

use super::initialize_app_store;
use super::markdown_refs::sync_message_markdown_references;
use super::records::*;
use super::support::*;

pub fn list_messages(thread_id: &str) -> Result<Vec<MessageRecord>, String> {
    initialize_app_store()?;
    let conn = connect()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, thread_id, run_id, role, content_type, content, status, created_at, updated_at
             FROM messages
             WHERE thread_id = ?1
             ORDER BY created_at ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![thread_id], message_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

pub fn append_message(input: AppendMessageInput) -> Result<MessageRecord, String> {
    initialize_app_store()?;
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
    )
    .map_err(|error| error.to_string())?;
    conn.execute(
        "UPDATE threads
         SET last_message_at = ?1, last_opened_at = ?1, updated_at = ?1
         WHERE id = ?2",
        params![now, input.thread_id],
    )
    .map_err(|error| error.to_string())?;
    let _ = sync_message_markdown_references(&conn, &id, &input.thread_id, &input.content);

    get_message(&id)?.ok_or_else(|| "Created message could not be loaded.".to_string())
}
