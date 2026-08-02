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
    // The Agent journal is canonical for every run, including settled ones.
    // GUI JSONL is read only as a compatibility fallback for logs written by
    // pre-journal builds while the Agent is unavailable.
    match agent_events(&run_id, -1).await {
        Ok(events) => Ok(events),
        Err(_) => store::list_run_events(&run_id),
    }
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
    match agent_events(&run_id, since_sequence).await {
        Ok(events) => Ok(events),
        Err(_) => store::list_run_events_since(&run_id, since_sequence),
    }
}

/// Pull canonical events from the Agent journal. `since_sequence` is passed
/// through to the native incremental query (-1 = whole run).
async fn agent_events(
    run_id: &str,
    since_sequence: i64,
) -> Result<Vec<store::RunEventRecord>, crate::AppError> {
    let run = store::get_run(run_id)?.ok_or_else(|| format!("Unknown run {run_id}"))?;
    let thread = store::get_thread(&run.thread_id)?
        .ok_or_else(|| format!("Missing thread for run {run_id}"))?;
    let sid = thread
        .agent_session_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| run.thread_id.clone());
    let agent_json =
        agent_bridge::get_events_since(sid, run_id.to_string(), since_sequence).await?;
    let Some(events) = agent_json.get("events").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    let records: Vec<store::RunEventRecord> = events
        .iter()
        .enumerate()
        .map(|(i, e)| store::RunEventRecord {
            id: e
                .get("eventId")
                .and_then(|v| v.as_str())
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("agent_{run_id}_{i}")),
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
            created_at: e.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0),
        })
        .collect();
    Ok(records)
}

#[tauri::command]
pub async fn list_run_events_bulk(
    run_ids: Vec<String>,
) -> Result<Vec<(String, Vec<store::RunEventRecord>)>, crate::AppError> {
    let mut result = Vec::new();
    for run_id in run_ids {
        let events = list_run_events(run_id.clone()).await?;
        if !events.is_empty() {
            result.push((run_id, events));
        }
    }
    Ok(result)
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
