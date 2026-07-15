// Hide the console window on Windows release builds — without this, launching
// the GUI also pops up a terminal showing the app/agent logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Suppress macOS system-framework debug chatter (TSM AdjustCapsLockLED,
    // IMKCFRunLoopWakeUpReliable) that WKWebView-based apps trigger but
    // cannot fix — harmless input-method internal logging from Apple frameworks.
    #[cfg(target_os = "macos")]
    std::env::set_var("OS_ACTIVITY_MODE", "disable");
    futureos_lib::run()
}
