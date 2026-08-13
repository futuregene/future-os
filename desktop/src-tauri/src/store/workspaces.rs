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
    let workspace = get_or_create_user_workspace(name, path, input.description)?;
    mark_catalog_dirty();
    Ok(workspace)
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
    const INSERT_SQL: &str = "INSERT INTO workspaces (
             id, name, kind, path, description, cleanup_status, last_opened_at,
             created_at, updated_at
         ) VALUES (?1, ?2, 'user', ?3, ?4, 'active', ?5, ?5, ?5)";
    let args = params![workspace_id, name, normalized_path, description, now];
    conn.execute(INSERT_SQL, args)?;

    loaded(get_workspace_in(conn, &workspace_id)?, "Created workspace")
}

pub fn get_or_create_chat_workspace(
    thread_id: &str,
    title: Option<String>,
) -> Result<WorkspaceRecord, crate::AppError> {
    let conn = connect()?;
    get_or_create_chat_workspace_in(&conn, thread_id, title)
}

/// Update a chat workspace record's path (e.g. from the initial thread-id
/// name to the session-id name after the agent session is created).
pub fn update_chat_workspace_path(thread_id: &str, new_path: &str) -> Result<(), crate::AppError> {
    let conn = connect()?;
    let old_path = chat_workspace_path(thread_id)?.display().to_string();
    if old_path == new_path {
        return Ok(());
    }
    conn.execute(
        "UPDATE workspaces SET path = ?1, updated_at = ?2
         WHERE path = ?3 AND kind = 'temporary'",
        rusqlite::params![new_path, super::util::now_millis(), old_path,],
    )?;
    Ok(())
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
    // Directory creation is deferred — we don't know the session id yet.
    // The real directory (named after the session id) is created when the
    // first prompt runs and the workspace path is updated.
    let now = now_millis();
    let workspace_id = create_id("ws");
    let name = format!(
        "{} Workspace",
        title.unwrap_or_else(|| "New Chat".to_string())
    );
    const INSERT_SQL: &str = "INSERT INTO workspaces (
             id, name, kind, path, cleanup_status, created_at, updated_at
         ) VALUES (?1, ?2, 'temporary', ?3, 'active', ?4, ?4)";
    let args = params![workspace_id, name, path.display().to_string(), now];
    conn.execute(INSERT_SQL, args)?;

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

    let workspace = loaded(get_workspace_in(&conn, &input.workspace_id)?, "Workspace")?;
    mark_catalog_dirty();
    Ok(workspace)
}

/// Hard-deletes a Workspace: every thread in it (via the same FK-safe cascade as
/// [`super::delete_thread`]) plus the workspace-scoped rows (artifacts,
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
    mark_catalog_dirty();
    Ok(workspace)
}

