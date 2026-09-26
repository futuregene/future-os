//! Windows suspend/shutdown notifications, captured by subclassing Tauri's
//! main window procedure for the lifetime of the process.

use std::sync::atomic::{AtomicIsize, Ordering};
use tauri::Manager;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, SetWindowLongPtrW, GWLP_WNDPROC, PBT_APMRESUMEAUTOMATIC,
        PBT_APMRESUMECRITICAL, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, WM_POWERBROADCAST,
        WM_QUERYENDSESSION, WNDPROC,
    },
};

static ORIGINAL_WNDPROC: AtomicIsize = AtomicIsize::new(0);

/// What the window procedure must do for one incoming message. Kept pure — and
/// separate from the actions it names — so every platform mapping (including the
/// resume constants the OS only sends on hardware resume) is testable without a
/// real window, a running bridge, or an actual suspend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerAction {
    Suspend,
    Resume,
    Disconnect,
    Ignore,
}

fn power_action(message: u32, wparam: usize) -> PowerAction {
    if message == WM_POWERBROADCAST && wparam == PBT_APMSUSPEND as usize {
        PowerAction::Suspend
    } else if message == WM_POWERBROADCAST
        && matches!(
            wparam,
            value if value == PBT_APMRESUMEAUTOMATIC as usize
                || value == PBT_APMRESUMESUSPEND as usize
                || value == PBT_APMRESUMECRITICAL as usize
        )
    {
        PowerAction::Resume
    } else if message == WM_QUERYENDSESSION {
        PowerAction::Disconnect
    } else {
        PowerAction::Ignore
    }
}

fn notify(reason: &'static str) {
    tauri::async_runtime::block_on(async move {
        crate::remote::notify_mobile_disconnect(reason).await;
    });
}

unsafe extern "system" fn power_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match power_action(message, wparam.0) {
        PowerAction::Suspend => {
            tauri::async_runtime::block_on(crate::remote::handle_system_suspend());
        }
        PowerAction::Resume => crate::remote::handle_system_resume(),
        PowerAction::Disconnect => notify("system_power_off"),
        PowerAction::Ignore => {}
    }

    let previous = ORIGINAL_WNDPROC.load(Ordering::Acquire);
    if previous == 0 {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    } else {
        let previous: WNDPROC = unsafe { std::mem::transmute(previous) };
        unsafe { CallWindowProcW(previous, hwnd, message, wparam, lparam) }
    }
}

