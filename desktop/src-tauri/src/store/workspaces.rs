use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

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
    pub pinned: bool,
    pub cleanup_status: String,
    pub cleanup_requested_at: Option<i64>,
    pub cleaned_at: Option<i64>,
    pub last_opened_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

sql_record!(pub(super) WORKSPACE_COLUMNS, workspace_from_row -> WorkspaceRecord {
    id, name, kind, path, description, pinned, cleanup_status, cleanup_requested_at,
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
    if let Some(workspace) = find_user_workspace_in(conn, &path)? {
        return Ok(workspace);
    }

    let now = now_millis();
    let workspace_id = create_id("ws");
    const INSERT_SQL: &str = "INSERT INTO workspaces (
             id, name, kind, path, description, cleanup_status, last_opened_at,
             created_at, updated_at
         ) VALUES (?1, ?2, 'user', ?3, ?4, 'active', ?5, ?5, ?5)";
    let args = params![
        workspace_id,
        name,
        normalize_workspace_path(&path).display().to_string(),
        description,
        now
    ];
    conn.execute(INSERT_SQL, args)?;

    loaded(get_workspace_in(conn, &workspace_id)?, "Created workspace")
}

/// Resolve the user workspace for a directory, or `None` when the directory
/// has no workspace yet. A client's spelling is never taken as identity here:
/// see [`normalize_workspace_path`].
pub fn find_user_workspace_by_path(
    path: &Path,
) -> Result<Option<WorkspaceRecord>, crate::AppError> {
    let conn = connect()?;
    find_user_workspace_in(&conn, path)
}

