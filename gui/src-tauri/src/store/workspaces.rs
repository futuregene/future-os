use rusqlite::{params, OptionalExtension};
use std::fs;

use super::initialize_app_store;
use super::records::*;
use super::support::*;

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
