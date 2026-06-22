use rusqlite::{params, OptionalExtension};

use super::initialize_app_store;
use super::records::*;
use super::support::*;
use super::workspaces::{get_or_create_chat_workspace, get_workspace};

pub fn list_threads() -> Result<Vec<ThreadRecord>, String> {
    initialize_app_store()?;
    let conn = connect()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, workspace_id, mode, title, status, pinned, readonly,
                    model_provider, model_id, agent_session_id, last_message_at,
                    last_opened_at, created_at, updated_at, archived_at, deleted_at
             FROM threads
             WHERE status != 'deleted'
             ORDER BY pinned DESC, COALESCE(last_message_at, updated_at) DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map([], thread_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

pub fn get_recent_thread() -> Result<Option<ThreadRecord>, String> {
    initialize_app_store()?;
    let conn = connect()?;
    conn.query_row(
        "SELECT id, workspace_id, mode, title, status, pinned, readonly,
                model_provider, model_id, agent_session_id, last_message_at,
                last_opened_at, created_at, updated_at, archived_at, deleted_at
         FROM threads
         WHERE status = 'active'
         ORDER BY COALESCE(last_opened_at, last_message_at, updated_at) DESC
         LIMIT 1",
        [],
        thread_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub fn create_thread(input: CreateThreadInput) -> Result<ThreadRecord, String> {
    initialize_app_store()?;
    let mode = normalize_mode(&input.mode)?;
    let now = now_millis();
    let thread_id = create_id("thread");
    let agent_session_id = create_id("agent_session");
    let title = input.title.unwrap_or_else(|| {
        if mode == "chat" {
            "New Chat".to_string()
        } else {
            "Workspace Thread".to_string()
        }
    });

    let workspace = if mode == "chat" {
        get_or_create_chat_workspace(&thread_id, Some(title.clone()))?
    } else if let Some(workspace_id) = input.workspace_id {
        get_workspace(&workspace_id)?.ok_or_else(|| "Workspace could not be loaded.".to_string())?
    } else {
        let raw_path = input
            .workspace_path
            .ok_or_else(|| "workspacePath is required for workspace threads.".to_string())?;
        let path = expand_tilde(&raw_path)?;
        let name = input
            .workspace_name
            .unwrap_or_else(|| workspace_name_from_path(&path));
        get_or_create_user_workspace(name, path, None)?
    };

    let conn = connect()?;
    conn.execute(
        "INSERT INTO threads (
             id, workspace_id, mode, title, status, pinned, readonly,
             model_provider, model_id, agent_session_id, last_opened_at,
             created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, 'active', 0, 0, ?5, ?6, ?7, ?8, ?8, ?8)",
        params![
            thread_id,
            workspace.id,
            mode,
            title,
            input.model_provider,
            input.model_id,
            agent_session_id,
            now
        ],
    )
    .map_err(|error| error.to_string())?;

    get_thread(&thread_id)?.ok_or_else(|| "Created thread could not be loaded.".to_string())
}

pub fn get_thread(thread_id: &str) -> Result<Option<ThreadRecord>, String> {
    initialize_app_store()?;
    let conn = connect()?;
    conn.query_row(
        "SELECT id, workspace_id, mode, title, status, pinned, readonly,
                model_provider, model_id, agent_session_id, last_message_at,
                last_opened_at, created_at, updated_at, archived_at, deleted_at
         FROM threads
         WHERE id = ?1",
        params![thread_id],
        thread_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub fn rename_thread(input: RenameThreadInput) -> Result<ThreadRecord, String> {
    initialize_app_store()?;
    let title = input.title.trim();
    if title.is_empty() {
        return Err("title cannot be empty.".to_string());
    }

    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "UPDATE threads
         SET title = ?1, updated_at = ?2
         WHERE id = ?3 AND status != 'deleted'",
        params![title, now, input.thread_id],
    )
    .map_err(|error| error.to_string())?;

    get_thread(&input.thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())
}

pub fn update_thread_model(input: UpdateThreadModelInput) -> Result<ThreadRecord, String> {
    initialize_app_store()?;
    let model_id = input.model_id.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    let model_provider = input.model_provider.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "UPDATE threads
         SET model_provider = ?1, model_id = ?2, updated_at = ?3
         WHERE id = ?4 AND status != 'deleted'",
        params![model_provider, model_id, now, input.thread_id],
    )
    .map_err(|error| error.to_string())?;

    get_thread(&input.thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())
}

pub fn pin_thread(input: PinThreadInput) -> Result<ThreadRecord, String> {
    initialize_app_store()?;
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "UPDATE threads
         SET pinned = ?1, updated_at = ?2
         WHERE id = ?3 AND status != 'deleted'",
        params![if input.pinned { 1 } else { 0 }, now, input.thread_id],
    )
    .map_err(|error| error.to_string())?;

    get_thread(&input.thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())
}

pub fn archive_thread(thread_id: &str) -> Result<ThreadRecord, String> {
    update_thread_status(thread_id, "archived")
}

pub fn restore_thread(thread_id: &str) -> Result<ThreadRecord, String> {
    update_thread_status(thread_id, "active")
}

pub fn delete_thread(thread_id: &str) -> Result<ThreadRecord, String> {
    initialize_app_store()?;
    let now = now_millis();
    let conn = connect()?;
    let thread = get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    conn.execute(
        "UPDATE threads
         SET status = 'deleted', deleted_at = ?1, updated_at = ?1
         WHERE id = ?2 AND status != 'deleted'",
        params![now, thread_id],
    )
    .map_err(|error| error.to_string())?;

    if thread.mode == "chat" {
        conn.execute(
            "UPDATE workspaces
             SET cleanup_status = 'pending_cleanup',
                 cleanup_requested_at = COALESCE(cleanup_requested_at, ?1),
                 updated_at = ?1
             WHERE id = ?2
               AND kind = 'temporary'
               AND cleanup_status = 'active'",
            params![now, thread.workspace_id],
        )
        .map_err(|error| error.to_string())?;
    }

    get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())
}
