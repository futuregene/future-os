//! Tauri IPC adapter for shared application operations.
use crate::store;
pub(crate) fn close_thread_terminals(thread_id: &str) {
    if let Some(manager) = crate::terminal::server::manager() {
        manager.close_thread(thread_id);
    }
}
#[tauri::command]
pub async fn fork_thread(
    thread_id: String,
    source_entry_id: String,
    request_id: String,
) -> Result<String, crate::AppError> {
    future_host::application::threads::fork_thread(thread_id, source_entry_id, request_id).await
}

#[tauri::command]
pub fn list_threads() -> Result<Vec<store::ThreadRecord>, crate::AppError> {
    future_host::application::threads::list_threads()
}

#[tauri::command]
pub fn get_thread(thread_id: String) -> Result<Option<store::ThreadRecord>, crate::AppError> {
    future_host::application::threads::get_thread(thread_id)
}

#[tauri::command]
pub fn get_recent_thread() -> Result<Option<store::ThreadRecord>, crate::AppError> {
    future_host::application::threads::get_recent_thread()
}

#[tauri::command]
pub fn mark_thread_opened(thread_id: String) -> Result<(), crate::AppError> {
    future_host::application::threads::mark_thread_opened(thread_id)
}

#[tauri::command]
pub fn create_thread(
    input: store::CreateThreadInput,
) -> Result<store::ThreadRecord, crate::AppError> {
    future_host::application::threads::create_thread(input)
}

#[tauri::command]
pub async fn generate_thread_title(
    thread_id: String,
    language: String,
) -> Result<serde_json::Value, crate::AppError> {
    future_host::application::threads::generate_thread_title(thread_id, language).await
}

#[tauri::command]
pub async fn rename_thread(
    input: store::RenameThreadInput,
) -> Result<store::ThreadRecord, crate::AppError> {
    future_host::application::threads::rename_thread(input).await
}

#[tauri::command]
pub async fn update_thread_model(
    input: store::UpdateThreadModelInput,
) -> Result<store::ThreadRecord, crate::AppError> {
    future_host::application::threads::update_thread_model(input).await
}

#[tauri::command]
pub async fn update_thread_thinking_level(
    input: store::UpdateThreadThinkingLevelInput,
) -> Result<store::ThreadRecord, crate::AppError> {
    future_host::application::threads::update_thread_thinking_level(input).await
}

#[tauri::command]
pub fn pin_thread(input: store::PinThreadInput) -> Result<store::ThreadRecord, crate::AppError> {
    future_host::application::threads::pin_thread(input)
}

#[tauri::command]
pub fn archive_thread(thread_id: String) -> Result<store::ThreadRecord, crate::AppError> {
    future_host::application::threads::archive_thread(thread_id)
}

#[tauri::command]
pub fn restore_thread(thread_id: String) -> Result<store::ThreadRecord, crate::AppError> {
    future_host::application::threads::restore_thread(thread_id)
}

#[tauri::command]
pub async fn delete_thread(
    input: store::DeleteThreadInput,
) -> Result<store::ThreadRecord, crate::AppError> {
    future_host::application::threads::delete_thread(input).await
}

#[tauri::command]
pub async fn batch_delete_threads(
    input: store::BatchDeleteThreadsInput,
) -> Result<store::BatchDeleteResult, crate::AppError> {
    future_host::application::threads::batch_delete_threads(input).await
}

#[tauri::command]
pub async fn list_streaming_thread_ids() -> Result<Vec<String>, crate::AppError> {
    future_host::application::threads::list_streaming_thread_ids().await
}

#[tauri::command]
pub async fn get_thread_agent_state(
    thread_id: String,
) -> Result<serde_json::Value, crate::AppError> {
    future_host::application::threads::get_thread_agent_state(thread_id).await
}

#[tauri::command]
pub async fn compact_thread_context(
    thread_id: String,
) -> Result<serde_json::Value, crate::AppError> {
    future_host::application::threads::compact_thread_context(thread_id).await
}

#[tauri::command]
pub async fn get_session_entries(thread_id: String) -> Result<serde_json::Value, crate::AppError> {
    future_host::application::threads::get_session_entries(thread_id).await
}

#[tauri::command]
pub async fn get_session_entries_page(
    thread_id: String,
    before: Option<i64>,
    limit: i64,
    history_source: Option<String>,
) -> Result<serde_json::Value, crate::AppError> {
    future_host::application::threads::get_session_entries_page(
        thread_id,
        before,
        limit,
        history_source,
    )
    .await
}

