use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::db::*;
use super::records::*;
use super::util::*;
use super::workspaces::{
    get_or_create_chat_workspace_in, get_or_create_user_workspace_in, get_workspace_in,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRecord {
    pub id: String,
    pub workspace_id: String,
    pub mode: String,
    pub title: String,
    pub status: String,
    pub pinned: bool,
    pub readonly: bool,
    // model_provider, model_id, thinking_level — dropped, now from agent
    pub agent_session_id: Option<String>,
    pub last_message_at: Option<i64>,
    pub last_opened_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
    pub deleted_at: Option<i64>,
}

// `pinned`/`readonly` are `bool` fields; rusqlite's `FromSql for bool` maps the
// stored 0/1 integers (same as the prior explicit `i64 != 0`).
sql_record!(pub(super) THREAD_COLUMNS, thread_from_row -> ThreadRecord {
    id, workspace_id, mode, title, status, pinned, readonly,
    agent_session_id, last_message_at, last_opened_at,
    created_at, updated_at, archived_at, deleted_at,
});

pub fn list_threads() -> Result<Vec<ThreadRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {THREAD_COLUMNS}
             FROM threads
             WHERE status != 'deleted'
             ORDER BY pinned DESC, COALESCE(last_message_at, updated_at, created_at) DESC"
    ))?;
    let rows = stmt.query_map([], thread_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn get_recent_thread() -> Result<Option<ThreadRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        &format!(
            "SELECT {THREAD_COLUMNS}
         FROM threads
         WHERE status = 'active'
         ORDER BY COALESCE(last_opened_at, last_message_at, updated_at) DESC
         LIMIT 1"
        ),
        [],
        thread_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

/// Find an active thread by its `agent_session_id` (used to map a remote
/// (phone) session id back to the GUI thread that owns it).
pub fn find_thread_by_agent_session(
    session_id: &str,
) -> Result<Option<ThreadRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        &format!(
            "SELECT {THREAD_COLUMNS} FROM threads \
             WHERE agent_session_id = ?1 AND status != 'deleted' LIMIT 1"
        ),
        params![session_id],
        thread_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

pub fn create_thread(input: CreateThreadInput) -> Result<ThreadRecord, crate::AppError> {
    let mode = normalize_mode(&input.mode)?;
    let now = now_millis();
    let thread_id = create_id("thread");
    // Only use a pre-existing agent session ID (e.g. from fork). For normal
    // threads leave it empty — the agent generates the ID on first prompt
    // and it's persisted back via update_thread_session_id.
    let agent_session_id = input.agent_session_id.filter(|id| !id.is_empty());
    let title = input.title.unwrap_or_else(|| {
        if mode == "chat" {
            "New Chat".to_string()
        } else {
            "Workspace Thread".to_string()
        }
    });

    // Resolve/create the workspace and insert the thread in one transaction so a
    // crash between the two writes can't leave an orphan workspace with no thread
    // pointing at it. `&tx` deref-coerces to `&Connection` for the `_in` helpers.
    // BEGIN IMMEDIATE because the `_in` helpers are SELECT-then-INSERT: under a
    // deferred transaction in WAL a concurrent commit between the read and the
    // write fails the whole create with SQLITE_BUSY_SNAPSHOT instead of being
    // serialized (matches the standalone get_or_create_user_workspace).
    let mut conn = connect()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    let workspace = if mode == "chat" {
        get_or_create_chat_workspace_in(&tx, &thread_id, Some(title.clone()))?
    } else if let Some(workspace_id) = input.workspace_id {
        loaded(get_workspace_in(&tx, &workspace_id)?, "Workspace")?
    } else {
        let raw_path = input
            .workspace_path
            .ok_or_else(|| "workspacePath is required for workspace threads.".to_string())?;
        let path = expand_tilde(&raw_path)?;
        let name = input
            .workspace_name
            .unwrap_or_else(|| workspace_name_from_path(&path));
        get_or_create_user_workspace_in(&tx, name, path, None)?
    };

    tx.execute(
        "INSERT INTO threads (
             id, workspace_id, mode, title, status, pinned, readonly,
             agent_session_id, last_opened_at, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, 'active', 0, 0, ?5, ?6, ?6, ?6)",
        params![thread_id, workspace.id, mode, title, agent_session_id, now],
    )?;

    let thread = loaded(get_thread_in(&tx, &thread_id)?, "Created thread")?;
    tx.commit()?;
    mark_catalog_dirty();
    Ok(thread)
}

pub fn get_thread(thread_id: &str) -> Result<Option<ThreadRecord>, crate::AppError> {
    let conn = connect()?;
    get_thread_in(&conn, thread_id)
}

