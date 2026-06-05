mod artifacts;
mod cleanup;
mod models;
mod research;
mod review;
mod schema;
mod support;

use rusqlite::{params, OptionalExtension};
use std::fs;

pub use artifacts::{
    create_artifact, delete_artifact, ensure_artifact, import_attachment_artifact, list_artifacts,
};
pub use cleanup::get_thread_cleanup_summary;
pub use models::*;
pub use research::{list_research_resources, promote_artifact_to_research};
pub use review::{
    decide_approval_request, ensure_approval_request, ensure_review_change, list_approval_requests,
    list_review_changesets, list_review_file_changes,
};
pub use support::get_approval_request;
use support::*;

pub fn app_data_path() -> Result<AppDataPath, String> {
    Ok(AppDataPath {
        app_dir: app_dir()?.display().to_string(),
        db_path: db_path()?.display().to_string(),
    })
}

pub fn initialize_app_store() -> Result<(), String> {
    ensure_app_dirs()?;
    let conn = connect()?;
    run_migrations(&conn)?;
    Ok(())
}

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

pub fn list_workspaces() -> Result<Vec<WorkspaceRecord>, String> {
    initialize_app_store()?;
    let conn = connect()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, kind, path, description, cleanup_status,
                    cleanup_requested_at, cleaned_at, last_opened_at,
                    created_at, updated_at, deleted_at
             FROM workspaces
             WHERE deleted_at IS NULL
             ORDER BY COALESCE(last_opened_at, updated_at) DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map([], workspace_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

pub fn create_workspace(input: CreateWorkspaceInput) -> Result<WorkspaceRecord, String> {
    initialize_app_store()?;
    let path = expand_tilde(&input.path)?;
    if input.create_directory.unwrap_or(false) {
        fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    } else if !path.is_dir() {
        return Err(format!(
            "Workspace path does not exist or is not a directory: {}",
            path.display()
        ));
    }

    let name = input
        .name
        .unwrap_or_else(|| workspace_name_from_path(&path));
    get_or_create_user_workspace(name, path, input.description)
}

pub fn get_or_create_chat_workspace(
    thread_id: &str,
    title: Option<String>,
) -> Result<WorkspaceRecord, String> {
    initialize_app_store()?;
    let conn = connect()?;
    let existing = conn
        .query_row(
            "SELECT id, name, kind, path, description, cleanup_status,
                    cleanup_requested_at, cleaned_at, last_opened_at,
                    created_at, updated_at, deleted_at
             FROM workspaces
             WHERE kind = 'temporary' AND path = ?1 AND deleted_at IS NULL
             LIMIT 1",
            params![chat_workspace_path(thread_id)?.display().to_string()],
            workspace_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())?;

    if let Some(workspace) = existing {
        return Ok(workspace);
    }

    let path = chat_workspace_path(thread_id)?;
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    let now = now_millis();
    let workspace_id = create_id("ws");
    let name = format!(
        "{} Workspace",
        title.unwrap_or_else(|| "New Chat".to_string())
    );
    conn.execute(
        "INSERT INTO workspaces (
             id, name, kind, path, cleanup_status, created_at, updated_at
         ) VALUES (?1, ?2, 'temporary', ?3, 'active', ?4, ?4)",
        params![workspace_id, name, path.display().to_string(), now],
    )
    .map_err(|error| error.to_string())?;

    get_workspace(&workspace_id)?
        .ok_or_else(|| "Created workspace could not be loaded.".to_string())
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

pub fn get_workspace(workspace_id: &str) -> Result<Option<WorkspaceRecord>, String> {
    initialize_app_store()?;
    let conn = connect()?;
    conn.query_row(
        "SELECT id, name, kind, path, description, cleanup_status,
                cleanup_requested_at, cleaned_at, last_opened_at,
                created_at, updated_at, deleted_at
         FROM workspaces
         WHERE id = ?1",
        params![workspace_id],
        workspace_from_row,
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

    get_message(&id)?.ok_or_else(|| "Created message could not be loaded.".to_string())
}

pub fn create_run(input: CreateRunInput) -> Result<RunRecord, String> {
    initialize_app_store()?;
    let id = create_id("run");
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "INSERT INTO runs (
             id, thread_id, trigger_message_id, status, model_provider, model_id,
             started_at, created_at, updated_at
         ) VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?6, ?6)",
        params![
            id,
            input.thread_id,
            input.trigger_message_id,
            input.model_provider,
            input.model_id,
            now
        ],
    )
    .map_err(|error| error.to_string())?;
    get_run(&id)?.ok_or_else(|| "Created run could not be loaded.".to_string())
}

