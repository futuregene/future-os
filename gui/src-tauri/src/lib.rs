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

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadRuntimeUpdate {
    pub thread_id: String,
    pub run_id: String,
    pub revision: i64,
    pub status: String,
    pub reset_projection: bool,
}

static NEXT_RUNTIME_REVISION: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
static RUNTIME_EMIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn next_runtime_revision() -> i64 {
    NEXT_RUNTIME_REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn coalesce_runtime_updates(
    first: ThreadRuntimeUpdate,
    rest: impl IntoIterator<Item = ThreadRuntimeUpdate>,
) -> Vec<ThreadRuntimeUpdate> {
    use std::collections::HashMap;

    let mut pending = HashMap::from([(first.run_id.clone(), first)]);
    for mut next in rest {
        match pending.entry(next.run_id.clone()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(next);
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let current = entry.get_mut();
                let reset_projection = current.reset_projection || next.reset_projection;
                if next.revision > current.revision {
                    next.reset_projection = reset_projection;
                    *current = next;
                } else {
                    current.reset_projection = reset_projection;
                }
            }
        }
    }
    let mut updates: Vec<_> = pending.into_values().collect();
    // Different runs from the same thread can settle/start inside one batch.
    // HashMap iteration order is undefined, so preserve the process-global
    // revision order before the frontend reduces them into thread-level state.
    updates.sort_unstable_by_key(|update| update.revision);
    updates
}

/// Coalesce token-heavy run updates into a single UI notification per run
/// roughly every 40ms. Persistence remains event-by-event and authoritative;
/// this channel is only a low-latency projection invalidation signal.
///
/// `revision` is assigned here from one process-global monotonic sequence.
/// Callers must not mix event cursors and wall-clock values into the UI ordering
/// contract; event-log cursors remain internal to the projection reader.
pub(crate) fn emit_thread_runtime_updated(
    thread_id: String,
    run_id: String,
    status: String,
    reset_projection: bool,
) {
    static TX: std::sync::OnceLock<std::sync::mpsc::Sender<ThreadRuntimeUpdate>> =
        std::sync::OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<ThreadRuntimeUpdate>();
        std::thread::Builder::new()
            .name("thread-runtime-updates".to_string())
            .spawn(move || {
                use std::time::{Duration, Instant};

                while let Ok(first) = rx.recv() {
                    let deadline = Instant::now() + Duration::from_millis(40);
                    let mut rest = Vec::new();
                    loop {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            break;
                        }
                        match rx.recv_timeout(remaining) {
                            Ok(next) => rest.push(next),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    if let Some(handle) = APP_HANDLE.get() {
                        use tauri::Emitter;
                        for update in coalesce_runtime_updates(first, rest) {
                            let _ = handle.emit("thread-runtime-updated", update);
                        }
                    }
                }
            })
            .expect("spawn thread runtime update emitter");
        tx
    });
    // Couple revision allocation to channel insertion. Without this short lock,
    // two producer threads could allocate revisions in one order but enqueue in
    // the opposite order, potentially placing a reset instruction behind an
    // update that the frontend had already accepted.
    let _emit_guard = RUNTIME_EMIT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = tx.send(ThreadRuntimeUpdate {
        thread_id,
        run_id,
        revision: next_runtime_revision(),
        status,
        reset_projection,
    });
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadStreamingUpdate {
    revision: i64,
    thread_ids: Vec<String>,
}

