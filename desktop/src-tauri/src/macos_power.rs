//! macOS sleep/shutdown notifications used to promptly mark the paired phone
//! offline. The OS may still terminate us before a broker round-trip finishes,
//! so mobile heartbeat expiry remains the failure fallback.

use block2::RcBlock;
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillPowerOffNotification,
    NSWorkspaceWillSleepNotification,
};
use objc2_foundation::NSNotification;
use std::ptr::NonNull;

/// Subscribe once, on Tauri's main thread, to the system's pre-sleep and
/// pre-power-off notifications. NSWorkspace retains the observer blocks for
/// the lifetime of the process.
pub fn install_disconnect_notifier() {
    let workspace = NSWorkspace::sharedWorkspace();
    let center = workspace.notificationCenter();

    let sleep = RcBlock::new(move |_: NonNull<NSNotification>| {
        tauri::async_runtime::block_on(async {
            crate::remote::handle_system_suspend().await;
        });
    });
    let wake = RcBlock::new(move |_: NonNull<NSNotification>| {
        crate::remote::handle_system_resume();
    });
    let power_off = RcBlock::new(move |_: NonNull<NSNotification>| {
        tauri::async_runtime::block_on(async {
            crate::remote::notify_mobile_disconnect("system_power_off").await;
        });
    });

    unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceWillSleepNotification),
            None,
            None,
            &sleep,
        );
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceWillPowerOffNotification),
            None,
            None,
            &power_off,
        );
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidWakeNotification),
            None,
            None,
            &wake,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Register the three observers once. The observer blocks themselves only
    /// fire on real sleep/wake/power-off notifications (impossible to drive in
    /// a unit test), so their bodies stay W7; the registration path is
    /// exercised here.
    #[test]
    fn install_disconnect_notifier_registers_observers() {
        install_disconnect_notifier();
    }
}
