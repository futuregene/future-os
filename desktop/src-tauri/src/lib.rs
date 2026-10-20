// UI-only entry points within shared business modules are unused in a server build.
#![cfg_attr(not(feature = "gui"), allow(dead_code))]

#[cfg_attr(not(feature = "gui"), allow(unused_imports))]
mod agent_bridge;
mod agent_host;
#[cfg_attr(not(feature = "gui"), allow(unused_imports))]
mod agent_providers;
#[cfg(feature = "gui")]
mod agent_supervisor;
mod auth_store;
#[cfg(feature = "gui")]
mod build_info;
mod cli_tool;
mod commands;
#[cfg(feature = "gui")]
mod config_io;
mod desktop_shutdown;
mod error;
mod future_login;
mod future_platform;
#[cfg(feature = "gui")]
mod git_review;
mod host_environment;
mod instance;
pub mod launch;

/// Sandbox children enter the existing Agent helper before constructing GUI
/// or headless state. The helper never starts a service or takes its lock.
pub fn dispatch_agent_helper(args: &[String]) -> Option<Result<(), String>> {
    let helper_args = if args.first().map(String::as_str) == Some("agent") {
        &args[1..]
    } else {
        args
    };
    (helper_args.first().map(String::as_str) == Some("--linux-sandbox-helper"))
        .then(|| future_agent::cli::run_from_args(helper_args).map_err(|error| error.to_string()))
}
#[cfg(all(feature = "gui", target_os = "linux"))]
mod linux_power;
#[cfg(all(feature = "gui", target_os = "macos"))]
mod macos_power;
#[cfg(all(feature = "gui", target_os = "macos"))]
mod menu;
mod remote;
mod remote_peer;
mod runtime;
mod scheduler;
#[cfg_attr(not(feature = "gui"), allow(unused_imports))]
mod shadow_review;
mod skills;
#[cfg(feature = "gui")]
mod skills_bootstrap;
#[cfg_attr(not(feature = "gui"), allow(unused_imports))]
mod store;
mod tasks;
#[cfg(feature = "gui")]
mod terminal;
#[cfg(all(feature = "gui", target_os = "windows"))]
mod windows_power;

#[cfg(feature = "gui")]
use commands::*;
use error::AppError;

/// Process-wide lock for tests that mutate the global `HOME` env var
/// (`auth_store` and the shadow-review smoke test). `HOME` is process-global, so
/// those tests must run one at a time or they clobber each other's paths.
#[cfg(all(test, feature = "gui"))]
pub(crate) use future_host::TEST_HOME_LOCK;

#[cfg(all(test, feature = "gui"))]
fn install_rustls_provider() {
    future_host::install_rustls_provider();
}

#[cfg(feature = "gui")]
pub(crate) use future_host::events::{emit_remote_activity, emit_threads_updated};
#[cfg(feature = "gui")]
pub use gui::{run, run_with};
// The menu builder is reachable from an integration test only through a public path:
// `cfg(test)` is false for an integration build, so a `#[cfg(test)]` re-export would be
// invisible exactly where the main-thread target needs it. Re-exported, not made
// `pub mod` — the module stays private and the item's own visibility is unchanged.
#[cfg(all(feature = "gui", target_os = "macos"))]
#[doc(hidden)]
pub use menu::build_macos_menu;

#[cfg(feature = "gui")]
mod gui {
    use super::*;

    /// App handle captured at setup, used to push events to the webview from
    /// background tasks (e.g. deferred shadow-review materialization).
    pub(crate) static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> =
        std::sync::OnceLock::new();

    fn present_host_event(event: future_host::events::Event<'_>) {
        use future_host::events::Event;
        match event {
            Event::CloseThreadTerminals(thread_id) => {
                crate::commands::close_thread_terminals(thread_id)
            }
            Event::AgentSettings {
                session_id,
                thread_id,
                event_type,
                data,
            } => crate::agent_bridge::forward_settings_event(
                APP_HANDLE.get(),
                session_id,
                thread_id,
                event_type,
                data,
            ),
            Event::ReviewUpdated(thread_id) => emit_review_updated(thread_id),
            Event::RemoteActivity(thread_id) => emit_remote_activity(thread_id),
            Event::ThreadsUpdated => emit_threads_updated(),
            Event::ThreadRuntimeUpdated {
                thread_id,
                run_id,
                status,
                reset_projection,
            } => emit_thread_runtime_updated(thread_id, run_id, status, reset_projection),
            Event::ApprovalsUpdated {
                thread_id,
                approval_request_id,
            } => emit_approvals_updated(thread_id, approval_request_id),
            Event::Notification { name, payload } => {
                if let Some(app) = APP_HANDLE.get() {
                    use tauri::Emitter;
                    let _ = app.emit(name, payload);
                }
            }
        }
    }

    /// Both the initial launch and a second-instance notification must activate
    /// the window, including when it was created hidden for icon setup.
    fn activate_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
        use tauri::Manager;
        let Some(window) = app.get_webview_window("main") else {
            crate::host_environment::report("main window is missing during activation");
            return;
        };
        if let Err(error) = window.unminimize() {
            crate::host_environment::report(&format!("main window could not be restored: {error}"));
        }
        if let Err(error) = window.show() {
            crate::host_environment::report(&format!("main window could not be shown: {error}"));
        }
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::WindowsAndMessaging::{IsWindowVisible, ShowWindow, SW_SHOW};

