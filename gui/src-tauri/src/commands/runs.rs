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
pub async fn list_run_events(run_id: String) -> Result<Vec<store::RunEventRecord>, crate::AppError> {
    let local = store::list_run_events(&run_id)?;
    // After a GUI restart the in-memory RUN_EVENT_BUFFER is empty — the agent
    // (still running in the background) holds the authoritative events.  Pull
    // them from the agent for active runs with no local events.
    if !local.is_empty() {
        return Ok(local);
    }
    let Some(run) = store::get_run(&run_id).ok().flatten() else {
        return Ok(local);
    };
    if matches!(run.status.as_str(), "completed" | "failed" | "cancelled") {
        return Ok(local);
    }
    // Active run — pull events from the agent
    let Some(thread) = store::get_thread(&run.thread_id).ok().flatten() else {
        return Ok(local);
    };
    let sid = thread
        .agent_session_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| run.thread_id.clone());
    let agent_json = match agent_bridge::get_events_since(sid, run_id.clone(), -1).await {
        Ok(v) => v,
        Err(_) => return Ok(local),
    };
    let Some(events) = agent_json.get("events").and_then(|v| v.as_array()) else {
        return Ok(local);
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
            run_id: run_id.clone(),
            event_type: e.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            payload: e.get("data").and_then(|v| v.as_str()).map(|s| s.to_string()),
            sequence: e.get("idx").and_then(|v| v.as_i64()).unwrap_or(i as i64),
            created_at: now,
        })
        .collect();
    if records.is_empty() {
        Ok(local)
    } else {
        Ok(records)
    }
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

#[tauri::command]
pub fn list_tool_outputs(
    run_id: String,
    tool_call_id: String,
) -> Result<Vec<store::ToolOutputRecord>, crate::AppError> {
    store::list_tool_outputs(&run_id, &tool_call_id)
}
