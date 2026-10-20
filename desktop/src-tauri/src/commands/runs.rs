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
}
