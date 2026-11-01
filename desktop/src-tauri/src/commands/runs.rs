//! Tauri IPC adapter for shared application operations.
use crate::store;
#[tauri::command]
pub fn create_run(input: store::CreateRunInput) -> Result<store::RunRecord, crate::AppError> {
    future_host::application::runs::create_run(input)
}

#[tauri::command]
pub fn list_runs(thread_id: String) -> Result<Vec<store::RunRecord>, crate::AppError> {
    future_host::application::runs::list_runs(thread_id)
}

#[tauri::command]
pub fn get_latest_run(thread_id: String) -> Result<Option<store::RunRecord>, crate::AppError> {
    future_host::application::runs::get_latest_run(thread_id)
}

#[tauri::command]
pub fn get_run(run_id: String) -> Result<Option<store::RunRecord>, crate::AppError> {
    future_host::application::runs::get_run(run_id)
}

#[tauri::command]
pub fn list_latest_run_infos(
    thread_ids: Vec<String>,
) -> Result<Vec<store::LatestRunInfo>, crate::AppError> {
    future_host::application::runs::list_latest_run_infos(thread_ids)
}

#[tauri::command]
pub fn update_run_status(
    input: store::UpdateRunStatusInput,
) -> Result<store::RunRecord, crate::AppError> {
    future_host::application::runs::update_run_status(input)
}

#[tauri::command]
pub async fn abort_run(
    thread_id: String,
    run_id: String,
) -> Result<store::RunRecord, crate::AppError> {
    future_host::application::runs::abort_run(thread_id, run_id).await
}

#[tauri::command]
pub async fn list_run_events(
    run_id: String,
) -> Result<Vec<store::RunEventRecord>, crate::AppError> {
    future_host::application::runs::list_run_events(run_id).await
}

#[tauri::command]
pub async fn list_run_events_since(
    run_id: String,
    since_sequence: i64,
) -> Result<Vec<store::RunEventRecord>, crate::AppError> {
    future_host::application::runs::list_run_events_since(run_id, since_sequence).await
}

#[tauri::command]
pub async fn list_run_events_bulk(
    run_ids: Vec<String>,
) -> Result<Vec<(String, Vec<store::RunEventRecord>)>, crate::AppError> {
    future_host::application::runs::list_run_events_bulk(run_ids).await
}

#[tauri::command]
pub async fn list_tool_calls(
    run_id: String,
) -> Result<Vec<store::ToolCallRecord>, crate::AppError> {
    future_host::application::runs::list_tool_calls(run_id).await
}

#[tauri::command]
pub async fn list_tool_calls_bulk(
    run_ids: Vec<String>,
) -> Result<Vec<(String, Vec<store::ToolCallRecord>)>, crate::AppError> {
    future_host::application::runs::list_tool_calls_bulk(run_ids).await
}

