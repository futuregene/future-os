//! Desktop task notification adapter; execution and persistence live in Host.
#[cfg(test)]
pub(crate) use future_host::tasks::reset_for_test;
#[cfg(all(test, feature = "gui"))]
pub(crate) use future_host::tasks::wake;
pub(crate) use future_host::tasks::{start_headless, stop_admission};

#[cfg(feature = "gui")]
pub fn start<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    let _ = app;
    crate::runtime::set(tauri::async_runtime::handle().inner().clone());
    let notify: future_host::tasks::Notifier = std::sync::Arc::new(|thread_id| {
        if let Some(thread_id) = thread_id {
            crate::emit_remote_activity(thread_id);
        }
        crate::emit_threads_updated();
    });
    future_host::tasks::start_with_notifier(notify);
}

#[cfg(all(test, feature = "gui"))]
mod tests {
    use super::*;
    use future_host::test_support::tasks::{script_agent_setup, task};
    use future_tasks::{RunOrigin, RunStatus, Store};
    use std::time::Duration;
    use tempfile::TempDir;
    /// The window's tick loop end to end: a queued request is claimed by a tick
    /// and executed by the spawned run, `wake()` makes that happen without
    /// waiting for the next interval, and `stop_admission` ends the loop.
    #[cfg(feature = "gui")]
    #[tokio::test]
    async fn the_gui_loop_runs_a_queued_request_it_is_woken_for() {
        let _home = crate::agent_bridge::test_support::TestHome::new("tasks-loop-gui");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let dir = TempDir::new().unwrap();
        std::env::set_var("FUTURE_HOME", dir.path());
        // The executor writes the conversation it opens into the *app* store,
        // which resolves under the same FutureOS home.
        crate::store::initialize_app_store().expect("app store");
        reset_for_test();
        let store = Store::open(dir.path()).unwrap();

        let mut queued = task("queued");
        queued.prompt = "summarize yesterday".into();
        queued.cwd = std::env::temp_dir().display().to_string();
        queued.pending_request_at = Some(crate::store::now_millis());
        queued.pending_origin = Some(RunOrigin::Cli);
        store.insert_task(&queued).unwrap();
        script_agent_setup(&mock, "sess_loop_gui");
        mock.push_stream(crate::agent_bridge::test_support::StreamScript::Events(
            vec![crate::agent_bridge::test_support::stream_event(
                "@attach",
                0,
                "agent_end",
                r#"{"reason":"complete"}"#,
            )],
            None,
        ));

        let app = tauri::test::mock_app();
        start(app.handle().clone());
        wake();

        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let settled = loop {
            let store = Store::open(dir.path()).unwrap();
            if let Some(run) = store
                .list_runs_for_task(&queued.id, 5)
                .unwrap()
                .into_iter()
                .find(|run| run.status != RunStatus::Running)
            {
                break run;
            }
            if std::time::Instant::now() > deadline {
                panic!("the woken loop never executed the queued request");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(settled.status, RunStatus::Completed, "{settled:?}");

        // Closing admission ends the loop; the run already settled is untouched.
        stop_admission();
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(
            store.get_run(&settled.id).unwrap().unwrap().status,
            RunStatus::Completed
        );
        std::env::remove_var("FUTURE_HOME");
        reset_for_test();
    }
}
