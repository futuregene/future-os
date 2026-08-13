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
    // These are display readers: any Agent-side failure (unreachable, or no
    // durable history for the run) degrades to the legacy GUI JSONL instead
    // of blanking the panel with an error.
    match agent_events(&run_id, -1).await {
        Ok(events) => Ok(events),
        Err(error) => {
            if !agent_unavailable(&error) {
                eprintln!("FutureOS: agent event read failed for {run_id}, falling back to legacy log: {error}");
            }
            store::list_run_events(&run_id)
        }
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
        Err(error) => {
            if !agent_unavailable(&error) {
                eprintln!("FutureOS: agent event read failed for {run_id}, falling back to legacy log: {error}");
            }
            store::list_run_events_since(&run_id, since_sequence)
        }
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
            created_at: e
                .get("timestamp")
                .and_then(|value| value.as_str())
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.timestamp_millis())
                .unwrap_or(0),
        })
        .collect();
    Ok(records)
}

fn agent_unavailable(error: &crate::AppError) -> bool {
    matches!(error, crate::AppError::AgentUnavailable(_))
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

/// Fetch a run's event tail for tool projection: Agent journal first (the
/// canonical source), legacy GUI JSONL only when the Agent is unreachable
/// (pre-journal compatibility).
async fn tool_events_since(
    run_id: &str,
    since_sequence: i64,
) -> Result<Vec<store::RunEventRecord>, crate::AppError> {
    match agent_events(run_id, since_sequence).await {
        Ok(events) => Ok(events),
        Err(error) if agent_unavailable(&error) => {
            store::list_run_events_since(run_id, since_sequence)
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
pub async fn list_tool_calls(
    run_id: String,
) -> Result<Vec<store::ToolCallRecord>, crate::AppError> {
    advance_tool_projection(&run_id).await
}

/// Batch variant: the context panel's poll needs tool calls for every run of
/// the thread — one IPC round-trip instead of N. Every run id appears in the
/// result (with an empty vec when it has no tool activity) so the caller's
/// `Object.fromEntries` shape is unchanged.
#[tauri::command]
pub async fn list_tool_calls_bulk(
    run_ids: Vec<String>,
) -> Result<Vec<(String, Vec<store::ToolCallRecord>)>, crate::AppError> {
    let mut result = Vec::with_capacity(run_ids.len());
    for run_id in run_ids {
        let tools = advance_tool_projection(&run_id).await?;
        result.push((run_id, tools));
    }
    Ok(result)
}

/// Advance the run's cached tool projection over the events appended since
/// the last read, keeping the poll at O(new events) instead of O(full log).
async fn advance_tool_projection(
    run_id: &str,
) -> Result<Vec<store::ToolCallRecord>, crate::AppError> {
    let cursor = store::tool_projection_cursor(run_id);
    let events = tool_events_since(run_id, cursor).await?;
    Ok(store::advance_tool_projection(run_id, &events))
}

#[tauri::command]
pub async fn list_tool_outputs(
    run_id: String,
    tool_call_id: String,
) -> Result<Vec<store::ToolOutputRecord>, crate::AppError> {
    let events = tool_events_since(&run_id, -1).await?;
    Ok(store::project_tool_outputs(&events, &tool_call_id))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::agent_unavailable;
    use super::*;

    use crate::auth_store::test_support::HomeGuard;
    use crate::store;

    fn seeded(label: &str) -> (HomeGuard, store::ThreadRecord) {
        let home = HomeGuard::new(label);
        crate::store::initialize_app_store().expect("init store");
        let ws = crate::store::create_workspace(store::CreateWorkspaceInput {
            name: Some("WS".into()),
            path: std::env::temp_dir()
                .join(format!("futureos-cmd-run-ws-{}", std::process::id()))
                .display()
                .to_string(),
            description: None,
            create_directory: Some(true),
        })
        .expect("create workspace");
        let thread = crate::store::create_thread(store::CreateThreadInput {
            mode: "workspace".into(),
            title: Some("Runs".into()),
            workspace_id: Some(ws.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create thread");
        (home, thread)
    }

    fn run_input(thread_id: &str, id: &str) -> store::CreateRunInput {
        store::CreateRunInput {
            id: Some(id.into()),
            thread_id: thread_id.into(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        }
    }

    #[test]
    fn run_read_wrappers_round_trip() {
        let (_home, thread) = seeded("cmd_runs");
        let created = create_run(run_input(&thread.id, "run_1")).expect("create run");
        assert_eq!(created.id, "run_1");

        assert_eq!(list_runs(thread.id.clone()).expect("list").len(), 1);
        assert_eq!(
            get_latest_run(thread.id.clone())
                .expect("latest")
                .map(|r| r.id),
            Some("run_1".into())
        );
        assert_eq!(
            get_run("run_1".into()).expect("get").map(|r| r.id),
            Some("run_1".into())
        );
        assert!(get_run("ghost".into()).expect("get ghost").is_none());

        let infos =
            list_latest_run_infos(vec![thread.id.clone(), "ghost".into()]).expect("latest infos");
        assert_eq!(infos.len(), 1);
    }

    #[test]
    fn update_run_status_returns_the_row_truth() {
        let (_home, thread) = seeded("cmd_run_status");
        create_run(run_input(&thread.id, "run_2")).expect("create run");
        let updated = update_run_status(store::UpdateRunStatusInput {
            run_id: "run_2".into(),
            status: "completed".into(),
            error_message: Some("boom".into()),
            error_type: Some("model".into()),
        })
        .expect("update status");
        assert_eq!(updated.status, "completed");
        assert_eq!(updated.error_message.as_deref(), Some("boom"));
    }

    #[test]
    fn update_run_status_errors_on_a_ghost_run() {
        let (_home, _thread) = seeded("cmd_run_ghost");
        assert!(update_run_status(store::UpdateRunStatusInput {
            run_id: "ghost".into(),
            status: "completed".into(),
            error_message: None,
            error_type: None,
        })
        .is_err());
    }

    #[test]
    fn legacy_fallback_requires_the_typed_transport_error() {
        assert!(agent_unavailable(&crate::AppError::AgentUnavailable(
            "connection refused".to_string()
        )));
        assert!(!agent_unavailable(&crate::AppError::Message(
            "model response says service unavailable".to_string()
        )));
    }

    #[tokio::test]
    async fn list_run_events_reads_the_agent_journal() {
        use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
        use std::collections::HashMap;

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_events");
        create_run(run_input(&thread.id, "run_ev")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "get_events_since".to_string(),
                "{\"events\":[]}".to_string(),
            )]),
            ..Default::default()
        });
        let events = list_run_events("run_ev".into()).await.expect("events");
        assert!(events.is_empty());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn list_run_events_maps_agent_journal_rows_into_records() {
        use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
        use std::collections::HashMap;

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_events_map");
        create_run(run_input(&thread.id, "run_map")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();
        let data = serde_json::json!({
            "events": [
                {
                    "eventId": "e1",
                    "type": "assistant",
                    "data": "{\"x\":1}",
                    "idx": 0,
                    "timestamp": "2024-01-01T00:00:00Z"
                },
                {
                    "eventId": "",
                    "type": null,
                    "data": 123,
                    "idx": null,
                    "timestamp": "not-a-date"
                }
            ]
        })
        .to_string();
        script_mock_agent(MockScript {
            data: HashMap::from([("get_events_since".to_string(), data)]),
            ..Default::default()
        });
        let events = list_run_events("run_map".into()).await.expect("events");
        assert_eq!(events.len(), 2);
        // Full row: every field read from the journal.
        assert_eq!(events[0].id, "e1");
        assert_eq!(events[0].event_type, "assistant");
        assert_eq!(events[0].payload.as_deref(), Some("{\"x\":1}"));
        assert_eq!(events[0].sequence, 0);
        assert_eq!(events[0].created_at, 1704067200000);
        // Sparse row: every fallback fires (generated id, empty type, no
        // payload, index-derived sequence, epoch timestamp).
        assert_eq!(events[1].id, "agent_run_map_1");
        assert_eq!(events[1].event_type, "");
        assert_eq!(events[1].payload, None);
        assert_eq!(events[1].sequence, 1);
        assert_eq!(events[1].created_at, 0);
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn list_run_events_falls_back_when_the_agent_is_down() {
        use crate::commands::agent_mock::{mock_agent_lock, with_broken_endpoint};

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_events_down");
        create_run(run_input(&thread.id, "run_down")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();
        let events = with_broken_endpoint(|| list_run_events("run_down".into()))
            .await
            .expect("fallback");
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn list_run_events_since_negative_delegates() {
        use crate::commands::agent_mock::{mock_agent_lock, with_broken_endpoint};

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_events_since");
        create_run(run_input(&thread.id, "run_since")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();
        let events = with_broken_endpoint(|| list_run_events_since("run_since".into(), -1))
            .await
            .expect("delegated fallback");
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn list_run_events_since_reads_the_agent_when_up() {
        use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
        use std::collections::HashMap;

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_events_since_up");
        create_run(run_input(&thread.id, "run_since_up")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "get_events_since".to_string(),
                "{\"events\":[]}".to_string(),
            )]),
            ..Default::default()
        });
        let events = list_run_events_since("run_since_up".into(), 0)
            .await
            .expect("events");
        assert!(events.is_empty());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn list_run_events_since_logs_and_falls_back_on_non_transport_error() {
        use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
        use std::collections::HashMap;

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_events_since_reject");
        create_run(run_input(&thread.id, "run_since_rej")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            errors: HashMap::from([("get_events_since".to_string(), "unknown run".to_string())]),
            ..Default::default()
        });
        let events = list_run_events_since("run_since_rej".into(), 0)
            .await
            .expect("fallback");
        assert!(events.is_empty());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn list_run_events_since_incremental_falls_back_when_down() {
        use crate::commands::agent_mock::{mock_agent_lock, with_broken_endpoint};

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_events_since_inc");
        create_run(run_input(&thread.id, "run_since_inc")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();
        let events = with_broken_endpoint(|| list_run_events_since("run_since_inc".into(), 3))
            .await
            .expect("incremental fallback");
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn agent_events_treats_a_non_array_events_field_as_empty() {
        use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
        use std::collections::HashMap;

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_events_noarray");
        create_run(run_input(&thread.id, "run_noarray")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "get_events_since".to_string(),
                "{\"events\":\"not-an-array\"}".to_string(),
            )]),
            ..Default::default()
        });
        let events = list_run_events("run_noarray".into()).await.expect("events");
        assert!(events.is_empty());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn list_run_events_bulk_keeps_non_empty_runs() {
        use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
        use std::collections::HashMap;

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_events_bulk_full");
        create_run(run_input(&thread.id, "run_bulk_full")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "get_events_since".to_string(),
                "{\"events\":[{\"eventId\":\"e1\",\"type\":\"assistant\",\"data\":\"{}\",\"idx\":0,\"timestamp\":\"2024-01-01T00:00:00Z\"}]}".to_string(),
            )]),
            ..Default::default()
        });
        let bulk = list_run_events_bulk(vec!["run_bulk_full".into()])
            .await
            .expect("bulk");
        assert_eq!(bulk.len(), 1);
        assert_eq!(bulk[0].1.len(), 1);
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn tool_events_since_reads_the_agent_when_up() {
        use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
        use std::collections::HashMap;

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_tools_up");
        create_run(run_input(&thread.id, "run_tools_up")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "get_events_since".to_string(),
                "{\"events\":[]}".to_string(),
            )]),
            ..Default::default()
        });
        let calls = list_tool_calls("run_tools_up".into())
            .await
            .expect("tool calls");
        assert!(calls.is_empty());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn list_run_events_bulk_skips_empty_runs() {
        use crate::commands::agent_mock::{mock_agent_lock, with_broken_endpoint};

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_events_bulk");
        create_run(run_input(&thread.id, "run_bulk")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();
        let bulk =
            with_broken_endpoint(|| list_run_events_bulk(vec!["run_bulk".into(), "ghost".into()]))
                .await
                .expect("bulk");
        assert!(bulk.is_empty());
    }

    #[tokio::test]
    async fn tool_projection_readers_degrade_to_legacy_when_down() {
        use crate::commands::agent_mock::{mock_agent_lock, with_broken_endpoint};

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_tools_down");
        create_run(run_input(&thread.id, "run_tools")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();

        let calls = with_broken_endpoint(|| list_tool_calls("run_tools".into()))
            .await
            .expect("tool calls");
        assert!(calls.is_empty());

        let bulk = with_broken_endpoint(|| list_tool_calls_bulk(vec!["run_tools".into()]))
            .await
            .expect("tool calls bulk");
        assert_eq!(bulk.len(), 1);
        assert!(bulk[0].1.is_empty());

        let outputs =
            with_broken_endpoint(|| list_tool_outputs("run_tools".into(), "tool_1".into()))
                .await
                .expect("tool outputs");
        assert!(outputs.is_empty());
    }

    #[tokio::test]
    async fn list_tool_calls_errors_on_an_unknown_run() {
        use crate::commands::agent_mock::{mock_agent_lock, with_broken_endpoint};

        let _lock = mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd_tools_ghost");
        crate::store::initialize_app_store().expect("init store");
        crate::commands::agent_mock::ensure_mock_agent();
        let result = with_broken_endpoint(|| list_tool_calls("ghost".into())).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn abort_run_cancels_locally_when_the_agent_is_down() {
        use crate::commands::agent_mock::{mock_agent_lock, with_broken_endpoint};

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_abort");
        create_run(run_input(&thread.id, "run_abort")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();
        let aborted = with_broken_endpoint(|| abort_run(thread.id.clone(), "run_abort".into()))
            .await
            .expect("abort");
        assert_eq!(aborted.status, "cancelled");
    }

    #[tokio::test]
    async fn abort_run_errors_for_a_missing_thread() {
        use crate::commands::agent_mock::{mock_agent_lock, with_broken_endpoint};

        let _lock = mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd_abort_ghost");
        crate::store::initialize_app_store().expect("init store");
        crate::commands::agent_mock::ensure_mock_agent();
        assert!(
            with_broken_endpoint(|| abort_run("ghost".into(), "run".into()))
                .await
                .is_err()
        );
    }
}
