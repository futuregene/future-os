use rusqlite::params;

use super::db::*;
use super::records::*;
use super::status::{TERMINAL_RUN_STATUSES, TERMINAL_RUN_STATUSES_SQL};
use super::util::*;

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

pub fn list_runs(thread_id: &str) -> Result<Vec<RunRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(
        "SELECT id, thread_id, trigger_message_id, status, model_provider, model_id,
                    started_at, ended_at, error_message, error_type, created_at, updated_at
             FROM runs
             WHERE thread_id = ?1
             ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map(params![thread_id], run_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn update_run_status(input: UpdateRunStatusInput) -> Result<RunRecord, crate::AppError> {
    let now = now_millis();
    let ended_at = if TERMINAL_RUN_STATUSES.contains(&input.status.as_str()) {
        Some(now)
    } else {
        None
    };
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE runs
         SET status = ?1,
             error_message = ?2,
             error_type = COALESCE(?3, error_type),
             ended_at = COALESCE(?4, ended_at),
             updated_at = ?5
         WHERE id = ?6",
        params![
            input.status,
            input.error_message,
            input.error_type,
            ended_at,
            now,
            input.run_id
        ],
    )?;
    if input.status == "cancelled" {
        tx.execute(
            "UPDATE approval_requests
             SET status = 'cancelled',
                 decision_note = COALESCE(decision_note, 'Cancelled because the run was terminated.'),
                 decided_at = COALESCE(decided_at, ?1),
                 updated_at = ?1
             WHERE run_id = ?2
               AND status = 'pending'",
            params![now, input.run_id],
        )
        ?;
        tx.execute(
            "UPDATE tool_calls
             SET status = 'cancelled',
                 ended_at = COALESCE(ended_at, ?1)
             WHERE run_id = ?2
               AND status = 'running'",
            params![now, input.run_id],
        )?;
    }
    tx.commit()?;
    loaded(get_run(&input.run_id)?, "Updated run")
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
    let conn = connect()?;
    let mut stmt = conn.prepare(
        "SELECT id, run_id, type, payload, sequence, created_at
             FROM run_events
             WHERE run_id = ?1
             ORDER BY sequence ASC, created_at ASC",
    )?;
    let rows = stmt.query_map(params![run_id], run_event_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn append_run_event(input: AppendRunEventInput) -> Result<RunEventRecord, crate::AppError> {
    let id = create_id("event");
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "INSERT INTO run_events (id, run_id, type, payload, sequence, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            input.run_id,
            input.event_type,
            input.payload,
            input.sequence,
            now
        ],
    )?;

    loaded(get_run_event(&id)?, "Created run event")
}

pub fn list_tool_calls(run_id: &str) -> Result<Vec<ToolCallRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(
        "SELECT id, run_id, name, kind, input, status, started_at, ended_at, created_at
             FROM tool_calls
             WHERE run_id = ?1
             ORDER BY COALESCE(started_at, created_at) ASC",
    )?;
    let rows = stmt.query_map(params![run_id], tool_call_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn list_tool_outputs(tool_call_id: &str) -> Result<Vec<ToolOutputRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(
        "SELECT id, tool_call_id, kind, content, created_at
             FROM tool_outputs
             WHERE tool_call_id = ?1
             ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![tool_call_id], tool_output_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn upsert_tool_call(input: UpsertToolCallInput) -> Result<(), crate::AppError> {
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "INSERT INTO tool_calls (
             id, run_id, name, kind, input, status, started_at, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             kind = excluded.kind,
             input = COALESCE(excluded.input, tool_calls.input),
             status = excluded.status,
             started_at = COALESCE(tool_calls.started_at, excluded.started_at)",
        params![
            input.tool_call_id,
            input.run_id,
            input.name,
            input.kind,
            input.input,
            input.status,
            now
        ],
    )?;
    Ok(())
}

pub fn complete_tool_call(input: CompleteToolCallInput) -> Result<(), crate::AppError> {
    let now = now_millis();
    let mut conn = connect()?;
    // The tool-call row and its output row are one logical write — commit them
    // atomically so a crash can't leave a tool call without its output.
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO tool_calls (
             id, run_id, name, kind, status, started_at, ended_at, created_at
         ) VALUES (?1, ?2, ?3, 'agent_tool', ?4, ?5, ?5, ?5)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             status = excluded.status,
             ended_at = excluded.ended_at",
        params![
            input.tool_call_id,
            input.run_id,
            input.name,
            input.status,
            now
        ],
    )?;

    tx.execute(
        "INSERT INTO tool_outputs (id, tool_call_id, kind, content, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            create_id("toolout"),
            input.tool_call_id,
            input.output_kind,
            input.output_content,
            now
        ],
    )?;
    tx.commit()?;
    Ok(())
}
