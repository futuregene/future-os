// Hide the console window on Windows release builds — without this, launching
// the GUI also pops up a terminal showing the app/agent logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(test))]
fn main() {
    configure_environment();
    futureos_lib::run()
}

/// Suppress macOS system-framework debug chatter (TSM AdjustCapsLockLED,
/// IMKCFRunLoopWakeUpReliable) that WKWebView-based apps trigger but cannot
/// fix — harmless input-method internal logging from Apple frameworks.
fn configure_environment() {
    #[cfg(target_os = "macos")]
    std::env::set_var("OS_ACTIVITY_MODE", "disable");
}

#[cfg(test)]
mod tests {
    use super::configure_environment;

    #[test]
    fn configure_environment_runs_without_panicking() {
        // The macOS-only `set_var` is the only launch-time setup in this
        // binary; the real `main` (which starts the blocking GUI) is compiled
        // out in tests via `#[cfg(not(test))]`.
        configure_environment();
    }
}