/// Connection-injecting variant of [`find_user_workspace_by_path`].
///
/// The stored spelling is matched first (one indexed lookup). Only when that
/// misses are the other rows compared by their canonical path, which is how a
/// workspace stored under an older aliasing spelling — a pre-normalization row,
/// or one created before its directory existed — is still found instead of
/// duplicated.
pub(super) fn find_user_workspace_in(
    conn: &Connection,
    path: &Path,
) -> Result<Option<WorkspaceRecord>, crate::AppError> {
    let normalized = normalize_workspace_path(path);
    let stored = conn
        .query_row(
            &format!(
                "SELECT {WORKSPACE_COLUMNS}
             FROM workspaces
             WHERE kind = 'user' AND path = ?1 AND deleted_at IS NULL
             LIMIT 1"
            ),
            params![normalized.display().to_string()],
            workspace_from_row,
        )
        .optional()?;
    if stored.is_some() {
        return Ok(stored);
    }

    let mut stmt = conn.prepare(&format!(
        "SELECT {WORKSPACE_COLUMNS}
             FROM workspaces
             WHERE kind = 'user' AND deleted_at IS NULL"
    ))?;
    let rows = stmt.query_map([], workspace_from_row)?;
    for row in rows {
        let row = row?;
        if normalize_workspace_path(Path::new(&row.path)) == normalized {
            return Ok(Some(row));
        }
    }
    Ok(None)
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

/// Pin a workspace above the unpinned groups (the phone's workspace tab reads
/// the flag from the pushed snapshot; the desktop rail keeps its recency order
/// for now). Like [`super::pin_thread`], this is an ordering flag and not
/// activity: `updated_at`/`last_opened_at` are left alone, so unpinning returns
/// the group to its recency position instead of jumping it to the front.
pub fn pin_workspace(input: PinWorkspaceInput) -> Result<WorkspaceRecord, crate::AppError> {
    let pinned = if input.pinned { 1 } else { 0 };
    let conn = connect()?;
    let updated = conn.execute(
        "UPDATE workspaces SET pinned = ?1 WHERE id = ?2 AND deleted_at IS NULL",
        params![pinned, input.workspace_id],
    )?;
    if updated == 0 {
        return Err("Workspace unavailable".into());
    }

    let workspace = loaded(get_workspace_in(&conn, &input.workspace_id)?, "Workspace")?;
    // The phone's workspace snapshot carries the flag, so the next heartbeat
    // tick republishes it. Dirty-marked like every other workspace mutation.
    mark_catalog_dirty();
    Ok(workspace)
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

    /// A pin persists as an ordering flag without becoming activity:
    /// `last_opened_at`/`updated_at` stay put, so unpinning drops the group back
    /// to its recency position instead of jumping it to the front of the list.
    #[test]
    fn pin_workspace_persists_without_touching_recency() {
        let (_home, conn) = guarded_conn("ws_pin");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, last_opened_at,
                 created_at, updated_at)
                 VALUES ('ws_new', 'New', 'user', '/tmp/new', 200, 1, 1),
                        ('ws_old', 'Old', 'user', '/tmp/old', 100, 5, 5);",
        )
        .expect("seed");
        drop(conn);

        let ids = || {
            list_workspaces()
                .expect("list")
                .into_iter()
                .map(|workspace| workspace.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(), vec!["ws_new", "ws_old"], "recency order");

        let pinned = pin_workspace(PinWorkspaceInput {
            workspace_id: "ws_old".to_string(),
            pinned: true,
        })
        .expect("pin");
        assert!(pinned.pinned);
        assert_eq!(
            ids(),
            vec!["ws_new", "ws_old"],
            "recency order is untouched"
        );

        let conn = connect().expect("reconnect");
        let stamps: (Option<i64>, i64, i64) = conn
            .query_row(
                "SELECT last_opened_at, updated_at, pinned FROM workspaces WHERE id = 'ws_old'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read stamps");
        assert_eq!(stamps, (Some(100), 5, 1), "pin is not activity");
        assert!(
            get_workspace("ws_old").expect("get").expect("some").pinned,
            "the flag survives a re-read"
        );

        let unpinned = pin_workspace(PinWorkspaceInput {
            workspace_id: "ws_old".to_string(),
            pinned: false,
        })
        .expect("unpin");
        assert!(!unpinned.pinned);
        assert_eq!(ids(), vec!["ws_new", "ws_old"], "recency order intact");

        let missing = pin_workspace(PinWorkspaceInput {
            workspace_id: "ws_ghost".to_string(),
            pinned: true,
        });
        assert!(missing.is_err(), "unknown workspace errors");
        conn.execute(
            "UPDATE workspaces SET deleted_at = 1 WHERE id = 'ws_old'",
            [],
        )
        .unwrap();
        assert!(
            pin_workspace(PinWorkspaceInput {
                workspace_id: "ws_old".into(),
                pinned: true,
            })
            .is_err(),
            "a soft-deleted workspace must not report success"
        );
        assert!(!get_workspace("ws_old").unwrap().unwrap().pinned);
        // Setting the same value on an active row is still a successful idempotent write.
        for _ in 0..2 {
            assert!(
                pin_workspace(PinWorkspaceInput {
                    workspace_id: "ws_new".into(),
                    pinned: true,
                })
                .unwrap()
                .pinned
            );
        }
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

    /// One directory is one workspace, however a client spells it: a path
    /// reached through a symlink (the macOS `/tmp` → `/private/tmp` shape)
    /// resolves to the workspace already stored under the other spelling
    /// instead of creating a second group for the same directory.
    #[cfg(unix)]
    #[test]
    fn create_workspace_resolves_an_aliased_path_to_one_workspace() {
        let _home = crate::auth_store::test_support::HomeGuard::new("ws_alias");
        let conn = connect().expect("connect");
        apply_schema(&conn).expect("apply schema");
        drop(conn);

        let base = std::env::temp_dir().join(format!("futureos-wsa-{}", std::process::id()));
        let real = base.join("real");
        let alias = base.join("alias");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&real).expect("create real dir");
        std::os::unix::fs::symlink(&real, &alias).expect("symlink");

        let created = create_workspace(CreateWorkspaceInput {
            name: None,
            description: None,
            path: real.display().to_string(),
            create_directory: Some(true),
        })
        .expect("create through the real path");

        // Same directory through the symlink → the same row.
        let aliased = create_workspace(CreateWorkspaceInput {
            name: Some("Ignored".to_string()),
            description: None,
            path: alias.display().to_string(),
            create_directory: Some(true),
        })
        .expect("create through the alias");
        assert_eq!(aliased.id, created.id);
        assert_eq!(
            list_workspaces()
                .expect("list")
                .into_iter()
                .filter(|workspace| workspace.kind == "user")
                .count(),
            1
        );

        // A legacy row kept under an aliasing spelling is still found: the
        // lookup compares canonical paths, not the text stored.
        let conn = connect().expect("connect");
        conn.execute(
            "UPDATE workspaces SET path = ?1 WHERE id = ?2",
            params![alias.display().to_string(), created.id],
        )
        .expect("re-spell the stored path");
        drop(conn);
        assert_eq!(
            find_user_workspace_by_path(&real)
                .expect("find")
                .map(|workspace| workspace.id),
            Some(created.id)
        );

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
        // Stored canonicalized: `~` expands to HOME, and a HOME sitting behind
        // a symlink (macOS `/var` → `/private/var`) is stored as resolved so
        // every client's spelling maps to this one workspace. The stored form
        // is the ordinary spelling, never Windows' `\\?\` extended-length one
        // (a workspace path is handed to shells and to the UI).
        let canonical = crate::store::strip_verbatim_prefix(dir.canonicalize().unwrap());
        assert_eq!(created.path, canonical.display().to_string());
        assert!(
            !created.path.starts_with(r"\\?\"),
            "a stored workspace path must not keep the verbatim prefix: {}",
            created.path
        );
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

    /// A row stored under an older spelling of the same directory must still be
    /// found instead of a duplicate workspace being created for it.
    #[test]
    fn find_user_workspace_matches_an_aliased_spelling_by_canonical_path() {
        let conn = test_conn();
        let dir = tempfile::tempdir().expect("temp dir");
        let canonical = dir.path().canonicalize().expect("canonicalize");
        let aliased = format!("{}{}", canonical.display(), std::path::MAIN_SEPARATOR);
        conn.execute(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
             VALUES ('ws-alias', 'W', 'user', ?1, 1, 1)",
            rusqlite::params![aliased],
        )
        .expect("seed aliased workspace");

        // The indexed spelling lookup cannot match the stored alias, so the
        // canonical comparison has to resolve it.
        let found = find_user_workspace_in(&conn, &canonical)
            .expect("query")
            .expect("an aliased spelling must resolve to its workspace");
        assert_eq!(found.id, "ws-alias");
        assert_eq!(found.path, aliased);
    }
}