            // Tao caches visibility and can skip ShowWindow when its flags say
            // "visible". Check the HWND too so a hidden native window cannot be
            // stranded until the user launches a second instance.
            match window.hwnd() {
                Ok(handle) => unsafe {
                    let hwnd = HWND(handle.0 as _);
                    if !IsWindowVisible(hwnd).as_bool() {
                        // ShowWindow returns the PREVIOUS visibility, not success.
                        let _ = ShowWindow(hwnd, SW_SHOW);
                        if !IsWindowVisible(hwnd).as_bool() {
                            crate::host_environment::report(
                                "main window remained hidden after native show",
                            );
                        }
                    }
                },
                Err(error) => crate::host_environment::report(&format!(
                    "main window handle is unavailable: {error}"
                )),
            }
        }
        if let Err(error) = window.set_focus() {
            crate::host_environment::report(&format!("main window could not be focused: {error}"));
        }
    }

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

    /// Paint the Windows title bar in the app's own colors.
    ///
    /// Windows draws the frame itself, so a decorated window gets DWM's caption
    /// color — the system light/dark gray, tinted by the wallpaper when
    /// transparency is on — which reads as a differently colored band sitting
    /// above the webview's white canvas. macOS never shows this seam because
    /// `titleBarStyle: "Overlay"` hands the top strip to the app. Windows 11
    /// (21H2+, build 22000) lets an app pin the caption and title-text colors, so
    /// pin them to the palette in `tailwind.config.js` (`surface` for the caption,
    /// `ink` for the text). Windows 10 only has the dark-mode attribute: clearing
    /// it keeps the caption light there too, instead of following a dark system
    /// theme. Attributes an older build doesn't support are rejected and leave the
    /// default, so this is safe everywhere.
    #[cfg(target_os = "windows")]
    fn match_windows_caption_color<R: tauri::Runtime>(app: &tauri::App<R>) {
        use tauri::Manager;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Dwm::{
            DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
        };

        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        let Ok(hwnd) = window.hwnd() else {
            return;
        };
        let hwnd = HWND(hwnd.0 as _);

        // COLORREF is 0x00BBGGRR: `surface` #ffffff, `ink` #172033.
        const CAPTION: u32 = 0x00FF_FFFF;
        const TEXT: u32 = 0x0033_2017;
        const LIGHT: i32 = 0;

        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                std::ptr::from_ref(&LIGHT).cast(),
                size_of::<i32>() as u32,
            );
            for (attribute, color) in [(DWMWA_CAPTION_COLOR, CAPTION), (DWMWA_TEXT_COLOR, TEXT)] {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    attribute,
                    std::ptr::from_ref(&color).cast(),
                    size_of::<u32>() as u32,
                );
            }
        }
    }

    /// The caption colors are only observable on a live window; what a mock app can
    /// assert is the no-op arm, mirroring [`size_main_window_to_screen`]'s tests.
    #[cfg(all(test, target_os = "windows"))]
    mod windows_caption_tests {
        use super::match_windows_caption_color;

        #[test]
        fn returns_without_a_main_window() {
            let app = tauri::test::mock_app();
            match_windows_caption_color(&app);
        }
    }

    /// Give the Dock a real icon when the process is not a bundled `.app`.
    ///
    /// macOS takes the Dock icon from the bundle's `Info.plist`, so a `.app` launch
    /// is fine — but an unbundled binary (a dev run, or `make install` followed by
    /// `future-desktop`) has no bundle and falls back to the generic executable
    /// icon. Setting the shared application's icon image explicitly fixes both
    /// launch styles.
    #[cfg(target_os = "macos")]
    fn set_macos_dock_icon() {
        // `NSApplication`/AppKit is main-thread only, and `setup` runs there.
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        if let Some(image) = dock_icon_image() {
            let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
            // SAFETY: main-thread AppKit call on the shared application.
            unsafe { app.setApplicationIconImage(Some(&image)) };
        }
    }

    /// Build an `NSImage` from the icon embedded in the binary.
    ///
    /// Embedded rather than read from `bundle.resources` for the same reason as
    /// [`icon_ico_bytes`]: an unbundled run has no resource directory to read from.
    #[cfg(target_os = "macos")]
    fn dock_icon_image() -> Option<objc2::rc::Retained<objc2_app_kit::NSImage>> {
        use objc2::AnyThread;
        use objc2_app_kit::NSImage;
        use objc2_foundation::NSData;

        let data = NSData::with_bytes(include_bytes!("../icons/icon.png"));
        NSImage::initWithData(NSImage::alloc(), &data)
    }

    /// Decoding the embedded icon is the only part of the Dock-icon path that can
    /// be checked without a running `NSApplication`; a corrupt or replaced icon
    /// file would otherwise fail silently at every launch.
    #[cfg(all(test, target_os = "macos"))]
    mod macos_dock_icon_tests {
        use super::{dock_icon_image, set_macos_dock_icon};

        #[test]
        fn embedded_dock_icon_decodes_to_an_image() {
            assert!(dock_icon_image().is_some());
        }

        /// An unbundled launch has no Dock icon, and setting one is an
        /// `NSApplication` call — main-thread only. Every libtest body runs on a
        /// spawned worker thread (verified: `pthread_main_np() == 0` even under
        /// `--test-threads=1`), so this is exactly the off-main-thread call site
        /// the guard exists for: assert the precondition, then that the call is a
        /// no-op rather than an AppKit call from the wrong thread.
        #[test]
        fn dock_icon_setup_is_a_noop_without_the_main_thread() {
            assert!(objc2::MainThreadMarker::new().is_none());
            set_macos_dock_icon();
        }
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

    /// Notify the frontend that the thread list changed behind its back — sessions
    /// created outside the GUI (TUI/CLI/channels) were imported into the store —
    /// so the sidebar re-lists threads. No payload: a bare invalidation signal.
    pub(crate) fn emit_threads_updated() {
        emit_threads_updated_via(APP_HANDLE.get());
    }

    /// Route the invalidation through an optional handle (see
    /// [`emit_review_updated_via`]).
    fn emit_threads_updated_via<R: tauri::Runtime>(handle: Option<&tauri::AppHandle<R>>) {
        if let Some(handle) = handle {
            emit_threads_updated_on(handle);
        }
    }

    /// Emit the "threads-updated" invalidation on a caller-supplied handle.
    fn emit_threads_updated_on<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
        use tauri::Emitter;
        let _ = handle.emit("threads-updated", ());
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

    static NEXT_RUNTIME_REVISION: std::sync::atomic::AtomicI64 =
        std::sync::atomic::AtomicI64::new(1);
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
            crate::runtime::spawn_native(|| runtime_update_drain_loop(rx));
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
        crate::runtime::spawn(thread_streaming_monitor_loop());
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
        while !crate::desktop_shutdown::is_stopping() && !crate::runtime::tasks_frozen() {
            let first = match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(first) => first,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            };
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
    pub fn run() -> Result<(), AppError> {
        run_with()
    }

    /// The plugin and event registration the Desktop app is built with.
    ///
    /// Generic over the runtime so this registration — the part that decides what
    /// the Desktop offers — is assertable: a test builds it for a mock runtime and
    /// checks what it registered (the closure *bodies* still need real window/menu
    /// events). Behaviour for the real runtime is unchanged: nothing here uses a
    /// `Wry`-only API (the builders, the plugins and every handle the bodies touch
    /// are `R`-generic), the chain is the one `run_with` used, and `run_with` is
    /// the only production caller.
    fn gui_builder<R: tauri::Runtime>() -> tauri::Builder<R> {
        tauri::Builder::<R>::new()
            .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                // A second instance was launched — activate the existing window.
                // Nothing is forwarded: the commands that change this machine's
                // pairing live in the window and in `future remote devices`, which
                // asks through `remote::control` rather than `argv`.
                activate_main_window(app);
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
                        menu::MENU_NEW_CONVERSATION | menu::MENU_SETTINGS => {
                            // The window runs the *command*, by id — the same command
                            // the palette, the rail's button and a keystroke reach,
                            // through the same `useCommand` handler. The menu names no
                            // key (see `menu.rs`), so a rebinding cannot leave a stale
                            // label behind, and the item works on whatever key the
                            // command has today.
                            //
                            // A window that is not up yet has no handler registered,
                            // and `runCommand` on the other side is then a no-op.
                            let command = if event.id().as_ref() == menu::MENU_NEW_CONVERSATION {
                                "chat.new"
                            } else {
                                "settings.open"
                            };
                            let _ = app.emit("menu-command", command);
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
                if window.label() != "main" {
                    return;
                }
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    use tauri::Manager;
                    match agent_supervisor::on_close_requested() {
                        agent_supervisor::QuitDecision::Proceed => {
                            if !agent_supervisor::exit_authorized() {
                                api.prevent_close();
                                agent_supervisor::request_quit(window.app_handle().clone(), false);
                            }
                        }
                        agent_supervisor::QuitDecision::Confirm { open_dialog } => {
                            api.prevent_close();
                            if open_dialog {
                                agent_supervisor::confirm_quit(window.app_handle().clone());
                            }
                        }
                    }
                }
            })
    }

    /// Start the window. Mobile enters through [`run`] as well.
    pub fn run_with() -> Result<(), AppError> {
        // Arm the stall watchdog before the first thing that can block. A launch
        // wedged in the kernel cannot be interrupted, but it can be *named*: the
        // report lands in the host log (and stderr) once the budget expires,
        // instead of the failure being a silent blank window.
        let startup = future_host::startup::global();
        startup.arm(
            crate::host_environment::report,
            future_host::startup::STALL_AFTER,
            future_host::startup::STALL_REPEAT,
        );
        // The combined Agent/Desktop dependency graph may include multiple
        // rustls providers. Select ring and hydrate tool environment before
        // Tauri starts threads or creates any HTTP clients.
        startup.enter("host environment");
        if let Err(error) = crate::host_environment::initialize() {
            startup.failed();
            return Err(error);
        }
        startup.enter("window");
        let mut context = tauri::generate_context!();
        context.config_mut().identifier = instance::gui_identifier(&context.config().identifier)?;
        // The `setup` hook and the invoke handler stay with the real runtime: the
        // hook publishes the process-global `AppHandle<Wry>`, and the commands
        // behind the handler take that same concrete handle. Appending them after
        // `gui_builder` is equivalent — `Builder` keeps these in separate fields.
        startup.enter("window build");
        let app = gui_builder::<tauri::Wry>()
            .setup(|app| {
                let _ = APP_HANDLE.set(app.handle().clone());
                let _ = future_host::events::attach(present_host_event);
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
                #[cfg(target_os = "macos")]
                set_macos_dock_icon();
                size_main_window_to_screen(app);
                // Windows: set a high-quality taskbar icon. Tauri's default path creates
                // a single-size HICON from the first PNG, and GDI's icon scaling is poor
                // when the taskbar needs to render it at a different size. Instead, load
                // the multi-size ICO and let Windows pick the exact match for its display
                // size — the ICO contains 16,20,24,30,32,36,40,48,64,72,96,128,256.
                #[cfg(target_os = "windows")]
                set_windows_taskbar_icon(app);
                // Windows: the frame is DWM's, so color its caption like the app's
                // own surface — otherwise the title bar is a visibly different band
                // at the top of the window (see the function for the palette).
                #[cfg(target_os = "windows")]
                match_windows_caption_color(app);
                // The window is created hidden (`"visible": false` in tauri.conf.json)
                // so the taskbar never flashes Tauri's default (blurry, upscaled) icon
                // before the crisp one above is in place. Tauri creates configured
                // windows just before `setup`, when the runtime reaches Ready.
                // The instance guard is already held before `app.run` is entered.
                // Showing between Builder::build and app.run silently misses this
                // window because it has not been created yet.
                activate_main_window(app.handle());
                Ok(())
            })
            .invoke_handler(tauri::generate_handler![
                app_build_info,
                get_agent_status,
                check_app_update,
                app_maintenance_status,
                acknowledge_update_repair,
                install_app_update,
                restart_after_app_update,
                open_path,
                list_directory,
                open_external_url,
                resolve_preview_link_path,
                read_text_file_preview,
                inspect_attachment,
                read_native_clipboard_file_paths,
                validate_image_attachment,
                prepare_image_preview,
                generate_image_thumbnail,
                import_ephemeral_attachment,
                delete_temp_attachment,
                export_artifact_file,
                initialize_app_store,
                get_app_settings,
                update_app_settings,
                generate_thread_title,
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
                get_future_auth_state,
                get_future_balance,
                archive_finished_runs,
                list_threads,
                list_workspaces,
                create_workspace,
                rename_workspace,
                pin_workspace,
                delete_workspace,
                ensure_workspace_git,
                save_pasted_image,
                save_pasted_file,
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
                get_session_entries_page,
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
                list_approval_assessments,
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
                list_tasks,
                create_task,
                update_task,
                delete_task,
                set_task_enabled,
                run_task_now,
                list_task_runs,
                list_task_deps,
                set_task_dep,
                remove_task_dep,
                list_task_revisions,
                apply_task_revision,
                list_installed_skills,
                list_available_skills,
                get_skill_guide,
                suggest_skill,
                skill_reco_today,
                record_skill_reco,
                install_skill,
                uninstall_skill,
                refresh_skills,
                sync_skills,
                bootstrap_builtin_skills,
                remote_start,
                remote_stop,
                remote_status,
                remote_unpair,
                remote_unpair_device,
                remote_disconnect_device,
                remote_add_device,
                remote_takeover,
                remote_pairing_status,
                remote_peer_list,
                remote_peer_pair,
                remote_peer_connect,
                remote_peer_connect_all,
                remote_peer_disconnect,
                remote_peer_unpair,
                remote_peer_set_label,
                remote_peer_set_visible_session,
                remote_peer_sessions,
                remote_peer_workspaces,
                remote_peer_list_files,
                remote_peer_download_file,
                remote_peer_preview_file,
                remote_peer_upload_file,
                remote_peer_request,
                terminal_server_info,
                open_url
            ])
            .build(context)
            .map_err(|error| AppError::from(format!("Could not start Desktop: {error}")))?;

        // A failure here ends the process, and a launch from Finder/Dock has no
        // terminal to show it on. Route it through the host log (started above)
        // so the next launch can say what happened instead of leaving a window
        // that appeared and vanished.
        if let Err(error) = initialize_process_state(&app) {
            crate::host_environment::report(&format!("startup failed: {error}"));
            startup.failed();
            return Err(error);
        }
        // Keep the Windows console attached until startup errors can be reported.
        #[cfg(target_os = "windows")]
        unsafe {
            let _ = windows::Win32::System::Console::FreeConsole();
        }
        startup.enter("event loop");
        app.run(dispatch_run_event);
        Ok(())
    }

    /// The startup steps that need no window: the shared GUI/headless lock, the
    /// host log, the app store and the terminal listener. Split out of
    /// [`run_with`] and generic over both the runtime and the manager so a test
    /// can drive it with `tauri::test::mock_app()` — the same shape as
    /// [`start_backend_background`]. `&app` stays borrowed only for `manage`.
    ///
    /// Only the lock and the app store are fatal: the lock because a second
    /// writer must never share the data directory, the store because no command
    /// can answer without it. The host log and the terminal listener degrade
    /// instead — a read-only install must still open a window.
    fn initialize_process_state<R: tauri::Runtime, M: tauri::Manager<R>>(
        app: &M,
    ) -> Result<(), AppError> {
        let startup = future_host::startup::global();
        // Build initializes the single-instance plugin first, so a second GUI
        // still activates the existing window. Acquire the shared GUI/headless
        // lock before entering the event loop: returning an error from `setup`
        // makes Tauri panic (and abort on macOS's non-unwinding callback).
        startup.enter("desktop data lock");
        let guard = instance::InstanceGuard::acquire()?;
        tauri::Manager::manage(app, guard);
        // Logging first, and never fatal: the log is the only place a later
        // startup failure is diagnosable from, and a read-only install or a full
        // disk must not turn "no log" into "no window".
        startup.enter("host log");
        if let Err(error) = crate::host_environment::start_logging() {
            crate::host_environment::report(&format!("host log unavailable: {error}"));
        }
        crate::runtime::set(tauri::async_runtime::handle().inner().clone());
        agent_bridge::install_channel_runtime();
        // Only the part commands need is on the launch path. Converging legacy
        // rows and orphaned directories walks the data tree, so it runs once the
        // event loop is live (see `start_backend_background`).
        startup.enter("app store");
        store::initialize_app_store()?;
        startup.enter("terminal server");
        if let Err(error) = terminal::server::bind() {
            eprintln!("FutureOS terminal server bind failed: {error}");
        }
        Ok(())
    }

    /// Dispatch one run event. Generic over the runtime and split out of
    /// [`run_with`] for the same reason: every callee here either takes
    /// `AppHandle<R>` already or takes no handle at all, so a mock handle
    /// exercises the arms. `ExitRequested` is the exception and cannot be
    /// dispatched from a test at all: `tauri::ExitRequestApi` has private fields
    /// and only Tauri's own event loop constructs it.
    fn dispatch_run_event<R: tauri::Runtime>(
        app_handle: &tauri::AppHandle<R>,
        event: tauri::RunEvent,
    ) {
        match event {
            // `setup` runs while Tauri is still constructing its event loop.
            // Starting the bridge there races the runtime initialization on
            // macOS and can leave the detached start task without a live
            // connection. `Ready` is emitted once the process runtime is live.
            tauri::RunEvent::Ready => {
                future_host::startup::global().enter("agent");
                let handle = app_handle.clone();
                crate::runtime::spawn_native(move || {
                    if let Err(error) = agent_supervisor::start_agent_supervision(&handle) {
                        eprintln!("FutureOS Agent startup failed: {error}");
                        return;
                    }
                    if crate::desktop_shutdown::is_stopping() {
                        return;
                    }
                    start_backend_background(handle);
                });
                if let Err(error) = terminal::server::serve() {
                    eprintln!("FutureOS terminal server failed to start: {error}");
                }
            }
            // ⌘Q / the menu's "Quit FutureOS" / a programmatic `app.exit()` come
            // through here, NOT the window's `CloseRequested`. Guard them the same
            // way so a running conversation can't be torn down without warning.
            tauri::RunEvent::ExitRequested { api, .. } => {
                match agent_supervisor::on_close_requested() {
                    agent_supervisor::QuitDecision::Proceed => {
                        if !agent_supervisor::exit_authorized() {
                            api.prevent_exit();
                            agent_supervisor::request_quit(app_handle.clone(), false);
                        }
                    }
                    agent_supervisor::QuitDecision::Confirm { open_dialog } => {
                        api.prevent_exit();
                        if open_dialog {
                            agent_supervisor::confirm_quit(app_handle.clone());
                        }
                    }
                }
            }
            tauri::RunEvent::Exit => {
                // The ordinary exit path already waited outside the UI thread.
                // Unexpected OS exits can only request best-effort background stop.
                let _ = crate::desktop_shutdown::begin("exit");
            }
            _ => {}
        }
    }

    /// Start the window's background work. Generic over the runtime so a test can
    /// drive it with a mock handle: every loop it starts takes `AppHandle<R>` and
    /// does its real work on the shared runtime, so the mock changes nothing about
    /// what is started. Behaviour for the real runtime is unchanged.
    fn start_backend_background<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
        // Leftover convergence (legacy soft-deleted rows, orphaned image/chat/
        // review directories) is hygiene, not startup: it walks the data tree
        // and no command depends on it. Running it here keeps a large or slow
        // data directory from delaying the first frame.
        crate::runtime::spawn_native(store::reconcile_after_startup);
        // Independent fixed-interval maintenance: app updates (24h),
        // Future balance (1h), and Future models (24h). Missed ticks while
        // suspended are skipped; each task runs at most once after resume.
        scheduler::start(app.clone());
        // User-defined tasks: the tick loop claims due tasks and runs
        // them through the Agent service (full permission). Headless
        // desktop starts the same loop from `headless/mod.rs`.
        tasks::start(app.clone());
        // Do not preemptively cancel non-terminal GUI rows at startup. The
        // Agent is authoritative and may have survived a GUI crash; the
        // watchdog below reattaches or settles each row only after it can
        // query that authority.
        start_thread_streaming_monitor();
        // Global provider/auth completion stream. This is independent of
        // chat observers and fans committed revisions to WebView + Mobile.
        agent_bridge::spawn_provider_config_observer();
        // Global session lifecycle stream: sessions created by other
        // clients (TUI/CLI/channels) are imported within milliseconds of
        // the agent's session_created announcement. Its initial attach also
        // performs the startup discovery pass, so startup has one owner for
        // session discovery instead of racing a duplicate one-shot import.
        agent_bridge::spawn_session_events_observer();
        // Low-frequency safety reconciliation for lifecycle notifications
        // missed during outages or emitted by an older Agent. The global
        // stream is the realtime path and reconciles on every reconnect.
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
        crate::runtime::spawn(async {
            // Reconcile on the shared, tracked runtime after Agent readiness.
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            // Delete threads whose agent session was removed externally
            // (TUI/CLI/manual) — detected via the agent's session list,
            // never file probing. Runs first so run reconciliation below
            // doesn't query sessions that no longer exist. Self-gates on
            // agent reachability (skips when the agent is down).
            if let Err(error) = store::reconcile_orphan_sessions().await {
                eprintln!("FutureOS orphan-session reconcile failed: {error}");
            }
            // Rows mirrored by older builds for sessions that were
            // created but never prompted are empty conversations
            // with no run to their name; drop them so the list
            // converges without waiting for the periodic pass.
            match store::reconcile_empty_conversations().await {
                Ok(0) => {}
                Ok(removed) => {
                    eprintln!("FutureOS: removed {removed} empty conversation(s)");
                    crate::emit_threads_updated();
                }
                Err(error) => {
                    eprintln!("FutureOS empty-conversation reconcile failed: {error}")
                }
            }
            // Rows produced by older startup convergence builds are
            // terminal locally (`cancelled/interrupted`) and therefore
            // outside the active-run watchdog. Reconcile that legacy
            // shape once before approvals so an Agent run that survived
            // the GUI restart is reanimated instead of staying hidden.
            agent_bridge::reconcile_interrupted_runs().await;
            agent_bridge::reconcile_pending_approvals().await;
        });
        // Shadow-review maintenance (consistency check + crash recovery) runs
        // off the launch path so it never delays the window.
        crate::runtime::spawn_native(shadow_review::run_startup_maintenance);
        // `future remote devices add` asks this process for an invitation, because
        // only a process that can serve the new pairing may mint one. Started here,
        // with the rest of the window's background work, so it is up as soon as the
        // fixture tests and the real launch report Ready.
        crate::remote::control::spawn_host();
        spawn_remote_auto_connect();
    }

    /// Start the persisted remote bridge only after Tauri's runtime has entered its
    /// event loop. Calling this from `setup` is too early on macOS: the detached
    /// task can be scheduled before the process runtime is ready and never obtain
    /// a lasting NATS connection.
    ///
    /// The preference is the Remote page's own switch (on by default), so a
    /// machine the user left serving comes back serving. An unreadable settings
    /// store does not count as "on": opening remote access without a preference
    /// that can be read would serve on a guess.
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

        crate::runtime::spawn(async {
            match remote::enable().await {
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
            emit_threads_updated, emit_threads_updated_on, emit_threads_updated_via,
            main_window_geometry, runtime_update_drain_loop, sample_thread_streaming,
            sample_thread_streaming_with, size_main_window_to_screen,
            thread_streaming_monitor_loop, ThreadRuntimeUpdate, ThreadRuntimeUpdateBatch,
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
            crate::commands::agent_mock::script_mock_agent(
                crate::commands::agent_mock::MockScript {
                    down: true,
                    ..Default::default()
                },
            );
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
            emit_threads_updated_via(Some(handle));
            // None arm: process-global APP_HANDLE unset in tests.
            emit_review_updated_via::<tauri::test::MockRuntime>(None, "thread-1");
            emit_remote_activity_via::<tauri::test::MockRuntime>(None, "thread-1");
            emit_approvals_updated_via::<tauri::test::MockRuntime>(None, "thread-1", "approval-1");
            emit_threads_updated_via::<tauri::test::MockRuntime>(None);
            emit_threads_updated_on(handle);
            // The public entry point reads the process-global handle, which no
            // test populates, so it takes the None arm without panicking.
            emit_threads_updated();
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

        /// The loop only enters its body while the process is neither stopping nor
        /// frozen, and both flags are process-global: a sibling test that freezes
        /// the registry leaves the drain loop exiting before its first `recv`. Reset
        /// them, then drive the exits one by one — an idle 100ms poll (`Timeout` →
        /// `continue`), a burst that keeps arriving until the 16ms window itself
        /// expires (`remaining.is_zero()` → `break`), a window that closes empty
        /// (`Timeout` → `break`), and a dropped sender (`Disconnected` → return).
        #[test]
        fn runtime_update_drain_loop_covers_every_receive_exit() {
            // Resetting the shared task/admission registry also clears owned
            // runs. Hold the same process-state lock as the real RPC shutdown
            // fixtures for the entire drain, including its final reset.
            let _home = crate::auth_store::test_support::HomeGuard::new("runtime-update-drain");
            crate::desktop_shutdown::reset_for_test();
            crate::runtime::cancel_test_tasks();
            let (tx, rx) = std::sync::mpsc::channel::<ThreadRuntimeUpdate>();
            let drain = std::thread::spawn(move || runtime_update_drain_loop(rx));
            // Empty channel: the 100ms poll expires and the loop continues.
            std::thread::sleep(std::time::Duration::from_millis(150));
            // A steady stream for longer than the coalescing window, so the inner
            // loop is still receiving when the 16ms budget runs out. The stream is
            // *time-bounded* rather than a fixed count of messages: a count can be
            // fully sent while the drain thread is descheduled on a loaded host,
            // which is exactly the run that left `remaining.is_zero()` unexecuted.
            let burst = tx.clone();
            let producer = std::thread::spawn(move || {
                let until = std::time::Instant::now() + std::time::Duration::from_millis(500);
                let mut revision = 1;
                while std::time::Instant::now() < until {
                    if burst
                        .send(update("run-1", revision, "running", false))
                        .is_err()
                    {
                        return;
                    }
                    revision += 1;
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            });
            producer.join().expect("the producer finishes");
            // One update on its own, then a quiet channel: the window closes empty.
            tx.send(update("run-1", 21, "completed", false)).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(80));
            // Sender gone: the next poll reports a disconnect and the loop returns.
            drop(tx);
            drain.join().expect("the drain loop exits on disconnect");
            crate::desktop_shutdown::reset_for_test();
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

        /// Points one of this process's descriptors at a file and restores it on
        /// drop, so a test can read what a path that writes to `stdout()` /
        /// `stderr()` really published. `println!`/`eprintln!` *macros* are captured
        /// by the test harness; the `std::io` handles and `libc::write` below them
        /// are not.
        #[cfg(unix)]
        struct FdCapture {
            saved: i32,
            fd: i32,
            label: &'static str,
            path: std::path::PathBuf,
        }

        #[cfg(unix)]
        impl FdCapture {
            /// A startup failure is reported on stderr, which is what a user sees.
            fn stderr() -> Self {
                Self::begin(2, "stderr")
            }

            fn begin(fd: i32, label: &'static str) -> Self {
                use std::os::fd::AsRawFd;
                let path = std::env::temp_dir().join(format!(
                    "futureos-fd-capture-{}-{}-{}.txt",
                    label,
                    std::process::id(),
                    line!()
                ));
                let _ = std::fs::remove_file(&path);
                let sink = std::fs::File::create(&path).expect("create the capture file");
                // SAFETY: both calls touch only this process's descriptor table;
                // `saved` is restored in `drop` even if the test body panics.
                let saved = unsafe { libc::dup(fd) };
                assert!(saved >= 0, "dup({fd}): {}", std::io::Error::last_os_error());
                let redirected = unsafe { libc::dup2(sink.as_raw_fd(), fd) };
                // Hoisted: a call inside the message is evaluated only when the
                // assertion fails, so its own line would go unexecuted.
                let dup2_failure = format!("dup2({fd}): {}", std::io::Error::last_os_error());
                assert_eq!(redirected, fd, "{dup2_failure}");
                Self {
                    saved,
                    fd,
                    label,
                    path,
                }
            }

            /// Everything captured so far.
            fn text(&self) -> String {
                std::fs::read_to_string(&self.path).unwrap_or_default()
            }

            /// The captured text, once it holds `needle` — the work runs on the
            /// background runtime, so poll for the effect instead of assuming it.
            fn wait_for(&self, needle: &str) -> String {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
                loop {
                    let text = self.text();
                    if text.contains(needle) {
                        return text;
                    }
                    let message = format!(
                        "nothing on this process's {} matching {needle:?}: {text:?}",
                        self.label
                    );
                    assert!(std::time::Instant::now() < deadline, "{message}");
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        }

        #[cfg(unix)]
        impl Drop for FdCapture {
            fn drop(&mut self) {
                // SAFETY: `saved` is the descriptor `dup` returned above.
                unsafe {
                    libc::dup2(self.saved, self.fd);
                    libc::close(self.saved);
                }
                let _ = std::fs::remove_file(&self.path);
            }
        }

        /// The child gets its own `HOME`: the launch path it drives must never resolve
        /// the parent's temporary store.
        fn in_child_process(
            marker: &str,
            test_path: &str,
            child: impl FnOnce(),
        ) -> Option<(std::process::ExitStatus, String)> {
            if std::env::var_os(marker).is_some() {
                child();
                return None;
            }
            let exe = std::env::current_exe().expect("test binary path");
            let child_home = std::env::temp_dir()
                .join(format!("futureos-child-{}-{marker}", std::process::id()));
            std::fs::create_dir_all(&child_home).expect("child HOME");
            let output = std::process::Command::new(exe)
                .args([test_path, "--exact", "--nocapture"])
                .env(marker, "1")
                .env("HOME", &child_home)
                .env_remove("FUTURE_HOME")
                .output()
                .expect("spawn the isolated test process");
            let _ = std::fs::remove_dir_all(&child_home);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            Some((output.status, stderr))
        }

        /// The child's report, rendered before any assertion so a non-zero exit is
        /// always explained (a lazily built message is never exercised on a pass).
        fn child_report(status: &std::process::ExitStatus, stderr: &str) -> String {
            format!("the isolated process must pass (exit {status}):\n{stderr}")
        }

        /// `serve()` is one-shot per process: the arm that reports a failure can only
        /// be observed in a process where the listener is already taken — which is
        /// exactly the state the fixture above consumes, so no test *body* can reach
        /// it here. A child process gets it deterministically, the pattern
        /// `commands::terminal::tests` uses for the pre-bind arm of the same server.
        #[test]
        fn a_ready_event_whose_listener_is_already_taken_reports_the_serve_failure() {
            const CHILD_MARKER: &str = "FUTUREOS_READY_SERVE_CHILD";
            if let Some((status, stderr)) = in_child_process(
                CHILD_MARKER,
                "gui::startup_paths_tests::a_ready_event_whose_listener_is_already_taken_reports_the_serve_failure",
                || {
                    // The child's only job: take the listener, then dispatch the event
                    // whose arm must fail to serve it.
                    terminal::server::bind().expect("bind the listener");
                    terminal::server::serve().expect("the child serves it first");
                    let app = tauri::test::mock_app();
                    dispatch_run_event(
                        app.handle(),
                        tauri::RunEvent::Ready,
                    );
                },
            ) {
                let report = child_report(&status, &stderr);
                assert!(status.success(), "{report}");
                assert!(
                    stderr.contains("FutureOS terminal server failed to start"),
                    "the arm must report the serve failure it could not avoid: {report}"
                );
            }
        }

        /// A store that cannot be opened is reported, not swallowed: both `Err` arms
        /// of the post-readiness reconciliation run, and the removal arm must *not*.
        #[cfg(unix)]
        #[test]
        fn a_broken_store_makes_the_startup_reconciliation_report_its_failures() {
            const CHILD_MARKER: &str = "FUTUREOS_RECONCILE_BROKEN_CHILD";
            if let Some((status, stderr)) = in_child_process(
                CHILD_MARKER,
                "gui::startup_paths_tests::a_broken_store_makes_the_startup_reconciliation_report_its_failures",
                || {
                    let _home =
                        crate::auth_store::test_support::HomeGuard::new("startup-reconcile-broken");
                    let _mock = crate::agent_bridge::test_support::mock_agent();
                    crate::store::initialize_app_store().expect("init store");
                    let database =
                        future_app_settings::app_db_path().expect("db path under HOME");
                    // A directory where the database should be makes `Connection::open`
                    // fail immediately, on every platform. Deliberately not "garbage
                    // bytes": a zero-filled header is what SQLite treats as a brand-new
                    // database, so that shape would silently succeed and this fixture
                    // would assert nothing.
                    crate::store::close_pool();
                    std::fs::remove_file(&database).expect("drop the fixture database");
                    std::fs::create_dir(&database).expect("make the store unopenable");
                    crate::runtime::cancel_test_tasks();
                    let capture = FdCapture::stderr();
                    let app = tauri::test::mock_app();
                    start_backend_background(app.handle().clone());
                    // The block runs three seconds after readiness; poll for the effect
                    // rather than sleeping blind, then hand the text to the parent.
                    capture.wait_for("FutureOS orphan-session reconcile failed");
                    let text = capture.wait_for("FutureOS empty-conversation reconcile failed");
                    drop(capture);
                    crate::runtime::cancel_test_tasks();
                    eprintln!("isolated-process stderr:\n{text}");
                },
            ) {
                let report = child_report(&status, &stderr);
                assert!(status.success(), "{report}");
                assert!(
                    stderr.contains("FutureOS orphan-session reconcile failed"),
                    "the unreadable store must be reported: {report}"
                );
                assert!(
                    stderr.contains("FutureOS empty-conversation reconcile failed"),
                    "{report}"
                );
                assert!(
                    !stderr.contains("empty conversation(s)"),
                    "an unreadable store cannot have removed a conversation: {report}"
                );
            }
        }

        /// The third arm: an empty conversation — a thread the Agent has never been
        /// asked to fill — is dropped by the same pass, and the count is reported.
        #[cfg(unix)]
        #[test]
        fn an_empty_conversation_is_removed_by_the_startup_reconciliation() {
            const CHILD_MARKER: &str = "FUTUREOS_RECONCILE_EMPTY_CHILD";
            const SESSION: &str = "sess-empty-conversation";
            if let Some((status, stderr)) = in_child_process(
                CHILD_MARKER,
                "gui::startup_paths_tests::an_empty_conversation_is_removed_by_the_startup_reconciliation",
                || {
                    let _home =
                        crate::auth_store::test_support::HomeGuard::new("startup-reconcile-empty");
                    let mock = crate::agent_bridge::test_support::mock_agent();
                    // Attribution: the low-frequency discovery backstop prints the
                    // *same* message from `agent_bridge::observer`, so pin that tick
                    // away — its interval is a cfg(test) seam read on every iteration,
                    // and in this child the env var cannot affect any other test.
                    std::env::set_var("FUTURE_TEST_DISCOVERY_INTERVAL_MS", "3600000");
                    crate::store::initialize_app_store().expect("init store");
                    // The first statement of the block hard-deletes threads whose Agent
                    // session is not live, so the Agent must claim this session for the
                    // row to reach the emptiness probe — and the launch path asks more
                    // than once (its global session observer imports on attach), so
                    // every queued reply agrees.
                    for _ in 0..3 {
                        mock.push_data("list_session_ids", serde_json::json!({"ids": [SESSION]}));
                    }
                    // A page with no message-role entry and no continuation proves
                    // emptiness.
                    mock.push_data("get_session_entries", serde_json::json!({"entries": []}));
                    let database =
                        future_app_settings::app_db_path().expect("db path under HOME");
                    crate::store::close_pool();
                    {
                        let conn =
                            rusqlite::Connection::open(&database).expect("open the fixture store");
                        conn.execute_batch(&format!(
                            "INSERT INTO workspaces(id,name,kind,path,created_at,updated_at)
                               VALUES('w','W','temporary','fixture',1,1);
                             INSERT INTO threads(id,workspace_id,mode,title,status,agent_session_id,created_at,updated_at)
                               VALUES('t','w','chat','T','active','{SESSION}',1,1);"
                        ))
                        .expect("plant one thread that never produced a run");
                    }
                    crate::runtime::cancel_test_tasks();
                    let capture = FdCapture::stderr();
                    let app = tauri::test::mock_app();
                    start_backend_background(app.handle().clone());
                    let text = capture.wait_for("FutureOS: removed 1 empty conversation(s)");
                    drop(capture);
                    crate::runtime::cancel_test_tasks();
                    // The arm's effect, not just its message: the row is gone. Read it
                    // back through a fresh connection (the pool was released above).
                    let remaining: i64 = {
                        let conn =
                            rusqlite::Connection::open(&database).expect("open the fixture store");
                        conn.query_row("SELECT COUNT(*) FROM threads WHERE id='t'", [], |row| {
                            row.get(0)
                        })
                        .expect("count the planted thread")
                    };
                    assert_eq!(remaining, 0, "the empty conversation must be dropped");
                    eprintln!("isolated-process stderr:\n{text}");
                },
            ) {
                let report = child_report(&status, &stderr);
                assert!(status.success(), "{report}");
                assert!(
                    stderr.contains("FutureOS: removed 1 empty conversation(s)"),
                    "the removal must be reported with its count: {report}"
                );
                assert!(
                    !stderr.contains("empty-conversation reconcile failed"),
                    "the removal arm must be taken, not the failure arm: {report}"
                );
            }
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

        /// **The deferred arm.** A Desktop that is signed in with a pairing on disk but
        /// whose platform cannot be reached starts the bridge anyway and reports the
        /// outcome instead of failing the launch: the pass runs, `remote::start`
        /// returns in a non-ready phase, and the caller says so.
        ///
        /// The state is what makes it deterministic and offline: the signed-in platform
        /// is a closed port, so the credential refresh is a connection-refused
        /// transport failure (`network`), which `start_failure` categorizes into a
        /// not-running status rather than an `Err`. The arm's own report is asserted —
        /// and the *other* arm is asserted absent, so an input that slipped into the
        /// failure path fails this test instead of passing it.
        #[cfg(unix)]
        #[test]
        fn an_auto_connect_whose_platform_is_unreachable_is_reported_as_deferred() {
            const CHILD_MARKER: &str = "FUTUREOS_AUTO_CONNECT_DEFERRED_CHILD";
            if let Some((status, stderr)) = in_child_process(
                CHILD_MARKER,
                "gui::startup_paths_tests::an_auto_connect_whose_platform_is_unreachable_is_reported_as_deferred",
                || {
                    let _home =
                        crate::auth_store::test_support::HomeGuard::new("auto-connect-deferred");
                    crate::store::initialize_app_store().expect("init store");
                    // Signed in (the gate needs a key) at a platform nothing listens on.
                    crate::auth_store::set_future_login("sekret", "http://127.0.0.1:1")
                        .expect("sign in");
                    crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
                        auto_connect_remote: Some(true),
                        ..Default::default()
                    })
                    .expect("enable auto-connect");
                    // A pairing on disk is the third gate: a valid NKey seed, so the
                    // refresh gets as far as the request that then cannot be sent.
                    let creds = crate::remote::test_support::pairing_creds("shell-deferred");
                    crate::remote::pairing::save_creds(&creds).expect("persist the pairing");

                    let capture = FdCapture::stderr();
                    spawn_remote_auto_connect();
                    let text = capture.wait_for("FutureOS remote auto-connect: deferred (");
                    drop(capture);
                    crate::runtime::cancel_test_tasks();
                    eprintln!("isolated-process stderr:\n{text}");
                },
            ) {
                let report = child_report(&status, &stderr);
                assert!(status.success(), "{report}");
                assert!(
                    stderr.contains("FutureOS remote auto-connect: deferred ("),
                    "the deferred arm must report the phase it stopped in: {report}"
                );
                assert!(
                    !stderr.contains("FutureOS remote auto-connect failed"),
                    "a categorized refusal is a deferred start, not a failure: {report}"
                );
                assert!(
                    !stderr.contains("FutureOS remote auto-connect: connected"),
                    "nothing connected; the ready arm must not have run: {report}"
                );
            }
        }

        /// **The failed arm.** A pairing whose desktop NKey cannot be read is a local
        /// fault, not a remote condition: `remote::start` returns `Err`, and the pass
        /// reports it as a failure. The two arms are separated by exactly this — a
        /// categorized remote answer versus an uncategorized local one — so this
        /// fixture asserts the failure text and the *absence* of the deferred one.
        #[cfg(unix)]
        #[test]
        fn an_auto_connect_with_an_unreadable_pairing_key_is_reported_as_failed() {
            const CHILD_MARKER: &str = "FUTUREOS_AUTO_CONNECT_FAILED_CHILD";
            if let Some((status, stderr)) = in_child_process(
                CHILD_MARKER,
                "gui::startup_paths_tests::an_auto_connect_with_an_unreadable_pairing_key_is_reported_as_failed",
                || {
                    let _home =
                        crate::auth_store::test_support::HomeGuard::new("auto-connect-failed");
                    crate::store::initialize_app_store().expect("init store");
                    crate::auth_store::set_future_login("sekret", "http://127.0.0.1:1")
                        .expect("sign in");
                    crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
                        auto_connect_remote: Some(true),
                        ..Default::default()
                    })
                    .expect("enable auto-connect");
                    // Same persisted shape as the deferred fixture — valid handshake
                    // version and peer identity, so the legacy-upgrade branch is not
                    // what runs — with the one field a local fault belongs in.
                    let mut creds = crate::remote::test_support::pairing_creds("shell-failed");
                    creds.nkey_seed = "not-a-nkey-seed".to_string();
                    crate::remote::pairing::save_creds(&creds).expect("persist the pairing");

                    let capture = FdCapture::stderr();
                    spawn_remote_auto_connect();
                    let text = capture.wait_for("FutureOS remote auto-connect failed: ");
                    drop(capture);
                    crate::runtime::cancel_test_tasks();
                    eprintln!("isolated-process stderr:\n{text}");
                },
            ) {
                let report = child_report(&status, &stderr);
                assert!(status.success(), "{report}");
                assert!(
                    stderr.contains("FutureOS remote auto-connect failed: read desktop NKey"),
                    "the failure must name the local fault that caused it: {report}"
                );
                assert!(
                    !stderr.contains("FutureOS remote auto-connect: deferred ("),
                    "an uncategorized local error is an Err from start, not a status: {report}"
                );
            }
        }

        /// **The ready arm.** The whole pass runs to a *connected* bridge: the mock
        /// platform is reachable, the fake broker is up, and the pass's own pairing
        /// request is answered — so `remote::start` returns `Ready` and the arm whose
        /// body reports a connection executed.
        ///
        /// The assertion is the pass's effect, not the log line: the bridge is
        /// installed in the process-global supervisor with `Ready` and with the
        /// pairing the pass minted. Nothing else in this test starts a bridge, so the
        /// state is attributable to `spawn_remote_auto_connect`, and the bridge is
        /// stopped before the test returns (a leaked one would break the fixture that
        /// asserts `SUPERVISOR.state` is empty).
        ///
        /// Two robustness facts (checked in the source, §S6.7.2): `web_client_enabled()`
        /// is `true` in test builds, and a *failed* `bind_web_listener` is non-fatal —
        /// the bridge still reports `Ready` — so another test process holding port 8022
        /// does not invalidate this fixture.
        #[tokio::test]
        async fn an_auto_connect_that_connects_is_reported_as_connected() {
            use crate::remote::{test_support::MockPlatform, RemotePhase};

            let _home = crate::auth_store::test_support::HomeGuard::new("auto-connect-ready");
            crate::store::initialize_app_store().expect("init store");
            crate::runtime::cancel_test_tasks();
            // Signed in against the mock platform: the gate needs a key, and the
            // credential refresh must reach a server that answers.
            let platform = MockPlatform::start().await;
            let nats = crate::remote::test_support::FakeNats::start().await;
            crate::remote::test_support::sign_in(platform.url());
            // The gate also requires a persisted pairing, and the pass then *refreshes*
            // it (`establish` → `refresh_bridge_jwt`), so the mock platform has to
            // answer the refresh with the broker's address. A valid seed and a v2
            // handshake keep this out of the legacy-upgrade branch, so the refresh is
            // what runs.
            let creds = crate::remote::test_support::pairing_creds("shell-ready");
            crate::remote::pairing::save_creds(&creds).expect("persist the pairing");
            platform.respond_refresh(nats.url());
            crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
                auto_connect_remote: Some(true),
                ..Default::default()
            })
            .expect("enable auto-connect");
            // No persisted pairing: `establish()` mints one against the mock platform,
            // so the pass walks its whole path to a connected bridge.
            assert!(
                crate::remote::pairing::load_creds().is_some(),
                "the gate needs a persisted pairing for the pass to run at all"
            );

            spawn_remote_auto_connect();

            // The pass runs on the shared runtime; poll for its effect.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                let status = crate::remote::status();
                if matches!(status.phase, RemotePhase::Ready) {
                    assert_eq!(
                        status.pair_id, creds.pair_id,
                        "the ready bridge runs the pairing the pass refreshed"
                    );
                    assert_eq!(
                        status.nats_url,
                        nats.url(),
                        "…against the broker the refresh pointed it at"
                    );
                    assert!(
                        crate::remote::pairing::load_creds().is_some(),
                        "a ready bridge keeps its pairing on disk"
                    );
                    break;
                }
                let message = format!(
                    "the ready arm never ran; last status {status:?}, last error {:?}",
                    status.reason
                );
                assert!(std::time::Instant::now() < deadline, "{message}");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            // Leave no bridge behind: the remote fixtures assert on an empty slot, and
            // the local web listener takes the *fixed* port 8022, which the rest of the
            // suite also needs. `remote::stop()` tears the generation down
            // asynchronously, so wait (bounded) until the port is bindable again — the
            // peers' own bridge test does the same, and skipping it would make this
            // fixture a contributor to the suite's port-8022 flake.
            crate::remote::stop();
            crate::runtime::cancel_test_tasks();
            let freed = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while std::net::TcpListener::bind(("127.0.0.1", 8022)).is_err() {
                assert!(
                    std::time::Instant::now() < freed,
                    "the web listener never released port 8022"
                );
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            drop((platform, nats));
        }

        /// The startup steps that need no window really run on the app they are
        /// handed — and the lock they take is *published as managed state*, which
        /// is what makes a second Desktop refuse to start rather than run beside
        /// this one.
        #[test]
        fn process_state_initializes_without_a_window() {
            use tauri::Manager;

            let _home = crate::auth_store::test_support::HomeGuard::new("process-state");
            let app = tauri::test::mock_app();
            initialize_process_state(&app).expect("initialize process state");

            // (1) The shared GUI/headless lock: the file exists under this HOME and
            // the *live* guard is managed on the app — a second acquisition of the
            // same data directory is refused, which is the property that stops two
            // Desktops, so the assertion is about the lock, not about a path.
            let data_directory = future_app_settings::app_dir().expect("app dir under HOME");
            // Hoisted: a format argument is evaluated only when the assertion
            // fails, so a message built inside it leaves its own line unexecuted
            // in a passing run (the trap §6.6 records).
            let lock_missing = format!("the lock file must exist at {}", data_directory.display());
            assert!(
                data_directory.join("desktop.lock").is_file(),
                "{lock_missing}"
            );
            assert!(
                app.try_state::<instance::InstanceGuard>().is_some(),
                "the acquired guard must be published as managed state"
            );
            let refused = instance::InstanceGuard::acquire()
                .err()
                .expect("the data directory is already owned by this app")
                .to_string();
            assert!(refused.contains("Desktop is already running"), "{refused}");

            // (2) The host log: `start_logging` opened the file this host reports
            // as its log, so a startup failure after this point has somewhere to go.
            let log = crate::host_environment::log_file_path();
            let log_missing = format!("the host log must exist at {}", log.display());
            assert!(log.is_file(), "{log_missing}");

            // (3) The app store: the database behind it exists and answers.
            let database = future_app_settings::app_db_path().expect("db path under HOME");
            let database_missing = format!("the database must exist at {}", database.display());
            assert!(database.is_file(), "{database_missing}");

            // (4) The terminal listener: bound (already memoised when a sibling
            // test bound it), and reported by the same accessor the webview uses.
            assert!(
                terminal::server::info().is_some(),
                "the terminal listener must be bound after startup"
            );
        }

        /// `Ready` starts the window's background work on the handle the event
        /// carries, and puts the terminal listener into serve mode. Both halves are
        /// asserted through an effect of the arm, not through the call itself.
        ///
        /// The Agent assertion is deliberately *not* "an observer attached": the
        /// global observers are once-per-process (`session_events::STARTED` and
        /// friends) and a sibling test that already started them leaves this one
        /// with no observable, which made an earlier draft of this fixture
        /// order-dependent. The launch path's own post-readiness reconciliation is
        /// created fresh by `start_backend_background`, so it runs here whatever
        /// earlier tests did — and it asks the Agent for its session list.
        #[test]
        fn the_ready_event_starts_the_window_work_and_serves_the_terminal() {
            let _home = crate::auth_store::test_support::HomeGuard::new("run-event-ready");
            let mock = crate::agent_bridge::test_support::mock_agent();
            crate::store::initialize_app_store().expect("init store");
            // A sibling test can have frozen the process-global task registry
            // (`perform_stop` does), and `spawn_native` is then a no-op — the very
            // line under test would not run. Open it, exactly as the shutdown
            // fixture does.
            crate::runtime::cancel_test_tasks();
            let app = tauri::test::mock_app();
            // `run_with` binds before the event loop starts, and `serve` is
            // one-shot per process, so this test owns the transition it asserts.
            let _bound = terminal::server::bind().expect("bind the terminal listener");
            // Attribution: the assertion below requires a *new* session-list
            // command to arrive after this dispatch, so a leftover reconciliation
            // from an earlier test cannot satisfy it.
            let before = mock.requests_of("list_session_ids").len();
            dispatch_run_event(app.handle(), tauri::RunEvent::Ready);

            // (1) The arm served the listener it was handed. `serve` can only
            // report this error after an accept loop took the listener, and the
            // loop that is live here is the one this dispatch started.
            let served_again = terminal::server::serve();
            let error = served_again.expect_err("the Ready arm must have started the accept loop");
            assert!(error.contains("already serving"), "{error}");

            // (2) The spawned native job is the launch path: supervision (a no-op
            // in a test build) then the window's background work, whose
            // reconciliation asks the Agent which sessions are still live. The mock
            // records every command it is asked for, and its log started empty for
            // this test.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while mock.requests_of("list_session_ids").len() == before {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the Ready event never started the window's background work"
                );
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            // Let the startup maintenance job finish before this test's HOME goes
            // away (the window it needs is the same one the assertion above waited
            // through).
            crate::runtime::cancel_test_tasks();
        }

        /// `Exit` reaches the shutdown barrier: producer admission closes and the
        /// stop operation is published, naming the reason the event gave.
        #[test]
        fn the_exit_event_closes_admission_and_publishes_the_stop() {
            let _home = crate::auth_store::test_support::HomeGuard::new("run-event-exit");
            let _mock = crate::agent_bridge::test_support::mock_agent();
            // Start from a known barrier state: this is the same flag a sibling
            // shutdown fixture deliberately leaves closed.
            crate::desktop_shutdown::reset_for_test();
            assert!(
                !crate::desktop_shutdown::is_stopping(),
                "the fixture needs an open barrier to observe the transition"
            );
            let app = tauri::test::mock_app();
            // An event this build has no arm for must change nothing: it is the
            // `_ => {}` arm Tauri's `#[non_exhaustive]` `RunEvent` forces, and it
            // must not start the launch path or close admission on the way past.
            dispatch_run_event(app.handle(), tauri::RunEvent::MainEventsCleared);
            assert!(
                !crate::desktop_shutdown::is_stopping(),
                "the wildcard arm must not close admission"
            );
            assert!(
                crate::desktop_shutdown::snapshot().is_none(),
                "the wildcard arm must not publish a stop"
            );
            dispatch_run_event(app.handle(), tauri::RunEvent::Exit);

            assert!(
                crate::desktop_shutdown::is_stopping(),
                "the exit event must close producer admission"
            );
            let published = crate::desktop_shutdown::snapshot().expect("a published stop");
            assert_eq!(published.reason, "exit", "the barrier names its reason");

            // The worker runs concurrently with this assertion; `reset_for_test`
            // waits for it (it asserts it finished) and clears the operation, so
            // later tests do not inherit a stopping Desktop.
            crate::desktop_shutdown::reset_for_test();
            assert!(
                !crate::desktop_shutdown::is_stopping(),
                "the barrier must be releasable for the tests after this one"
            );
        }

        /// `activate_main_window` must reach the window's three activation calls:
        /// restoring, showing and focusing is what a second instance (or a launch
        /// that created the window hidden for icon setup) needs. The mock runtime
        /// answers `Ok(())` to `unminimize`, `show` and `set_focus`
        /// (`tauri-2.11.5/src/test/mock_runtime.rs`), so the `Some` arm runs here;
        /// the `Err` bodies need a window that fails those calls and stay
        /// unreachable in this environment. What is observable is the window
        /// itself: activation must leave it in place under its label.
        #[test]
        fn activate_main_window_restores_shows_and_focuses_the_main_window() {
            use tauri::Manager;
            let app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .expect("build mock app");
            let _window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
                .build()
                .expect("build webview");
            activate_main_window(app.handle());
            assert!(
                app.handle().get_webview_window("main").is_some(),
                "activation must leave the main window in place"
            );
        }

        /// The other side of the same branch: with no "main" window the function
        /// reports and returns instead of panicking. Exercised explicitly so the
        /// `None` arm does not depend on which other test calls the function.
        #[test]
        fn activate_main_window_tolerates_a_missing_main_window() {
            use tauri::Manager;
            let app = tauri::test::mock_app();
            assert!(
                app.handle().get_webview_window("main").is_none(),
                "the fixture must start without a main window"
            );
            activate_main_window(app.handle());
        }

        /// `ExitRequested` from the **mock runtime's own loop**, with the real
        /// `tauri::ExitRequestApi` it constructs.
        ///
        /// The record waived these six lines because "`ExitRequestApi` has private fields and
        /// only Tauri's own event loop constructs it". The construction half is true and
        /// irrelevant: `tauri::test::mock_app()`'s loop builds the event and hands the API to
        /// the callback (`tauri-2.11.5/src/test/mock_runtime.rs`), so the production arm
        /// receives exactly the value it receives in a real app. Destroying the only window is
        /// what makes the mock loop request an exit.
        ///
        /// The Proceed arm is the one this test takes: no conversation is running here, so
        /// `on_close_requested` answers Proceed and — because the exit is not yet authorised —
        /// the handler vetoes the exit and starts the guarded quit (`prevent_exit` +
        /// `request_quit`), which publishes a stop named `quit`. The panic is not the subject:
        /// it is how a probe leaves Tauri's loop after `prevent_exit` (the loop would otherwise
        /// keep running), and it is caught.
        // macOS only, measured: the arm runs on both platforms (the probe panic proves
        // `dispatch_run_event` was reached), but on the headless Linux runner
        // `agent_supervisor::on_close_requested()` takes the CONFIRM path - there is no
        // desktop session to show the dialog in - so the Proceed branch that publishes
        // the stop never executes and `snapshot()` stays None. Exercising Proceed there
        // needs a session, not a different assertion.
        //
        // **Why the body runs in a child process.** `exit_authorized()` reads a
        // process-global (`QUIT_CONFIRMED`) that `agent_supervisor::tests::
        // an_authorized_exit_proceeds_without_a_dialog` sets, and libtest runs the
        // `agent_supervisor::` module before `gui::` in the same process. With the flag
        // already set the Proceed arm is still taken, but its `if !exit_authorized()` is
        // false, so `prevent_exit`/`request_quit` - the two rows this test exists for -
        // are skipped and no stop is published. That is a test-isolation defect, not an
        // arm defect: the flag has no reset, and `agent_supervisor.rs` is outside this
        // unit's write set, so the only state this test can control is a fresh process.
        // The child is the same binary with `--exact`, so it runs this body only, and its
        // profile merges like any spawned fixture's (the pty launcher tests rely on the
        // same property).
        #[cfg(target_os = "macos")]
        #[test]
        fn an_exit_requested_from_the_mock_loop_takes_the_guarded_quit_arm() {
            const CHILD: &str = "FUTUREOS_EXIT_REQUESTED_CHILD";
            let name = "gui::startup_paths_tests::an_exit_requested_from_the_mock_loop_takes_the_guarded_quit_arm";
            if std::env::var_os(CHILD).is_none() {
                let output = std::process::Command::new(
                    std::env::current_exe().expect("the test binary's own path"),
                )
                .args(["--exact", name, "--nocapture", "--test-threads=1"])
                .env(CHILD, "1")
                .output()
                .expect("re-run this test in a fresh process");
                // One line on purpose: an assertion whose message spans lines leaves the
                // continuation lines uncovered in the report (this test is inside the
                // measured scope), and `{output:?}` carries both streams.
                assert!(output.status.success(), "the child run failed: {output:?}");
                return;
            }
            let _home = crate::auth_store::test_support::HomeGuard::new("run-event-exit-requested");
            crate::desktop_shutdown::reset_for_test();
            let app = tauri::test::mock_app();
            let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
                .build()
                .expect("a mock window");
            let destroyer = window.clone();
            std::thread::spawn(move || {
                // Long enough for `run` to enter its loop, short enough that the test stays
                // inside the suite's budget.
                std::thread::sleep(std::time::Duration::from_millis(250));
                let _ = destroyer.destroy();
            });

            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                app.run(move |handle, event| {
                    if let tauri::RunEvent::ExitRequested { .. } = &event {
                        dispatch_run_event(handle, event);
                        panic!("EXIT-REQUESTED-PROBE: the arm ran");
                    }
                });
            }));
            let error = outcome.expect_err("the probe panics out of the mock loop");
            let message = error.downcast_ref::<&str>().copied().unwrap_or_default();
            assert!(
                message.contains("EXIT-REQUESTED-PROBE"),
                "the loop must have delivered ExitRequested to the handler: {message}"
            );

            // The arm's effect, read the way `stop()` reads it: `request_quit` linearizes
            // the operation into `OPERATION` before it returns (coordinator::begin_with_budget
            // assigns `*current = Some(...)` on the calling thread), so the snapshot is
            // already there — no poll, and no uncovered wait loop in the measured region.
            let published =
                crate::desktop_shutdown::snapshot().expect("the Proceed arm published a stop");
            assert_eq!(published.reason, "quit", "a guarded quit names itself");
            // Leave the barrier and the operation clear for the tests after this one.
            crate::desktop_shutdown::reset_for_test();
        }

        /// `Ready` after a quit has already begun: the launch path is spawned, but
        /// the closing admission must stop it before it starts the window's work.
        /// This is the `return` guarded by `is_stopping()` in `dispatch_run_event`.
        ///
        /// The only production input is a stop that lands between readiness and the
        /// check — a race — so the fixture closes the same process-global flag
        /// through the test-only guard in `desktop_shutdown` (nothing in production
        /// may call it) and drops the guard once the window has passed.
        ///
        /// The assertion is the *absence* of the work, so it has to outlast the
        /// work's own startup latency: the background path's first Agent query is
        /// the post-readiness reconciliation, which sleeps three seconds in
        /// `start_backend_background`. Waiting past that window is what turns
        /// "nothing happened yet" into evidence;
        /// `the_ready_event_starts_the_window_work_and_serves_the_terminal` is the
        /// positive control for the same request in the same fixture shape.
        #[test]
        fn quit_admission_closed_before_the_ready_event_withholds_the_window_work() {
            let _home = crate::auth_store::test_support::HomeGuard::new("run-event-ready-stopping");
            let mock = crate::agent_bridge::test_support::mock_agent();
            crate::store::initialize_app_store().expect("init store");
            // Open the process-global task registry: a sibling shutdown fixture may
            // have frozen it, and `spawn_native` would then be a no-op instead of
            // running the arm under test.
            crate::runtime::cancel_test_tasks();
            let app = tauri::test::mock_app();
            let before = mock.requests_of("list_session_ids").len();
            let closed = crate::desktop_shutdown::close_admission_guard_for_test();
            assert!(
                crate::desktop_shutdown::is_stopping(),
                "the fixture closes admission"
            );
            dispatch_run_event(app.handle(), tauri::RunEvent::Ready);
            // The spawned job runs on its own thread; this window is longer than the
            // three-second reconciliation it would start if it fell through.
            std::thread::sleep(std::time::Duration::from_millis(4_500));
            let started = mock.requests_of("list_session_ids").len();
            drop(closed);
            // Drain whatever the positive-control path would have started, so this
            // fixture cannot leave a loop behind for the next HOME.
            crate::runtime::cancel_test_tasks();
            assert_eq!(
                started, before,
                "a closed admission must stop the Ready arm before it starts the window's work"
            );
        }

        /// `ExitRequested` while the Agent is still `Starting` answers
        /// `QuitDecision::Confirm { open_dialog: true }`, so the arm that vetoes the
        /// exit and asks the user runs — including `confirm_quit`, which arms the
        /// dialog flag and builds the native dialog.
        ///
        /// The arm cannot be driven from a lib test: `ExitRequestApi` has private
        /// fields and only Tauri's event loop constructs it, and the process-global
        /// Agent host has no test setter for `Starting`. A fresh process supplies
        /// both: it holds a TCP peer that accepts and never answers, so the startup
        /// probe stays in flight and the host publishes `Starting`, then lets the
        /// mock runtime's own loop deliver `ExitRequested` (the window destroy
        /// pattern `an_exit_requested_from_the_mock_loop_takes_the_guarded_quit_arm`
        /// already uses).
        ///
        /// **The dialog is contained, not answered.** `confirm_quit` builds the
        /// message and hands `show` to `run_on_main_thread`; on the mock runtime that
        /// is a queue the loop would drain on its next iteration, and the probe
        /// panics out of the callback before it can. Nothing reaches `rfd`, so a host
        /// with no window server cannot hang here — this fixture never opens a real
        /// dialog.
        #[cfg(target_os = "macos")]
        #[test]
        fn quit_admission_exit_requested_while_the_agent_is_starting_confirms_the_quit() {
            const CHILD: &str = "FUTUREOS_QUIT_CONFIRM_CHILD";
            const ARM: &str = "QUIT-CONFIRM-ARM";
            let name = "gui::startup_paths_tests::quit_admission_exit_requested_while_the_agent_is_starting_confirms_the_quit";
            if std::env::var_os(CHILD).is_none() {
                let output = std::process::Command::new(
                    std::env::current_exe().expect("the test binary's own path"),
                )
                .args(["--exact", name, "--nocapture", "--test-threads=1"])
                .env(CHILD, "1")
                .output()
                .expect("re-run this test in a fresh process");
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                assert!(output.status.success(), "the child run failed: {stderr}");
                // The marker proves the child reached the arm; a child that returned
                // before it would otherwise look like a pass (F.113).
                assert!(
                    stderr.contains(ARM),
                    "the child never reached the arm: {stderr}"
                );
                return;
            }

            let _home = crate::auth_store::test_support::HomeGuard::new("quit-confirm-arm");
            crate::desktop_shutdown::reset_for_test();
            // A peer that accepts and never answers keeps the startup probe in
            // flight, which is exactly the `Phase::Starting` window the arm needs.
            let endpoint = std::net::TcpListener::bind("127.0.0.1:0").expect("probe endpoint");
            endpoint.set_nonblocking(true).expect("nonblocking probe");
            let address = endpoint.local_addr().expect("probe address");
            std::thread::spawn(move || {
                // The listener stays alive and unanswered until this process exits.
                let _hold = endpoint;
                let _ = crate::agent_host::host().start(format!("http://{address}"));
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while crate::agent_host::host().phase() != crate::agent_host::Phase::Starting {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the host never published `Starting`"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            // The real registration, so `app.dialog()` finds the plugin's state.
            // The identifier is load-bearing: the single-instance plugin derives its
            // socket from it, and the mock context's empty one is shared by every
            // process on the host (`the_desktop_registration_builds_for_any_runtime`
            // records the silent-exit failure that causes).
            let builder =
                gui_builder::<tauri::test::MockRuntime>().enable_macos_default_menu(false);
            let mut context = tauri::test::mock_context(tauri::test::noop_assets());
            context.config_mut().identifier =
                format!("com.futureos.quitconfirm.p{}", std::process::id());
            context.config_mut().plugins.0.insert(
                "updater".to_string(),
                serde_json::json!({ "pubkey": "fixture", "endpoints": [] }),
            );
            let app = builder.build(context).expect("build the mock app");
            let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
                .build()
                .expect("a mock window");
            let destroyer = window.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(250));
                let _ = destroyer.destroy();
            });

            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                app.run(move |handle, event| {
                    if let tauri::RunEvent::ExitRequested { .. } = &event {
                        dispatch_run_event(handle, event);
                        // A literal payload: this value is downcast as `&str` below,
                        // and a formatted panic would arrive as a `String`.
                        panic!("QUIT-CONFIRM-ARM: the Confirm arm ran");
                    }
                });
            }));
            let error = outcome.expect_err("the probe panics out of the mock loop");
            let message = error.downcast_ref::<&str>().copied().unwrap_or_default();
            assert!(
                message.contains("QUIT-CONFIRM-ARM"),
                "the loop must have delivered ExitRequested to the handler: {message}"
            );

            // The arm's own effect, read through the public decision: `confirm_quit`
            // sets the dialog flag before building the dialog, and a second question
            // therefore answers "confirm, already asking" instead of queueing a
            // second dialog.
            match crate::agent_supervisor::on_close_requested() {
                crate::agent_supervisor::QuitDecision::Confirm { open_dialog } => {
                    assert!(!open_dialog, "confirm_quit must have armed the dialog flag")
                }
                decision => panic!("the Confirm arm must have run: {decision:?}"),
            }
            eprintln!("{ARM}: the stopping-process confirm path executed");
        }
    }
}

