//! future_tui — Rust port of the TypeScript future-tui.
//!
//! Goal: 1:1 behavior — UI rendering, key handling, interaction, argument
//! parsing, and help text — with a self-implemented terminal backend
//! (`terminal.rs`, libc on POSIX / windows-sys planned for Windows), replacing
//! Node's `process.stdin`/`process.stdout` machinery.
//!
//! P0 modules (pure logic, ported 1:1 from `tui/src/*.ts`):
//!   - `stdin_buffer` — escape-sequence buffering / bracketed paste
//!   - `keys`         — raw bytes → normalized key ids (Kitty CSI-u,
//!     modifyOtherKeys, legacy)
//!   - `theme`        — color constants, style helpers
//!   - `utils`        — grapheme width, ANSI tracking, wrap/slice/truncate
//!   - `terminal`     — self-implemented POSIX terminal backend
//!   - `help`         — verbatim `--help` text (index.ts `printHelp`)
//!   - `version`      — build-injected version (scripts/version.mjs)
//!
//! P1 modules (components layer, ported 1:1 from `tui/src/components/*.ts` +
//! `tui/src/tui.ts` + `tui/src/help-screen.ts` + `tui/src/keybindings.ts`):
//!   - `tui`              — Component architecture, constants, overlay layout
//!   - `components`       — input / autocomplete / select-list /
//!     scoped-models-selector / footer
//!   - `help_screen`      — `renderHelp` card
//!   - `keybindings`      — KeybindingManager
//!   - `rpc`              — RPC types (ModelInfo for the selector)
//!
//! P3 modules (app layer, ported 1:1 from `tui/src/app.ts` +
//! `tui/src/index.ts` + `tui/src/rpc/grpc-client.ts`):
//!   - `rpc`              — full types + tonic `GrpcClient`
//!   - `app`              — the App (session orchestration, slash commands,
//!     overlays, diff-based render pipeline)
//!   - `index`            — CLI arg parsing / print mode / list-models /
//!     interactive wiring (`main.rs` calls `index::run`)
//!   - `rpc`              — full types + tonic `GrpcClient` (wire types via
//!     the future-rpc crate — the single proto codegen owner, PR #112)
//!   - `insert_history`   — finished transcript rows → the terminal's own
//!     scrollback (leave the alternate screen, append, come back)

pub mod agent_host;
pub mod agent_supervisor;
pub mod app;
pub mod clipboard;
pub mod components;
#[cfg(all(test, windows))]
pub mod console_harness;
pub mod crash;
pub mod external_editor;
pub mod help;
pub mod help_screen;
pub mod home;
pub mod index;
pub mod insert_history;
pub mod keybindings;
pub mod keys;
pub mod notifications;
pub mod paste;
pub mod paste_burst;
pub mod rpc;
pub mod session_rows;
pub mod skill_reco;
pub mod skills_cli;
pub mod stderr_log;
pub mod stdin_buffer;
pub mod terminal;
pub mod terminal_image;
pub mod theme;
pub mod themes;
pub mod tui;
mod update;
pub mod utils;
pub mod version;
pub mod workbench;
pub mod worktree;

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
