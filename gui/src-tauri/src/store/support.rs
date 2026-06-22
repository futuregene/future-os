use rusqlite::{params, Connection, OptionalExtension};
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use super::records::*;
use super::schema::SCHEMA;
use super::{get_thread, get_workspace, initialize_app_store};

pub(super) fn app_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME environment variable is not set.")?;
    Ok(PathBuf::from(home).join(".future").join("app"))
}

pub(super) fn db_path() -> Result<PathBuf, String> {
    Ok(app_dir()?.join("app.db"))
}

pub(super) fn chat_workspace_path(thread_id: &str) -> Result<PathBuf, String> {
    Ok(app_dir()?.join("workspaces").join("chat").join(thread_id))
}

pub(super) fn ensure_app_dirs() -> Result<(), String> {
    fs::create_dir_all(app_dir()?.join("workspaces").join("chat"))
        .map_err(|error| error.to_string())
}

pub(super) fn connect() -> Result<Connection, String> {
    ensure_app_dirs()?;
    let conn = Connection::open(db_path()?).map_err(|error| error.to_string())?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         PRAGMA journal_mode = WAL;",
    )
    .map_err(|error| error.to_string())?;
    Ok(conn)
}

pub(super) fn apply_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(SCHEMA)
        .map_err(|error| error.to_string())
}