/// Bridge the Agent's compatibility-only `is_streaming` projection into a
/// desktop push signal. React performs one initial snapshot read and then
/// consumes only deltas from this monitor, avoiding a permanent sidebar IPC /
/// render timer.
///
/// Long term, replace this single process-level sampler with a global Agent
/// runtime subscription once the gRPC protocol exposes one. Until then this is
/// the only source that can see runs started by older TUI/CLI clients which do
/// not create a GUI StoredRun or route through the Tauri collector.
fn start_thread_streaming_monitor() {
    tauri::async_runtime::spawn(async move {
        let mut previous: Option<Vec<String>> = None;
        loop {
            let mut thread_ids = list_streaming_thread_ids().await.unwrap_or_default();
            thread_ids.sort_unstable();
            thread_ids.dedup();
            if previous.as_ref() != Some(&thread_ids) {
                previous = Some(thread_ids.clone());
                if let Some(handle) = APP_HANDLE.get() {
                    use tauri::Emitter;
                    let _ = handle.emit(
                        "thread-streaming-updated",
                        ThreadStreamingUpdate {
                            revision: next_runtime_revision(),
                            thread_ids,
                        },
                    );
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second instance was launched — activate the existing window.
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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
                        // Reload a hung/crashed webview in place (native reload,
                        // so it recovers even when the JS context is dead)
                        // instead of relaunching the app.
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
            // Do not preemptively cancel non-terminal GUI rows at startup. The
            // Agent is authoritative and may have survived a GUI crash; the
            // watchdog below reattaches or settles each row only after it can
            // query that authority.
            // Import sessions created outside the GUI (TUI, channels, another
            // machine). Runs off the launch path — failures are logged but the
            // UI renders immediately. The store must be initialized first.
            std::thread::spawn(|| {
                // Single-threaded runtime: `Runtime::new()` is multi_thread and
                // would spawn num_cpus workers for this one-shot task.
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
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
            // Start the bundled agent off the launch path — it does a blocking
            // TCP probe and we don't want to delay the window. In dev (no
            // sidecar binary) this no-ops and the user runs the agent manually.
            let agent_handle = app.handle().clone();
            std::thread::spawn(move || agent_supervisor::ensure_agent_running(&agent_handle));
            start_thread_streaming_monitor();
            // Per-session observers: the always-on tap into every agent
            // session's event stream (settings fan-out, projection of runs no
            // pipeline collector owns, NATS mirroring). Attach/retry happens
            // inside each observer task, so a down agent never blocks startup.
            agent_bridge::seed_observers_from_store();
            // Discovery: conversations created by other clients (TUI/CLI) get
            // a thread stub + an observer — streaming ones within ~1s, idle
            // ones on the 60s import pass.
            agent_bridge::spawn_session_discovery();
            // Continuously flush local deletion tombstones. This is independent
            // of the current UI route and makes offline GUI deletes converge.
            agent_bridge::spawn_delete_outbox_worker();
            // Periodically reconcile non-terminal run rows against the Agent's
            // authoritative state (mirrors terminal markers, reattaches lost
            // collectors, settles orphans). Guards against rows whose owning
            // pipeline never settled them — e.g. a suspended webview that never
            // applied the invoke response. Self-gates on Agent reachability.
            agent_bridge::spawn_active_run_watchdog();
            // After the agent has had time to start, reconcile pending approvals
            // against its authoritative state. Active-run reconciliation itself
            // is handled continuously by the watchdog above.
            std::thread::spawn(|| {
                // Single-threaded runtime: `Runtime::new()` is multi_thread and
                // would spawn num_cpus workers for this one-shot task.
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                rt.block_on(async {
                    // Give the agent a few seconds to come up; then test.
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    // Rows produced by older startup convergence builds are
                    // terminal locally (`cancelled/interrupted`) and therefore
                    // outside the active-run watchdog. Reconcile that legacy
                    // shape once before approvals so an Agent run that survived
                    // the GUI restart is reanimated instead of staying hidden.
                    agent_bridge::reconcile_interrupted_runs().await;
                    agent_bridge::reconcile_pending_approvals().await;
                });
            });
            // Shadow-review maintenance (consistency check + crash recovery) runs
            // off the launch path so it never delays the window.
            std::thread::spawn(shadow_review::run_startup_maintenance);
            // Remote auto-connect: a desktop can pair with exactly one phone, so
            // when the user has opted in (Settings → Remote) and a pairing is
            // already persisted, reconnect to that device on launch — they can
            // still disconnect by hand from the Remote view. Runs off the launch
            // path (NATS connect does network IO) so it never delays the window.
            // Gated to non-release builds to match the Remote nav entry, which is
            // hidden in release builds: autostarting an invisible bridge there
            // would leave the user no way to stop it. The store is initialized
            // above, so the setting read is safe here.
            if !build_info::is_release()
                && store::get_app_settings()
                    .map(|settings| settings.auto_connect_remote)
                    .unwrap_or(false)
                && remote::pairing::load_creds().is_some()
            {
                std::thread::spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("tokio runtime");
                    rt.block_on(async {
                        if let Err(error) = remote::start(remote::RemoteStartInput {}).await {
                            eprintln!("FutureOS remote auto-connect failed: {error}");
                        }
                    });
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_build_info,
            check_app_update,
            install_app_update,
            restart_after_app_update,
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
            get_future_balance,
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
            batch_delete_threads,
            fork_thread,
            get_session_entries,
            get_thread_agent_state,
            list_streaming_thread_ids,
            get_thread_cleanup_summary,
            attach_remote_stream,
            observe_session,
            reconcile_thread_workspace,
            create_run,
            get_latest_run,
            get_run,
            list_runs,
            list_latest_run_infos,
            update_run_status,
            abort_run,
            list_run_events,
            list_run_events_bulk,
            list_run_events_since,
            list_tool_calls,
            list_tool_calls_bulk,
            list_tool_outputs,
            list_approval_requests,
            list_pending_approval_requests,
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
            sync_future_models,
            set_default_model,
            agent_prompt,
            list_installed_skills,
            list_available_skills,
            install_skill,
            uninstall_skill,
            refresh_skills,
            bootstrap_builtin_skills,
            remote_start,
            remote_stop,
            remote_status,
            remote_unpair,
            remote_pairing_status,
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

#[cfg(test)]
mod runtime_update_tests {
    use super::{coalesce_runtime_updates, ThreadRuntimeUpdate};

    fn update(run_id: &str, revision: i64, status: &str, reset: bool) -> ThreadRuntimeUpdate {
        ThreadRuntimeUpdate {
            thread_id: "thread-1".to_string(),
            run_id: run_id.to_string(),
            revision,
            status: status.to_string(),
            reset_projection: reset,
        }
    }

    #[test]
    fn coalescing_preserves_reset_and_cross_run_revision_order() {
        let updates = coalesce_runtime_updates(
            update("run-old", 1, "running", true),
            [
                update("run-new", 2, "running", false),
                update("run-old", 3, "completed", false),
                update("run-new", 4, "completed", false),
                update("run-new", 2, "running", true),
            ],
        );

        assert_eq!(
            updates,
            vec![
                update("run-old", 3, "completed", true),
                update("run-new", 4, "completed", true),
            ]
        );
    }
}