pub(super) fn get_thread_in(
    conn: &Connection,
    thread_id: &str,
) -> Result<Option<ThreadRecord>, crate::AppError> {
    conn.query_row(
        &format!("SELECT {THREAD_COLUMNS} FROM threads WHERE id = ?1"),
        params![thread_id],
        thread_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

pub fn rename_thread(input: RenameThreadInput) -> Result<ThreadRecord, crate::AppError> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err("title cannot be empty.".to_string().into());
    }

    let now = now_millis();
    const SQL: &str = "UPDATE threads
         SET title = ?1, updated_at = ?2
         WHERE id = ?3 AND status != 'deleted'";
    let conn = connect()?;
    conn.execute(SQL, params![title, now, input.thread_id])?;

    let thread = loaded(get_thread(&input.thread_id)?, "Thread")?;
    mark_catalog_dirty();
    Ok(thread)
}

pub(super) fn sync_thread_title_in(
    conn: &Connection,
    thread_id: &str,
    title: &str,
) -> Result<bool, crate::AppError> {
    let title = title.trim();
    if title.is_empty() {
        return Ok(false);
    }
    const SQL: &str = "UPDATE threads
         SET title = ?1
         WHERE id = ?2 AND status != 'deleted' AND title != ?1";
    let changed = conn.execute(SQL, params![title, thread_id])?;
    if changed > 0 {
        mark_catalog_dirty();
    }
    Ok(changed > 0)
}

/// Background convergence of the title toward the agent's `session_name` —
/// the name shared with every client (TUI `/name`, CLI, channels), whose
/// renames never reach the GUI DB. Unlike `rename_thread` this is not a user
/// edit: `updated_at` is untouched so the sidebar order is undisturbed, and
/// matching titles are a no-op. Returns whether the title changed.
pub fn sync_thread_title(thread_id: &str, title: &str) -> Result<bool, crate::AppError> {
    let conn = connect()?;
    sync_thread_title_in(&conn, thread_id, title)
}

pub fn update_thread_model(input: UpdateThreadModelInput) -> Result<ThreadRecord, crate::AppError> {
    // Model is now managed by the agent (set_model RPC). The GUI cache
    // (agentStateCache) handles reads; DB write is a no-op.
    loaded(get_thread(&input.thread_id)?, "Thread")
}

pub fn update_thread_thinking_level(
    input: UpdateThreadThinkingLevelInput,
) -> Result<ThreadRecord, crate::AppError> {
    // Thinking level is now managed by the agent (set_thinking_level RPC).
    loaded(get_thread(&input.thread_id)?, "Thread")
}

/// Persist the agent-generated session id after the first prompt creates it.
pub fn update_thread_session_id(thread_id: &str, session_id: &str) -> Result<(), crate::AppError> {
    let now = now_millis();
    const SQL: &str = "UPDATE threads SET agent_session_id = ?1, updated_at = ?2
         WHERE id = ?3 AND status != 'deleted'";
    let conn = connect()?;
    conn.execute(SQL, params![session_id, now, thread_id])?;
    mark_catalog_dirty();
    Ok(())
}

/// Record that a thread was opened without treating the visit as message
/// activity. `last_opened_at` chooses the startup thread, but deliberately
/// does not affect the sidebar's conversation order.
pub fn mark_thread_opened(thread_id: &str) -> Result<(), crate::AppError> {
    let now = now_millis();
    const SQL: &str = "UPDATE threads SET last_opened_at = ?1
         WHERE id = ?2 AND status != 'deleted'";
    let conn = connect()?;
    conn.execute(SQL, params![now, thread_id])?;
    Ok(())
}

/// Record conversation activity. This is kept separate from `updated_at`:
/// metadata changes such as a rename must not reorder the rail.
pub(super) fn mark_thread_message_activity_in(
    conn: &Connection,
    thread_id: &str,
    now: i64,
) -> Result<(), crate::AppError> {
    const SQL: &str = "UPDATE threads SET last_message_at = ?1
         WHERE id = ?2 AND status != 'deleted'";
    conn.execute(SQL, params![now, thread_id])?;
    Ok(())
}

/// Move a thread to a different workspace (e.g. when cwd changes).
pub fn move_thread_to_workspace(
    thread_id: &str,
    workspace_id: &str,
) -> Result<(), crate::AppError> {
    let now = now_millis();
    const SQL: &str = "UPDATE threads SET workspace_id = ?1, updated_at = ?2
         WHERE id = ?3 AND status != 'deleted'";
    let conn = connect()?;
    conn.execute(SQL, params![workspace_id, now, thread_id])?;
    mark_catalog_dirty();
    Ok(())
}

