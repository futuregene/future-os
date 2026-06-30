use rusqlite::{params, Connection, OptionalExtension};

use super::db::*;
use super::records::*;
use super::util::*;
use super::workspaces::{get_or_create_chat_workspace_in, get_workspace_in};

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

pub fn create_thread(input: CreateThreadInput) -> Result<ThreadRecord, crate::AppError> {
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

    // Resolve/create the workspace and insert the thread in one transaction so a
    // crash between the two writes can't leave an orphan workspace with no thread
    // pointing at it. `&tx` deref-coerces to `&Connection` for the `_in` helpers.
    let mut conn = connect()?;
    let tx = conn.transaction()?;

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

pub fn archive_thread(thread_id: &str) -> Result<ThreadRecord, crate::AppError> {
    update_thread_status(thread_id, "archived")
}

pub fn restore_thread(thread_id: &str) -> Result<ThreadRecord, crate::AppError> {
    update_thread_status(thread_id, "active")
}

pub fn delete_thread(thread_id: &str) -> Result<ThreadRecord, crate::AppError> {
    let now = now_millis();
    let conn = connect()?;
    let thread = loaded(get_thread(thread_id)?, "Thread")?;
    conn.execute(
        "UPDATE threads
         SET status = 'deleted', deleted_at = ?1, updated_at = ?1
         WHERE id = ?2 AND status != 'deleted'",
        params![now, thread_id],
    )?;

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
        )?;
    }

    loaded(get_thread(thread_id)?, "Thread")
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
    use crate::store::db::get_or_create_user_workspace_in;
    use crate::store::schema::SCHEMA;
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
