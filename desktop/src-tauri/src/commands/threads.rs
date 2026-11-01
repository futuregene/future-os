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
                // The paged reader's last argument is an `Option`, which a
                // missing key legitimately fills with `None`, so the body above
                // fails earlier — on `limit`. A *wrong-typed* last argument is
                // what reaches the wrapper's final `?`, the arm the macro
                // attributes to the `#[tauri::command]` line.
                (
                    "get_session_entries_page",
                    serde_json::json!({
                        "threadId": "t",
                        "before": null,
                        "limit": 20,
                        "historySource": 5,
                    }),
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

    /// The store-backed half of the conversation surface: the rows the sidebar
    /// and the cleanup dialog read, plus the terminal closer every deletion
    /// path runs first. Each assertion is on a value derived from the argument
    /// (the created title, the recency stamp the id was written to, the count
    /// for *this* conversation), so a dropped argument changes the answer.
    #[test]
    fn the_store_backed_thread_commands_forward_their_arguments() {
        let _home = init("cmd_threads_wave");

        let created = create_thread(store::CreateThreadInput {
            mode: "chat".into(),
            title: Some("Created".into()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create thread");
        assert_eq!(created.title, "Created");
        assert_eq!(created.mode, "chat");

        // The terminal closer answers nothing: it forwards the id to the
        // terminal server when one is running and is a documented no-op when
        // none is (a libtest process usually has none, but another test in the
        // same binary may have started one, so the assertion cannot rest on
        // which). What it must never do is take the conversation with it.
        close_thread_terminals(&created.id);
        close_thread_terminals("thread_absent");
        assert!(
            get_thread(created.id.clone()).expect("get").is_some(),
            "closing a conversation's terminals does not remove the conversation"
        );

        mark_thread_opened(created.id.clone()).expect("mark opened");
        let reopened = get_thread(created.id.clone())
            .expect("get thread")
            .expect("the thread exists");
        assert!(
            reopened.last_opened_at.is_some(),
            "the recency stamp was written to the conversation the caller named"
        );

        // Nothing has finished in a fresh conversation, so the archive call
        // reports nothing ...
        assert_eq!(
            archive_finished_runs(created.id.clone()).expect("archive with no run"),
            0
        );
        crate::store::create_run(store::CreateRunInput {
            id: Some("run_wave_finished".into()),
            thread_id: created.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("create run");
        crate::update_run_status(store::UpdateRunStatusInput {
            run_id: "run_wave_finished".into(),
            status: "completed".into(),
            error_message: None,
            error_type: None,
        })
        .expect("finish run");
        // ... and exactly the one finished run once there is one.
        assert_eq!(
            archive_finished_runs(created.id.clone()).expect("archive"),
            1
        );

        let summary = get_thread_cleanup_summary(created.id.clone()).expect("cleanup summary");
        assert_eq!(summary.thread_id, created.id);
        assert_eq!(
            summary.cleanup_status, "active",
            "a conversation that still exists is reported as active, not pending cleanup"
        );
    }

    /// The agent-backed half. Every call here is given an argument the shared
    /// service refuses by name (no session, a blank title, a mismatched
    /// observer binding) or one whose effect it echoes back, so the assertion
    /// both covers the forwarding and names the layer that rejected it.
    ///
    /// `update_thread_model` / `update_thread_thinking_level` are deliberately
    /// thin: the row write is a documented no-op (the values live in the agent
    /// session), so what this module can prove is that the conversation the
    /// caller named is the one returned — the agent-side propagation is covered
    /// where the RPC is built.
    #[tokio::test]
    async fn the_agent_backed_thread_commands_forward_their_arguments() {
        use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
        use std::collections::HashMap;

        let _lock = mock_agent_lock();
        let home = init("cmd_threads_agent");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "generate_session_title".to_string(),
                serde_json::json!({ "title": "Generated" }).to_string(),
            )]),
            ..Default::default()
        });

        let bound = make_thread(&home, Some("wave-session"));
        let plain = make_thread(&home, None);

        let no_session = fork_thread(plain.id.clone(), "entry".into(), "req".into())
            .await
            .expect_err("a conversation without an agent session cannot be forked");
        assert!(
            no_session.to_string().contains("No agent session"),
            "{no_session}"
        );

        let title = generate_thread_title(bound.id.clone(), "zh".into())
            .await
            .expect("generate title");
        assert_eq!(title["title"], serde_json::json!("Generated"));

        let blank = rename_thread(store::RenameThreadInput {
            thread_id: bound.id.clone(),
            title: "   ".into(),
        })
        .await
        .expect_err("a blank title is refused before it reaches the agent");
        assert!(
            blank.to_string().contains("title cannot be empty"),
            "{blank}"
        );
        let renamed = rename_thread(store::RenameThreadInput {
            thread_id: bound.id.clone(),
            title: "Renamed".into(),
        })
        .await
        .expect("rename");
        assert_eq!(renamed.id, bound.id);
        assert_eq!(renamed.title, "Renamed");

        let model = update_thread_model(store::UpdateThreadModelInput {
            thread_id: bound.id.clone(),
            model_provider: Some("future".into()),
            model_id: Some("future/gpt-5".into()),
        })
        .await
        .expect("update model");
        assert_eq!(model.id, bound.id);
        let thinking = update_thread_thinking_level(store::UpdateThreadThinkingLevelInput {
            thread_id: bound.id.clone(),
            thinking_level: Some("high".into()),
        })
        .await
        .expect("update thinking level");
        assert_eq!(thinking.id, bound.id);
        let ghost_model = update_thread_model(store::UpdateThreadModelInput {
            thread_id: "thread_absent".into(),
            model_provider: None,
            model_id: Some("future/gpt-5".into()),
        })
        .await
        .expect_err("an unknown conversation cannot take a model");
        assert_eq!(
            ghost_model.to_string(),
            "Thread could not be loaded.",
            "the id the caller named is the one the store looked up"
        );

        let no_context = compact_thread_context(plain.id.clone())
            .await
            .expect_err("nothing to compact without an agent session");
        assert!(
            no_context.to_string().contains("no context to compact"),
            "{no_context}"
        );

        let entries = get_session_entries(plain.id.clone())
            .await
            .expect("history of a conversation with no agent session");
        assert_eq!(entries["entries"], serde_json::json!([]));

        let no_stream = attach_remote_stream(plain.id.clone())
            .await
            .expect_err("there is no remote stream to attach to");
        assert!(
            no_stream.to_string().contains("no agent session"),
            "{no_stream}"
        );

        let mismatch = observe_session(bound.id.clone(), "other-session".into())
            .expect_err("the observer must name this conversation's own session");
        assert!(
            mismatch.to_string().contains("observer_binding_mismatch"),
            "{mismatch}"
        );

        reconcile_thread_workspace(
            "wave-session".into(),
            std::env::temp_dir().display().to_string(),
        )
        .expect("file the session under a new cwd");

        let deleted = delete_thread(store::DeleteThreadInput {
            thread_id: bound.id.clone(),
            delete_files: false,
        })
        .await
        .expect("delete thread");
        assert_eq!(deleted.id, bound.id);
        assert!(get_thread(bound.id.clone()).expect("get deleted").is_none());

        let batch = batch_delete_threads(store::BatchDeleteThreadsInput {
            thread_ids: vec![plain.id.clone(), "thread_absent".into()],
            delete_files: false,
        })
        .await
        .expect("batch delete");
        assert_eq!(batch.deleted_count, 1, "only the conversation that exists");
        assert_eq!(batch.failed.len(), 1, "{:?}", batch.failed);
        assert!(
            batch.failed[0].contains("thread_absent"),
            "{:?}",
            batch.failed
        );
        script_mock_agent(MockScript::default());
    }
}
