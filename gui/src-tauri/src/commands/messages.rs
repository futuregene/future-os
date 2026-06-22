//! Message Tauri commands.

use crate::store;

#[tauri::command]
pub fn list_messages(thread_id: String) -> Result<Vec<store::MessageRecord>, crate::AppError> {
    store::list_messages(&thread_id)
}

#[tauri::command]
pub fn append_message(
    input: store::AppendMessageInput,
) -> Result<store::MessageRecord, crate::AppError> {
    store::append_message(input)
}