pub fn pin_thread(input: PinThreadInput) -> Result<ThreadRecord, crate::AppError> {
    // `updated_at` is deliberately untouched: pinning is an ordering flag, not
    // an activity event. If it stamped `updated_at`, unpinning a thread would
    // push it to the top of the recency sort (it just became "most recently
    // updated") and the sidebar order would appear not to change.
    let pinned = if input.pinned { 1 } else { 0 };
    const SQL: &str = "UPDATE threads
         SET pinned = ?1
         WHERE id = ?2 AND status != 'deleted'";
    let conn = connect()?;
    conn.execute(SQL, params![pinned, input.thread_id])?;

    loaded(get_thread(&input.thread_id)?, "Thread")
}

pub(super) fn update_thread_status(
    thread_id: &str,
    status: &str,
) -> Result<ThreadRecord, crate::AppError> {
    let now = now_millis();
    let archived_at = if status == "archived" {
        Some(now)
    } else {
        None
    };
    const SQL: &str = "UPDATE threads
         SET status = ?1, archived_at = ?2, updated_at = ?3
         WHERE id = ?4 AND status != 'deleted'";
    let conn = connect()?;
    conn.execute(SQL, params![status, archived_at, now, thread_id])?;

    loaded(get_thread(thread_id)?, "Thread")
}

pub fn archive_thread(thread_id: &str) -> Result<ThreadRecord, crate::AppError> {
    update_thread_status(thread_id, "archived")
}

pub fn restore_thread(thread_id: &str) -> Result<ThreadRecord, crate::AppError> {
    update_thread_status(thread_id, "active")
}

/// FK-safe hard delete of every child row belonging to `thread_id` (the
/// `threads` row itself is left to the caller). `PRAGMA foreign_keys = ON` is
/// enforced, so the order matters: children before parents, and the
/// `runs.trigger_message_id` ↔ `messages.run_id` cycle is broken by nulling
/// `runs.trigger_message_id` before deleting messages. Artifacts are workspace
/// assets, not conversation data — they are detached (`thread_id`/`run_id`
/// nulled), never destroyed with the thread.
pub(super) fn delete_thread_children_in(
    conn: &Connection,
    thread_id: &str,
) -> rusqlite::Result<()> {
    // Review data: file changes → changesets → snapshots (all source kinds).
    const DELETE_FILE_CHANGES_SQL: &str = "DELETE FROM review_file_changes WHERE changeset_id IN (
             SELECT id FROM review_changesets WHERE thread_id = ?1
         )";
    conn.execute(DELETE_FILE_CHANGES_SQL, params![thread_id])?;
    const DELETE_CHANGESETS_SQL: &str = "DELETE FROM review_changesets WHERE thread_id = ?1";
    conn.execute(DELETE_CHANGESETS_SQL, params![thread_id])?;
    const DELETE_SNAPSHOTS_SQL: &str = "DELETE FROM review_snapshots WHERE thread_id = ?1";
    conn.execute(DELETE_SNAPSHOTS_SQL, params![thread_id])?;
    // Messages table dropped — markdown references cleaned by markdown_refs module.
    // Approvals reference threads/runs/tool_calls — clear before tool_calls.
    const DELETE_APPROVALS_SQL: &str = "DELETE FROM approval_requests WHERE thread_id = ?1";
    conn.execute(DELETE_APPROVALS_SQL, params![thread_id])?;
    // Tool outputs → tool calls (scoped through the thread's runs).
    // tool_outputs/run_events/tool_calls tables dropped

    // Detach workspace-level artifacts from the thread/run being removed.
    const DETACH_RUN_SQL: &str = "UPDATE artifacts SET run_id = NULL
         WHERE run_id IN (SELECT id FROM runs WHERE thread_id = ?1)";
    conn.execute(DETACH_RUN_SQL, params![thread_id])?;
    const DETACH_THREAD_SQL: &str = "UPDATE artifacts SET thread_id = NULL WHERE thread_id = ?1";
    conn.execute(DETACH_THREAD_SQL, params![thread_id])?;
    // Drop the per-run event logs (and any buffered events) for this thread's
    // runs before the run rows go — otherwise the JSONL files leak on disk.
    {
        let mut stmt = conn.prepare("SELECT id FROM runs WHERE thread_id = ?1")?;
        let rows = stmt.query_map(params![thread_id], |row| row.get::<_, String>(0))?;
        let run_ids = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        for run_id in run_ids {
            super::delete_run_events_file(&run_id);
            super::clear_run_event_buffer(&run_id);
        }
    }
    // Break the runs ↔ messages FK cycle, then delete both.
    const NULL_TRIGGER_SQL: &str = "UPDATE runs SET trigger_message_id = NULL WHERE thread_id = ?1";
    conn.execute(NULL_TRIGGER_SQL, params![thread_id])?;
    conn.execute("DELETE FROM runs WHERE thread_id = ?1", params![thread_id])?;
    Ok(())
}

