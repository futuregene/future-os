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
    use objc2_app_kit::{
        NSWorkspaceDidWakeNotification, NSWorkspaceWillPowerOffNotification,
        NSWorkspaceWillSleepNotification,
    };

    /// Register the three observers once. The observer blocks themselves only
    /// fire on real sleep/wake/power-off notifications (impossible to drive in
    /// a unit test), so their bodies stay W7; the registration path is
    /// exercised here.
    #[test]
    fn install_disconnect_notifier_registers_observers() {
        install_disconnect_notifier();
    }

    /// **Posting the notification runs the observer body**, which is what the
    /// waived rows actually claim cannot happen (`docs/testing/module-tauri-shell.md`
    /// §S12.5; the review that falsified the waiver is §3.1 of
    /// `review-tauri-shell.md`).
    ///
    /// The three blocks are registered with `-[NSNotificationCenter
    /// addObserverForName:object:queue:usingBlock:]` on a nil object and a nil
    /// queue, i.e. with the *posting* thread as the delivery queue: posting the
    /// same names to the same centre invokes them synchronously, on whatever
    /// thread posts. No OS power event, no session state, and no seam is
    /// involved — `NSNotificationCenter` is a plain in-process dispatcher.
    ///
    /// What the bodies do here is deliberately only the first statement: this
    /// process has no `RemoteState`, so `handle_system_suspend`/`resume` and
    /// `notify_mobile_disconnect` return immediately. The rows are statements
    /// inside those blocks, so entering the block is exactly the coverage they
    /// describe.
    #[test]
    fn posting_the_workspace_notifications_runs_the_observer_bodies() {
        install_disconnect_notifier();
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        // SAFETY: the names come from the same AppKit bindings the registration
        // uses, and `None` matches an observer registered with a nil object —
        // the delivery contract of `NSNotificationCenter`. Delivery is
        // synchronous on this thread, so no sleep or wait is needed.
        unsafe {
            center.postNotificationName_object(NSWorkspaceWillSleepNotification, None);
            center.postNotificationName_object(NSWorkspaceDidWakeNotification, None);
            center.postNotificationName_object(NSWorkspaceWillPowerOffNotification, None);
        }
    }
}
