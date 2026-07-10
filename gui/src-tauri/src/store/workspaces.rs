use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

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

pub(super) fn get_or_create_user_workspace(
    name: String,
    path: PathBuf,
    description: Option<String>,
) -> Result<WorkspaceRecord, crate::AppError> {
    let mut conn = connect()?;
    // BEGIN IMMEDIATE so the SELECT-then-INSERT is atomic against a concurrent
    // create for the same path (mirrors the approvals/artifacts write paths).
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let workspace = get_or_create_user_workspace_in(&tx, name, path, description)?;
    tx.commit()?;
    Ok(workspace)
}

/// Connection-injecting variant so a composite write (e.g. `create_thread`) can
/// resolve/create the workspace and insert its own row in one transaction.
pub(super) fn get_or_create_user_workspace_in(
    conn: &Connection,
    name: String,
    path: PathBuf,
    description: Option<String>,
) -> Result<WorkspaceRecord, crate::AppError> {
    let normalized_path = path.display().to_string();
    let existing = conn
        .query_row(
            &format!(
                "SELECT {WORKSPACE_COLUMNS}
             FROM workspaces
             WHERE kind = 'user' AND path = ?1 AND deleted_at IS NULL
             LIMIT 1"
            ),
            params![normalized_path],
            workspace_from_row,
        )
        .optional()?;

    if let Some(workspace) = existing {
        return Ok(workspace);
    }

    let now = now_millis();
    let workspace_id = create_id("ws");
    conn.execute(
        "INSERT INTO workspaces (
             id, name, kind, path, description, cleanup_status, last_opened_at,
             created_at, updated_at
         ) VALUES (?1, ?2, 'user', ?3, ?4, 'active', ?5, ?5, ?5)",
        params![workspace_id, name, normalized_path, description, now],
    )?;

    loaded(get_workspace_in(conn, &workspace_id)?, "Created workspace")
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

/// Resolve the agent session id of every thread in `workspace_id` (session id =
/// `agent_session_id` when set, else the thread id). Read *before* a workspace
/// hard-delete so the caller can delete each thread's agent JSONL.
pub fn workspace_agent_session_ids(workspace_id: &str) -> Result<Vec<String>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(
        "SELECT COALESCE(NULLIF(TRIM(agent_session_id), ''), id)
         FROM threads WHERE workspace_id = ?1",
    )?;
    let rows = stmt.query_map(params![workspace_id], |row| row.get::<_, String>(0))?;
    rows.collect::<rusqlite::Result<_>>()
        .map_err(crate::AppError::from)
}

/// Hard-deletes a Workspace: every thread in it (via the same FK-safe cascade as
/// [`super::delete_thread`]) plus the workspace-scoped rows (artifacts, research,
/// references, file index) and finally the workspace row itself. The user's files
/// on disk are NEVER touched — a user Workspace's `path` is their own directory,
/// and GUI-managed scratch/review dirs are reclaimed by the startup reconcilers
/// (keyed by thread/workspace id), not by removing `workspace.path`. The agent
/// JSONLs and those physical dirs are cleaned by the command layer.
pub fn delete_workspace(workspace_id: &str) -> Result<WorkspaceRecord, crate::AppError> {
    let mut conn = connect()?;
    let workspace = loaded(get_workspace_in(&conn, workspace_id)?, "Workspace")?;
    let tx = conn.transaction()?;
    delete_workspace_in(&tx, workspace_id)?;
    tx.commit()?;
    Ok(workspace)
}

/// The FK-safe cascade for a workspace hard-delete, split out so the (subtle)
/// deletion order can be unit-tested against an in-memory DB with foreign keys
/// enforced. Deletes every thread's children and the threads, then the
/// workspace-scoped rows, then the workspace itself. Does not touch any files.
pub(super) fn delete_workspace_in(conn: &Connection, workspace_id: &str) -> rusqlite::Result<()> {
    // 1. Cascade every thread's children, then the threads themselves.
    let thread_ids: Vec<String> = {
        let mut stmt = conn.prepare("SELECT id FROM threads WHERE workspace_id = ?1")?;
        let rows = stmt.query_map(params![workspace_id], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for thread_id in &thread_ids {
        super::threads::delete_thread_children_in(conn, thread_id)?;
    }
    conn.execute(
        "DELETE FROM threads WHERE workspace_id = ?1",
        params![workspace_id],
    )?;

    // 2. Workspace-scoped rows, FK-safe (children before parents).
    conn.execute(
        "DELETE FROM research_resources WHERE collection_id IN (
             SELECT id FROM research_collections WHERE workspace_id = ?1
         )",
        params![workspace_id],
    )?;
    conn.execute(
        "DELETE FROM research_collections WHERE workspace_id = ?1",
        params![workspace_id],
    )?;
    conn.execute(
        "DELETE FROM artifacts WHERE workspace_id = ?1",
        params![workspace_id],
    )?;
    conn.execute(
        "DELETE FROM object_references WHERE reference_target_id IN (
             SELECT id FROM reference_targets WHERE workspace_id = ?1
         )",
        params![workspace_id],
    )?;
    conn.execute(
        "DELETE FROM reference_targets WHERE workspace_id = ?1",
        params![workspace_id],
    )?;
    conn.execute(
        "DELETE FROM workspace_files WHERE workspace_id = ?1",
        params![workspace_id],
    )?;

    // 3. The workspace row.
    conn.execute(
        "DELETE FROM workspaces WHERE id = ?1",
        params![workspace_id],
    )?;
    Ok(())
}

