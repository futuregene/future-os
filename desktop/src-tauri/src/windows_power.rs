//! Windows suspend/shutdown notifications, captured by subclassing Tauri's
//! main window procedure for the lifetime of the process.

use std::sync::atomic::{AtomicIsize, Ordering};
use tauri::Manager;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, SetWindowLongPtrW, GWLP_WNDPROC, PBT_APMRESUMEAUTOMATIC,
        PBT_APMSUSPEND, WM_POWERBROADCAST, WM_QUERYENDSESSION, WNDPROC,
    },
};

static ORIGINAL_WNDPROC: AtomicIsize = AtomicIsize::new(0);

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
    if message == WM_POWERBROADCAST && wparam.0 == PBT_APMSUSPEND as usize {
        tauri::async_runtime::block_on(crate::remote::handle_system_suspend());
    } else if message == WM_POWERBROADCAST && wparam.0 == PBT_APMRESUMEAUTOMATIC as usize {
        crate::remote::handle_system_resume();
    } else if message == WM_QUERYENDSESSION {
        notify("system_power_off");
    }

    let previous = ORIGINAL_WNDPROC.load(Ordering::Acquire);
    if previous == 0 {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    } else {
        let previous: WNDPROC = unsafe { std::mem::transmute(previous) };
        unsafe { CallWindowProcW(previous, hwnd, message, wparam, lparam) }
    }
}

pub fn install_disconnect_notifier(app: &tauri::AppHandle) {
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
