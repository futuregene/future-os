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

sql_record!(pub(super) MESSAGE_COLUMNS, message_from_row -> MessageRecord {
    id, thread_id, run_id, role, content_type, content, status, created_at, updated_at,
});

pub fn list_messages(_thread_id: &str) -> Result<Vec<MessageRecord>, crate::AppError> {
    // Messages now read from agent (get_session_entries RPC).
    Ok(vec![])
}

pub fn append_message(input: AppendMessageInput) -> Result<MessageRecord, crate::AppError> {
    // Messages are now persisted by the agent (session JSONL).
    // Return a dummy record so callers don't crash.
    let now = now_millis();
    Ok(MessageRecord {
        id: format!("msg_{now}"),
        thread_id: input.thread_id,
        run_id: input.run_id,
        role: input.role,
        content_type: input.content_type.unwrap_or_else(|| "markdown".to_string()),
        content: input.content,
        status: input.status.unwrap_or_else(|| "complete".to_string()),
        created_at: now,
        updated_at: now,
    })
}