/// Defensive / one-time sweep: hard-delete any workspaces left in the legacy
/// soft-deleted state (`deleted_at IS NOT NULL`), along with all their scoped
/// rows. `delete_workspace` now hard-deletes, so this only reclaims pre-existing
/// rows. Runs once at startup. Returns the number purged.
pub fn purge_soft_deleted_workspaces() -> Result<usize, crate::AppError> {
    let mut conn = connect()?;
    let ids: Vec<String> = {
        let mut stmt = conn.prepare("SELECT id FROM workspaces WHERE deleted_at IS NOT NULL")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    if ids.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    for id in &ids {
        delete_workspace_in(&tx, id)?;
    }
    tx.commit()?;
    Ok(ids.len())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::schema::SCHEMA;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(SCHEMA).expect("initialize test schema");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("enable fk");
        conn
    }

    fn seed_workspace(conn: &Connection, ws: &str) {
        conn.execute_batch(&format!(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                 VALUES ('{ws}', 'W', 'user', '/tmp/{ws}', 1, 1);
             INSERT INTO threads (id, workspace_id, mode, title, status, pinned,
                 readonly, created_at, updated_at)
                 VALUES ('{ws}_t', '{ws}', 'workspace', 'T', 'active', 0, 0, 1, 1);
             INSERT INTO runs (id, thread_id, status, created_at, updated_at)
                 VALUES ('{ws}_r', '{ws}_t', 'completed', 1, 1);
             INSERT INTO artifacts (id, workspace_id, thread_id, run_id, title,
                 artifact_type, created_at, updated_at)
                 VALUES ('{ws}_a', '{ws}', '{ws}_t', '{ws}_r', 'A', 'markdown', 1, 1);
             INSERT INTO research_collections (id, workspace_id, name, created_at,
                 updated_at) VALUES ('{ws}_c', '{ws}', 'C', 1, 1);
             INSERT INTO research_resources (id, collection_id, source_artifact_id,
                 title, resource_type, created_at, updated_at)
                 VALUES ('{ws}_rr', '{ws}_c', '{ws}_a', 'R', 'note', 1, 1);
             INSERT INTO workspace_files (id, workspace_id, path, name, created_at,
                 updated_at) VALUES ('{ws}_f', '{ws}', '/p', 'f', 1, 1);
             INSERT INTO reference_targets (id, target_type, target_id, scope,
                 workspace_id, title, created_at, updated_at)
                 VALUES ('{ws}_rt', 'artifact', '{ws}_a', 'workspace', '{ws}', 'T', 1, 1);
             INSERT INTO object_references (id, source_type, source_id,
                 reference_target_id, created_at)
                 VALUES ('{ws}_or', 'message', '{ws}_m', '{ws}_rt', 1);",
        ))
        .expect("seed workspace graph");
    }

    fn total(conn: &Connection) -> i64 {
        let tables = [
            "workspaces",
            "threads",
            "runs",
            "messages",
            "artifacts",
            "research_collections",
            "research_resources",
            "workspace_files",
            "reference_targets",
            "object_references",
        ];
        tables
            .iter()
            .map(|t| {
                conn.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
            })
            .sum()
    }

    /// A workspace hard-delete removes the workspace, its threads, and every
    /// workspace-scoped row in an FK-safe order (foreign keys ON, so a wrong
    /// order errors), and leaves an unrelated workspace fully intact.
    #[test]
    fn delete_workspace_in_cascades_and_isolates() {
        let conn = test_conn();
        seed_workspace(&conn, "keep");
        seed_workspace(&conn, "drop");
        let keep_before = total(&conn);

        delete_workspace_in(&conn, "drop").expect("cascade delete");

        // "keep" and "drop" seeded identical graphs, so the surviving row count
        // across every table must be exactly half — proving "drop" was fully
        // cascaded and "keep" was left entirely intact.
        assert_eq!(total(&conn), keep_before / 2);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM workspaces WHERE id = 'keep'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM workspaces WHERE id = 'drop'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