pub fn install_disconnect_notifier<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let previous =
        unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, power_wnd_proc as *const () as isize) };
    if previous != 0 {
        ORIGINAL_WNDPROC.store(previous, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassLongPtrW, GetDesktopWindow, GCLP_WNDPROC,
    };

    /// An unrelated message: not a power broadcast, not an end-session query.
    const UNRELATED_MESSAGE: u32 = 0x0400; // WM_USER

    #[test]
    fn power_action_maps_every_broadcast_it_cares_about() {
        // Suspend is the only broadcast that tears the generation down.
        assert_eq!(
            power_action(WM_POWERBROADCAST, PBT_APMSUSPEND as usize),
            PowerAction::Suspend
        );
        // All three resume flavours must be recognized; a firmware/hardware
        // resume reports the CRITICAL one and a missed arm would leave the
        // bridge suspended forever.
        for resume in [
            PBT_APMRESUMEAUTOMATIC,
            PBT_APMRESUMESUSPEND,
            PBT_APMRESUMECRITICAL,
        ] {
            assert_eq!(
                power_action(WM_POWERBROADCAST, resume as usize),
                PowerAction::Resume,
                "resume code {resume:#x} must be classified as a resume"
            );
        }
        // Logoff/shutdown asks us to tell the phone we are going away.
        assert_eq!(power_action(WM_QUERYENDSESSION, 0), PowerAction::Disconnect);
        // Boundary/negative cases: an unknown power broadcast, a known
        // broadcast code on the wrong message, and an unrelated message are all
        // ignored rather than acted on.
        assert_eq!(power_action(WM_POWERBROADCAST, 0xDEAD), PowerAction::Ignore);
        assert_eq!(
            power_action(UNRELATED_MESSAGE, PBT_APMSUSPEND as usize),
            PowerAction::Ignore
        );
        assert_eq!(
            power_action(WM_QUERYENDSESSION | 0x8000, 0),
            PowerAction::Ignore
        );
    }

    #[test]
    fn install_disconnect_notifier_returns_without_a_main_window() {
        let app = tauri::test::mock_app();
        // No "main" window exists → nothing to subclass.
        install_disconnect_notifier(app.handle());
        assert_eq!(ORIGINAL_WNDPROC.load(Ordering::Acquire), 0);
    }

    #[test]
    fn install_disconnect_notifier_returns_when_the_window_has_no_hwnd() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app");
        let _window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("build webview");
        // A mock webview has no native handle, so the subclassing step is
        // skipped instead of being attempted with a null handle.
        install_disconnect_notifier(app.handle());
        assert_eq!(ORIGINAL_WNDPROC.load(Ordering::Acquire), 0);
    }

    /// Drive the real window procedure. The desktop window is a always-present,
    /// valid HWND, so `DefWindowProcW`/`CallWindowProcW` are called with a handle
    /// they can actually process — no window is created and none is subclassed
    /// for the lifetime of the process (the previous-procedure slot is restored).
    #[test]
    fn power_wnd_proc_dispatches_every_action_and_both_fallbacks() {
        // The dispatch arms call into the process-global remote supervisor;
        // hold the same serialization point the bridge tests use so a suspend
        // cannot tear down a bridge another test is driving.
        let _guard = crate::remote::test_support::mock_agent_lock();
        let hwnd = unsafe { GetDesktopWindow() };
        assert!(!hwnd.is_invalid(), "the desktop window must exist");

        // Ignore arm, and the default-procedure fallback when no previous
        // procedure was captured: the answer must be the default procedure's
        // own, i.e. the message reached it unchanged instead of being swallowed.
        let ignored = unsafe { power_wnd_proc(hwnd, UNRELATED_MESSAGE, WPARAM(0), LPARAM(0)) };
        let ignored_default =
            unsafe { DefWindowProcW(hwnd, UNRELATED_MESSAGE, WPARAM(0), LPARAM(0)) };
        assert_eq!(ignored.0, ignored_default.0);

        // Query-end-session → mobile disconnect notice ("system_power_off").
        let disconnecting =
            unsafe { power_wnd_proc(hwnd, WM_QUERYENDSESSION, WPARAM(0), LPARAM(0)) };
        let disconnecting_default =
            unsafe { DefWindowProcW(hwnd, WM_QUERYENDSESSION, WPARAM(0), LPARAM(0)) };
        assert_eq!(disconnecting.0, disconnecting_default.0);

        // Suspend then resume: the pair leaves the supervisor's `suspended`
        // flag back where it started instead of stranding it while other tests
        // run.
        let suspended = unsafe {
            power_wnd_proc(
                hwnd,
                WM_POWERBROADCAST,
                WPARAM(PBT_APMSUSPEND as usize),
                LPARAM(0),
            )
        };
        let suspended_default = unsafe {
            DefWindowProcW(
                hwnd,
                WM_POWERBROADCAST,
                WPARAM(PBT_APMSUSPEND as usize),
                LPARAM(0),
            )
        };
        assert_eq!(suspended.0, suspended_default.0);
        let resumed = unsafe {
            power_wnd_proc(
                hwnd,
                WM_POWERBROADCAST,
                WPARAM(PBT_APMRESUMECRITICAL as usize),
                LPARAM(0),
            )
        };
        let resumed_default = unsafe {
            DefWindowProcW(
                hwnd,
                WM_POWERBROADCAST,
                WPARAM(PBT_APMRESUMECRITICAL as usize),
                LPARAM(0),
            )
        };
        assert_eq!(resumed.0, resumed_default.0);

        // The captured-previous arm: a real WNDPROC (the desktop window class's
        // own) is chained to instead of the default procedure, with the message
        // forwarded unchanged. Windows keeps a window's own procedure in
        // GWLP_WNDPROC and falls back to the class's, which is what a window
        // that never had `install_disconnect_notifier` run reports.
        let previous = unsafe { GetClassLongPtrW(hwnd, GCLP_WNDPROC) } as isize;
        assert_ne!(
            previous, 0,
            "the desktop window class must have a procedure"
        );
        ORIGINAL_WNDPROC.store(previous, Ordering::Release);
        let chained = unsafe { power_wnd_proc(hwnd, UNRELATED_MESSAGE, WPARAM(0), LPARAM(0)) };
        let chained_directly = unsafe {
            CallWindowProcW(
                std::mem::transmute::<isize, WNDPROC>(previous),
                hwnd,
                UNRELATED_MESSAGE,
                WPARAM(0),
                LPARAM(0),
            )
        };
        assert_eq!(chained.0, chained_directly.0);
        ORIGINAL_WNDPROC.store(0, Ordering::Release);
        assert_eq!(ORIGINAL_WNDPROC.load(Ordering::Acquire), 0);
    }
}
