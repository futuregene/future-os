#![allow(dead_code)]
#![allow(dead_code)]
use std::collections::HashMap;

use rusqlite::params;
use serde::Serialize;

use super::db::*;
use super::records::*;
use super::status::{TERMINAL_RUN_STATUSES, TERMINAL_RUN_STATUSES_SQL};
use super::util::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: String,
    pub thread_id: String,
    pub trigger_message_id: Option<String>,
    pub status: String,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub error_message: Option<String>,
    /// Structured error classification. One of:
    /// 'stream_disconnected', 'command_failed', 'model_failed',
    /// 'abort_requested', 'timeout', 'unknown'. NULL when the run did not
    /// fail or the error type is unknown.
    pub error_type: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct RunEventRecord {
    pub id: String,
    pub run_id: String,
    pub event_type: String,
    pub payload: Option<String>,
    pub sequence: i64,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ToolCallRecord {
    pub id: String,
    pub run_id: String,
    pub name: String,
    pub kind: String,
    pub input: Option<String>,
    pub status: String,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ToolOutputRecord {
    pub id: String,
    pub tool_call_id: String,
    pub kind: String,
    pub content: Option<String>,
    pub created_at: i64,
}

sql_record!(pub(super) RUN_COLUMNS, run_from_row -> RunRecord {
    id, thread_id, trigger_message_id, status, model_provider, model_id,
    started_at, ended_at, error_message, error_type, created_at, updated_at,
});

sql_record!(pub(super) RUN_EVENT_COLUMNS, run_event_from_row -> RunEventRecord {
    id, run_id, event_type, payload, sequence, created_at,
});

sql_record!(pub(super) TOOL_CALL_COLUMNS, tool_call_from_row -> ToolCallRecord {
    id, run_id, name, kind, input, status, started_at, ended_at, created_at,
});

// TOOL_OUTPUT_COLUMNS & tool_output_from_row removed — table dropped

pub fn create_run(input: CreateRunInput) -> Result<RunRecord, crate::AppError> {
    let id = create_id("run");
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "INSERT INTO runs (
             id, thread_id, trigger_message_id, status, model_provider, model_id,
             started_at, created_at, updated_at
         ) VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?6, ?6)",
        params![
            id,
            input.thread_id,
            input.trigger_message_id,
            input.model_provider,
            input.model_id,
            now
        ],
    )?;
    loaded(get_run(&id)?, "Created run")
}

