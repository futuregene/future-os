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
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub thinking_level: Option<String>,
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
    id, workspace_id, mode, title, status, pinned, readonly, model_provider,
    model_id, thinking_level, agent_session_id, last_message_at, last_opened_at,
    created_at, updated_at, archived_at, deleted_at,
});

pub fn list_threads() -> Result<Vec<ThreadRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {THREAD_COLUMNS}
             FROM threads
             WHERE status != 'deleted'
             ORDER BY pinned DESC, COALESCE(last_message_at, updated_at) DESC"
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
    let agent_session_id = input
        .agent_session_id
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| create_id("agent_session"));
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
             model_provider, model_id, thinking_level, agent_session_id, last_opened_at,
             created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, 'active', 0, 0, ?5, ?6, ?7, ?8, ?9, ?9, ?9)",
        params![
            thread_id,
            workspace.id,
            mode,
            title,
            input.model_provider,
            input.model_id,
            normalize_optional_thinking_level(input.thinking_level),
            agent_session_id,
            now
        ],
    )?;

    let thread = loaded(get_thread_in(&tx, &thread_id)?, "Created thread")?;
    tx.commit()?;
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
    let conn = connect()?;
    conn.execute(
        "UPDATE threads
         SET title = ?1, updated_at = ?2
         WHERE id = ?3 AND status != 'deleted'",
        params![title, now, input.thread_id],
    )?;

    loaded(get_thread(&input.thread_id)?, "Thread")
}

pub fn update_thread_model(input: UpdateThreadModelInput) -> Result<ThreadRecord, crate::AppError> {
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
    )?;

    loaded(get_thread(&input.thread_id)?, "Thread")
}

pub fn update_thread_thinking_level(
    input: UpdateThreadThinkingLevelInput,
) -> Result<ThreadRecord, crate::AppError> {
    let thinking_level = normalize_optional_thinking_level(input.thinking_level);
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "UPDATE threads
         SET thinking_level = ?1, updated_at = ?2
         WHERE id = ?3 AND status != 'deleted'",
        params![thinking_level, now, input.thread_id],
    )?;

    loaded(get_thread(&input.thread_id)?, "Thread")
}

pub fn pin_thread(input: PinThreadInput) -> Result<ThreadRecord, crate::AppError> {
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "UPDATE threads
         SET pinned = ?1, updated_at = ?2
         WHERE id = ?3 AND status != 'deleted'",
        params![if input.pinned { 1 } else { 0 }, now, input.thread_id],
    )?;

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
    let conn = connect()?;
    conn.execute(
        "UPDATE threads
         SET status = ?1, archived_at = ?2, updated_at = ?3
         WHERE id = ?4 AND status != 'deleted'",
        params![status, archived_at, now, thread_id],
    )?;

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
fn delete_thread_children_in(conn: &Connection, thread_id: &str) -> rusqlite::Result<()> {
    // Review data: file changes → changesets → snapshots (all source kinds).
    conn.execute(
        "DELETE FROM review_file_changes WHERE changeset_id IN (
             SELECT id FROM review_changesets WHERE thread_id = ?1
         )",
        params![thread_id],
    )?;
    conn.execute(
        "DELETE FROM review_changesets WHERE thread_id = ?1",
        params![thread_id],
    )?;
    conn.execute(
        "DELETE FROM review_snapshots WHERE thread_id = ?1",
        params![thread_id],
    )?;
    // Denormalized markdown references originating from this thread's messages.
    conn.execute(
        "DELETE FROM object_references
         WHERE source_type = 'message'
           AND source_id IN (SELECT id FROM messages WHERE thread_id = ?1)",
        params![thread_id],
    )?;
    // Approvals reference threads/runs/tool_calls — clear before tool_calls.
    conn.execute(
        "DELETE FROM approval_requests WHERE thread_id = ?1",
        params![thread_id],
    )?;
    // Tool outputs → tool calls (scoped through the thread's runs).
    conn.execute(
        "DELETE FROM tool_outputs WHERE tool_call_id IN (
             SELECT tc.id FROM tool_calls tc
             JOIN runs r ON r.id = tc.run_id
             WHERE r.thread_id = ?1
         )",
        params![thread_id],
    )?;
    conn.execute(
        "DELETE FROM run_events WHERE run_id IN (
             SELECT id FROM runs WHERE thread_id = ?1
         )",
        params![thread_id],
    )?;
    conn.execute(
        "DELETE FROM tool_calls WHERE run_id IN (
             SELECT id FROM runs WHERE thread_id = ?1
         )",
        params![thread_id],
    )?;
    // Detach workspace-level artifacts from the thread/run being removed.
    conn.execute(
        "UPDATE artifacts SET run_id = NULL
         WHERE run_id IN (SELECT id FROM runs WHERE thread_id = ?1)",
        params![thread_id],
    )?;
    conn.execute(
        "UPDATE artifacts SET thread_id = NULL WHERE thread_id = ?1",
        params![thread_id],
    )?;
    // Break the runs ↔ messages FK cycle, then delete both.
    conn.execute(
        "UPDATE runs SET trigger_message_id = NULL WHERE thread_id = ?1",
        params![thread_id],
    )?;
    conn.execute(
        "DELETE FROM messages WHERE thread_id = ?1",
        params![thread_id],
    )?;
    conn.execute("DELETE FROM runs WHERE thread_id = ?1", params![thread_id])?;
    Ok(())
}

