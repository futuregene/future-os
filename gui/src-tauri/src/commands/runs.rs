//! Run and tool-call Tauri commands. `abort_run` delegates its agent + store
//! orchestration to [`crate::agent_bridge`].

use crate::{agent_bridge, store};

#[tauri::command]
pub fn create_run(input: store::CreateRunInput) -> Result<store::RunRecord, crate::AppError> {
    store::create_run(input)
}

#[tauri::command]
pub fn list_runs(thread_id: String) -> Result<Vec<store::RunRecord>, crate::AppError> {
    store::list_runs(&thread_id)
}

/// The thread's most recent run, or None. Used for initial load and pushed
/// terminal reconciliation without decoding the thread's full run history.
#[tauri::command]
pub fn get_latest_run(thread_id: String) -> Result<Option<store::RunRecord>, crate::AppError> {
    store::latest_run(&thread_id)
}

/// A single run by id — direct primary-key lookup for the send pipeline's
/// settle checks (was a full per-thread list + client-side find).
#[tauri::command]
pub fn get_run(run_id: String) -> Result<Option<store::RunRecord>, crate::AppError> {
    store::get_run(&run_id)
}

/// Batch query: the latest run identity/status for each thread in one IPC.
/// Used by the low-frequency thread-list reconciliation path.
#[tauri::command]
pub fn list_latest_run_infos(
    thread_ids: Vec<String>,
) -> Result<Vec<store::LatestRunInfo>, crate::AppError> {
    store::latest_run_infos(&thread_ids)
}

/// Update a run's status from the frontend's completion/failure paths. Guarded:
/// a run that is already terminal (e.g. a concurrent `abort_run` set `cancelled`)
/// is not clobbered. Returns the run's real current state so the caller
/// reconciles its bubble from the truth rather than the status it tried to write.
#[tauri::command]
pub fn update_run_status(
    input: store::UpdateRunStatusInput,
) -> Result<store::RunRecord, crate::AppError> {
    let run_id = input.run_id.clone();
    store::update_run_status_if_active(input)?;
    store::get_run(&run_id)?.ok_or_else(|| "Run could not be loaded.".to_string().into())
}

#[tauri::command]
pub async fn abort_run(
    thread_id: String,
    run_id: String,
) -> Result<store::RunRecord, crate::AppError> {
    agent_bridge::abort_run(thread_id, run_id).await
}

#[tauri::command]
pub async fn list_run_events(
    run_id: String,
) -> Result<Vec<store::RunEventRecord>, crate::AppError> {
    let local = store::list_run_events(&run_id)?;
    // After a GUI restart the in-memory RUN_EVENT_BUFFER is empty — the agent
    // (still running in the background) holds the authoritative events.  Pull
    // them from the agent for active runs with no local events.
    if !local.is_empty() {
        return Ok(local);
    }
    agent_events_fallback(&run_id, -1).await
}

/// Incremental variant of [`list_run_events`] for pushed live-preview updates:
/// returns only events with `sequence > since_sequence`. A run's event log
/// grows monotonically, so the steady-state payload is a handful of events
/// instead of the whole log crossing IPC (and being re-parsed) every tick.
#[tauri::command]
pub async fn list_run_events_since(
    run_id: String,
    since_sequence: i64,
) -> Result<Vec<store::RunEventRecord>, crate::AppError> {
    if since_sequence < 0 {
        return list_run_events(run_id).await;
    }
    let local = store::list_run_events_since(&run_id, since_sequence)?;
    // An empty tail usually means "no new events this tick" — but when the
    // local log is entirely cold (post-restart) AND the run is still active,
    // the events only exist agent-side (the earlier full fetch came from the
    // agent and was never persisted locally), so keep falling back with the
    // same incremental query the agent natively supports.
    if !local.is_empty() || store::has_run_events(&run_id) {
        return Ok(local);
    }
    agent_events_fallback(&run_id, since_sequence).await
}

/// Pull an active run's events from the agent (authoritative while streaming,
/// survives GUI restarts) when the local log is cold. `since_sequence` is
/// passed through to the agent's native incremental query (-1 = whole run).
/// Returns an empty vec for settled runs, unknown runs, or an unreachable
/// agent — the caller's empty local result is the right answer in all three.
async fn agent_events_fallback(
    run_id: &str,
    since_sequence: i64,
) -> Result<Vec<store::RunEventRecord>, crate::AppError> {
    let empty = Vec::new();
    let Some(run) = store::get_run(run_id).ok().flatten() else {
        return Ok(empty);
    };
    if matches!(run.status.as_str(), "completed" | "failed" | "cancelled") {
        return Ok(empty);
    }
    // Active run — pull events from the agent
    let Some(thread) = store::get_thread(&run.thread_id).ok().flatten() else {
        return Ok(empty);
    };
    let sid = thread
        .agent_session_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| run.thread_id.clone());
    let agent_json =
        match agent_bridge::get_events_since(sid, run_id.to_string(), since_sequence).await {
            Ok(v) => v,
            Err(_) => return Ok(empty),
        };
    let Some(events) = agent_json.get("events").and_then(|v| v.as_array()) else {
        return Ok(empty);
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let records: Vec<store::RunEventRecord> = events
        .iter()
        .enumerate()
        .map(|(i, e)| store::RunEventRecord {
            id: format!("agent_{run_id}_{i}"),
            run_id: run_id.to_string(),
            event_type: e
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            payload: e
                .get("data")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            sequence: e.get("idx").and_then(|v| v.as_i64()).unwrap_or(i as i64),
            created_at: now,
        })
        .collect();
    Ok(records)
}

#[tauri::command]
pub fn list_run_events_bulk(
    run_ids: Vec<String>,
) -> Result<Vec<(String, Vec<store::RunEventRecord>)>, crate::AppError> {
    store::list_run_events_bulk(&run_ids)
}

#[tauri::command]
pub fn list_tool_calls(run_id: String) -> Result<Vec<store::ToolCallRecord>, crate::AppError> {
    store::list_tool_calls(&run_id)
}

/// Batch variant: the context panel's 1.5s poll needs tool calls for every
/// run of the thread — one IPC round-trip instead of N.
#[tauri::command]
pub fn list_tool_calls_bulk(
    run_ids: Vec<String>,
) -> Result<Vec<(String, Vec<store::ToolCallRecord>)>, crate::AppError> {
    store::list_tool_calls_bulk(&run_ids)
}

#[tauri::command]
pub fn list_tool_outputs(
    run_id: String,
    tool_call_id: String,
) -> Result<Vec<store::ToolOutputRecord>, crate::AppError> {
    store::list_tool_outputs(&run_id, &tool_call_id)
}
