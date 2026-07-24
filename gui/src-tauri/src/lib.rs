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

/// Set a crisp taskbar icon on Windows by loading the multi-size ICO directly.
///
/// Tauri's `default_window_icon()` creates a single-size HICON from the first
/// PNG in `bundle.icon` and calls `WM_SETICON(ICON_BIG, ...)`. When Windows
/// renders that HICON in the taskbar at a different size (e.g. 40px on a 100%
/// DPI system where SM_CXICON is only 32), GDI's icon scaling is visibly
/// blurry. Instead, we parse the ICO directory, find the entry that matches
/// the size Windows actually needs, and create an HICON from its exact pixel
/// data — no scaling needed.
#[cfg(target_os = "windows")]
fn set_windows_taskbar_icon(app: &tauri::App) {
    use tauri::Manager;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIconFromResourceEx, SendMessageW, ICON_BIG, ICON_SMALL, WM_SETICON,
    };

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let hwnd = HWND(hwnd.0 as _);

    let ico_data = icon_ico_bytes();

    // Parse the ICO directory and pick the best entry for a given target size.
    fn find_best_entry(data: &[u8], target: u32) -> Option<(u32, u32)> {
        if data.len() < 6 {
            return None;
        }
        let count = u16::from_le_bytes([data[4], data[5]]) as usize;
        let mut best: Option<(u32, u32, u32)> = None; // (offset, size, score)
        for i in 0..count {
            let base = 6 + i * 16;
            if base + 16 > data.len() {
                break;
            }
            let w = if data[base] == 0 {
                256u32
            } else {
                data[base] as u32
            };
            let entry_size = u32::from_le_bytes([
                data[base + 8],
                data[base + 9],
                data[base + 10],
                data[base + 11],
            ]);
            let offset = u32::from_le_bytes([
                data[base + 12],
                data[base + 13],
                data[base + 14],
                data[base + 15],
            ]);
            // Score: prefer exact match (0), then larger (w - target), then
            // smaller (2*(target - w) + 1 so any larger beats any smaller).
            let score = if w >= target {
                w - target
            } else {
                (target - w) * 2 + 1
            };
            if best.map_or(true, |(_, _, bs)| score < bs) {
                best = Some((offset, entry_size, score));
            }
        }
        best.map(|(o, s, _)| (o, s))
    }

    // Create an HICON from an ICO entry at the given offset/size.
    unsafe fn hicon_from_ico_entry(
        data: &[u8],
        offset: u32,
        size: u32,
    ) -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
        let start = offset as usize;
        let end = start + size as usize;
        if end > data.len() {
            return None;
        }
        let icon_bits = &data[start..end];
        match CreateIconFromResourceEx(
            icon_bits,
            true,       // fIcon
            0x00030000, // dwVersion
            0,          // cxDesired (0 = use entry's own size)
            0,          // cyDesired
            windows::Win32::UI::WindowsAndMessaging::LR_DEFAULTSIZE,
        ) {
            Ok(hicon) if !hicon.is_invalid() => Some(hicon),
            _ => None,
        }
    }

    // ICON_BIG: used by Alt+Tab and the taskbar.
    let big_target = 256u32;
    // ICON_SMALL: used by the title bar and small taskbar mode.
    let small_target = 128u32;

    unsafe {
        if let Some((offset, size)) = find_best_entry(&ico_data, big_target) {
            if let Some(hicon) = hicon_from_ico_entry(&ico_data, offset, size) {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_BIG as _)),
                    Some(LPARAM(hicon.0 as _)),
                );
            }
        }
        if let Some((offset, size)) = find_best_entry(&ico_data, small_target) {
            if let Some(hicon) = hicon_from_ico_entry(&ico_data, offset, size) {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_SMALL as _)),
                    Some(LPARAM(hicon.0 as _)),
                );
            }
        }
    }
}

/// The multi-size ICO, embedded into the binary at compile time.
///
/// Reading it from disk at runtime would depend on the process working
/// directory, which is unreliable for installed release builds (e.g. launched
/// from the Start menu). Embedding costs ~230KB in the exe but makes the icon
/// setup behave identically in dev, release, and packaged installs.
#[cfg(target_os = "windows")]
fn icon_ico_bytes() -> &'static [u8] {
    include_bytes!("../icons/icon.ico")
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
            // Windows: set a high-quality taskbar icon. Tauri's default path creates
            // a single-size HICON from the first PNG, and GDI's icon scaling is poor
            // when the taskbar needs to render it at a different size. Instead, load
            // the multi-size ICO and let Windows pick the exact match for its display
            // size — the ICO contains 16,20,24,30,32,36,40,48,64,72,96,128,256.
            #[cfg(target_os = "windows")]
            set_windows_taskbar_icon(app);
            // The window is created hidden (`"visible": false` in tauri.conf.json)
            // so the taskbar never flashes Tauri's default (blurry, upscaled) icon
            // before the crisp one above is in place. Reveal it now.
            {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                }
            }
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
            list_streaming_thread_ids,
            get_thread_cleanup_summary,
            attach_remote_stream,
            observe_session,
            reconcile_thread_workspace,
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
            remote_status
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
