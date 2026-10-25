//! future_tui — the FutureOS terminal UI.
//!
//! One surface: the full-screen **workbench** (`workbench/`), started by
//! `future tui` and by the `future-tui` binary. It owns the screen, repaints
//! absolute rows, and answers mouse clicks from what it painted; the agent is
//! reached through `rpc/`, never directly.
//!
//! What the crate provides, bottom-up:
//!
//! * `terminal` (+ `terminal_posix` / `terminal_windows`, `stdin_buffer`,
//!   `paste_burst`) — the self-implemented terminal backend: raw mode, the
//!   alternate screen, the input reader thread, resizes and signals.
//! * `keys` — raw bytes → normalised key ids (Kitty CSI-u, modifyOtherKeys,
//!   legacy).
//! * `utils`, `theme`, `themes`, `tui` — grapheme/ANSI width, palettes, the
//!   `Component` trait the renderers below share.
//! * `components` — the transcript (`chat_area`, with `diff` + `markdown`) and
//!   the prompt box (`input`). Shared with nothing: the workbench is the only
//!   consumer, and it draws them into rectangles.
//! * `workbench` — the surface: layout, palette, hit map, mouse protocol,
//!   panels, launchpad, and the driver `future tui` runs.
//! * `rpc` — wire types and the tonic `GrpcClient` (proto codegen lives in
//!   `future-rpc`, the single owner).
//! * `session_rows` — journal entries → chat messages, the one mapping both the
//!   live path and history replay use.
//! * `index` — argument parsing, `--help`, print mode (`-p`), `--list-models`,
//!   and the entry into the workbench.

pub mod agent_attach;
pub mod approval_mode;
pub mod components;
pub mod crash;
pub mod help;
pub mod home;
pub mod index;
pub mod keys;
pub mod paste;
pub mod paste_burst;
pub mod rpc;
pub mod session_rows;
pub mod settings;
pub mod stderr_log;
pub mod stdin_buffer;
pub mod subprocess;
pub mod terminal;
pub mod terminal_image;
pub mod theme;
pub mod themes;
pub mod tui;
mod update;
pub mod utils;
pub mod version;
pub mod workbench;

/// Shared lock for tests that mutate process-global state (env vars), so
/// parallel `cargo test` threads cannot race each other. Same pattern as
/// `cli/rust/src/test_env.rs`.
#[cfg(test)]
pub mod test_env {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    pub static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Lock ignoring poisoning — a panicking test must not cascade-fail
    /// every other test that mutates process-global state.
    pub fn lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Set a process-global environment variable and put it back **when the guard
    /// drops — including when the test panics**.
    ///
    /// This exists because setting a variable and restoring it on the success path
    /// is *destructive*, not merely untidy (`FLAKES.md` §4c): `PATH`/`HOME`/`FUTURE_*`
    /// are process-global, `cargo test` runs every test of a binary in threads of one
    /// process, and a mutation that survives a panic fails every *later* test in that
    /// binary for reasons of its own. `FLAKES.md` item 27 measured the shape: one
    /// leaked `PATH` produced **27 cascading failures** and voided an instrumented
    /// run. The lock above does **not** help — a mutex is not a restore; it stops the
    /// race, not the leak.
    pub struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        /// Set `key` for as long as the guard lives.
        pub fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }

        /// Remove `key` for as long as the guard lives.
        pub fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    /// An absolute path to the external tool `name`.
    ///
    /// A test that spawns a program must not let a *peer test's* legitimate — and
    /// still-racy — swap of the process-global `PATH` decide whether its own child
    /// starts. Measured cost of getting this wrong: `FLAKES.md` item 24, where a
    /// sibling test's `PATH` swap made a real-`git` fixture fail with
    /// `Os { code: 2, kind: NotFound }` — or, when the app's own spawn was the
    /// victim, with a 120 s pump budget expiring instead, at **3 of 12 concurrent
    /// whole-suite runs (137-147 s each instead of 25-30 s)**.
    ///
    /// Resolution order: the platform's standard locations first (so a swapped
    /// `PATH` cannot change the answer), then whatever `PATH` says, then the bare
    /// name (last resort — the old behaviour, for an exotic layout).
    pub fn absolute_tool(name: &str) -> PathBuf {
        let exe_suffix = if cfg!(windows) { ".exe" } else { "" };
        let dirs: &[&str] = if cfg!(windows) {
            &[
                r"C:\Windows\System32",
                r"C:\Windows\System32\WindowsPowerShell\v1.0",
                r"C:\Program Files\Git\cmd",
                r"C:\Program Files\Git\bin",
                r"C:\Program Files\Git\usr\bin",
            ]
        } else {
            &["/usr/bin", "/bin", "/usr/local/bin", "/opt/homebrew/bin"]
        };
        for dir in dirs {
            let candidate = Path::new(dir).join(format!("{name}{exe_suffix}"));
            if candidate.is_file() {
                return candidate;
            }
        }
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                let candidate = dir.join(format!("{name}{exe_suffix}"));
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
        PathBuf::from(format!("{name}{exe_suffix}"))
    }

    #[cfg(test)]
    mod tests {
        /// The guard must put the value back **when the test panics**, which is the
        /// whole reason it exists (`FLAKES.md` §4c). The lock below is held for the
        /// assertion's benefit — it is not what makes this safe, and taking it twice
        /// on one thread would deadlock, so a helper that locks must not be called
        /// from a test that already holds it.
        #[test]
        fn env_var_guard_restores_the_value_after_a_panic() {
            let _g = super::lock();
            let before = std::env::var_os("FUTURE_TUI_GUARD_PROBE");
            let caught = std::panic::catch_unwind(|| {
                let _value = super::EnvVarGuard::set("FUTURE_TUI_GUARD_PROBE", "swapped");
                assert_eq!(
                    std::env::var("FUTURE_TUI_GUARD_PROBE").as_deref(),
                    Ok("swapped")
                );
                panic!("the body panics");
            });
            assert!(caught.is_err(), "the body must have panicked");
            assert_eq!(
                std::env::var_os("FUTURE_TUI_GUARD_PROBE"),
                before,
                "the guard must restore the value while unwinding"
            );
        }

        /// An absolute tool path does not depend on the current `PATH`.
        #[test]
        fn absolute_tool_ignores_a_swapped_path() {
            let _g = super::lock();
            let _swapped = super::EnvVarGuard::set("PATH", "/nonexistent-stub-dir");
            let resolved = super::absolute_tool("sh");
            assert!(
                resolved.is_absolute() && resolved.is_file(),
                "the standard locations must answer even with PATH hijacked: {resolved:?}"
            );
        }

        #[test]
        fn lock_survives_poisoning() {
            // Poison ENV_LOCK (panic while held), then re-acquire: the
            // unwrap_or_else arm must recover the guard.
            let _ = std::panic::catch_unwind(|| {
                let _g = super::lock();
                panic!("intentional poison");
            });
            let _g = super::lock();
        }
    }
}