/// Resolved agent session ids of every run that is not yet terminal — i.e. the
/// conversations the user still sees as "generating". Each id is the thread's
/// `agent_session_id` when set (trimmed, non-empty), else the thread id, mirroring
/// the GUI's own session-id resolution (see `useAgentThreadState` /
/// `cleanup::orphan_thread_ids`). Deduplicated. Powers the quit guard: whether to
/// warn before exit, and which sessions to abort on force-quit. Within a live
/// process this is a faithful "is anything running" signal — startup convergence
/// (`cancel_stale_approval_requests`) has already cancelled every orphaned
/// non-terminal run left by a previous process.
pub fn active_run_sessions() -> Result<Vec<String>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT COALESCE(NULLIF(TRIM(t.agent_session_id), ''), t.id)
             FROM runs r
             JOIN threads t ON t.id = r.thread_id
             WHERE r.status NOT IN ({TERMINAL_RUN_STATUSES_SQL})"
    ))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn list_runs(thread_id: &str) -> Result<Vec<RunRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS}
             FROM runs
             WHERE thread_id = ?1
             ORDER BY created_at DESC"
    ))?;
    let rows = stmt.query_map(params![thread_id], run_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

/// Which runs' still-open children (pending approvals, running tool calls) a
/// cancel-cascade settles. The two scopes differ only in how a child's owning
/// run is matched — everything else about the cascade is identical, which is why
/// [`cancel_children_of_runs`] is shared between the single-run and startup paths.
pub(super) enum CancelScope<'a> {
    /// One run: match children whose `run_id` equals this id.
    Run(&'a str),
    /// Startup convergence: match children whose run is already terminal — plus
    /// run-less orphan approvals (`run_id IS NULL`), which can never settle
    /// themselves once their collector is gone.
    TerminalRuns,
}

/// Cancel the still-open approvals and running tool calls belonging to `scope`,
/// stamping the cancelled approvals with `note`. Shared by the `cancelled` path
/// of [`update_run_status_if_active`] (single run) and cleanup's startup
/// convergence (every terminal run). The run-membership predicate is either a
/// bound parameter (single run) or a splice of the constant terminal-status list
/// — no caller value is ever string-interpolated.
pub(super) fn cancel_children_of_runs(
    tx: &rusqlite::Transaction<'_>,
    scope: CancelScope<'_>,
    note: &str,
    now: i64,
) -> rusqlite::Result<()> {
    let terminal_membership =
        format!("run_id IN (SELECT id FROM runs WHERE status IN ({TERMINAL_RUN_STATUSES_SQL}))");
    // `?1` = now, `?2` (approvals only) = note, `?3`/`?2` (single run only) = run id.
    let (approval_where, _tool_where) = match scope {
        CancelScope::Run(_) => ("run_id = ?3".to_string(), "run_id = ?2".to_string()),
        CancelScope::TerminalRuns => (
            format!("(run_id IS NULL OR {terminal_membership})"),
            terminal_membership.clone(),
        ),
    };
    let approval_sql = format!(
        "UPDATE approval_requests
             SET status = 'cancelled',
                 decision_note = COALESCE(decision_note, ?2),
                 decided_at = COALESCE(decided_at, ?1),
                 updated_at = ?1
             WHERE status = 'pending' AND {approval_where}"
    );
    // tool_calls table dropped — only cancel approvals.
    match scope {
        CancelScope::Run(run_id) => {
            tx.execute(&approval_sql, params![now, note, run_id])?;
        }
        CancelScope::TerminalRuns => {
            tx.execute(&approval_sql, params![now, note])?;
        }
    }
    Ok(())
}

/// Like [`update_run_status`], but only transitions a run that is *not already
/// terminal* — the guard is part of the `UPDATE`'s `WHERE`, so a concurrent
/// `abort_run`/`fail_run_if_active` (which sets `cancelled`/`failed`) is never
/// clobbered by a late read-then-write. Returns whether a row changed; the
/// `cancelled` cascade runs only when it did.
pub fn update_run_status_if_active(input: UpdateRunStatusInput) -> Result<bool, crate::AppError> {
    let now = now_millis();
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    let changed = update_run_status_if_active_tx(&tx, &input, now)?;
    tx.commit()?;
    Ok(changed)
}

fn update_run_status_if_active_tx(
    tx: &rusqlite::Transaction<'_>,
    input: &UpdateRunStatusInput,
    now: i64,
) -> rusqlite::Result<bool> {
    let ended_at = if TERMINAL_RUN_STATUSES.contains(&input.status.as_str()) {
        Some(now)
    } else {
        None
    };
    let affected = tx.execute(
        &format!(
            "UPDATE runs
         SET status = ?1,
             error_message = ?2,
             error_type = COALESCE(?3, error_type),
             ended_at = COALESCE(?4, ended_at),
             updated_at = ?5
         WHERE id = ?6
           AND status NOT IN ({TERMINAL_RUN_STATUSES_SQL})"
        ),
        params![
            input.status,
            input.error_message,
            input.error_type,
            ended_at,
            now,
            input.run_id
        ],
    )?;
    if affected > 0 && input.status == "cancelled" {
        cancel_children_of_runs(
            tx,
            CancelScope::Run(&input.run_id),
            "Cancelled because the run was terminated.",
            now,
        )?;
    }
    Ok(affected > 0)
}

/// Transition a run to `failed` only if it is not already in a terminal state,
/// in a single atomic statement. Returns `true` if a row was updated. This is a
/// compare-and-set so a concurrent abort (which sets `cancelled`) is never
/// clobbered by a late failure projection.
pub fn fail_run_if_active(
    run_id: &str,
    error_message: &str,
    error_type: &str,
) -> Result<bool, crate::AppError> {
    let now = now_millis();
    let conn = connect()?;
    let affected = conn.execute(
        &format!(
            "UPDATE runs
         SET status = 'failed',
             error_message = ?1,
             error_type = ?2,
             ended_at = COALESCE(ended_at, ?3),
             updated_at = ?3
         WHERE id = ?4
           AND status NOT IN ({TERMINAL_RUN_STATUSES_SQL})"
        ),
        params![error_message, error_type, now, run_id],
    )?;
    Ok(affected > 0)
}

pub fn list_run_events(run_id: &str) -> Result<Vec<RunEventRecord>, crate::AppError> {
    if let Ok(buf) = RUN_EVENT_BUFFER.lock() {
        if let Some(events) = buf.get(run_id) {
            return Ok(events.clone());
        }
    }
    Ok(vec![])
}

pub fn list_run_events_bulk(
    run_ids: &[String],
) -> Result<Vec<(String, Vec<RunEventRecord>)>, crate::AppError> {
    if let Ok(buf) = RUN_EVENT_BUFFER.lock() {
        let mut result = Vec::new();
        for rid in run_ids {
            if let Some(events) = buf.get(rid) {
                result.push((rid.clone(), events.clone()));
            }
        }
        return Ok(result);
    }
    Ok(vec![])
}

/// In-memory buffer for streaming run events (replaces SQLite run_events table).
/// Keyed by run_id; cleared when the run settles (agent_end received).
static RUN_EVENT_BUFFER: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Vec<RunEventRecord>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

pub fn append_run_event(input: AppendRunEventInput) -> Result<RunEventRecord, crate::AppError> {
    let id = create_id("event");
    let now = now_millis();
    let record = RunEventRecord {
        id,
        run_id: input.run_id.clone(),
        event_type: input.event_type,
        payload: input.payload,
        sequence: input.sequence,
        created_at: now,
    };
    if let Ok(mut buf) = RUN_EVENT_BUFFER.lock() {
        buf.entry(input.run_id.clone()).or_default().push(record.clone());
    }
    Ok(record)
}

/// Clear buffered events for a settled run (called when agent_end is received).
pub fn clear_run_event_buffer(run_id: &str) {
    if let Ok(mut buf) = RUN_EVENT_BUFFER.lock() {
        buf.remove(run_id);
    }
}

pub fn list_tool_calls(_run_id: &str) -> Result<Vec<ToolCallRecord>, crate::AppError> { Ok(vec![]) }

pub fn get_tool_call_input(_run_id: &str, _tool_call_id: &str) -> Result<Option<String>, crate::AppError> { Ok(None) }

pub fn list_tool_outputs(_tool_call_id: &str) -> Result<Vec<ToolOutputRecord>, crate::AppError> { Ok(vec![]) }

#[allow(dead_code)]
pub fn upsert_tool_call(_input: UpsertToolCallInput) -> Result<(), crate::AppError> { Ok(()) }

#[allow(dead_code)]
pub fn complete_tool_call(_input: CompleteToolCallInput) -> Result<(), crate::AppError> { Ok(()) }

#[cfg(test)]
mod tests {
    use rusqlite::{params, Connection};

    use super::*;
    use crate::store::schema::SCHEMA;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(SCHEMA).expect("initialize test schema");
        // These tests exercise the run-status CAS in isolation, so insert run
        // rows directly without their thread/workspace parents.
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("disable foreign keys");
        conn
    }

    fn insert_run(conn: &Connection, id: &str, status: &str) {
        conn.execute(
            "INSERT INTO runs (id, thread_id, status, created_at, updated_at)
             VALUES (?1, 'thread', ?2, 1, 1)",
            params![id, status],
        )
        .expect("insert run");
    }

    fn insert_thread(conn: &Connection, id: &str, agent_session_id: Option<&str>) {
        conn.execute(
            "INSERT INTO threads
                 (id, workspace_id, mode, title, status, pinned, readonly,
                  agent_session_id, created_at, updated_at)
             VALUES (?1, 'ws', 'chat', 'T', 'active', 0, 0, ?2, 1, 1)",
            params![id, agent_session_id],
        )
        .expect("insert thread");
    }

    fn insert_thread_run(conn: &Connection, run_id: &str, thread_id: &str, status: &str) {
        conn.execute(
            "INSERT INTO runs (id, thread_id, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, 1)",
            params![run_id, thread_id, status],
        )
        .expect("insert run");
    }

    /// `active_run_sessions` returns exactly the sessions of non-terminal runs,
    /// deduplicated, resolving the session id to `agent_session_id` when set and
    /// the thread id otherwise (blank/whitespace ids fall back to the thread id).
    #[test]
    fn active_run_sessions_resolves_and_filters() {
        let conn = test_conn();

        // Live run, thread has an agent session id -> resolves to that id.
        insert_thread(&conn, "tA", Some("sessA"));
        insert_thread_run(&conn, "rA", "tA", "running");

        // Live run, no agent session id -> resolves to the thread id.
        insert_thread(&conn, "tB", None);
        insert_thread_run(&conn, "rB", "tB", "waiting_approval");

        // Blank agent session id -> falls back to the thread id.
        insert_thread(&conn, "tC", Some("   "));
        insert_thread_run(&conn, "rC", "tC", "running");

        // Two live runs on one thread -> a single deduplicated session id.
        insert_thread(&conn, "tD", Some("sessD"));
        insert_thread_run(&conn, "rD1", "tD", "running");
        insert_thread_run(&conn, "rD2", "tD", "running");

        // Terminal-only thread -> excluded entirely.
        insert_thread(&conn, "tE", Some("sessE"));
        insert_thread_run(&conn, "rE", "tE", "completed");

        let mut sessions = {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT DISTINCT COALESCE(NULLIF(TRIM(t.agent_session_id), ''), t.id)
                         FROM runs r
                         JOIN threads t ON t.id = r.thread_id
                         WHERE r.status NOT IN ({TERMINAL_RUN_STATUSES_SQL})"
                ))
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        sessions.sort();
        assert_eq!(sessions, vec!["sessA", "sessD", "tB", "tC"]);
    }

    fn run_status(conn: &Connection, id: &str) -> String {
        conn.query_row(
            "SELECT status FROM runs WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("read run status")
    }

    fn running_input(run_id: &str) -> UpdateRunStatusInput {
        UpdateRunStatusInput {
            run_id: run_id.to_string(),
            status: "running".to_string(),
            error_message: None,
            error_type: None,
        }
    }

    /// B-13: a terminal run is never resurrected by the if-active CAS.
    #[test]
    fn if_active_skips_terminal_run() {
        let mut conn = test_conn();
        insert_run(&conn, "run_cancelled", "cancelled");
        let tx = conn.transaction().unwrap();
        let changed =
            update_run_status_if_active_tx(&tx, &running_input("run_cancelled"), 99).unwrap();
        tx.commit().unwrap();
        assert!(!changed);
        assert_eq!(run_status(&conn, "run_cancelled"), "cancelled");
    }

    /// a completed run is not rewritten to cancelled by a late
    /// abort (nor to any other status by a late completion projection).
    #[test]
    fn if_active_skips_completed_run() {
        let mut conn = test_conn();
        insert_run(&conn, "run_done", "completed");
        let cancel = UpdateRunStatusInput {
            run_id: "run_done".to_string(),
            status: "cancelled".to_string(),
            error_message: Some("Terminated by user.".to_string()),
            error_type: Some("abort_requested".to_string()),
        };
        let tx = conn.transaction().unwrap();
        let changed = update_run_status_if_active_tx(&tx, &cancel, 99).unwrap();
        tx.commit().unwrap();
        assert!(!changed);
        assert_eq!(run_status(&conn, "run_done"), "completed");
    }

    /// A non-terminal run does transition, and the cancelled cascade fires.
    #[test]
    fn if_active_cancels_active_run_and_cascades() {
        let mut conn = test_conn();
        insert_run(&conn, "run_live", "running");
        conn.execute(
            "INSERT INTO approval_requests (id, thread_id, run_id, kind, status, title, created_at, updated_at)
             VALUES ('ap1', 'thread', 'run_live', 'bash', 'pending', 't', 1, 1)",
            [],
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        let input = UpdateRunStatusInput {
            run_id: "run_live".to_string(),
            status: "cancelled".to_string(),
            error_message: Some("stop".to_string()),
            error_type: None,
        };
        let changed = update_run_status_if_active_tx(&tx, &input, 99).unwrap();
        tx.commit().unwrap();
        assert!(changed);
        assert_eq!(run_status(&conn, "run_live"), "cancelled");
        let approval_status: String = conn
            .query_row(
                "SELECT status FROM approval_requests WHERE id = 'ap1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(approval_status, "cancelled");
    }
}
