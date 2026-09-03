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
#[cfg(target_os = "linux")]
mod linux_power;
#[cfg(target_os = "macos")]
mod macos_power;
#[cfg(target_os = "macos")]
mod menu;
mod proc;
mod remote;
mod run_error;
mod scheduler;
mod shadow_review;
mod skills;
mod skills_bootstrap;
mod store;
#[cfg(target_os = "windows")]
mod windows_power;

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
fn size_main_window_to_screen<R: tauri::Runtime>(app: &tauri::App<R>) {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    apply_main_window_geometry(
        &window,
        monitor.scale_factor(),
        monitor.work_area().size.width as f64,
        monitor.work_area().size.height as f64,
        monitor.work_area().position.x as f64,
        monitor.work_area().position.y as f64,
    );
}

/// Size + position the window from a monitor's scale factor and work area.
/// Extracted so the geometry application is testable with a mock window (the
/// monitor lookup itself needs a real display server).
fn apply_main_window_geometry<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    scale: f64,
    area_width: f64,
    area_height: f64,
    area_x: f64,
    area_y: f64,
) {
    let (width, height, x, y) =
        main_window_geometry(scale, area_width, area_height, area_x, area_y);
    let _ = window.set_size(tauri::LogicalSize::new(width, height));
    // Center horizontally; sit a bit above vertical center (smaller top gap).
    let _ = window.set_position(tauri::LogicalPosition::new(x, y));
}

