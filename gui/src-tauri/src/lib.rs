mod agent_bridge;
mod agent_proto;
mod agent_providers;
mod agent_supervisor;
mod auth_store;
mod commands;
mod error;
mod future_login;
mod git_diff_parse;
mod git_review;
mod run_error;
mod shadow_review;
mod skills;
mod store;

use commands::*;
use error::AppError;

/// Cross-platform home directory. Prefers `HOME` (always set on macOS/Linux, and
/// what the test suite overrides to redirect storage) and falls back to
/// `USERPROFILE` on Windows, where `HOME` is normally unset.
pub(crate) fn home_dir() -> Option<String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
}

/// Process-wide lock for tests that mutate the global `HOME` env var
/// (`auth_store` and the shadow-review smoke test). `HOME` is process-global, so
/// those tests must run one at a time or they clobber each other's paths.
#[cfg(test)]
pub(crate) static TEST_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// App handle captured at setup, used to push events to the webview from
/// background tasks (e.g. deferred shadow-review materialization).
static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Size the main window to most of the monitor's work area (which already
/// excludes the taskbar/dock/menubar) and center it there — near-fullscreen but
/// not maximized, correct on every OS. Best effort: any failure leaves the
/// config default (1440x960).
fn size_main_window_to_screen(app: &tauri::App) {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let area_w = area.size.width as f64 / scale;
    let area_h = area.size.height as f64 / scale;
    let area_x = area.position.x as f64 / scale;
    let area_y = area.position.y as f64 / scale;

    let width = (area_w * 0.94).clamp(1024.0, area_w);
    let height = (area_h * 0.94).clamp(720.0, area_h);
    let _ = window.set_size(tauri::LogicalSize::new(width, height));
    // Center horizontally; sit a bit above vertical center (smaller top gap).
    let _ = window.set_position(tauri::LogicalPosition::new(
        area_x + (area_w - width) / 2.0,
        area_y + (area_h - height) * 0.35,
    ));
}

/// Notify the frontend that a Thread's "上一轮变更" changeset has updated. The
/// frontend bridges this to its typed event bus (§6.1, C1).
pub(crate) fn emit_review_updated(thread_id: &str) {
    if let Some(handle) = APP_HANDLE.get() {
        use tauri::Emitter;
        let _ = handle.emit("review-updated", thread_id.to_string());
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let _ = APP_HANDLE.set(app.handle().clone());
            size_main_window_to_screen(app);
            if let Err(error) = store::initialize_app_store() {
                eprintln!("FutureOS store initialization failed: {error}");
            }
            // Start the bundled agent off the launch path — it does a blocking
            // TCP probe and we don't want to delay the window. In dev (no
            // sidecar binary) this no-ops and the user runs the agent manually.
            let agent_handle = app.handle().clone();
            std::thread::spawn(move || agent_supervisor::ensure_agent_running(&agent_handle));
            // Shadow-review maintenance (consistency check + crash recovery) runs
            // off the launch path so it never delays the window.
            std::thread::spawn(shadow_review::run_startup_maintenance);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            app_data_path,
            open_path,
            read_text_file_preview,
            inspect_attachment,
            read_file_base64,
            write_thumbnail,
            delete_temp_attachment,
            export_artifact_file,
            initialize_app_store,
            cancel_stale_approval_requests,
            get_app_settings,
            update_app_settings,
            clear_app_data,
            get_future_environment,
            set_future_environment,
            list_agent_providers,
            upsert_custom_provider,
            delete_custom_provider,
            start_future_login,
            poll_future_login,
            logout_future_provider,
            clear_finished_runs,
            list_threads,
            list_workspaces,
            create_workspace,
            ensure_workspace_git,
            save_pasted_image,
            get_or_create_chat_workspace,
            get_thread,
            get_recent_thread,
            create_thread,
            rename_thread,
            update_thread_model,
            update_thread_thinking_level,
            pin_thread,
            archive_thread,
            restore_thread,
            delete_thread,
            get_thread_cleanup_summary,
            list_messages,
            append_message,
            create_run,
            list_runs,
            update_run_status,
            abort_run,
            list_run_events,
            list_tool_calls,
            list_tool_outputs,
            list_approval_requests,
            decide_approval_request,
            get_git_review,
            get_workspace_review_capabilities,
            get_last_run_review,
            retry_run_review,
            list_artifacts,
            create_artifact,
            import_attachment_artifact,
            delete_artifact,
            promote_artifact_to_research,
            list_research_resources,
            resolve_markdown_references,
            search_reference_targets,
            list_agent_models,
            agent_prompt,
            list_installed_skills,
            list_available_skills,
            install_skill,
            uninstall_skill
        ])
        .build(tauri::generate_context!())
        .expect("error while running FutureOS")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                agent_supervisor::shutdown_agent();
            }
        });
}
