use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::PathBuf;

use super::db::*;
use super::records::*;
use super::util::*;
use future_app_workspaces as workspaces;

pub use future_app_workspaces::Workspace as WorkspaceRecord;

const WORKSPACE_COLUMNS: &str = workspaces::COLUMNS;

/// The row mapper for the desktop-only queries in this module (chat workspaces,
/// read-back by id). The user-workspace rules shared with the CLI live in
/// `future-app-workspaces`.
pub(super) fn workspace_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceRecord> {
    workspaces::from_row(row)
}

pub fn list_workspaces() -> Result<Vec<WorkspaceRecord>, crate::Error> {
    let conn = connect()?;
    Ok(workspaces::list(&conn)?)
}

/// Create (or reopen) the user workspace for a directory. The rules — `~`
/// expansion, the directory having to exist unless `create_directory` says
/// otherwise, the name defaulting to the directory's own name, and one
/// directory meaning one workspace however it is spelled — are
/// `future_app_workspaces::create`, shared with `future workspace add`.
pub fn create_workspace(input: CreateWorkspaceInput) -> Result<WorkspaceRecord, crate::Error> {
    let mut conn = connect()?;
    let workspace = workspaces::create(
        &mut conn,
        &input.path,
        input.name,
        input.description,
        input.create_directory.unwrap_or(false),
        now_millis(),
    )?;
    // The phone's workspace snapshot is signature-based, but a create is worth
    // marking dirty like every other workspace mutation.
    mark_catalog_dirty();
    Ok(workspace)
}

/// Connection-injecting variant so a composite write (e.g. `create_thread`) can
/// resolve/create the workspace and insert its own row in one transaction.
pub(super) fn get_or_create_user_workspace_in(
    conn: &Connection,
    name: String,
    path: PathBuf,
    description: Option<String>,
) -> Result<WorkspaceRecord, crate::Error> {
    Ok(workspaces::find_or_create_user(
        conn,
        name,
        &path,
        description,
        now_millis(),
    )?)
}

/// File a conversation under the workspace for its session's new cwd — the rule
/// the app applies when it sees a `cwd_changed`, for the app's own callers
/// (`agent_bridge::reconcile_thread_workspace`) and for the CLI
/// (`future session set <id> --cwd`), which reaches the same store directly.
pub fn file_session_workspace(
    session_id: &str,
    cwd: &str,
) -> Result<Option<workspaces::CwdFiling>, crate::Error> {
    let mut conn = connect()?;
    let filing = workspaces::file_session_for_cwd(&mut conn, session_id, cwd, now_millis())?;
    if matches!(
        filing,
        Some(
            workspaces::CwdFiling::WorkspaceCreated(_) | workspaces::CwdFiling::WorkspaceReused(_)
        )
    ) {
        mark_catalog_dirty();
    }
    Ok(filing)
}

pub fn get_or_create_chat_workspace(
    thread_id: &str,
    title: Option<String>,
) -> Result<WorkspaceRecord, crate::Error> {
    let conn = connect()?;
    let workspace = get_or_create_chat_workspace_in(&conn, thread_id, title)?;
    // The row's directory is what the Files tab reads, so it must exist from the
    // moment the row does (see `materialize_chat_workspace_dir`).
    materialize_chat_workspace_dir(PathBuf::from(&workspace.path).as_path());
    Ok(workspace)
}

/// Create a chat workspace's scratch directory if it is missing.
///
/// A chat conversation's workspace is the directory its Files tab lists, and the
/// tab is reachable the instant the conversation exists — before any prompt, so
/// before the Agent has created a cwd. Deferring the directory to the first run
/// meant the panel's first read hit a path that did not exist yet and painted
/// "cannot read this directory" until the user pressed Refresh (issue reported
/// 2026-11-06). Creating it with the row costs one `create_dir_all` and makes the
/// empty state ("this directory is empty") the honest first answer instead.
///
/// Best-effort by design: a failure here is not worth failing the conversation
/// for (the row is still a valid workspace and the Agent recreates the directory
/// when the first prompt runs), so the error is logged and dropped.
pub(super) fn materialize_chat_workspace_dir(path: &std::path::Path) {
    if let Err(error) = std::fs::create_dir_all(path) {
        tracing::warn!(
            event = "chat_workspace_dir_create_failed",
            path = %path.display(),
            %error,
            "FutureOS: could not create the chat workspace directory"
        );
    }
}