pub fn list_runs(thread_id: &str) -> Result<Vec<RunRecord>, String> {
    initialize_app_store()?;
    let conn = connect()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, thread_id, trigger_message_id, status, model_provider, model_id,
                    started_at, ended_at, error_message, created_at, updated_at
             FROM runs
             WHERE thread_id = ?1
             ORDER BY created_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![thread_id], run_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

pub fn update_run_status(input: UpdateRunStatusInput) -> Result<RunRecord, String> {
    initialize_app_store()?;
    let now = now_millis();
    let ended_at = if matches!(input.status.as_str(), "completed" | "failed" | "cancelled") {
        Some(now)
    } else {
        None
    };
    let conn = connect()?;
    conn.execute(
        "UPDATE runs
         SET status = ?1, error_message = ?2, ended_at = COALESCE(?3, ended_at), updated_at = ?4
         WHERE id = ?5",
        params![
            input.status,
            input.error_message,
            ended_at,
            now,
            input.run_id
        ],
    )
    .map_err(|error| error.to_string())?;
    get_run(&input.run_id)?.ok_or_else(|| "Updated run could not be loaded.".to_string())
}

pub fn list_run_events(run_id: &str) -> Result<Vec<RunEventRecord>, String> {
    initialize_app_store()?;
    let conn = connect()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, run_id, type, payload, sequence, created_at
             FROM run_events
             WHERE run_id = ?1
             ORDER BY sequence ASC, created_at ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![run_id], run_event_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

pub fn append_run_event(input: AppendRunEventInput) -> Result<RunEventRecord, String> {
    initialize_app_store()?;
    let id = create_id("event");
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "INSERT INTO run_events (id, run_id, type, payload, sequence, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            input.run_id,
            input.event_type,
            input.payload,
            input.sequence,
            now
        ],
    )
    .map_err(|error| error.to_string())?;

    get_run_event(&id)?.ok_or_else(|| "Created run event could not be loaded.".to_string())
}

pub fn list_tool_calls(run_id: &str) -> Result<Vec<ToolCallRecord>, String> {
    initialize_app_store()?;
    let conn = connect()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, run_id, name, kind, input, status, started_at, ended_at, created_at
             FROM tool_calls
             WHERE run_id = ?1
             ORDER BY COALESCE(started_at, created_at) ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![run_id], tool_call_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

pub fn list_tool_outputs(tool_call_id: &str) -> Result<Vec<ToolOutputRecord>, String> {
    initialize_app_store()?;
    let conn = connect()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, tool_call_id, kind, content, created_at
             FROM tool_outputs
             WHERE tool_call_id = ?1
             ORDER BY created_at ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![tool_call_id], tool_output_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

pub fn upsert_tool_call(input: UpsertToolCallInput) -> Result<(), String> {
    initialize_app_store()?;
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "INSERT INTO tool_calls (
             id, run_id, name, kind, input, status, started_at, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             kind = excluded.kind,
             input = COALESCE(excluded.input, tool_calls.input),
             status = excluded.status,
             started_at = COALESCE(tool_calls.started_at, excluded.started_at)",
        params![
            input.tool_call_id,
            input.run_id,
            input.name,
            input.kind,
            input.input,
            input.status,
            now
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn complete_tool_call(input: CompleteToolCallInput) -> Result<(), String> {
    initialize_app_store()?;
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "INSERT INTO tool_calls (
             id, run_id, name, kind, status, started_at, ended_at, created_at
         ) VALUES (?1, ?2, ?3, 'agent_tool', ?4, ?5, ?5, ?5)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             status = excluded.status,
             ended_at = excluded.ended_at",
        params![
            input.tool_call_id,
            input.run_id,
            input.name,
            input.status,
            now
        ],
    )
    .map_err(|error| error.to_string())?;

    conn.execute(
        "INSERT INTO tool_outputs (id, tool_call_id, kind, content, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            create_id("toolout"),
            input.tool_call_id,
            input.output_kind,
            input.output_content,
            now
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}
