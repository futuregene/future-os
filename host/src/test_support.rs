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

/// Run a process-global environment test in its own single-test process.
/// Returns true in the parent after verification, false in the isolated body.
pub fn reexec_test() -> bool {
    let thread = std::thread::current();
    let name = thread.name().expect("named libtest thread");
    const CHILD: &str = "FUTURE_HOST_ISOLATED_ENV_TEST";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        return false;
    }
    let child = {
        // Snapshot HOME while no other home fixture is publishing/restoring it.
        // The child owns its environment after spawn; do not serialize its body.
        let _lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", name, "--nocapture"])
            .env(CHILD, name)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn isolated environment test")
    };
    let output = child.wait_with_output().expect("isolated test result");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success()
            && stdout.contains("running 1 test")
            && stdout.contains("1 passed; 0 failed"),
        "isolated {name} failed: {stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

/// Canonical real-loopback RPC and temporary SQLite fixture.
pub mod agent;

pub mod provider_responses;

pub mod tasks;

pub(crate) mod timing;

/// Retained entry unit fixtures opt into their original test configuration.
/// Cargo's test-support feature alone never changes a product child process.
pub fn configure_entry_fixture() {
    crate::remote::enable_fixture_mode();
}

pub mod command_agent;