#[tauri::command]
pub async fn list_tool_outputs(
    run_id: String,
    tool_call_id: String,
) -> Result<Vec<store::ToolOutputRecord>, crate::AppError> {
    future_host::application::runs::list_tool_outputs(run_id, tool_call_id).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
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
            agent_session_id: Some("synthetic-session".into()),
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
    fn async_command_wrappers_reject_malformed_bodies() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![
                abort_run,
                list_run_events,
                list_run_events_since,
                list_run_events_bulk,
                list_tool_calls,
                list_tool_calls_bulk,
                list_tool_outputs
            ],
            &[
                "abort_run",
                "list_run_events",
                "list_run_events_since",
                "list_run_events_bulk",
                "list_tool_calls",
                "list_tool_calls_bulk",
                "list_tool_outputs",
            ],
        );
        // Multi-argument commands fail their *first* missing key with the
        // empty body above, whose error arm is attributed to the signature
        // line. Fail the *last* argument instead to hit the error arm on the
        // `#[tauri::command]` attribute line.
        crate::commands::ipc_harness::assert_all_reject_bodies(
            tauri::generate_handler![abort_run, list_run_events_since, list_tool_outputs],
            &[
                ("abort_run", serde_json::json!({ "threadId": "x" })),
                ("list_run_events_since", serde_json::json!({ "runId": "x" })),
                ("list_tool_outputs", serde_json::json!({ "runId": "x" })),
            ],
        );
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

    /// The agent-backed run commands the transcript polls. Each result carries
    /// the id the wrapper passed (the journal rows are labelled with the run
    /// they were asked for, the tool output with the tool call it was asked
    /// for), so a dropped argument shows up as a different id, and the cursor
    /// command's two arms are decided by the number it forwarded.
    #[tokio::test]
    async fn the_agent_backed_run_commands_forward_their_arguments() {
        use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
        use std::collections::HashMap;

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_runs_agent");
        create_run(run_input(&thread.id, "run_wave")).expect("create run");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([
                (
                    "get_events_since".into(),
                    serde_json::json!({
                        "events": [{
                            "eventId": "e1",
                            "type": "assistant",
                            "data": "{}",
                            "idx": 0,
                            "timestamp": "2024-01-01T00:00:00Z",
                        }],
                    })
                    .to_string(),
                ),
                (
                    "list_tool_calls".into(),
                    serde_json::json!({
                        "tools": [{
                            "toolCallId": "tc-1",
                            "name": "Read",
                            "status": "completed",
                            "startedAtMs": 3,
                        }],
                        "hasMore": false,
                    })
                    .to_string(),
                ),
                (
                    "get_tool_output".into(),
                    serde_json::json!({
                        "tools": [],
                        "output": { "isError": false, "text": "done", "createdAtMs": 7 },
                    })
                    .to_string(),
                ),
            ]),
            ..Default::default()
        });

        let events = list_run_events("run_wave".into()).await.expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "e1");
        assert_eq!(
            events[0].run_id, "run_wave",
            "the row is labelled with the run the caller asked for"
        );

        let at_cursor = list_run_events_since("run_wave".into(), 0)
            .await
            .expect("since");
        assert!(
            at_cursor.is_empty(),
            "the cursor is forwarded: an event at the cursor is not repeated"
        );
        let whole = list_run_events_since("run_wave".into(), -1)
            .await
            .expect("negative cursor");
        assert_eq!(whole.len(), 1, "a negative cursor asks for the whole run");

        let bulk = list_run_events_bulk(vec!["run_wave".into()])
            .await
            .expect("bulk events");
        assert_eq!(bulk.len(), 1);
        assert_eq!(bulk[0].0, "run_wave");
        assert_eq!(bulk[0].1[0].run_id, "run_wave");

        let tools = list_tool_calls("run_wave".into()).await.expect("tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].id, "tc-1");
        assert_eq!(tools[0].run_id, "run_wave");

        let tools_bulk = list_tool_calls_bulk(vec!["run_wave".into()])
            .await
            .expect("bulk tools");
        assert_eq!(tools_bulk.len(), 1);
        assert_eq!(tools_bulk[0].0, "run_wave");

        let outputs = list_tool_outputs("run_wave".into(), "tc-1".into())
            .await
            .expect("tool outputs");
        assert_eq!(outputs.len(), 1);
        assert_eq!(
            outputs[0].tool_call_id, "tc-1",
            "the tool call id is forwarded, not the run's own id"
        );
        assert_eq!(outputs[0].kind, "text");

        // Abort is a mutation, so its contract is the two refusals: an unknown
        // run, and a run that belongs to another conversation.
        let ghost = abort_run(thread.id.clone(), "run_absent".into())
            .await
            .expect_err("an unknown run cannot be aborted");
        assert!(
            ghost.to_string().contains("Run could not be loaded"),
            "{ghost}"
        );
        let foreign = abort_run("thread_absent".into(), "run_wave".into())
            .await
            .expect_err("a run is only abortable from its own conversation");
        assert!(foreign.to_string().contains("does not belong"), "{foreign}");

        let aborted = abort_run(thread.id.clone(), "run_wave".into())
            .await
            .expect("abort");
        assert_eq!(aborted.id, "run_wave");
        assert_eq!(aborted.status, "cancelled");
        script_mock_agent(MockScript::default());
    }
}