#[tauri::command]
pub fn archive_finished_runs(thread_id: String) -> Result<usize, crate::AppError> {
    future_host::application::threads::archive_finished_runs(thread_id)
}

#[tauri::command]
pub fn get_thread_cleanup_summary(
    thread_id: String,
) -> Result<store::ThreadCleanupSummary, crate::AppError> {
    future_host::application::threads::get_thread_cleanup_summary(thread_id)
}

#[tauri::command]
pub async fn attach_remote_stream(thread_id: String) -> Result<serde_json::Value, crate::AppError> {
    future_host::application::threads::attach_remote_stream(thread_id).await
}

#[tauri::command]
pub fn observe_session(thread_id: String, session_id: String) -> Result<(), crate::AppError> {
    future_host::application::threads::observe_session(thread_id, session_id)
}

#[tauri::command]
pub fn reconcile_thread_workspace(session_id: String, cwd: String) -> Result<(), crate::AppError> {
    future_host::application::threads::reconcile_thread_workspace(session_id, cwd)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    fn init(label: &str) -> HomeGuard {
        let home = HomeGuard::new(label);
        crate::store::initialize_app_store().expect("init store");
        home
    }

    fn make_thread(home: &HomeGuard, agent_session_id: Option<&str>) -> store::ThreadRecord {
        let _ = home;
        crate::store::create_thread(store::CreateThreadInput {
            mode: "chat".into(),
            title: Some("Chat".into()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: agent_session_id.map(str::to_string),
        })
        .expect("create thread")
    }

    #[test]
    fn async_command_wrappers_reject_malformed_bodies() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![
                fork_thread,
                rename_thread,
                update_thread_model,
                update_thread_thinking_level,
                delete_thread,
                batch_delete_threads,
                get_thread_agent_state,
                get_session_entries,
                archive_finished_runs,
                attach_remote_stream,
                compact_thread_context
            ],
            &[
                "fork_thread",
                "rename_thread",
                "update_thread_model",
                "update_thread_thinking_level",
                "delete_thread",
                "batch_delete_threads",
                "get_thread_agent_state",
                "get_session_entries",
                "archive_finished_runs",
                "attach_remote_stream",
                "compact_thread_context",
            ],
        );
        // `fork_thread` takes three arguments, so the empty-body rejection only
        // exercises its *first* argument's error arm (attributed to the signature
        // line). Fail the *last* argument instead to hit the error arm attributed
        // to the `#[tauri::command]` attribute line.
        crate::commands::ipc_harness::assert_all_reject_bodies(
            tauri::generate_handler![fork_thread],
            &[(
                "fork_thread",
                serde_json::json!({ "threadId": "t", "sourceEntryId": "e" }),
            )],
        );
        // The same last-argument rule for the other multi-argument commands:
        // the title generator and the paged history reader each fail on their
        // *last* argument once the earlier ones are satisfied.
        crate::commands::ipc_harness::assert_all_reject_bodies(
            tauri::generate_handler![generate_thread_title, get_session_entries_page],
            &[
                (
                    "generate_thread_title",
                    serde_json::json!({ "threadId": "t" }),
                ),
                (
                    "get_session_entries_page",
                    serde_json::json!({ "threadId": "t", "before": null }),
                ),
            ],
        );
    }

    #[test]
    fn thread_read_and_pin_commands_round_trip() {
        let _home = init("cmd_threads");
        assert!(list_threads().expect("list empty").is_empty());
        assert!(get_recent_thread().expect("recent empty").is_none());

        let thread = make_thread(&_home, None);
        assert_eq!(list_threads().expect("list").len(), 1);
        assert_eq!(
            get_thread(thread.id.clone()).expect("get").map(|t| t.id),
            Some(thread.id.clone())
        );
        assert!(get_thread("ghost".into()).expect("ghost").is_none());
        assert_eq!(
            get_recent_thread().expect("recent").map(|t| t.id),
            Some(thread.id.clone())
        );

        let pinned = pin_thread(store::PinThreadInput {
            thread_id: thread.id.clone(),
            pinned: true,
        })
        .expect("pin");
        assert!(pinned.pinned);

        let archived = archive_thread(thread.id.clone()).expect("archive");
        assert_eq!(archived.status, "archived");

        let restored = restore_thread(thread.id.clone()).expect("restore");
        assert_eq!(restored.status, "active");
    }
}