/// The FK-safe cascade for a workspace hard-delete, split out so the (subtle)
/// deletion order can be unit-tested against an in-memory DB with foreign keys
/// enforced. Deletes every thread's children and the threads, then the
/// workspace-scoped rows, then the workspace itself. Does not touch any files.
pub(super) fn delete_workspace_in(conn: &Connection, workspace_id: &str) -> rusqlite::Result<()> {
    // Tombstone only sessions for which this workspace deletes the final GUI
    // owner. This is deliberately in the same transaction as the thread
    // cascade, so a crash cannot leave a deleted thread without delivery
    // intent for its Agent source of truth.
    let session_ids: Vec<String> = {
        const SQL: &str = "SELECT DISTINCT COALESCE(NULLIF(TRIM(agent_session_id), ''), id)
             FROM threads WHERE workspace_id = ?1";
        let mut stmt = conn.prepare(SQL)?;
        let ids = stmt
            .query_map(params![workspace_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        ids
    };
    for session_id in session_ids {
        const OWNER_SQL: &str = "SELECT COUNT(*) FROM threads
             WHERE COALESCE(NULLIF(TRIM(agent_session_id), ''), id) = ?1";
        let owner_count: i64 = conn.query_row(OWNER_SQL, [&session_id], |row| row.get(0))?;
        const DELETING_SQL: &str = "SELECT COUNT(*) FROM threads
             WHERE workspace_id = ?1
               AND COALESCE(NULLIF(TRIM(agent_session_id), ''), id) = ?2";
        let args = params![workspace_id, session_id];
        let deleting_here: i64 = conn.query_row(DELETING_SQL, args, |row| row.get(0))?;
        if owner_count == deleting_here {
            super::deletions::enqueue_agent_session_delete_in(conn, &session_id)?;
        }
    }
    // 1. Cascade every thread's children, then the threads themselves.
    let thread_ids: Vec<String> = {
        let mut stmt = conn.prepare("SELECT id FROM threads WHERE workspace_id = ?1")?;
        let rows = stmt.query_map(params![workspace_id], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for thread_id in &thread_ids {
        super::threads::delete_thread_children_in(conn, thread_id)?;
    }
    const DELETE_THREADS_SQL: &str = "DELETE FROM threads WHERE workspace_id = ?1";
    conn.execute(DELETE_THREADS_SQL, params![workspace_id])?;

    // 2. Workspace-scoped rows, FK-safe (children before parents).
    const DELETE_ARTIFACTS_SQL: &str = "DELETE FROM artifacts WHERE workspace_id = ?1";
    conn.execute(DELETE_ARTIFACTS_SQL, params![workspace_id])?;
    const DELETE_LINKS_SQL: &str = "DELETE FROM object_references WHERE reference_target_id IN (
             SELECT id FROM reference_targets WHERE workspace_id = ?1
         )";
    conn.execute(DELETE_LINKS_SQL, params![workspace_id])?;
    const DELETE_TARGETS_SQL: &str = "DELETE FROM reference_targets WHERE workspace_id = ?1";
    conn.execute(DELETE_TARGETS_SQL, params![workspace_id])?;
    const DELETE_FILES_SQL: &str = "DELETE FROM workspace_files WHERE workspace_id = ?1";
    conn.execute(DELETE_FILES_SQL, params![workspace_id])?;

    // 3. The workspace row.
    const DELETE_WORKSPACE_SQL: &str = "DELETE FROM workspaces WHERE id = ?1";
    conn.execute(DELETE_WORKSPACE_SQL, params![workspace_id])?;
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
            "artifacts",
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

    /// A session still owned by a thread in another workspace must NOT be
    /// tombstoned when one of its workspaces is deleted.
    #[test]
    fn delete_workspace_in_tombstones_only_orphaned_sessions() {
        let conn = test_conn();
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                 VALUES ('ws_a', 'A', 'user', '/tmp/a', 1, 1),
                        ('ws_b', 'B', 'user', '/tmp/b', 1, 1);
             INSERT INTO threads (id, workspace_id, mode, title, agent_session_id,
                 created_at, updated_at)
                 VALUES ('t_shared_a', 'ws_a', 'chat', 'T', 'sess_shared', 1, 1),
                        ('t_shared_b', 'ws_b', 'chat', 'T', 'sess_shared', 1, 1),
                        ('t_solo', 'ws_a', 'chat', 'T', 'sess_solo', 1, 1);",
        )
        .expect("seed threads");

        delete_workspace_in(&conn, "ws_a").expect("delete");

        let tombstoned = |id: &str| {
            conn.query_row(
                "SELECT COUNT(*) FROM agent_delete_outbox WHERE session_id = ?1",
                [id],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
        };
        assert_eq!(tombstoned("sess_solo"), 1, "sole-owned session tombstoned");
        assert_eq!(
            tombstoned("sess_shared"),
            0,
            "session still owned by ws_b's thread is not tombstoned"
        );
    }

    // ── connect()-backed API surface (fake HOME) ────────────────────────────

    use crate::store::db::test_support::guarded_conn;

    #[test]
    fn list_and_get_workspaces() {
        let (_home, conn) = guarded_conn("ws_list");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, last_opened_at,
                 created_at, updated_at)
                 VALUES ('ws_old', 'Old', 'user', '/tmp/old', 100, 1, 1),
                        ('ws_new', 'New', 'user', '/tmp/new', 200, 1, 1),
                        ('ws_gone', 'Gone', 'user', '/tmp/gone', 300, 1, 1);
             UPDATE workspaces SET deleted_at = 1 WHERE id = 'ws_gone';",
        )
        .expect("seed");
        drop(conn);

        let list = list_workspaces().expect("list");
        assert_eq!(list.len(), 2, "soft-deleted rows are hidden");
        assert_eq!(list[0].id, "ws_new", "most recently opened first");

        let one = get_workspace("ws_old").expect("get").expect("some");
        assert_eq!(one.name, "Old");
        assert!(get_workspace("ws_ghost").expect("get").is_none());
    }

    #[test]
    fn create_workspace_validates_and_creates_the_directory() {
        let _home = crate::auth_store::test_support::HomeGuard::new("ws_create");
        let conn = connect().expect("connect");
        apply_schema(&conn).expect("apply schema");
        drop(conn);

        let base = std::env::temp_dir().join(format!("futureos-wsc-{}", std::process::id()));
        let target = base.join("nested/proj");

        // Missing directory without create_directory → error.
        let missing = create_workspace(CreateWorkspaceInput {
            name: None,
            description: None,
            path: target.display().to_string(),
            create_directory: None,
        });
        assert!(missing.is_err(), "missing dir rejected");

        // With create_directory the dir is made and the name defaults to the
        // last path component.
        let created = create_workspace(CreateWorkspaceInput {
            name: None,
            description: Some("d".to_string()),
            path: target.display().to_string(),
            create_directory: Some(true),
        })
        .expect("create");
        assert!(target.is_dir());
        assert_eq!(created.name, "proj");
        assert_eq!(created.kind, "user");

        // Same path again → the existing record is returned (no duplicate).
        let again = create_workspace(CreateWorkspaceInput {
            name: Some("Ignored".to_string()),
            description: None,
            path: target.display().to_string(),
            create_directory: Some(true),
        })
        .expect("idempotent");
        assert_eq!(again.id, created.id);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn create_workspace_expands_a_tilde_path() {
        let home = crate::auth_store::test_support::HomeGuard::new("ws_tilde");
        let conn = connect().expect("connect");
        apply_schema(&conn).expect("apply schema");
        drop(conn);

        let dir = std::env::var("HOME").unwrap();
        let dir = std::path::Path::new(&dir).join("tilde-ws");
        std::fs::create_dir_all(&dir).unwrap();

        let created = create_workspace(CreateWorkspaceInput {
            name: Some("Tilde".to_string()),
            description: None,
            path: "~/tilde-ws".to_string(),
            create_directory: None,
        })
        .expect("create");
        assert_eq!(created.path, dir.display().to_string());
        drop(home);
    }

    #[test]
    fn chat_workspace_lifecycle() {
        let _home = crate::auth_store::test_support::HomeGuard::new("ws_chat");
        let conn = connect().expect("connect");
        apply_schema(&conn).expect("apply schema");
        drop(conn);

        let created = get_or_create_chat_workspace("thread_x", None).expect("create chat ws");
        assert_eq!(created.kind, "temporary");
        assert_eq!(created.name, "New Chat Workspace");

        // Idempotent on the same thread id…
        let again = get_or_create_chat_workspace("thread_x", Some("Titled".to_string()))
            .expect("idempotent");
        assert_eq!(again.id, created.id);

        // …and a titled creation uses the title.
        let titled =
            get_or_create_chat_workspace("thread_y", Some("Poem".to_string())).expect("titled");
        assert_eq!(titled.name, "Poem Workspace");

        // Path update: same path is a no-op; a new path rewrites the row.
        let current = created.path.clone();
        update_chat_workspace_path("thread_x", &current).expect("no-op update");
        update_chat_workspace_path("thread_x", "/tmp/renamed-chat").expect("update");
        let moved = get_workspace(&created.id).expect("get").expect("some");
        assert_eq!(moved.path, "/tmp/renamed-chat");
    }

    #[test]
    fn rename_workspace_validates_and_persists() {
        let (_home, conn) = guarded_conn("ws_rename");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
             VALUES ('ws1', 'Before', 'user', '/tmp/ws1', 1, 1);",
        )
        .expect("seed");
        drop(conn);

        let empty = rename_workspace(RenameWorkspaceInput {
            workspace_id: "ws1".to_string(),
            name: "  ".to_string(),
        });
        assert!(empty.is_err(), "blank name rejected");

        let renamed = rename_workspace(RenameWorkspaceInput {
            workspace_id: "ws1".to_string(),
            name: "After".to_string(),
        })
        .expect("rename");
        assert_eq!(renamed.name, "After");

        let missing = rename_workspace(RenameWorkspaceInput {
            workspace_id: "ws_ghost".to_string(),
            name: "X".to_string(),
        });
        assert!(missing.is_err(), "unknown workspace errors");
    }

    #[test]
    fn delete_workspace_public_wrapper_and_missing_error() {
        let (_home, conn) = guarded_conn("ws_delete");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
             VALUES ('ws1', 'W', 'user', '/tmp/ws1', 1, 1);",
        )
        .expect("seed");
        drop(conn);

        let deleted = delete_workspace("ws1").expect("delete");
        assert_eq!(deleted.id, "ws1");
        assert!(get_workspace("ws1").expect("get").is_none());
        assert!(delete_workspace("ws1").is_err(), "second delete errors");
    }

    #[test]
    fn purge_soft_deleted_workspaces_cascades() {
        let (_home, conn) = guarded_conn("ws_purge");
        drop(conn);

        assert_eq!(purge_soft_deleted_workspaces().expect("purge"), 0);

        let conn = connect().expect("reconnect");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
             VALUES ('ws_dead', 'W', 'user', '/tmp/dead', 1, 1);
             UPDATE workspaces SET deleted_at = 5 WHERE id = 'ws_dead';",
        )
        .expect("seed");
        drop(conn);

        assert_eq!(purge_soft_deleted_workspaces().expect("purge"), 1);
        assert!(get_workspace("ws_dead").expect("get").is_none());
    }
}