/// Hard delete a thread and every row that hangs off it. The conversation
/// content the GUI stores is only a rendered mirror of the agent's JSONL (the
/// source of truth); the caller deletes that JSONL separately (see
/// `commands::delete_thread`). Returns the pre-delete record so callers that
/// expected the old soft-delete return value keep working. Temp chat workspaces
/// are flagged for cleanup exactly as before.
///
/// When `delete_files` is true and the thread is chat-mode, the temporary
/// workspace directory on disk is removed immediately instead of being flagged
/// for background cleanup. Workspace-mode threads are never touched.
pub fn delete_thread(thread_id: &str) -> Result<ThreadRecord, crate::AppError> {
    delete_thread_inner(thread_id, false)
}

/// Like [`delete_thread`] but also removes the temporary chat workspace
/// directory on disk when `delete_files` is true. Workspace-mode threads
/// are never touched on disk regardless of this flag.
pub fn delete_thread_with_files(
    thread_id: &str,
    delete_files: bool,
) -> Result<ThreadRecord, crate::AppError> {
    delete_thread_inner(thread_id, delete_files)
}

/// Internal helper with the `delete_files` flag (see [`delete_thread`]).
pub(crate) fn delete_thread_inner(
    thread_id: &str,
    delete_files: bool,
) -> Result<ThreadRecord, crate::AppError> {
    let now = now_millis();
    let mut conn = connect()?;
    let thread = loaded(get_thread(thread_id)?, "Thread")?;
    // One transaction: the child cascade, the temp-workspace cleanup flag, and
    // the thread delete must land together — a crash between them would leak the
    // chat workspace directory forever (mirrors delete_workspace).
    let tx = conn.transaction()?;
    let session_id = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(&thread.id);
    // A session can be represented by more than one GUI thread during import
    // or migration. Only the last owner is allowed to tombstone its canonical
    // Agent session.
    const OWNER_SQL: &str = "SELECT COUNT(*) FROM threads
         WHERE COALESCE(NULLIF(TRIM(agent_session_id), ''), id) = ?1";
    let owner_count: i64 = tx.query_row(OWNER_SQL, [session_id], |row| row.get(0))?;
    if owner_count == 1 {
        super::deletions::enqueue_agent_session_delete_in(&tx, session_id)?;
    }
    delete_thread_children_in(&tx, thread_id)?;

    if thread.mode == "chat" {
        if delete_files {
            // Mark cleaned immediately (skip the pending_cleanup phase) so
            // orphans reconcilers won't re-attempt the now-removed directory.
            const CLEANED_SQL: &str = "UPDATE workspaces
                 SET cleanup_status = 'cleaned',
                     cleanup_requested_at = COALESCE(cleanup_requested_at, ?1),
                     cleaned_at = ?1,
                     updated_at = ?1
                 WHERE id = ?2
                   AND kind = 'temporary'";
            tx.execute(CLEANED_SQL, params![now, thread.workspace_id])?;
        } else {
            const PENDING_SQL: &str = "UPDATE workspaces
                 SET cleanup_status = 'pending_cleanup',
                     cleanup_requested_at = COALESCE(cleanup_requested_at, ?1),
                     updated_at = ?1
                 WHERE id = ?2
                   AND kind = 'temporary'
                   AND cleanup_status = 'active'";
            tx.execute(PENDING_SQL, params![now, thread.workspace_id])?;
        }
    }
    tx.execute("DELETE FROM threads WHERE id = ?1", params![thread_id])?;
    tx.commit()?;
    mark_catalog_dirty();

    // Remove the directory on disk AFTER the transaction commits — if the
    // directory deletion fails the DB row is already gone, which is the safer
    // failure mode (side-effect-last).
    if delete_files && thread.mode == "chat" {
        let dir = super::db::chat_workspace_path(thread_id)?;
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    Ok(thread)
}

/// Batch-delete multiple threads. For each thread:
/// - The DB row and children are hard-deleted.
/// - For chat-mode threads with `delete_files`, the temporary workspace
///   directory on disk is removed.
/// - For workspace-mode threads, only the DB row is deleted; files are never
///   touched regardless of `delete_files`.
///
/// Each thread delete is independent — one failure does not roll back
/// already-deleted siblings. Returns a summary.
pub fn batch_delete_threads(
    input: &super::records::BatchDeleteThreadsInput,
) -> Result<super::records::BatchDeleteResult, crate::AppError> {
    let mut deleted_count = 0usize;
    let mut failed: Vec<String> = Vec::new();

    for thread_id in &input.thread_ids {
        match delete_thread_inner(thread_id, input.delete_files) {
            Ok(_) => {
                deleted_count += 1;
            }
            Err(error) => {
                failed.push(format!("{thread_id}: {error}"));
            }
        }
    }

    Ok(super::records::BatchDeleteResult {
        deleted_count,
        failed,
    })
}

/// Defensive / one-time sweep: hard-delete any threads still parked in the
/// legacy `status = 'deleted'` soft-delete state, along with all their orphaned
/// child rows. `delete_thread` now hard-deletes, so no new such rows are
/// created; this reclaims pre-existing ones (their temp workspaces were already
/// flagged at soft-delete time). Runs once at startup. Returns the count purged.
pub fn purge_soft_deleted_threads() -> Result<usize, crate::AppError> {
    let mut conn = connect()?;
    let ids: Vec<String> = {
        let mut stmt = conn.prepare("SELECT id FROM threads WHERE status = 'deleted'")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    if ids.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    for id in &ids {
        delete_thread_children_in(&tx, id)?;
        tx.execute("DELETE FROM threads WHERE id = ?1", params![id])?;
    }
    tx.commit()?;
    Ok(ids.len())
}

#[cfg(test)]
mod tests {
    use super::get_thread_in;
    use crate::store::schema::SCHEMA;
    use crate::store::workspaces::get_or_create_user_workspace_in;
    use crate::store::workspaces::get_workspace_in;
    use rusqlite::{params, Connection};
    use std::path::PathBuf;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(SCHEMA).expect("initialize test schema");
        conn
    }

    fn workspace_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM workspaces", [], |row| row.get(0))
            .expect("count workspaces")
    }

    /// The workspace resolve/create and the thread insert commit together — both
    /// rows are visible after `commit`. (`create_thread` runs this on one tx; the
    /// `_in` helpers make that injectable for the in-memory DB here.)
    #[test]
    fn create_thread_persists_workspace_and_thread_atomically() {
        let mut conn = test_conn();
        let tx = conn.transaction().unwrap();
        let workspace = get_or_create_user_workspace_in(
            &tx,
            "Test Workspace".to_string(),
            PathBuf::from("/tmp/futureos-test-ws"),
            None,
        )
        .unwrap();
        tx.execute(
            "INSERT INTO threads (
                 id, workspace_id, mode, title, status, pinned, readonly,
                 agent_session_id, created_at, updated_at
             ) VALUES ('thread_ok', ?1, 'workspace', 'T', 'active', 0, 0, 'sess', 1, 1)",
            params![workspace.id],
        )
        .unwrap();
        let thread = get_thread_in(&tx, "thread_ok")
            .unwrap()
            .expect("thread row");
        tx.commit().unwrap();

        assert_eq!(thread.workspace_id, workspace.id);
        assert!(get_workspace_in(&conn, &workspace.id).unwrap().is_some());
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |row| row.get(0)).expect("count")
    }

    /// The hard-delete cascade removes every child row of a thread in an
    /// FK-safe order (foreign keys ON here, so a wrong order would error),
    /// breaks the runs↔messages cycle, and detaches — never deletes —
    /// workspace-level artifacts.
    #[test]
    fn delete_thread_children_hard_deletes_and_detaches_artifacts() {
        let conn = test_conn();
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("enable fk");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                 VALUES ('ws', 'W', 'user', '/tmp/ws', 1, 1);
             INSERT INTO threads (id, workspace_id, mode, title, status, pinned,
                 readonly, created_at, updated_at)
                 VALUES ('t1', 'ws', 'workspace', 'T', 'active', 0, 0, 1, 1);
             INSERT INTO runs (id, thread_id, status, created_at, updated_at)
                 VALUES ('r1', 't1', 'completed', 1, 1);
             INSERT INTO approval_requests (id, thread_id, run_id, tool_call_id,
                 kind, status, title, created_at, updated_at)
                 VALUES ('ap1', 't1', 'r1', 'tc1', 'tool', 'pending', 'A', 1, 1);
             INSERT INTO review_snapshots (id, workspace_id, thread_id, run_id,
                 phase, status, created_at)
                 VALUES ('rs1', 'ws', 't1', 'r1', 'before', 'ready', 1);
             INSERT INTO review_changesets (id, thread_id, run_id, title, status,
                 created_at, updated_at)
                 VALUES ('rc1', 't1', 'r1', 'C', 'ready', 1, 1);
             INSERT INTO review_file_changes (id, changeset_id, target_type,
                 change_type, created_at, updated_at)
                 VALUES ('rf1', 'rc1', 'file', 'modified', 1, 1);
             INSERT INTO artifacts (id, workspace_id, thread_id, run_id, title,
                 artifact_type, created_at, updated_at)
                 VALUES ('a1', 'ws', 't1', 'r1', 'Art', 'markdown', 1, 1);",
        )
        .expect("seed thread graph");

        super::delete_thread_children_in(&conn, "t1").expect("cascade delete");

        // Every conversation child row is gone. (run_events / tool_calls /
        // tool_outputs tables were dropped — their data now lives in the agent
        // JSONL / in-memory buffer, not SQLite.)
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM runs"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM approval_requests"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM review_snapshots"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM review_changesets"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM review_file_changes"), 0);
        // The thread row itself is left to the caller.
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM threads WHERE id = 't1'"),
            1
        );
        // The artifact survives, detached from the thread and run.
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM artifacts
                 WHERE id = 'a1' AND thread_id IS NULL AND run_id IS NULL"
            ),
            1
        );
    }

    /// Regression for B-11: a crash between the workspace write and the thread
    /// insert (modeled by dropping the tx without committing) must not leave an
    /// orphan workspace behind.
    #[test]
    fn rolled_back_create_thread_leaves_no_orphan_workspace() {
        let mut conn = test_conn();
        {
            let tx = conn.transaction().unwrap();
            get_or_create_user_workspace_in(
                &tx,
                "Doomed".to_string(),
                PathBuf::from("/tmp/futureos-doomed-ws"),
                None,
            )
            .unwrap();
            // tx dropped here without commit -> rollback.
        }
        assert_eq!(workspace_count(&conn), 0);
    }

    /// Title convergence from the agent's session_name must not disturb the
    /// sidebar order (`updated_at` untouched), skip matching titles, and
    /// reject empty titles.
    #[test]
    fn sync_thread_title_converges_without_touching_updated_at() {
        let conn = test_conn();
        let workspace = get_or_create_user_workspace_in(
            &conn,
            "Sync Workspace".to_string(),
            PathBuf::from("/tmp/futureos-sync-ws"),
            None,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (
                 id, workspace_id, mode, title, status, pinned, readonly,
                 agent_session_id, created_at, updated_at
             ) VALUES ('thread_sync', ?1, 'chat', 'old', 'active', 0, 0, 'sess', 1, 42)",
            params![workspace.id],
        )
        .unwrap();

        // Matching title: no-op. New title: applied. Empty title: rejected.
        assert!(!super::sync_thread_title_in(&conn, "thread_sync", "old").unwrap());
        assert!(super::sync_thread_title_in(&conn, "thread_sync", "agent name").unwrap());
        assert!(!super::sync_thread_title_in(&conn, "thread_sync", "   ").unwrap());

        let (title, updated_at): (String, i64) = conn
            .query_row(
                "SELECT title, updated_at FROM threads WHERE id = 'thread_sync'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(title, "agent name");
        assert_eq!(updated_at, 42);
    }

    // ── connect()-backed API surface (fake HOME) ────────────────────────────

    use super::*;
    use crate::store::db::test_support::guarded_conn;
    use crate::store::workspaces::get_workspace;

    fn seed_two_threads(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                 VALUES ('ws1', 'W1', 'user', '/tmp/ws1', 1, 1),
                        ('ws2', 'W2', 'user', '/tmp/ws2', 1, 1);
             INSERT INTO threads (id, workspace_id, mode, title, status, pinned,
                 agent_session_id, last_message_at, last_opened_at, created_at, updated_at)
                 VALUES
                 ('t1', 'ws1', 'chat', 'One', 'active', 1, 'sess1', 200, NULL, 1, 1),
                 ('t2', 'ws1', 'chat', 'Two', 'active', 0, NULL, 100, 300, 1, 1),
                 ('t3', 'ws1', 'chat', 'Gone', 'deleted', 0, 'sess3', 400, 400, 1, 1);",
        )
        .expect("seed threads");
    }

    #[test]
    fn list_recent_and_session_lookup() {
        let (_home, conn) = guarded_conn("threads_lists");
        seed_two_threads(&conn);
        drop(conn);

        let list = list_threads().expect("list");
        let ids: Vec<&str> = list.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["t1", "t2"], "pinned first, deleted hidden");

        let recent = get_recent_thread().expect("recent").expect("some");
        assert_eq!(recent.id, "t2", "last_opened_at wins");

        let found = find_thread_by_agent_session("sess1")
            .expect("find")
            .expect("some");
        assert_eq!(found.id, "t1");
        assert!(find_thread_by_agent_session("ghost")
            .expect("find")
            .is_none());
        // A deleted thread's session is not found.
        assert!(find_thread_by_agent_session("sess3")
            .expect("find")
            .is_none());
    }

    #[test]
    fn sidebar_sort_ignores_open_time_but_opening_is_still_recorded() {
        let (_home, conn) = guarded_conn("threads_activity_times");
        seed_two_threads(&conn);
        conn.execute("UPDATE threads SET pinned = 0 WHERE id = 't1'", [])
            .expect("unpin t1");
        drop(conn);

        // t2 was opened more recently (300), but t1's message is newer (200).
        let ids: Vec<String> = list_threads()
            .expect("list")
            .into_iter()
            .map(|thread| thread.id)
            .collect();
        assert_eq!(ids, vec!["t1", "t2"]);

        mark_thread_opened("t1").expect("record open");
        let thread = get_thread("t1").expect("get").expect("thread");
        assert!(thread.last_opened_at.is_some());
        assert_eq!(
            thread.updated_at, 1,
            "opening does not become sidebar activity"
        );
    }

    fn chat_input() -> CreateThreadInput {
        CreateThreadInput {
            mode: "chat".to_string(),
            title: None,
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        }
    }

    #[test]
    fn create_thread_modes_and_defaults() {
        let (_home, conn) = guarded_conn("threads_create");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
             VALUES ('ws_user', 'User WS', 'user', '/tmp/userws', 1, 1);",
        )
        .expect("seed workspace");
        drop(conn);

        // Chat mode: default title + a temporary workspace.
        let chat = create_thread(chat_input()).expect("chat thread");
        assert_eq!(chat.title, "New Chat");
        assert_eq!(chat.mode, "chat");

        // A bogus mode is rejected.
        let mut bad = chat_input();
        bad.mode = "party".to_string();
        assert!(create_thread(bad).is_err());

        // Workspace mode against an existing workspace id.
        let mut by_id = chat_input();
        by_id.mode = "workspace".to_string();
        by_id.title = Some("Titled".to_string());
        by_id.workspace_id = Some("ws_user".to_string());
        by_id.agent_session_id = Some(String::new());
        let thread = create_thread(by_id).expect("workspace thread by id");
        assert_eq!(thread.title, "Titled");
        assert_eq!(thread.workspace_id, "ws_user");
        assert_eq!(thread.agent_session_id, None, "empty session id filtered");

        // Workspace mode with a fresh path: workspace created, name defaulted.
        let dir = std::env::temp_dir().join(format!("futureos-tc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let mut by_path = chat_input();
        by_path.mode = "workspace".to_string();
        by_path.workspace_path = Some(dir.display().to_string());
        let thread = create_thread(by_path).expect("workspace thread by path");
        assert_eq!(thread.title, "Workspace Thread");
        let ws = get_workspace(&thread.workspace_id)
            .expect("get")
            .expect("some");
        assert_eq!(ws.kind, "user");
        let _ = std::fs::remove_dir_all(&dir);

        // Workspace mode with neither id nor path errors.
        let mut neither = chat_input();
        neither.mode = "workspace".to_string();
        assert!(create_thread(neither).is_err());
    }

    #[test]
    fn thread_field_updates_round_trip() {
        let (_home, conn) = guarded_conn("threads_updates");
        seed_two_threads(&conn);
        drop(conn);

        // rename_thread
        assert!(rename_thread(RenameThreadInput {
            thread_id: "t1".to_string(),
            title: "  ".to_string(),
        })
        .is_err());
        let renamed = rename_thread(RenameThreadInput {
            thread_id: "t1".to_string(),
            title: "Renamed".to_string(),
        })
        .expect("rename");
        assert_eq!(renamed.title, "Renamed");
        assert!(rename_thread(RenameThreadInput {
            thread_id: "ghost".to_string(),
            title: "X".to_string(),
        })
        .is_err());

        // sync_thread_title (pub wrapper)
        assert!(sync_thread_title("t1", "From Agent").expect("sync"));
        assert_eq!(
            get_thread("t1").expect("get").expect("some").title,
            "From Agent"
        );

        // Model / thinking level are agent-owned: reads return the row.
        let record = update_thread_model(UpdateThreadModelInput {
            thread_id: "t1".to_string(),
            model_provider: Some("p".to_string()),
            model_id: Some("m".to_string()),
        })
        .expect("model");
        assert_eq!(record.id, "t1");
        let record = update_thread_thinking_level(UpdateThreadThinkingLevelInput {
            thread_id: "t1".to_string(),
            thinking_level: Some("high".to_string()),
        })
        .expect("thinking");
        assert_eq!(record.id, "t1");
        assert!(update_thread_model(UpdateThreadModelInput {
            thread_id: "ghost".to_string(),
            model_provider: None,
            model_id: None,
        })
        .is_err());

        // update_thread_session_id + find_by_session round trip.
        update_thread_session_id("t2", "sess_new").expect("session id");
        let found = find_thread_by_agent_session("sess_new")
            .expect("find")
            .expect("some");
        assert_eq!(found.id, "t2");

        // move_thread_to_workspace
        move_thread_to_workspace("t2", "ws2").expect("move");
        assert_eq!(
            get_thread("t2").expect("get").expect("some").workspace_id,
            "ws2"
        );

        // pin / archive / restore
        let pinned = pin_thread(PinThreadInput {
            thread_id: "t2".to_string(),
            pinned: true,
        })
        .expect("pin");
        assert!(pinned.pinned);
        assert!(pin_thread(PinThreadInput {
            thread_id: "ghost".to_string(),
            pinned: false,
        })
        .is_err());

        let archived = archive_thread("t2").expect("archive");
        assert_eq!(archived.status, "archived");
        assert!(archived.archived_at.is_some());
        let restored = restore_thread("t2").expect("restore");
        assert_eq!(restored.status, "active");
    }

    #[test]
    fn delete_thread_variants() {
        let (_home, conn) = guarded_conn("threads_delete");
        seed_two_threads(&conn);
        drop(conn);

        // Plain delete of a chat thread flags its temp workspace for cleanup…
        // (t1/t2 are chat threads in a *user* workspace here, so the UPDATE
        // simply matches nothing — the thread row is still removed).
        let deleted = delete_thread("t2").expect("delete");
        assert_eq!(deleted.id, "t2");
        assert!(get_thread("t2").expect("get").is_none());
        assert!(delete_thread("t2").is_err(), "second delete errors");

        // delete_files on a workspace-mode thread never touches the workspace.
        let ws_thread = create_thread(CreateThreadInput {
            mode: "workspace".to_string(),
            title: None,
            workspace_id: Some("ws1".to_string()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("workspace thread");
        delete_thread_with_files(&ws_thread.id, true).expect("delete ws thread");
        assert!(get_workspace("ws1").expect("get").is_some());

        // A chat thread with files: the temp workspace dir is removed and the
        // workspace row is marked cleaned.
        let chat = create_thread(chat_input()).expect("chat thread");
        let chat_dir = std::path::PathBuf::from(std::env::var("HOME").expect("home"))
            .join(".future/workspaces/chat")
            .join(&chat.id);
        std::fs::create_dir_all(&chat_dir).expect("create chat dir");
        std::fs::write(chat_dir.join("scratch.txt"), b"x").expect("write");
        delete_thread_with_files(&chat.id, true).expect("delete with files");
        assert!(!chat_dir.exists(), "chat workspace dir removed");
        let ws = get_workspace(&chat.workspace_id)
            .expect("get")
            .expect("some");
        assert_eq!(ws.cleanup_status, "cleaned");
    }

    #[test]
    fn delete_tombstones_only_the_last_owner_of_a_session() {
        let (_home, conn) = guarded_conn("threads_delete_shared");
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                 VALUES ('ws1', 'W', 'user', '/tmp/ws1', 1, 1);
             INSERT INTO threads (id, workspace_id, mode, title, agent_session_id,
                 created_at, updated_at)
                 VALUES ('ta', 'ws1', 'chat', 'A', 'sess_shared', 1, 1),
                        ('tb', 'ws1', 'chat', 'B', 'sess_shared', 1, 1);",
        )
        .expect("seed");
        drop(conn);

        delete_thread("ta").expect("delete first");
        assert!(
            !crate::store::is_agent_session_tombstoned("sess_shared").expect("check"),
            "a surviving owner keeps the session untombstoned"
        );
        delete_thread("tb").expect("delete last");
        assert!(
            crate::store::is_agent_session_tombstoned("sess_shared").expect("check"),
            "the last owner tombstones the session"
        );
    }

    #[test]
    fn batch_delete_reports_per_thread_outcomes() {
        let (_home, conn) = guarded_conn("threads_batch");
        seed_two_threads(&conn);
        drop(conn);

        let result = batch_delete_threads(&BatchDeleteThreadsInput {
            thread_ids: vec!["t1".to_string(), "ghost".to_string(), "t2".to_string()],
            delete_files: false,
        })
        .expect("batch");
        assert_eq!(result.deleted_count, 2);
        assert_eq!(result.failed.len(), 1);
        assert!(result.failed[0].starts_with("ghost: "));
    }

    #[test]
    fn purge_soft_deleted_threads_cascades() {
        let (_home, conn) = guarded_conn("threads_purge");
        seed_two_threads(&conn);
        conn.execute_batch(
            "INSERT INTO runs (id, thread_id, status, created_at, updated_at)
             VALUES ('r3', 't3', 'completed', 1, 1);",
        )
        .expect("seed run for deleted thread");
        drop(conn);

        assert_eq!(purge_soft_deleted_threads().expect("purge"), 1);
        assert!(get_thread("t3").expect("get").is_none());
        let conn = connect().expect("reconnect");
        let runs: i64 = conn
            .query_row("SELECT COUNT(*) FROM runs WHERE id = 'r3'", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(runs, 0, "child rows of the purged thread are gone");

        assert_eq!(purge_soft_deleted_threads().expect("purge again"), 0);
    }
}