/// Hard delete a thread and every row that hangs off it. The conversation
/// content the GUI stores is only a rendered mirror of the agent's JSONL (the
/// source of truth); the caller deletes that JSONL separately (see
/// `commands::delete_thread`). Returns the pre-delete record so callers that
/// expected the old soft-delete return value keep working. Temp chat workspaces
/// are flagged for cleanup exactly as before.
pub fn delete_thread(thread_id: &str) -> Result<ThreadRecord, crate::AppError> {
    let now = now_millis();
    let mut conn = connect()?;
    let thread = loaded(get_thread(thread_id)?, "Thread")?;
    // One transaction: the child cascade, the temp-workspace cleanup flag, and
    // the thread delete must land together — a crash between them would leak the
    // chat workspace directory forever (mirrors delete_workspace).
    let tx = conn.transaction()?;
    delete_thread_children_in(&tx, thread_id)?;

    if thread.mode == "chat" {
        tx.execute(
            "UPDATE workspaces
             SET cleanup_status = 'pending_cleanup',
                 cleanup_requested_at = COALESCE(cleanup_requested_at, ?1),
                 updated_at = ?1
             WHERE id = ?2
               AND kind = 'temporary'
               AND cleanup_status = 'active'",
            params![now, thread.workspace_id],
        )?;
    }
    tx.execute("DELETE FROM threads WHERE id = ?1", params![thread_id])?;
    tx.commit()?;

    Ok(thread)
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

fn normalize_optional_thinking_level(level: Option<String>) -> Option<String> {
    level.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    })
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
             INSERT INTO messages (id, thread_id, run_id, role, content_type,
                 content, status, created_at, updated_at)
                 VALUES ('m1', 't1', 'r1', 'assistant', 'markdown', 'hi',
                         'complete', 1, 1);
             -- Close the runs↔messages FK cycle.
             UPDATE runs SET trigger_message_id = 'm1' WHERE id = 'r1';
             INSERT INTO run_events (id, run_id, event_type, sequence, created_at)
                 VALUES ('e1', 'r1', 'text_chunk', 0, 1);
             INSERT INTO tool_calls (id, run_id, name, kind, status, created_at)
                 VALUES ('tc1', 'r1', 'bash', 'agent_tool', 'completed', 1);
             INSERT INTO tool_outputs (id, tool_call_id, kind, created_at)
                 VALUES ('to1', 'tc1', 'text', 1);
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

        // Every conversation child row is gone.
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM messages"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM runs"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM run_events"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tool_calls"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tool_outputs"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM approval_requests"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM review_snapshots"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM review_changesets"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM review_file_changes"), 0);
        // The thread row itself is left to the caller.
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM threads WHERE id = 't1'"), 1);
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
}
