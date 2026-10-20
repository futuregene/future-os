//! Environment fixture helpers shared by library and entry tests.
/// `cargo test` runs every test of one target in ONE process, so a value left
/// behind by a panicking assertion is inherited by unrelated tests: replacing
/// `PATH` with a fixture directory and panicking before the restore made 25
/// later tests in `commands::*`/`git_review::*` fail with `NotFound` while
/// spawning `git` (FLAKES.md item 27, measured in `docs/testing/flaky-tauri.md`).
/// Holding a `Mutex` is not a substitute — a lock prevents a race between tests,
/// it does not restore a leaked value — so a global-state test needs both.
///
/// ```text
/// let _path = EnvGuard::set("PATH", directory.as_os_str());
/// ```
#[must_use = "the previous value is restored when the guard is dropped; bind it to a variable"]
pub struct EnvGuard {
    key: &'static str,
    saved: Option<std::ffi::OsString>,
}

impl EnvGuard {
    /// Replaces the variable `key` and remembers what to put back on drop.
    pub fn set(key: &'static str, value: &std::ffi::OsStr) -> Self {
        let saved = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.saved.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

/// Canonical real-loopback RPC and temporary SQLite fixture.
pub mod agent;

pub mod provider_responses;

pub mod tasks;

/// Retained entry unit fixtures opt into their original test configuration.
/// Cargo's test-support feature alone never changes a product child process.
pub fn configure_entry_fixture() {
    crate::remote::enable_fixture_mode();
}

pub mod command_agent;