/// Compute the main-window size (near-fullscreen, clamped) and centered position
/// — both in logical units — from a monitor's scale factor and physical work
/// area. Extracted so the geometry is unit-testable without a live
/// window/monitor.
fn main_window_geometry(
    scale: f64,
    area_w: f64,
    area_h: f64,
    area_x: f64,
    area_y: f64,
) -> (f64, f64, f64, f64) {
    let area_w = area_w / scale;
    let area_h = area_h / scale;
    let area_x = area_x / scale;
    let area_y = area_y / scale;

    let width = (area_w * 0.94).clamp(1024.0, area_w);
    let height = (area_h * 0.94).clamp(720.0, area_h);
    let x = area_x + (area_w - width) / 2.0;
    let y = area_y + (area_h - height) * 0.35;
    (width, height, x, y)
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
            if best.is_none_or(|(_, _, bs)| score < bs) {
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
        if let Some((offset, size)) = find_best_entry(ico_data, big_target) {
            if let Some(hicon) = hicon_from_ico_entry(ico_data, offset, size) {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_BIG as _)),
                    Some(LPARAM(hicon.0 as _)),
                );
            }
        }
        if let Some((offset, size)) = find_best_entry(ico_data, small_target) {
            if let Some(hicon) = hicon_from_ico_entry(ico_data, offset, size) {
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

/// Notify the frontend that a Thread's "last-run changes" changeset has updated. The
/// frontend bridges this to its typed event bus (§6.1, C1).
pub(crate) fn emit_review_updated(thread_id: &str) {
    emit_review_updated_via(APP_HANDLE.get(), thread_id);
}

/// Route an emit through an optional handle (see [`emit_review_updated_on`]).
/// Extracted so the process-global `APP_HANDLE` Some/None arms are testable
/// with a mock handle.
fn emit_review_updated_via<R: tauri::Runtime>(
    handle: Option<&tauri::AppHandle<R>>,
    thread_id: &str,
) {
    if let Some(handle) = handle {
        emit_review_updated_on(handle, thread_id);
    }
}

/// Emit the "review-updated" event on a caller-supplied handle. Extracted so the
/// `.emit()` body is testable with a mock handle (the process-global
/// `APP_HANDLE` is only populated when the real app runs).
fn emit_review_updated_on<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, thread_id: &str) {
    use tauri::Emitter;
    let _ = handle.emit("review-updated", thread_id.to_string());
}

/// Notify the frontend that a remote (phone) client created/drove a thread, so
/// the thread list + runs refresh and the conversation shows up live.
pub(crate) fn emit_remote_activity(thread_id: &str) {
    emit_remote_activity_via(APP_HANDLE.get(), thread_id);
}

/// Route an emit through an optional handle (see [`emit_remote_activity_on`]).
fn emit_remote_activity_via<R: tauri::Runtime>(
    handle: Option<&tauri::AppHandle<R>>,
    thread_id: &str,
) {
    if let Some(handle) = handle {
        emit_remote_activity_on(handle, thread_id);
    }
}

/// Emit the "remote-activity" event on a caller-supplied handle (see
/// [`emit_review_updated_on`]).
fn emit_remote_activity_on<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, thread_id: &str) {
    use tauri::Emitter;
    let _ = handle.emit("remote-activity", thread_id.to_string());
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

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadRuntimeUpdateBatch {
    updates: Vec<ThreadRuntimeUpdate>,
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
                // A terminal status is final for its run, but the abort path
                // emits it (run-row CAS) while the collector is still draining
                // the stream — so the trailing "finalizing" push can carry a
                // HIGHER revision. A plain max-revision dedup would keep
                // "finalizing" and silently drop the terminal push, leaving
                // the sidebar spinning until the next reconciliation pass.
                // Terminal beats non-terminal regardless of revision; between
                // equal terminality the newer revision wins as before.
                let next_terminal = is_terminal_run_status(&next.status);
                let current_terminal = is_terminal_run_status(&current.status);
                let next_wins = (next_terminal && !current_terminal)
                    || (next_terminal == current_terminal && next.revision > current.revision);
                if next_wins {
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

/// Run statuses after which a run can never produce work again. Must match the
/// reducer's terminal set in `useThreadStore.reduceThreadRunStatus`.
fn is_terminal_run_status(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled")
}

/// Coalesce token-heavy run updates into one UI batch roughly once per display
/// frame. Persistence remains authoritative and complete; this channel carries
/// only low-latency projection invalidations.
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
            .spawn(|| runtime_update_drain_loop(rx))
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

/// Notify the webview that the approval queue changed: a new pending request
/// was persisted or a decision was recorded. Emitted from the store write
/// sites, so the signal never races the row it announces. Approval changes
/// are rare next to run events, so this goes out directly instead of through
/// the 40ms coalescing channel — the composer card and the sidebar badge
/// react as soon as the write lands, and polling is only a backstop.
pub(crate) fn emit_approvals_updated(thread_id: &str, approval_request_id: &str) {
    emit_approvals_updated_via(APP_HANDLE.get(), thread_id, approval_request_id);
}

/// Route an emit through an optional handle (see [`emit_approvals_updated_on`]).
fn emit_approvals_updated_via<R: tauri::Runtime>(
    handle: Option<&tauri::AppHandle<R>>,
    thread_id: &str,
    approval_request_id: &str,
) {
    if let Some(handle) = handle {
        emit_approvals_updated_on(handle, thread_id, approval_request_id);
    }
}

/// Emit the "approvals-updated" event on a caller-supplied handle (see
/// [`emit_review_updated_on`]).
fn emit_approvals_updated_on<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    thread_id: &str,
    approval_request_id: &str,
) {
    use tauri::Emitter;
    let _ = handle.emit(
        "approvals-updated",
        ApprovalsUpdate {
            thread_id: thread_id.to_string(),
            approval_request_id: approval_request_id.to_string(),
        },
    );
}

/// Emit one coalesced "thread-runtime-updated" batch on a caller-supplied
/// handle (see [`emit_review_updated_on`]). One Tauri event crosses the WebView
/// boundary regardless of how many runs changed during the frame.
fn emit_runtime_updates_on<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    updates: Vec<ThreadRuntimeUpdate>,
) {
    use tauri::Emitter;
    let _ = handle.emit(
        "thread-runtime-updated",
        ThreadRuntimeUpdateBatch { updates },
    );
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalsUpdate {
    thread_id: String,
    approval_request_id: String,
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
    tauri::async_runtime::spawn(thread_streaming_monitor_loop());
}

/// Poll the agent's streaming thread ids every second. Extracted so the loop
/// body (sample + sleep) is testable; the `APP_HANDLE` Some arm needs the
/// real Wry handle and stays untestable.
async fn thread_streaming_monitor_loop() {
    let mut previous: Option<Vec<String>> = None;
    loop {
        if let Some(handle) = APP_HANDLE.get() {
            sample_thread_streaming(handle, &mut previous).await;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// Drain the runtime-update channel: coalesce bursts within a 16ms frame window,
/// then emit the coalesced update. Extracted so the drain/coalesce/emit body
/// is testable; the `APP_HANDLE` Some arm needs the real Wry handle.
fn runtime_update_drain_loop(rx: std::sync::mpsc::Receiver<ThreadRuntimeUpdate>) {
    use std::time::{Duration, Instant};
    while let Ok(first) = rx.recv() {
        let deadline = Instant::now() + Duration::from_millis(16);
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
            emit_runtime_updates_on(handle, coalesce_runtime_updates(first, rest));
        }
    }
}

/// Sample the agent's streaming thread ids once and, if the set changed, emit a
/// "thread-streaming-updated" notification on the given handle. Extracted so the
/// change-detection + emit body is testable without the process-global
/// `APP_HANDLE` or a real agent.
async fn sample_thread_streaming<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    previous: &mut Option<Vec<String>>,
) {
    sample_thread_streaming_with(handle, previous, list_streaming_thread_ids).await;
}

/// The injectable core of [`sample_thread_streaming`]: fetch the current
/// streaming thread ids, then — if the set changed — emit a
/// "thread-streaming-updated" notification on the given handle. Extracted so the
/// change-detection + emit body is testable with a deterministic id source.
async fn sample_thread_streaming_with<R, F, Fut>(
    handle: &tauri::AppHandle<R>,
    previous: &mut Option<Vec<String>>,
    fetch: F,
) where
    R: tauri::Runtime,
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<String>, crate::AppError>>,
{
    let mut thread_ids = fetch().await.unwrap_or_default();
    thread_ids.sort_unstable();
    thread_ids.dedup();
    if previous.as_ref() != Some(&thread_ids) {
        *previous = Some(thread_ids.clone());
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // `reqwest` intentionally uses `rustls-no-provider` so it shares the
    // `ring` backend selected by async-nats/Tauri instead of pulling aws-lc
    // into the same process. Rustls requires the application to select that
    // backend before the first HTTP client is built.
    install_rustls_provider();
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
            #[cfg(target_os = "macos")]
            macos_power::install_disconnect_notifier();
            #[cfg(target_os = "windows")]
            windows_power::install_disconnect_notifier(app.handle());
            #[cfg(target_os = "linux")]
            linux_power::install_disconnect_notifier();
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
            // Independent fixed-interval maintenance: app updates (24h),
            // Future balance (1h), and Future models (24h). Missed ticks while
            // suspended are skipped; each task runs at most once after resume.
            scheduler::start(app.handle().clone());
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
                    agent_bridge::import_missing_sessions().await;
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
            // Global provider/auth completion stream. This is independent of
            // chat observers and fans committed revisions to WebView + Mobile.
            agent_bridge::spawn_provider_config_observer();
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
                    // Delete threads whose agent session was removed externally
                    // (TUI/CLI/manual) — detected via the agent's session list,
                    // never file probing. Runs first so run reconciliation below
                    // doesn't query sessions that no longer exist. Self-gates on
                    // agent reachability (skips when the agent is down).
                    if let Err(error) = store::reconcile_orphan_sessions().await {
                        eprintln!("FutureOS orphan-session reconcile failed: {error}");
                    }
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
            prepare_image_preview,
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
            update_builtin_provider,
            set_builtin_provider_base_url,
            delete_custom_provider,
            start_future_login,
            poll_future_login,
            logout_future_provider,
            get_future_profile,
            get_future_balance,
            archive_finished_runs,
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
            mark_thread_opened,
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
            compact_thread_context,
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
            save_approval_rules,
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
            probe_sandbox,
            probe_windows_sandbox,
            reset_windows_sandbox,
            agent_prompt,
            list_installed_skills,
            list_available_skills,
            get_skill_guide,
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
            // `setup` runs while Tauri is still constructing its event loop.
            // Starting the bridge there races the runtime initialization on
            // macOS and can leave the detached start task without a live
            // connection. `Ready` is emitted once the process runtime is live.
            tauri::RunEvent::Ready => spawn_remote_auto_connect(),
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
                // A normal app/window exit still has a live runtime. Flush a
                // short disconnect notice so the phone disables sending at
                // once instead of waiting for heartbeat expiry. Crashes and
                // power loss cannot run this handler and remain timeout-based.
                tauri::async_runtime::block_on(remote::stop_gracefully("app_exit"));
                agent_supervisor::shutdown_agent_gracefully();
            }
            _ => {}
        });
}

fn install_rustls_provider() {
    // A test or an embedding host may have already chosen this same provider;
    // in that case `install_default` returns an error and no action is needed.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Start the persisted remote bridge only after Tauri's runtime has entered its
/// event loop. Calling this from `setup` is too early on macOS: the detached
/// task can be scheduled before the process runtime is ready and never obtain
/// a lasting NATS connection.
fn spawn_remote_auto_connect() {
    // Pairing codes are issued by the FutureOS service, so auto-connect needs a
    // sign-in even when the feature is enabled and credentials are persisted.
    let enabled = future_login::future_api_key().is_ok()
        && store::get_app_settings()
            .map(|settings| settings.auto_connect_remote)
            .unwrap_or(false)
        && remote::pairing::load_creds().is_some();
    if !enabled {
        return;
    }

    tauri::async_runtime::spawn(async {
        match remote::start(remote::RemoteStartInput {}).await {
            Ok(status) if matches!(status.phase, remote::RemotePhase::Ready) => {
                eprintln!("FutureOS remote auto-connect: connected")
            }
            Ok(status) => eprintln!(
                "FutureOS remote auto-connect: deferred ({:?})",
                status.reason
            ),
            Err(error) => eprintln!("FutureOS remote auto-connect failed: {error}"),
        }
    });
}

#[cfg(test)]
mod runtime_update_tests {
    use super::{
        apply_main_window_geometry, coalesce_runtime_updates, emit_approvals_updated_on,
        emit_approvals_updated_via, emit_remote_activity_on, emit_remote_activity_via,
        emit_review_updated_on, emit_review_updated_via, emit_runtime_updates_on,
        main_window_geometry, runtime_update_drain_loop, sample_thread_streaming,
        sample_thread_streaming_with, size_main_window_to_screen, thread_streaming_monitor_loop,
        ThreadRuntimeUpdate, ThreadRuntimeUpdateBatch,
    };

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

    #[test]
    fn coalescing_keeps_terminal_over_later_non_terminal_push() {
        // Abort race: the run-row CAS emits the terminal push while the
        // collector is still draining, so its trailing "finalizing" carries a
        // higher revision. The terminal push must survive the dedup — dropping
        // it strands the sidebar spinner until the next reconciliation pass.
        let updates = coalesce_runtime_updates(
            update("run-1", 1, "cancelled", false),
            [update("run-1", 2, "finalizing", false)],
        );

        assert_eq!(updates, vec![update("run-1", 1, "cancelled", false)]);
    }

    #[test]
    fn coalescing_keeps_newer_revision_among_non_terminal_pushes() {
        let updates = coalesce_runtime_updates(
            update("run-1", 1, "running", false),
            [update("run-1", 2, "finalizing", false)],
        );

        assert_eq!(updates, vec![update("run-1", 2, "finalizing", false)]);
    }

    #[test]
    fn runtime_update_batch_serializes_the_frontend_contract() {
        let value = serde_json::to_value(ThreadRuntimeUpdateBatch {
            updates: vec![update("run-1", 7, "running", true)],
        })
        .expect("serialize runtime batch");

        assert_eq!(value["updates"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["updates"][0]["runId"], "run-1");
        assert_eq!(value["updates"][0]["resetProjection"], true);
    }

    #[test]
    fn main_window_geometry_scales_clamps_and_centers() {
        // 2x scale → logical work area is 2000x1000 at (50, 25).
        let (width, height, x, y) = main_window_geometry(2.0, 4000.0, 2000.0, 100.0, 50.0);
        assert_eq!(width, 1880.0);
        assert_eq!(height, 940.0);
        assert_eq!(x, 50.0 + (2000.0 - 1880.0) / 2.0);
        assert_eq!(y, 25.0 + (1000.0 - 940.0) * 0.35);
    }

    #[test]
    fn main_window_geometry_clamps_to_minimums() {
        // 0.94×1050 = 987 < 1024 → clamps to the 1024 floor; same for height.
        let (width, height, _, _) = main_window_geometry(1.0, 1050.0, 730.0, 0.0, 0.0);
        assert_eq!(width, 1024.0);
        assert_eq!(height, 720.0);
    }

    #[test]
    fn size_main_window_returns_without_main_window() {
        // `mock_app` has no "main" window, so this takes the early-return path.
        let app = tauri::test::mock_app();
        size_main_window_to_screen(&app);
    }

    #[test]
    fn size_main_window_returns_without_monitor() {
        // A mock "main" window has no monitor, so the monitor lookup fails and
        // the function returns before sizing/positioning.
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app");
        let _window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("build webview");
        size_main_window_to_screen(&app);
    }

    #[test]
    fn emit_helpers_emit_on_mock_handle() {
        let app = tauri::test::mock_app();
        let handle = app.handle();
        emit_review_updated_on(handle, "thread-1");
        emit_remote_activity_on(handle, "thread-1");
        emit_approvals_updated_on(handle, "thread-1", "approval-1");
        emit_runtime_updates_on(
            handle,
            vec![
                update("run-1", 1, "running", false),
                update("run-1", 2, "completed", false),
            ],
        );
    }

    #[tokio::test]
    async fn sample_thread_streaming_emits_only_on_change() {
        let app = tauri::test::mock_app();
        let handle = app.handle();
        let mut previous: Option<Vec<String>> = None;
        // First sample emits and records the new set.
        sample_thread_streaming_with(handle, &mut previous, || async {
            Ok(vec!["a".to_string(), "a".to_string(), "b".to_string()])
        })
        .await;
        assert_eq!(previous, Some(vec!["a".to_string(), "b".to_string()]));
        // Unchanged sample → no emit, `previous` keeps the same value.
        sample_thread_streaming_with(handle, &mut previous, || async {
            Ok(vec!["b".to_string(), "a".to_string()])
        })
        .await;
        assert_eq!(previous, Some(vec!["a".to_string(), "b".to_string()]));
        // Changed sample → emit + record the new set.
        sample_thread_streaming_with(handle, &mut previous, || async {
            Ok(vec!["c".to_string()])
        })
        .await;
        assert_eq!(previous, Some(vec!["c".to_string()]));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn sample_thread_streaming_wrapper_delegates_to_real_id_source() {
        // The thin wrapper feeds the real `list_streaming_thread_ids` (which
        // hits the agent) into `sample_thread_streaming_with`. With the mock
        // agent down (health check still up, streaming list empty) it records
        // an empty sample without panicking — exercising the wrapper's own
        // delegation line rather than the injectable core.
        let _lock = crate::commands::agent_mock::mock_agent_lock();
        crate::commands::agent_mock::ensure_mock_agent();
        crate::commands::agent_mock::script_mock_agent(crate::commands::agent_mock::MockScript {
            down: true,
            ..Default::default()
        });
        let app = tauri::test::mock_app();
        let handle = app.handle();
        let mut previous: Option<Vec<String>> = None;
        sample_thread_streaming(handle, &mut previous).await;
        assert_eq!(previous, Some(Vec::new()));
        crate::commands::agent_mock::script_mock_agent(Default::default());
    }

    #[test]
    fn apply_main_window_geometry_sizes_and_positions() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app");
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("build webview");
        // Mock windows accept the set_* calls without a display server.
        apply_main_window_geometry(&window, 2.0, 4000.0, 2000.0, 100.0, 50.0);
    }

    #[test]
    fn emit_via_helpers_route_both_arms() {
        let app = tauri::test::mock_app();
        let handle = app.handle();
        emit_review_updated_via(Some(handle), "thread-1");
        emit_remote_activity_via(Some(handle), "thread-1");
        emit_approvals_updated_via(Some(handle), "thread-1", "approval-1");
        // None arm: process-global APP_HANDLE unset in tests.
        emit_review_updated_via::<tauri::test::MockRuntime>(None, "thread-1");
        emit_remote_activity_via::<tauri::test::MockRuntime>(None, "thread-1");
        emit_approvals_updated_via::<tauri::test::MockRuntime>(None, "thread-1", "approval-1");
    }

    #[test]
    fn runtime_update_drain_loop_coalesces_and_exits() {
        let (tx, rx) = std::sync::mpsc::channel::<ThreadRuntimeUpdate>();
        let handle = std::thread::spawn(|| runtime_update_drain_loop(rx));
        // One burst: both messages coalesce into a single emit attempt (the
        // APP_HANDLE arm is skipped — no handle in tests — but the drain body
        // runs end to end).
        tx.send(update("run-1", 1, "running", false)).unwrap();
        tx.send(update("run-2", 2, "running", false)).unwrap();
        drop(tx);
        handle.join().expect("drain loop exits on disconnect");
    }

    #[tokio::test]
    async fn streaming_monitor_loop_enters_and_sleeps() {
        // One iteration, then abort — the loop is infinite by design; this
        // proves the iteration body (APP_HANDLE check + sleep) executes.
        let task = tokio::spawn(thread_streaming_monitor_loop());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        task.abort();
        let _ = task.await;
    }
}

#[cfg(test)]
mod startup_paths_tests {
    use super::*;

    #[test]
    fn start_thread_streaming_monitor_spawns_the_loop() {
        // The spawned loop parks (APP_HANDLE is unset in tests) and sleeps; this
        // exercises the spawn wrapper itself. It is harmless — no handle, no
        // sample, just an idle poll on a process-lifetime runtime.
        start_thread_streaming_monitor();
    }

    #[test]
    fn spawn_remote_auto_connect_returns_when_no_creds() {
        // API key present + auto-connect enabled + no persisted pairing creds →
        // the enabled gate evaluates fully (load_creds is None) and returns
        // without spawning the bridge.
        let _home = crate::auth_store::test_support::HomeGuard::new("remote-auto-connect");
        crate::store::initialize_app_store().expect("init store");
        crate::auth_store::set_future_login("sekret", "https://future-os.cn/api").unwrap();
        crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
            auto_connect_remote: Some(true),
            ..Default::default()
        })
        .unwrap();
        spawn_remote_auto_connect();
    }
}
