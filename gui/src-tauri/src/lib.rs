mod agent_bridge;
mod agent_proto;
mod agent_providers;
mod agent_supervisor;
mod approval_rules;
mod auth_store;
mod build_info;
mod commands;
mod config_io;
mod error;
mod future_login;
mod future_platform;
mod git_diff_parse;
mod git_review;
#[cfg(target_os = "macos")]
mod menu;
mod proc;
mod remote;
mod run_error;
mod shadow_review;
mod skills;
mod skills_bootstrap;
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

/// Notify the frontend that a Thread's "previous turn changes" changeset has updated. The
/// frontend bridges this to its typed event bus (§6.1, C1).
pub(crate) fn emit_review_updated(thread_id: &str) {
    if let Some(handle) = APP_HANDLE.get() {
        use tauri::Emitter;
        let _ = handle.emit("review-updated", thread_id.to_string());
    }
}

/// Notify the frontend that a remote (phone) client created/drove a thread, so
/// the thread list + runs refresh and the conversation shows up live.
pub(crate) fn emit_remote_activity(thread_id: &str) {
    if let Some(handle) = APP_HANDLE.get() {
        use tauri::Emitter;
        let _ = handle.emit("remote-activity", thread_id.to_string());
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .on_menu_event(|app, event| {
            #[cfg(target_os = "macos")]
            {
                use tauri::{Emitter, Manager};
                match event.id().as_ref() {
                    menu::MENU_ABOUT => {
                        // No native About dialog — open the in-app Settings page.
                        let _ = app.emit("open-settings", ());
                    }
                    menu::MENU_RESTART_WEBVIEW => {
                        // Debug escape hatch: reload a hung/crashed webview in
                        // place (native reload, so it recovers even when the JS
                        // context is dead) instead of relaunching the app.
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.reload();
                        }
                    }
                    _ => {}
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        })
        .on_window_event(|window, event| {
            // Guard quit: if a conversation is still generating, warn before we
            // tear the agent down. The confirmation is a native dialog (see
            // agent_supervisor) so it survives even a hung webview.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                use tauri::Manager;
                match agent_supervisor::on_close_requested() {
                    agent_supervisor::QuitDecision::Proceed => {}
                    agent_supervisor::QuitDecision::Confirm { open_dialog } => {
                        api.prevent_close();
                        if open_dialog {
                            agent_supervisor::confirm_quit(window.app_handle().clone());
                        }
                    }
                }
            }
        })
        .setup(|app| {
            let _ = APP_HANDLE.set(app.handle().clone());
            // Replace Tauri's default macOS menu so the brand name always reads
            // "FutureOS" (the default falls back to the lowercase executable name
            // in dev/unbundled runs) and to add the About/Restart Webview items.
            #[cfg(target_os = "macos")]
            if let Err(error) =
                menu::build_macos_menu(app.handle()).and_then(|m| app.set_menu(m).map(|_| ()))
            {
                eprintln!("FutureOS menu setup failed: {error}");
            }
            size_main_window_to_screen(app);
            if let Err(error) = store::initialize_app_store() {
                eprintln!("FutureOS store initialization failed: {error}");
            }
            // Startup convergence for interrupted runs. Runs exactly once per
            // *process*: its correctness argument ("a fresh process has no live
            // event collector, so every non-terminal run is an orphan") does
            // not hold for a webview reload, so it must not be reachable from
            // the frontend — a reload-triggered call would cancel a live run
            // whose collector survived in this process.
            if let Err(error) = store::cancel_stale_approval_requests() {
                eprintln!("FutureOS run convergence failed: {error}");
            }
            // Import sessions created outside the GUI (TUI, channels, another
            // machine). Runs off the launch path — failures are logged but the
            // UI renders immediately. The store must be initialized first.
            std::thread::spawn(|| {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                rt.block_on(async {
                    if let Err(error) = agent_bridge::import_missing_sessions().await {
                        eprintln!("FutureOS session import failed: {error}");
                    }
                });
            });
            // Pin the FutureGene environment for this build channel before the
            // agent starts: release builds are production-locked, dev builds
            // default to the test environment on first launch. The agent reads
            // base_url from auth.json once at startup, so this must run first.
            if let Err(error) = future_platform::apply_channel_environment_default() {
                eprintln!("FutureOS environment policy failed: {error}");
            }
            // First-launch built-in skill install, off the launch path. Runs
            // after the environment pin above so the bundled CLI resolves the
            // same platform URL. Fully silent — a one-shot marker gates it and
            // every failure is logged, never surfaced.
            let skills_handle = app.handle().clone();
            std::thread::spawn(move || skills_bootstrap::ensure_builtin_skills(&skills_handle));
            // Start the bundled agent off the launch path — it does a blocking
            // TCP probe and we don't want to delay the window. In dev (no
            // sidecar binary) this no-ops and the user runs the agent manually.
            let agent_handle = app.handle().clone();
            std::thread::spawn(move || agent_supervisor::ensure_agent_running(&agent_handle));
            // After the agent has had time to start, reanimate any runs that
            // were cancelled by convergence but whose agent sessions are still
            // streaming (the agent survived a GUI crash). Spawned off the launch
            // path so it never delays the window.
            std::thread::spawn(|| {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                rt.block_on(async {
                    // Give the agent a few seconds to come up; then test.
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    agent_bridge::reconcile_interrupted_runs().await;
                });
            });
            // Shadow-review maintenance (consistency check + crash recovery) runs
            // off the launch path so it never delays the window.
            std::thread::spawn(shadow_review::run_startup_maintenance);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_build_info,
            check_app_update,
            download_app_update,
            open_path,
            list_directory,
            open_external_url,
            resolve_preview_link_path,
            read_text_file_preview,
            inspect_attachment,
            validate_image_attachment,
            read_file_base64,
            generate_image_thumbnail,
            import_ephemeral_image,
            delete_temp_attachment,
            export_artifact_file,
            initialize_app_store,
            get_app_settings,
            update_app_settings,
            clear_app_data,
            get_future_environment,
            set_future_environment,
            list_agent_providers,
            upsert_custom_provider,
            update_builtin_provider_key,
            set_builtin_provider_base_url,
            delete_custom_provider,
            start_future_login,
            poll_future_login,
            logout_future_provider,
            get_future_profile,
            clear_finished_runs,
            list_threads,
            list_workspaces,
            create_workspace,
            rename_workspace,
            delete_workspace,
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
            fork_thread,
            get_session_entries,
            get_thread_agent_state,
            get_thread_cleanup_summary,
            list_messages,
            append_message,
            create_run,
            list_runs,
            update_run_status,
            abort_run,
            list_run_events,
            list_run_events_bulk,
            list_tool_calls,
            list_tool_outputs,
            list_approval_requests,
            decide_approval_request,
            save_approval_rule,
            get_git_review,
            get_workspace_review_capabilities,
            get_last_run_review,
            retry_run_review,
            list_artifacts,
            create_artifact,
            import_attachment_artifact,
            delete_artifact,
            resolve_markdown_references,
            search_workspace_files,
            list_agent_models,
            agent_prompt,
            list_installed_skills,
            list_available_skills,
            install_skill,
            uninstall_skill,
            remote_start,
            remote_stop,
            remote_status,
            open_url
        ])
        .build(tauri::generate_context!())
        .expect("error while running FutureOS")
        .run(|app_handle, event| match event {
            // ⌘Q / the menu's "Quit FutureOS" / a programmatic `app.exit()` come
            // through here, NOT the window's `CloseRequested`. Guard them the same
            // way so a running conversation can't be torn down without warning.
            tauri::RunEvent::ExitRequested { api, .. } => {
                match agent_supervisor::on_close_requested() {
                    agent_supervisor::QuitDecision::Proceed => {}
                    agent_supervisor::QuitDecision::Confirm { open_dialog } => {
                        api.prevent_exit();
                        if open_dialog {
                            agent_supervisor::confirm_quit(app_handle.clone());
                        }
                    }
                }
            }
            tauri::RunEvent::Exit => {
                agent_supervisor::shutdown_agent();
            }
            _ => {}
        });
}
