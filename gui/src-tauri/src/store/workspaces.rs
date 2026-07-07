use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::fs;

use super::db::*;
use super::records::*;
use super::util::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub path: String,
    pub description: Option<String>,
    pub cleanup_status: String,
    pub cleanup_requested_at: Option<i64>,
    pub cleaned_at: Option<i64>,
    pub last_opened_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

sql_record!(pub(super) WORKSPACE_COLUMNS, workspace_from_row -> WorkspaceRecord {
    id, name, kind, path, description, cleanup_status, cleanup_requested_at,
    cleaned_at, last_opened_at, created_at, updated_at, deleted_at,
});

pub fn list_workspaces() -> Result<Vec<WorkspaceRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {WORKSPACE_COLUMNS}
             FROM workspaces
             WHERE deleted_at IS NULL
             ORDER BY COALESCE(last_opened_at, updated_at) DESC"
    ))?;
    let rows = stmt.query_map([], workspace_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn create_workspace(input: CreateWorkspaceInput) -> Result<WorkspaceRecord, crate::AppError> {
    let path = expand_tilde(&input.path)?;
    if input.create_directory.unwrap_or(false) {
        fs::create_dir_all(&path)?;
    } else if !path.is_dir() {
        return Err(format!(
            "Workspace path does not exist or is not a directory: {}",
            path.display()
        )
        .into());
    }

    let name = input
        .name
        .unwrap_or_else(|| workspace_name_from_path(&path));
    get_or_create_user_workspace(name, path, input.description)
}

pub fn get_or_create_chat_workspace(
    thread_id: &str,
    title: Option<String>,
) -> Result<WorkspaceRecord, crate::AppError> {
    let conn = connect()?;
    get_or_create_chat_workspace_in(&conn, thread_id, title)
}

/// Connection-injecting variant so a composite write (e.g. `create_thread`) can
/// resolve/create the workspace and insert its own row in one transaction.
pub(super) fn get_or_create_chat_workspace_in(
    conn: &Connection,
    thread_id: &str,
    title: Option<String>,
) -> Result<WorkspaceRecord, crate::AppError> {
    let existing = conn
        .query_row(
            &format!(
                "SELECT {WORKSPACE_COLUMNS}
             FROM workspaces
             WHERE kind = 'temporary' AND path = ?1 AND deleted_at IS NULL
             LIMIT 1"
            ),
            params![chat_workspace_path(thread_id)?.display().to_string()],
            workspace_from_row,
        )
        .optional()?;

    if let Some(workspace) = existing {
        return Ok(workspace);
    }

    let path = chat_workspace_path(thread_id)?;
    fs::create_dir_all(&path)?;
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
    )?;

    loaded(get_workspace_in(conn, &workspace_id)?, "Created workspace")
}

pub fn rename_workspace(input: RenameWorkspaceInput) -> Result<WorkspaceRecord, crate::AppError> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("Workspace name cannot be empty.".to_string().into());
    }

    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "UPDATE workspaces
         SET name = ?1, updated_at = ?2
         WHERE id = ?3 AND deleted_at IS NULL",
        params![name, now, input.workspace_id],
    )?;

    loaded(get_workspace_in(&conn, &input.workspace_id)?, "Workspace")
}

/// Soft-deletes a Workspace and its threads (they'd otherwise dangle in the
/// sidebar). Files on disk are left untouched — only the sidebar records are
/// removed. A temporary Workspace is additionally flagged for cleanup.
pub fn delete_workspace(workspace_id: &str) -> Result<WorkspaceRecord, crate::AppError> {
    let now = now_millis();
    let mut conn = connect()?;
    let workspace = loaded(get_workspace_in(&conn, workspace_id)?, "Workspace")?;
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE workspaces
         SET deleted_at = ?1, updated_at = ?1
         WHERE id = ?2 AND deleted_at IS NULL",
        params![now, workspace_id],
    )?;
    tx.execute(
        "UPDATE threads
         SET status = 'deleted', deleted_at = ?1, updated_at = ?1
         WHERE workspace_id = ?2 AND status != 'deleted'",
        params![now, workspace_id],
    )?;
    if workspace.kind == "temporary" {
        tx.execute(
            "UPDATE workspaces
             SET cleanup_status = 'pending_cleanup',
                 cleanup_requested_at = COALESCE(cleanup_requested_at, ?1)
             WHERE id = ?2 AND cleanup_status = 'active'",
            params![now, workspace_id],
        )?;
    }
    tx.commit()?;

    loaded(get_workspace_in(&conn, workspace_id)?, "Workspace")
}

pub fn get_workspace(workspace_id: &str) -> Result<Option<WorkspaceRecord>, crate::AppError> {
    let conn = connect()?;
    get_workspace_in(&conn, workspace_id)
}

pub(super) fn get_workspace_in(
    conn: &Connection,
    workspace_id: &str,
) -> Result<Option<WorkspaceRecord>, crate::AppError> {
    conn.query_row(
        &format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE id = ?1"),
        params![workspace_id],
        workspace_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}