/// Connection-injecting variant so a composite write (e.g. `create_thread`) can
/// resolve/create the workspace and insert its own row in one transaction.
pub(super) fn get_or_create_chat_workspace_in(
    conn: &Connection,
    thread_id: &str,
    title: Option<String>,
) -> Result<WorkspaceRecord, crate::Error> {
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
    // The directory itself is created by the caller that owns the store
    // connection, once the row is committed (`materialize_chat_workspace_dir`).
    // It is not created here: this helper runs inside a transaction, and a
    // filesystem write must not be what decides whether the row commits.
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
pub fn pin_workspace(input: PinWorkspaceInput) -> Result<WorkspaceRecord, crate::Error> {
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

/// Shelve a user workspace: it leaves the default lists (and takes its
/// conversations with it, by JOIN filter), while every conversation keeps its
/// own status — restoring the workspace brings back exactly what it hid.
///
/// Like a conversation archive, this refuses while anything inside is still
/// running: a hidden `waiting_approval` run is a request nobody can see. The
/// container is marked with `archived_at` and loses its pin (the pinned section
/// is an active-list shortcut).
pub fn archive_workspace(
    workspace_id: &str,
    streaming_thread_ids: &[String],
) -> Result<WorkspaceRecord, crate::Error> {
    let conn = connect()?;
    let workspace = loaded(get_workspace_in(&conn, workspace_id)?, "Workspace")?;
    if workspace.kind != "user" {
        return Err("Only user workspaces can be archived.".to_string().into());
    }
    if workspace.archived_at.is_some() {
        return Ok(workspace);
    }
    let thread_ids: Vec<String> = {
        let mut stmt =
            conn.prepare("SELECT id FROM threads WHERE workspace_id = ?1 AND deleted_at IS NULL")?;
        let rows = stmt.query_map(params![workspace_id], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let conflicts = super::threads::active_members(thread_ids, streaming_thread_ids)?;
    if !conflicts.is_empty() {
        return Err(super::threads::archive_refusal(&conflicts));
    }
    let now = now_millis();
    conn.execute(
        "UPDATE workspaces SET archived_at = ?1, pinned = 0, updated_at = ?1
         WHERE id = ?2 AND deleted_at IS NULL",
        params![now, workspace_id],
    )?;
    mark_catalog_dirty();
    loaded(get_workspace_in(&conn, workspace_id)?, "Workspace")
}

/// Bring a shelved workspace (and every conversation it was hiding) back into
/// the default lists. Idempotent for a workspace that is already active.
/// Bring a shelved workspace back.
///
/// The shelf's folder showed every conversation it held — the ones it was
/// hiding and the ones shelved on their own before it — so restoring it returns
/// all of them: the action is "unarchive this folder", and a conversation left
/// behind would be one the reader just saw inside it.
pub fn restore_workspace(workspace_id: &str) -> Result<WorkspaceRecord, crate::Error> {
    let mut conn = connect()?;
    loaded(get_workspace_in(&conn, workspace_id)?, "Workspace")?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let now = now_millis();
    tx.execute(
        "UPDATE threads SET status = 'active', archived_at = NULL, updated_at = ?1
         WHERE workspace_id = ?2 AND status = 'archived' AND deleted_at IS NULL",
        params![now, workspace_id],
    )?;
    tx.execute(
        "UPDATE workspaces SET archived_at = NULL, updated_at = ?1
         WHERE id = ?2 AND deleted_at IS NULL",
        params![now, workspace_id],
    )?;
    tx.commit()?;
    mark_catalog_dirty();
    loaded(get_workspace_in(&conn, workspace_id)?, "Workspace")
}

pub fn rename_workspace(input: RenameWorkspaceInput) -> Result<WorkspaceRecord, crate::Error> {
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
pub fn delete_workspace(workspace_id: &str) -> Result<WorkspaceRecord, crate::Error> {
    let mut conn = connect()?;
    let workspace = loaded(get_workspace_in(&conn, workspace_id)?, "Workspace")?;
    let tx = conn.transaction()?;
    delete_workspace_in(&tx, workspace_id)?;
    tx.commit()?;
    mark_catalog_dirty();
    Ok(workspace)
}

/// What a workspace delete would remove, and what it would keep.
///
/// The delete is not an archive delete: a conversation the reader did not
/// shelve along with its workspace survives it as an ordinary chat on a shelf of
/// its own. Which conversations those are depends on which list the delete was
/// started from — see [`delete_workspace_in`] — so both halves are reported
/// separately rather than as one "kept" number that could only be wrong on one
/// of the two paths.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDeleteSummary {
    /// Conversations deleted together with the workspace.
    pub deleted_threads: usize,
    /// Conversations that survive as live, ordinary chats (they land in the
    /// conversations list, ready to continue).
    pub kept_live_threads: usize,
    /// Conversations that survive as shelved, ordinary chats (they land in the
    /// archived section of the conversations list).
    pub kept_archived_threads: usize,
}

pub fn delete_summary(workspace_id: &str) -> Result<WorkspaceDeleteSummary, crate::Error> {
    let conn = connect()?;
    let workspace = loaded(get_workspace_in(&conn, workspace_id)?, "Workspace")?;
    let (active, archived): (i64, i64) = conn.query_row(
        "SELECT
             COALESCE(SUM(CASE WHEN status = 'archived' THEN 0 ELSE 1 END), 0),
             COALESCE(SUM(CASE WHEN status = 'archived' THEN 1 ELSE 0 END), 0)
         FROM threads WHERE workspace_id = ?1 AND deleted_at IS NULL",
        params![workspace_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(kept_and_deleted(
        workspace.archived_at.is_some(),
        active,
        archived,
    ))
}

/// The counts for one delete, from the two status counts inside the workspace.
///
/// A conversation is kept exactly when the reader's own decision about it
/// *differs* from the decision about its workspace:
///
/// - deleting from the active list means "this folder and what it shows go" — so
///   the conversations still in it go with it, and the ones the reader had
///   already shelved out of it survive (they were already living without it);
/// - deleting from the archived shelf means "this folder and what it was hiding
///   go" — so the conversations it hid go with it, and the ones the reader had
///   already shelved out of it survive.
///
/// A kept conversation keeps its own status, which is what makes the two halves
/// land in different lists: shelved out of an active workspace it stays shelved
/// (the archived section), and live out of a shelved workspace it stays live
/// (the conversations list).
fn kept_and_deleted(workspace_shelved: bool, active: i64, archived: i64) -> WorkspaceDeleteSummary {
    let (deleted, kept_live, kept_archived) = if workspace_shelved {
        (archived, active, 0)
    } else {
        (active, 0, archived)
    };
    WorkspaceDeleteSummary {
        deleted_threads: deleted as usize,
        kept_live_threads: kept_live as usize,
        kept_archived_threads: kept_archived as usize,
    }
}

/// Tables whose rows carry both `thread_id` and `workspace_id`, and therefore
/// have to be re-pointed when a conversation is kept out of a workspace that is
/// being deleted.
///
/// The list is a constant rather than three inline statements because missing
/// one is not a cosmetic bug: each of these columns is
/// `REFERENCES workspaces(id)`, so a forgotten table makes the whole delete fail
/// with a foreign-key error after the reader confirmed it. Every table in the
/// schema with a `workspace_id` reference belongs here or in the cascade below.
const MIGRATED_TO_KEPT_THREAD: &[&str] = &["artifacts", "review_snapshots", "review_changesets"];

/// The FK-safe cascade for a workspace hard-delete, split out so the (subtle)
/// deletion order can be unit-tested against an in-memory DB with foreign keys
/// enforced. Deletes every thread's children and the threads, then the
/// workspace-scoped rows, then the workspace itself. Does not touch any files.
pub(super) fn delete_workspace_in(
    conn: &Connection,
    workspace_id: &str,
) -> Result<(), crate::Error> {
    // Which conversations outlive the workspace is decided by which list the
    // delete was started from, and the rule is one sentence: a conversation is
    // kept when the reader's own decision about it differs from the decision
    // about its workspace.
    //
    // - From the active list, "delete this workspace" means this folder and what
    //   it shows go: its live conversations are deleted with it, and the ones
    //   already shelved out of it survive — the reader had already decided to
    //   keep them without the folder.
    // - From the archived shelf, "delete this workspace" means this folder and
    //   what it was hiding go: the conversations still in it are deleted, and the
    //   ones shelved out of it survive, still shelved.
    //
    // A kept conversation keeps its own status, which is what sends it to the
    // right list afterwards: live out of a shelved workspace it appears in the
    // conversations list ready to continue; shelved out of an active workspace it
    // appears in that list's archived section, where it can be unarchived.
    let workspace_archived_at: Option<i64> = conn
        .query_row(
            "SELECT archived_at FROM workspaces WHERE id = ?1",
            params![workspace_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let kept: Vec<(String, String, String)> = {
        let mut stmt = conn.prepare(
            // A shelved workspace keeps the live conversations it was hiding; an
            // active one keeps the conversations shelved out of it. Exactly one
            // arm applies to any given workspace.
            "SELECT id, title, status FROM threads
             WHERE workspace_id = ?1 AND deleted_at IS NULL
               AND ((?2 IS NULL AND status = 'archived')
                 OR (?2 IS NOT NULL AND status <> 'archived'))",
        )?;
        let rows = stmt.query_map(params![workspace_id, workspace_archived_at], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let now = now_millis();
    for (thread_id, title, status) in &kept {
        let chat = get_or_create_chat_workspace_in(conn, thread_id, Some(title.clone()))?;
        // `status` is already the one this conversation keeps: a live one stays
        // live, a shelved one keeps its shelf.
        conn.execute(
            "UPDATE threads SET workspace_id = ?1, mode = 'chat', pinned = 0, updated_at = ?2
             WHERE id = ?3",
            params![chat.id, now, thread_id],
        )?;
        debug_assert!(status == "active" || status == "archived", "{status}");
        // Everything that names a workspace travels with a conversation that
        // travels. `artifacts` was the only table moved here, which left the
        // review rows (`review_snapshots`, `review_changesets`) still pointing
        // at the workspace about to be deleted — and since they reference
        // `workspaces(id)`, the delete then failed outright with "FOREIGN KEY
        // constraint failed" (reproduced on a copy of a real database: 2
        // snapshots and 1 changeset on the deleted workspace's id). A
        // conversation kept as a chat still owns its review history, so its
        // rows follow it to the chat's own shelf instead of being dropped.
        for table in MIGRATED_TO_KEPT_THREAD {
            conn.execute(
                &format!(
                    "UPDATE {table} SET workspace_id = ?1
                     WHERE thread_id = ?2 AND workspace_id = ?3"
                ),
                params![chat.id, thread_id, workspace_id],
            )?;
        }
    }
    // Tombstone only sessions for which this workspace deletes the final GUI
    // owner. This is deliberately in the same transaction as the thread
    // cascade, so a crash cannot leave a deleted thread without delivery
    // intent for its Agent source of truth. The kept conversations have already
    // left the workspace and still count as owners, so their sessions survive.
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
    super::managed_cleanup::request_workspace_in(conn, workspace_id)?;
    for thread_id in &thread_ids {
        super::managed_cleanup::request_thread_in(conn, thread_id)?;
        super::threads::delete_thread_children_in(conn, thread_id)?;
    }
    const DELETE_THREADS_SQL: &str = "DELETE FROM threads WHERE workspace_id = ?1";
    conn.execute(DELETE_THREADS_SQL, params![workspace_id])?;

    // 2. Workspace-scoped rows, FK-safe (children before parents).
    //
    // Review rows are cleared here as well as re-pointed above: one whose
    // `thread_id` is NULL — or whose thread went with the workspace — still
    // names this workspace, and its column is `REFERENCES workspaces(id)`, so
    // leaving it behind is the same foreign-key failure that made this delete
    // unusable.
    for table in MIGRATED_TO_KEPT_THREAD {
        conn.execute(
            &format!("DELETE FROM {table} WHERE workspace_id = ?1"),
            params![workspace_id],
        )?;
    }

    // 3. The workspace row.
    const DELETE_WORKSPACE_SQL: &str = "DELETE FROM workspaces WHERE id = ?1";
    conn.execute(DELETE_WORKSPACE_SQL, params![workspace_id])?;
    Ok(())
}

/// Defensive / one-time sweep: hard-delete any workspaces left in the legacy
/// soft-deleted state (`deleted_at IS NOT NULL`), along with all their scoped
/// rows. `delete_workspace` now hard-deletes, so this only reclaims pre-existing
/// rows. Runs once at startup. Returns the number purged.
pub fn purge_soft_deleted_workspaces() -> Result<usize, crate::Error> {
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

pub fn get_workspace(workspace_id: &str) -> Result<Option<WorkspaceRecord>, crate::Error> {
    let conn = connect()?;
    get_workspace_in(&conn, workspace_id)
}

pub(super) fn get_workspace_in(
    conn: &Connection,
    workspace_id: &str,
) -> Result<Option<WorkspaceRecord>, crate::Error> {
    conn.query_row(
        &format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE id = ?1"),
        params![workspace_id],
        workspace_from_row,
    )
    .optional()
    .map_err(crate::Error::from)
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
                 VALUES ('{ws}_a', '{ws}', '{ws}_t', '{ws}_r', 'A', 'markdown', 1, 1);",
        ))
        .expect("seed workspace graph");
    }

    fn total(conn: &Connection) -> i64 {
        let tables = ["workspaces", "threads", "runs", "artifacts"];
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
    /// Deleting a workspace keeps the conversations that were archived out of
    /// it: each becomes an ordinary (chat-mode) conversation with a shelf of its
    /// own, while the conversations still in the workspace are deleted with it.
    #[test]
    fn delete_workspace_keeps_archived_conversations_as_chats() {
        let (_home, conn) = guarded_conn("ws_delete_keeps_archived");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                 VALUES ('ws1', 'W', 'user', '/tmp/ws1', 1, 1);
             INSERT INTO threads (id, workspace_id, mode, title, status, agent_session_id,
                 created_at, updated_at)
                 VALUES ('t_live', 'ws1', 'workspace', 'Live', 'active', 'sess_live', 1, 1),
                        ('t_shelf', 'ws1', 'workspace', 'Shelved', 'archived', 'sess_shelf', 1, 1);
             INSERT INTO artifacts (id, workspace_id, thread_id, title, artifact_type,
                 created_at, updated_at)
                 VALUES ('a1', 'ws1', 't_shelf', 'A', 'file', 1, 1);",
        )
        .expect("seed");

        delete_workspace_in(&conn, "ws1").expect("delete");

        // The conversation still in the workspace went with it; the archived one
        // survives as a chat, still shelved, on a temporary shelf of its own.
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM threads WHERE id = 't_live'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        let (workspace_id, mode, status): (String, String, String) = conn
            .query_row(
                "SELECT workspace_id, mode, status FROM threads WHERE id = 't_shelf'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("kept thread");
        assert_eq!(mode, "chat");
        assert_eq!(status, "archived");
        let kind: String = conn
            .query_row(
                "SELECT kind FROM workspaces WHERE id = ?1",
                [&workspace_id],
                |r| r.get(0),
            )
            .expect("chat shelf");
        assert_eq!(kind, "temporary");
        // Its artifacts and its Agent session travel with it: the conversation is
        // still restorable, so none of its rows may be deleted or tombstoned.
        let artifact_workspace: String = conn
            .query_row(
                "SELECT workspace_id FROM artifacts WHERE id = 'a1'",
                [],
                |r| r.get(0),
            )
            .expect("artifact");
        assert_eq!(artifact_workspace, workspace_id);
        let tombstoned: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_delete_outbox WHERE session_id = 'sess_shelf'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(tombstoned, 0);
        assert!(get_workspace("ws1").expect("get").is_none());
    }

    /// Deleting a *shelved* workspace keeps the conversations it was hiding, as
    /// live chats, and deletes the ones the reader had already shelved out of it.
    ///
    /// The rule is "a conversation is kept when the reader's own decision about
    /// it differs from the decision about its workspace". Shelving a folder
    /// hides everything in it without deciding anything about those
    /// conversations — so "delete this shelved folder" is about the folder and
    /// what it hid, and the conversations the reader never shelved come back to
    /// the conversations list instead of being deleted with it. A conversation
    /// shelved on its own was an explicit decision, and it goes with the folder.
    #[test]
    fn deleting_a_shelved_workspace_releases_its_hidden_conversations() {
        let (_home, conn) = guarded_conn("ws_delete_shelved_workspace");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, archived_at, created_at, updated_at)
                 VALUES ('ws2', 'W', 'user', '/tmp/ws2', 50, 1, 1);
             INSERT INTO threads (id, workspace_id, mode, title, status, agent_session_id,
                 created_at, updated_at)
                 VALUES ('t_hidden', 'ws2', 'workspace', 'Hidden', 'active', 'sess_hidden', 1, 1),
                        ('t_shelf', 'ws2', 'workspace', 'Shelved', 'archived', 'sess_shelf', 1, 1);",
        )
        .expect("seed");

        delete_workspace_in(&conn, "ws2").expect("delete");

        // The conversation the folder was hiding lives on, as a live chat: the
        // reader gets it back in the conversations list, ready to continue.
        let (workspace_id, mode, status, archived_at): (String, String, String, Option<i64>) = conn
            .query_row(
                "SELECT workspace_id, mode, status, archived_at FROM threads WHERE id = 't_hidden'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("kept thread");
        assert_eq!(mode, "chat");
        assert_eq!(
            status, "active",
            "it was not shelved, so it comes back live"
        );
        assert_eq!(archived_at, None);
        let kind: String = conn
            .query_row(
                "SELECT kind FROM workspaces WHERE id = ?1",
                [&workspace_id],
                |r| r.get(0),
            )
            .expect("chat shelf");
        assert_eq!(kind, "temporary");

        // The conversation the reader had shelved on its own goes with the
        // folder: two shelvings of the same thing.
        let shelved_left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE id = 't_shelf'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(shelved_left, 0);
        assert!(get_workspace("ws2").expect("get").is_none());
    }

    /// A shelved conversation carries its review history out of the workspace.
    ///
    /// `review_snapshots` and `review_changesets` name a workspace through an FK,
    /// and only `artifacts` used to be re-pointed at the kept conversation's new
    /// shelf — so deleting a workspace that held an archived conversation failed
    /// outright with "FOREIGN KEY constraint failed" (measured on a copy of a
    /// real database: 2 snapshots and 1 changeset on the workspace being
    /// deleted). The whole delete is one transaction, so the reader was left with
    /// nothing done and only "FOREIGN KEY constraint failed" in the dialog.
    #[test]
    fn a_kept_conversation_takes_its_review_rows_with_it() {
        let (_home, conn) = guarded_conn("ws_delete_keeps_review_rows");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                 VALUES ('ws1', 'W', 'user', '/tmp/ws1', 1, 1);
             INSERT INTO threads (id, workspace_id, mode, title, status, agent_session_id,
                 created_at, updated_at)
                 VALUES ('t_shelf', 'ws1', 'workspace', 'Shelved', 'archived', 'sess_shelf', 1, 1);
             INSERT INTO runs (id, thread_id, status, created_at, updated_at)
                 VALUES ('r1', 't_shelf', 'completed', 1, 1);
             INSERT INTO review_snapshots (id, workspace_id, thread_id, run_id, phase,
                 status, created_at)
                 VALUES ('s1', 'ws1', 't_shelf', 'r1', 'before', 'ready', 1);
             INSERT INTO review_changesets (id, thread_id, run_id, title, status,
                 workspace_id, created_at, updated_at)
                 VALUES ('c1', 't_shelf', 'r1', 'C', 'ready', 'ws1', 1, 1);",
        )
        .expect("seed");

        delete_workspace_in(&conn, "ws1")
            .expect("a kept conversation's review rows must not block the delete");

        let (thread_workspace, mode): (String, String) = conn
            .query_row(
                "SELECT workspace_id, mode FROM threads WHERE id = 't_shelf'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("kept thread");
        assert_eq!(mode, "chat");
        // The rows and the conversation share one workspace, so the review
        // history is still readable from the conversation it belongs to.
        for (table, id) in [("review_snapshots", "s1"), ("review_changesets", "c1")] {
            let owner: String = conn
                .query_row(
                    &format!("SELECT workspace_id FROM {table} WHERE id = ?1"),
                    [id],
                    |row| row.get(0),
                )
                .unwrap_or_else(|error| panic!("{table} {id} vanished or moved: {error}"));
            assert_eq!(owner, thread_workspace, "{table} stayed behind");
        }
        assert!(get_workspace("ws1").expect("get").is_none());
    }

    /// Review rows whose conversation goes with the workspace are deleted with
    /// it rather than blocking it: their `workspace_id` is `NOT NULL` on
    /// `review_snapshots`, so they cannot simply be orphaned.
    #[test]
    fn review_rows_of_a_deleted_conversation_do_not_block_the_delete() {
        let (_home, conn) = guarded_conn("ws_delete_review_rows_of_deleted");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                 VALUES ('ws1', 'W', 'user', '/tmp/ws1', 1, 1);
             INSERT INTO threads (id, workspace_id, mode, title, status, agent_session_id,
                 created_at, updated_at)
                 VALUES ('t_live', 'ws1', 'workspace', 'Live', 'active', 'sess_live', 1, 1);
             INSERT INTO runs (id, thread_id, status, created_at, updated_at)
                 VALUES ('r1', 't_live', 'completed', 1, 1);
             INSERT INTO review_snapshots (id, workspace_id, thread_id, run_id, phase,
                 status, created_at)
                 VALUES ('s1', 'ws1', 't_live', 'r1', 'after', 'ready', 1);
             INSERT INTO review_changesets (id, thread_id, run_id, title, status,
                 workspace_id, created_at, updated_at)
                 VALUES ('c1', 't_live', 'r1', 'C', 'ready', 'ws1', 1, 1);",
        )
        .expect("seed");

        delete_workspace_in(&conn, "ws1").expect("delete");

        for table in ["review_snapshots", "review_changesets"] {
            let left: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE workspace_id = 'ws1'"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(left, 0, "{table} rows outlived the workspace they name");
        }
        assert!(get_workspace("ws1").expect("get").is_none());
    }

    /// Deleting a workspace that holds a shelved conversation leaves that
    /// conversation where the reader can find it: the archived list, with a
    /// heading of its own, and restorable.
    ///
    /// The delete moves it onto a chat shelf (`kind = 'temporary'`), and the
    /// archived *page* only draws `kind = 'user'` rows as headings — so the
    /// confirmation's "kept as an independent conversation you can unarchive" is
    /// only true if the row is still listed. This test is the end-to-end answer:
    /// delete, then read both lists the way the rail reads them.
    #[test]
    fn a_kept_conversation_stays_on_the_shelf_and_can_be_restored() {
        let (_home, conn) = guarded_conn("ws_delete_kept_stays_findable");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                 VALUES ('ws1', 'W', 'user', '/tmp/ws1', 1, 1);
             INSERT INTO threads (id, workspace_id, mode, title, status, agent_session_id,
                 created_at, updated_at)
                 VALUES ('t_live', 'ws1', 'workspace', 'Live', 'active', 'sess_live', 1, 1),
                        ('t_shelf', 'ws1', 'workspace', 'Shelved', 'archived', 'sess_shelf', 1, 1);",
        )
        .expect("seed");

        delete_workspace_in(&conn, "ws1").expect("delete");
        drop(conn);

        // The archived stream is what the shelf's loose rows come from.
        let archived =
            crate::store::paging::list_threads_page(&crate::store::ThreadScope::Archived, None, 50)
                .expect("archived page")
                .threads;
        assert!(
            archived.iter().any(|thread| thread.id == "t_shelf"),
            "the kept conversation is not in the archived list: {:?}",
            archived.iter().map(|t| &t.id).collect::<Vec<_>>()
        );
        // And the chat section does not claim it: it is still shelved.
        let chat =
            crate::store::paging::list_threads_page(&crate::store::ThreadScope::Chat, None, 50)
                .expect("chat page")
                .threads;
        assert!(!chat.iter().any(|thread| thread.id == "t_shelf"));

        // Restoring it needs nothing else: it lands in the chat section on its
        // own shelf, which is what the confirmation promised.
        let restored = crate::store::restore_thread_tree("t_shelf").expect("restore");
        assert_eq!(restored.status, "active");
        let chat_after =
            crate::store::paging::list_threads_page(&crate::store::ThreadScope::Chat, None, 50)
                .expect("chat page")
                .threads;
        assert!(
            chat_after.iter().any(|thread| thread.id == "t_shelf"),
            "a restored kept conversation did not reach the chat list"
        );
    }

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
        let conn = connect().expect("connect");
        assert_eq!(
            future_app_workspaces::find_user_by_path(&conn, &real)
                .expect("find")
                .map(|workspace| workspace.id),
            Some(created.id)
        );
        drop(conn);

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
        // The row and its directory arrive together: the Files tab lists this
        // path as soon as the conversation exists, so a missing directory here
        // is the "cannot read this directory" state the panel used to show.
        assert!(
            std::path::Path::new(&created.path).is_dir(),
            "the chat workspace directory exists with its row: {}",
            created.path
        );

        // Idempotent on the same thread id…
        let again = get_or_create_chat_workspace("thread_x", Some("Titled".to_string()))
            .expect("idempotent");
        assert_eq!(again.id, created.id);

        // …and a titled creation uses the title.
        let titled =
            get_or_create_chat_workspace("thread_y", Some("Poem".to_string())).expect("titled");
        assert_eq!(titled.name, "Poem Workspace");
    }

    /// A chat conversation's cwd lands on its own temporary workspace (the
    /// scratch directory is not a project), and the workspace's stored path
    /// follows it — the desktop half of the `cwd_changed` filing.
    #[test]
    fn filing_a_chat_session_moves_its_temporary_workspace() {
        let _home = crate::auth_store::test_support::HomeGuard::new("ws_file_chat");
        let conn = connect().expect("connect");
        apply_schema(&conn).expect("apply schema");
        drop(conn);

        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Chat".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("sess-chat".to_string()),
        })
        .expect("chat thread");
        let before = get_workspace(&thread.workspace_id)
            .expect("get")
            .expect("exists");
        assert_eq!(before.kind, "temporary");

        // The agent reports the session-named scratch directory.
        let moved_path = crate::store::chat_workspace_path("sess-chat").expect("chat path");
        assert_eq!(
            file_session_workspace("sess-chat", &moved_path.display().to_string())
                .expect("file")
                .expect("a filing"),
            workspaces::CwdFiling::ChatWorkspaceUpdated
        );
        let after = get_workspace(&thread.workspace_id)
            .expect("get")
            .expect("exists");
        assert_eq!(after.path, moved_path.display().to_string());
        assert_eq!(after.kind, "temporary");
        assert!(
            list_workspaces()
                .expect("list")
                .iter()
                .all(|workspace| workspace.kind != "user"),
            "a scratch directory must not become a user workspace"
        );
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

    /// The delete confirmation's numbers are the store's, not the loaded pages':
    /// a workspace counts what the delete removes and what it keeps, on both
    /// paths.
    #[test]
    fn delete_summary_counts_what_survives_and_what_goes() {
        let (_home, conn) = guarded_conn("ws_delete_summary");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                 VALUES ('ws1', 'W', 'user', '/tmp/ws1', 1, 1),
                        ('ws2', 'S', 'user', '/tmp/ws2', 1, 1);
             UPDATE workspaces SET archived_at = 9 WHERE id = 'ws2';
             INSERT INTO threads (id, workspace_id, mode, title, status, created_at, updated_at)
                 VALUES ('a1', 'ws1', 'workspace', 'A1', 'active', 1, 1),
                        ('a2', 'ws1', 'workspace', 'A2', 'active', 1, 1),
                        ('s1', 'ws1', 'workspace', 'S1', 'archived', 1, 1),
                        ('h1', 'ws2', 'workspace', 'H1', 'active', 1, 1),
                        ('h2', 'ws2', 'workspace', 'H2', 'active', 1, 1);",
        )
        .expect("seed");
        drop(conn);

        // From the active list: what it shows goes, what was already shelved out
        // of it survives on the shelf.
        let active = delete_summary("ws1").expect("summary");
        assert_eq!(active.deleted_threads, 2);
        assert_eq!(active.kept_live_threads, 0);
        assert_eq!(active.kept_archived_threads, 1);

        // From the archived shelf: what the folder hid goes, and the
        // conversations it was hiding come back as live chats.
        let shelved = delete_summary("ws2").expect("summary");
        assert_eq!(shelved.deleted_threads, 0);
        assert_eq!(shelved.kept_live_threads, 2);
        assert_eq!(shelved.kept_archived_threads, 0);
    }

    /// The two halves of the rule, side by side: the same workspace graph
    /// answers "delete" from one list and "keep" from the other.
    #[test]
    fn delete_summary_treats_the_two_lists_differently() {
        let (_home, conn) = guarded_conn("ws_delete_summary_paths");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                 VALUES ('ws1', 'W', 'user', '/tmp/ws1', 1, 1),
                        ('ws2', 'S', 'user', '/tmp/ws2', 1, 1);
             UPDATE workspaces SET archived_at = 9 WHERE id = 'ws2';
             INSERT INTO threads (id, workspace_id, mode, title, status, created_at, updated_at)
                 VALUES ('live_ws1', 'ws1', 'workspace', 'L1', 'active', 1, 1),
                        ('shelf_ws1', 'ws1', 'workspace', 'S1', 'archived', 1, 1),
                        ('live_ws2', 'ws2', 'workspace', 'L2', 'active', 1, 1),
                        ('shelf_ws2', 'ws2', 'workspace', 'S2', 'archived', 1, 1);",
        )
        .expect("seed");
        drop(conn);

        let from_active = delete_summary("ws1").expect("active summary");
        let from_shelf = delete_summary("ws2").expect("shelved summary");

        // Same shape, opposite answers: the live one dies for the active folder
        // and survives for the shelved one; the shelved one is the mirror image.
        assert_eq!(
            (
                from_active.deleted_threads,
                from_active.kept_archived_threads
            ),
            (1, 1)
        );
        assert_eq!(
            (from_shelf.deleted_threads, from_shelf.kept_live_threads),
            (1, 1)
        );
    }

    /// A folder on the shelf shows every conversation it holds — the ones it was
    /// hiding and the ones shelved on their own before it. Restoring it returns
    /// all of them: the reader should not have to bring the rest back one by one.
    #[test]
    fn restoring_a_shelved_workspace_returns_every_conversation_it_holds() {
        let (_home, conn) = guarded_conn("ws_restore_all");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, archived_at, created_at, updated_at)
                 VALUES ('ws', 'W', 'user', '/tmp/ws', 5, 1, 1);
             INSERT INTO threads (id, workspace_id, mode, title, status, archived_at,
                 created_at, updated_at)
                 VALUES ('shelved_before', 'ws', 'workspace', 'S', 'archived', 4, 1, 1),
                        ('hidden_one', 'ws', 'workspace', 'H1', 'active', NULL, 1, 1),
                        ('hidden_two', 'ws', 'workspace', 'H2', 'active', NULL, 1, 1);",
        )
        .expect("seed");

        let restored = restore_workspace("ws").expect("restore");
        assert_eq!(restored.archived_at, None, "the folder is back");
        let still_shelved: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM threads
                  WHERE workspace_id = 'ws' AND (status != 'active' OR archived_at IS NOT NULL)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(still_shelved, 0, "every conversation came back with it");
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

    /// A workspace `future workspace add` wrote — the CLI creates the database
    /// and the table without the app having run — must be a workspace the
    /// desktop can read: listed in the sidebar, found by path, and surviving the
    /// app's own schema pass when it starts.
    #[test]
    fn a_workspace_written_by_the_cli_is_readable_by_the_desktop() {
        let _home = crate::auth_store::test_support::HomeGuard::new("ws_cli_row");
        let dir = std::env::temp_dir().join(format!("futureos-cli-ws-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the directory the CLI would point at");

        // What `future workspace add` does: make the app directory, open the
        // database (the app has never created it), create the table, insert.
        {
            let database = db_path().expect("app db path");
            std::fs::create_dir_all(database.parent().expect("app dir")).expect("create app dir");
            let mut conn = Connection::open(database).expect("open");
            future_app_workspaces::ensure_table(&conn).expect("create the table");
            future_app_workspaces::create(
                &mut conn,
                &dir.display().to_string(),
                Some("From CLI".to_string()),
                None,
                false,
                1,
            )
            .expect("create");
        }

        let listed = list_workspaces().expect("the desktop lists the CLI's row");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "From CLI");
        assert_eq!(listed[0].kind, "user");
        let conn = connect().expect("connect");
        let found = future_app_workspaces::find_user_by_path(&conn, &dir)
            .expect("find")
            .expect("the desktop finds the CLI's workspace by path");
        assert_eq!(found.id, listed[0].id);
        drop(conn);

        // The app's startup schema pass runs over whatever is there.
        let conn = connect().expect("connect");
        apply_schema(&conn).expect("apply the desktop schema over the CLI's database");
        drop(conn);
        assert_eq!(list_workspaces().expect("list").len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The CLI creates this table itself when the desktop app has never run
    /// (`future_app_workspaces::ensure_table`), and the desktop creates it from
    /// its own migrations. The two must describe the same table, or a
    /// CLI-created database would decode (or fail to decode) differently once
    /// the app opens it.
    #[test]
    fn the_shared_table_matches_the_desktop_schema() {
        fn columns(conn: &Connection) -> Vec<String> {
            let mut statement = conn
                .prepare("PRAGMA table_info(workspaces)")
                .expect("read table info");
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })
                .expect("query table info");
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .expect("collect columns")
                .into_iter()
                .map(|(name, kind, not_null, default)| {
                    format!("{name} {kind} notnull={not_null} default={default:?}")
                })
                .collect()
        }

        let desktop = test_conn();
        let shared = Connection::open_in_memory().expect("open in-memory database");
        future_app_workspaces::ensure_table(&shared).expect("shared schema");

        assert_eq!(
            columns(&desktop),
            columns(&shared),
            "future-app-workspaces must create the table the desktop migrates"
        );
    }

    /// A conversation's cwd change files it under the workspace for the
    /// directory — the desktop-side half of what the app does when it observes a
    /// `cwd_changed`, reachable without a bridge so the CLI can do it too.
    #[test]
    fn filing_a_session_files_the_thread_and_marks_the_catalogue() {
        let _home = crate::auth_store::test_support::HomeGuard::new("ws_file_session");
        let conn = connect().expect("connect");
        apply_schema(&conn).expect("apply schema");
        drop(conn);

        // An imported conversation: a thread bound to the agent session.
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("New".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("sess-file".to_string()),
        })
        .expect("thread");
        crate::store::take_catalog_dirty();

        let directory = tempfile::tempdir().expect("directory");
        let filing = file_session_workspace("sess-file", &directory.path().display().to_string())
            .expect("file")
            .expect("a filing");
        let workspaces::CwdFiling::WorkspaceCreated(workspace_id) = filing else {
            panic!("expected a created workspace, got {filing:?}");
        };
        assert!(
            crate::store::take_catalog_dirty(),
            "a filing is a workspace change the phone's catalogue must learn about"
        );
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("get")
                .expect("exists")
                .workspace_id,
            workspace_id
        );

        // A session the desktop has never imported has nothing to file.
        assert_eq!(
            file_session_workspace("sess-ghost", &directory.path().display().to_string())
                .expect("file"),
            Some(workspaces::CwdFiling::NotImported)
        );
    }

    /// The desktop's own workspace reads are `future-app-workspaces`: a row
    /// stored under an older spelling of the same directory resolves to its
    /// workspace instead of a duplicate, and the app sees the row the CLI wrote
    /// (`a_workspace_written_by_the_cli_is_readable_by_the_desktop` above).
    #[test]
    fn an_aliased_spelling_resolves_to_its_workspace() {
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

        let found = future_app_workspaces::find_user_by_path(&conn, &canonical)
            .expect("query")
            .expect("an aliased spelling must resolve to its workspace");
        assert_eq!(found.id, "ws-alias");
        assert_eq!(found.path, aliased);
    }
}