#[cfg(test)]
mod helper_dispatch_tests {
    /// Two callers reach this dispatcher with the same request spelled
    /// differently: a launcher passes its own `argv` unchanged, while a sandbox
    /// child arrives behind the `agent` subcommand token. Stripping that token is
    /// the whole of `dispatch_agent_helper`'s parsing, so both spellings must end
    /// up in the same place — an unstripped `agent` would be read as the helper's
    /// own positional argument and silently change what runs.
    ///
    /// `--linux-sandbox-helper` is Linux-only: on every other platform the helper
    /// CLI refuses the request (agent/src/cli.rs:213-218), which is the answer
    /// asserted here. On Linux the same argv would execute the real helper
    /// instead, so this comparison is not a portable test — see
    /// docs/testing/module-tauri-shell.md for the `platform-unmeasured` row that
    /// covers the Linux success arm.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn an_agent_subcommand_prefix_reaches_the_same_helper_request() {
        let request: Vec<String> = ["--linux-sandbox-helper", "probe"]
            .iter()
            .map(|part| (*part).to_string())
            .collect();
        let direct = super::dispatch_agent_helper(&request).expect("the flag dispatches");
        let mut prefixed_args = vec!["agent".to_string()];
        prefixed_args.extend(request.iter().cloned());
        let prefixed = super::dispatch_agent_helper(&prefixed_args).expect("the flag dispatches");

        assert!(
            direct.is_err(),
            "off Linux the helper CLI refuses the request instead of running it"
        );
        assert_eq!(
            direct.map_err(|error| error.to_string()),
            prefixed.map_err(|error| error.to_string()),
            "the `agent` prefix must be stripped before the helper CLI parses the request"
        );
    }
}