pub(super) fn get_message(id: &str) -> Result<Option<MessageRecord>, String> {
    let conn = connect()?;
    conn.query_row(
        "SELECT id, thread_id, run_id, role, content_type, content, status, created_at, updated_at
         FROM messages
         WHERE id = ?1",
        params![id],
        message_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub fn get_run(id: &str) -> Result<Option<RunRecord>, String> {
    let conn = connect()?;
    conn.query_row(
        "SELECT id, thread_id, trigger_message_id, status, model_provider, model_id,
                started_at, ended_at, error_message, error_type, created_at, updated_at
         FROM runs
         WHERE id = ?1",
        params![id],
        run_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn get_run_event(id: &str) -> Result<Option<RunEventRecord>, String> {
    let conn = connect()?;
    conn.query_row(
        "SELECT id, run_id, type, payload, sequence, created_at
         FROM run_events
         WHERE id = ?1",
        params![id],
        run_event_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn run_thread_id(conn: &Connection, run_id: &str) -> Result<String, String> {
    conn.query_row(
        "SELECT thread_id FROM runs WHERE id = ?1",
        params![run_id],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

pub fn get_approval_request(id: &str) -> Result<Option<ApprovalRequestRecord>, String> {
    let conn = connect()?;
    conn.query_row(
        "SELECT id, thread_id, run_id, tool_call_id, kind, status, title, summary,
                risk_level, requested_action, decision_note, decided_at, created_at, updated_at,
                action_category, action_payload, sandbox_boundary, reviewer, decision_scope, decision_source
         FROM approval_requests
         WHERE id = ?1",
        params![id],
        approval_request_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn get_or_create_user_workspace(
    name: String,
    path: PathBuf,
    description: Option<String>,
) -> Result<WorkspaceRecord, String> {
    let normalized_path = path.display().to_string();
    let conn = connect()?;
    let existing = conn
        .query_row(
            "SELECT id, name, kind, path, description, cleanup_status,
                    cleanup_requested_at, cleaned_at, last_opened_at,
                    created_at, updated_at, deleted_at
             FROM workspaces
             WHERE kind = 'user' AND path = ?1 AND deleted_at IS NULL
             LIMIT 1",
            params![normalized_path],
            workspace_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())?;

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
    )
    .map_err(|error| error.to_string())?;

    get_workspace(&workspace_id)?
        .ok_or_else(|| "Created workspace could not be loaded.".to_string())
}

pub(super) fn update_thread_status(thread_id: &str, status: &str) -> Result<ThreadRecord, String> {
    initialize_app_store()?;
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
    )
    .map_err(|error| error.to_string())?;

    get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())
}

pub(super) fn thread_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadRecord> {
    Ok(ThreadRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        mode: row.get(2)?,
        title: row.get(3)?,
        status: row.get(4)?,
        pinned: row.get::<_, i64>(5)? != 0,
        readonly: row.get::<_, i64>(6)? != 0,
        model_provider: row.get(7)?,
        model_id: row.get(8)?,
        agent_session_id: row.get(9)?,
        last_message_at: row.get(10)?,
        last_opened_at: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        archived_at: row.get(14)?,
        deleted_at: row.get(15)?,
    })
}

pub(super) fn message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRecord> {
    Ok(MessageRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        run_id: row.get(2)?,
        role: row.get(3)?,
        content_type: row.get(4)?,
        content: row.get(5)?,
        status: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

pub(super) fn run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRecord> {
    Ok(RunRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        trigger_message_id: row.get(2)?,
        status: row.get(3)?,
        model_provider: row.get(4)?,
        model_id: row.get(5)?,
        started_at: row.get(6)?,
        ended_at: row.get(7)?,
        error_message: row.get(8)?,
        error_type: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

pub(super) fn workspace_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceRecord> {
    Ok(WorkspaceRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        path: row.get(3)?,
        description: row.get(4)?,
        cleanup_status: row.get(5)?,
        cleanup_requested_at: row.get(6)?,
        cleaned_at: row.get(7)?,
        last_opened_at: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        deleted_at: row.get(11)?,
    })
}

pub(super) fn run_event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunEventRecord> {
    Ok(RunEventRecord {
        id: row.get(0)?,
        run_id: row.get(1)?,
        event_type: row.get(2)?,
        payload: row.get(3)?,
        sequence: row.get(4)?,
        created_at: row.get(5)?,
    })
}

pub(super) fn tool_call_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ToolCallRecord> {
    Ok(ToolCallRecord {
        id: row.get(0)?,
        run_id: row.get(1)?,
        name: row.get(2)?,
        kind: row.get(3)?,
        input: row.get(4)?,
        status: row.get(5)?,
        started_at: row.get(6)?,
        ended_at: row.get(7)?,
        created_at: row.get(8)?,
    })
}

pub(super) fn tool_output_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ToolOutputRecord> {
    Ok(ToolOutputRecord {
        id: row.get(0)?,
        tool_call_id: row.get(1)?,
        kind: row.get(2)?,
        content: row.get(3)?,
        created_at: row.get(4)?,
    })
}

pub(super) fn approval_request_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ApprovalRequestRecord> {
    Ok(ApprovalRequestRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        run_id: row.get(2)?,
        tool_call_id: row.get(3)?,
        kind: row.get(4)?,
        status: row.get(5)?,
        title: row.get(6)?,
        summary: row.get(7)?,
        risk_level: row.get(8)?,
        requested_action: row.get(9)?,
        decision_note: row.get(10)?,
        decided_at: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        action_category: row.get(14)?,
        action_payload: row.get(15)?,
        sandbox_boundary: row.get(16)?,
        reviewer: row.get(17)?,
        decision_scope: row.get(18)?,
        decision_source: row.get(19)?,
    })
}

pub(super) fn review_changeset_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ReviewChangesetRecord> {
    Ok(ReviewChangesetRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        run_id: row.get(2)?,
        tool_call_id: row.get(3)?,
        title: row.get(4)?,
        summary: row.get(5)?,
        status: row.get(6)?,
        files_changed: row.get(7)?,
        additions: row.get(8)?,
        deletions: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

pub(super) fn review_file_change_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ReviewFileChangeRecord> {
    Ok(ReviewFileChangeRecord {
        id: row.get(0)?,
        changeset_id: row.get(1)?,
        target_type: row.get(2)?,
        target_id: row.get(3)?,
        path: row.get(4)?,
        change_type: row.get(5)?,
        before_ref: row.get(6)?,
        after_ref: row.get(7)?,
        diff: row.get(8)?,
        summary: row.get(9)?,
        additions: row.get(10)?,
        deletions: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

pub(super) fn artifact_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactRecord> {
    Ok(ArtifactRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        thread_id: row.get(2)?,
        run_id: row.get(3)?,
        title: row.get(4)?,
        artifact_type: row.get(5)?,
        path: row.get(6)?,
        content: row.get(7)?,
        content_storage: row.get(8)?,
        summary: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        deleted_at: row.get(12)?,
    })
}

pub(super) fn research_collection_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ResearchCollectionRecord> {
    Ok(ResearchCollectionRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

pub(super) fn research_resource_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ResearchResourceRecord> {
    Ok(ResearchResourceRecord {
        id: row.get(0)?,
        collection_id: row.get(1)?,
        workspace_id: row.get(2)?,
        source_artifact_id: row.get(3)?,
        title: row.get(4)?,
        resource_type: row.get(5)?,
        source_uri: row.get(6)?,
        content: row.get(7)?,
        content_storage: row.get(8)?,
        summary: row.get(9)?,
        metadata: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

pub(super) fn normalize_mode(mode: &str) -> Result<String, String> {
    match mode {
        "chat" | "workspace" => Ok(mode.to_string()),
        _ => Err("mode must be either 'chat' or 'workspace'.".to_string()),
    }
}

pub(super) fn expand_tilde(path: &str) -> Result<PathBuf, String> {
    if path == "~" {
        return Ok(PathBuf::from(
            std::env::var("HOME").map_err(|_| "HOME environment variable is not set.")?,
        ));
    }

    if let Some(rest) = path.strip_prefix("~/") {
        return Ok(PathBuf::from(
            std::env::var("HOME").map_err(|_| "HOME environment variable is not set.")?,
        )
        .join(rest));
    }

    Ok(PathBuf::from(path))
}

pub(super) fn workspace_name_from_path(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Workspace")
        .to_string()
}

pub(super) fn create_id(prefix: &str) -> String {
    static ID_COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{nanos}_{counter}")
}

pub(super) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

pub(super) fn count_workspace_files(path: &str) -> Result<i64, String> {
    let root = PathBuf::from(path);
    if !root.exists() {
        return Ok(0);
    }
    if !root.is_dir() {
        return Ok(0);
    }

    let mut count = 0_i64;
    let mut visited_dirs = HashSet::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let canonical_dir = fs::canonicalize(&dir).map_err(|error| error.to_string())?;
        if !visited_dirs.insert(canonical_dir) {
            continue;
        }
        for entry in fs::read_dir(&dir).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() {
                count += 1;
            }
        }
    }
    Ok(count)
}
