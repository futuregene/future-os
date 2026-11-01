//! Test-only seams for the desktop shell.
//!
//! **Not part of a default build.** `lib.rs` declares this module behind
//! `#[cfg(any(test, feature = "test-support"))]` and `test-support` is not in `default`, so
//! `cargo build -p futureos` (and the shipped bundle) contains neither the module nor the
//! symbols below; the coverage lane enables the feature explicitly
//! (`cargo llvm-cov -p futureos --features test-support …`). The form mirrors
//! `future_host::test_support`, which the rest of this crate's tests already use.
//!
//! It exists because two things are private to this crate and cannot be reached from an
//! integration test target:
//!
//! * the process-global [`app_handle`] that `run_with`'s `setup` hook publishes. The IPC
//!   command signatures take the concrete `tauri::AppHandle` (default runtime `Wry`), so a
//!   `tauri::test::mock_app()` handle — `AppHandle<MockRuntime>` — cannot stand in for one.
//!   The only real `Wry` handle in a test process is the one the running app published.
//! * the IPC commands themselves: `mod commands` is private, and a `#[tauri::command]`
//!   wrapper's body is the one thing its own unit tests cannot reach when its argument type
//!   is runtime-specific.

/// The `AppHandle` the running app published, or `None` before its `setup` hook has run.
///
/// Concrete `tauri::AppHandle` (= `AppHandle<Wry>`), i.e. the type the IPC command
/// signatures take.
#[cfg(feature = "gui")]
pub fn app_handle() -> Option<&'static tauri::AppHandle> {
    crate::gui::APP_HANDLE.get()
}

/// One `#[tauri::command]` wrapper whose argument is runtime-specific, re-exported so a
/// test target that holds the real handle can call the command and not just its seam.
#[cfg(feature = "gui")]
pub use crate::commands::prepare_image_preview;

/// The macOS update-preparation chain, end to end, with `helper` standing in for the
/// privileged worker.
///
/// `tests/gui_main_thread_drive` is its only caller: the chain's *successful* half needs a
/// cooperating helper (a test binary has none — `current_exe()` is itself) **and** a process
/// that is not spawning a few hundred instrumented libtest cases at the same time (`probe`
/// gives each candidate executable 10 s; see `coverage/2026-11/tauri/wave3.md` §4, where the
/// libtest version of this case failed 2 of 4 rounds on exactly that).
#[cfg(all(feature = "gui", target_os = "macos"))]
pub use crate::commands::prepare_and_cancel_update_for_tests;
#[cfg(all(feature = "gui", target_os = "macos"))]
pub use crate::commands::prepare_update_bundle_for_tests;

/// The macOS update **worker**, so a test target can be the helper instead of standing in for
/// it: production reaches it as `--app-update-macos-worker` on the app's own executable, and a
/// `harness = false` driver dispatches the same flag here (`tests/gui_main_thread_drive`).
#[cfg(all(feature = "gui", target_os = "macos"))]
pub use crate::commands::run_update_worker_for_tests;

/// [`commands::update_macos::probe`], whose timeout arm needs a candidate that outlives the
/// production deadline — only a process with no parallel suite can afford to wait for it.
#[cfg(all(feature = "gui", target_os = "macos"))]
pub use crate::commands::probe_update_candidate_for_tests;

/// `commands::update_macos::confirm` with the installed bundle's path injected — the step that
/// removes a confirmed install's staging directory, whose only production caller reads that
/// path from `current_exe()`.
#[cfg(all(feature = "gui", target_os = "macos"))]
pub use crate::commands::confirm_update_for_tests;

/// How long a fixture waits for a fact about **another process**, and the seam that decides
/// it.
///
/// **Why a seam and not a constant.** Three gates in this crate have now been decided by how
/// long *this host* takes to schedule a child process rather than by the code under test, all
/// measured 2026-11-01 under the instrumented suite:
///
/// * `cli_tool`'s 3 s probe budget (`future_host::cli_tool::PROBE_DEADLINE`) —
///   `commands::update::tests::app_maintenance_status_reports_the_cli_it_discovered` red at
///   load 7.76 with 389 libtest cases in flight, green alone;
/// * the same budget across `spawn_blocking`, which carries no thread-locals, so the guard
///   armed by that test never reached the probe (`core::ProbeDeadlineGuard::inherited`);
/// * `skills_bootstrap::block_fixtures`' 15 s wait for its fixture child to start, red at
///   load ≈ 28 with the child taking 15.02 s (`stop-live` / `stop-unconfirmed`).
///
/// In each case the failing assertion *reads* as a property of the unit and is in fact a
/// property of the host: the number was measured on an idle machine. Raising the constant in
/// place keeps that reading at whatever magnitude was last measured, so a budget that decides
/// a fixture wait is **armed explicitly, per thread, by the fixture that wants it** — the same
/// shape as `future_host::cli_tool::ProbeDeadlineGuard`.
///
/// Thread-local for the same reason as that guard: libtest runs one case per thread, so a
/// process-global value would leak into every other case's waits.
#[cfg(any(test, feature = "test-support"))]
pub mod wait_budget {
    use std::{
        cell::Cell,
        time::{Duration, Instant},
    };

    thread_local! {
        /// `None` = nothing armed on this thread, so the call site's own default applies.
        static ARMED: Cell<Option<Duration>> = const { Cell::new(None) };
    }

    /// Arms a wait budget on this thread until dropped.
    pub struct WaitBudgetGuard;

    impl WaitBudgetGuard {
        /// Every [`super::wait_budget`] call on this thread uses `budget` until this guard
        /// drops.
        pub fn armed(budget: Duration) -> Self {
            ARMED.with(|cell| cell.set(Some(budget)));
            WaitBudgetGuard
        }
    }

    impl Drop for WaitBudgetGuard {
        fn drop(&mut self) {
            ARMED.with(|cell| cell.set(None));
        }
    }

    /// The budget a wait on this thread may take: what a fixture armed, else the call site's
    /// `default`.
    ///
    /// There is deliberately **no** "unbounded" arm. A wait that is genuinely stuck must say
    /// so itself: the enclosing harness kills the fixture process at its own ceiling
    /// (`skills_bootstrap::block_fixtures::run_case`), and that death carries no message from
    /// the assertion that was waiting.
    pub fn wait_budget(default: Duration) -> Duration {
        ARMED.with(|cell| cell.get()).unwrap_or(default)
    }

    /// The instant a wait armed on this thread must give up: `now + wait_budget(default)`.
    ///
    /// The one derivation of it — a call site that polled `wait_budget` itself on every pass
    /// would push its own deadline out for as long as the wait ran.
    pub fn deadline(default: Duration) -> Instant {
        Instant::now() + wait_budget(default)
    }

    #[cfg(test)]
    mod tests {
        use super::{wait_budget, WaitBudgetGuard};
        use std::time::Duration;

        /// Absurd on purpose: an unarmed read returning this is unmistakable, while a
        /// realistic default would look the same as a leak.
        const DEFAULT: Duration = Duration::from_secs(3600);

        #[test]
        fn a_budget_armed_for_a_fixture_applies_until_it_drops() {
            assert_eq!(
                wait_budget(DEFAULT),
                DEFAULT,
                "nothing is armed before the fixture arms anything"
            );
            {
                let _armed = WaitBudgetGuard::armed(Duration::from_millis(50));
                assert_eq!(
                    wait_budget(DEFAULT),
                    Duration::from_millis(50),
                    "the armed budget must win over the call site's default"
                );
            }
            assert_eq!(
                wait_budget(DEFAULT),
                DEFAULT,
                "dropping the guard must restore the call site's default"
            );
        }

        /// The reason this is thread-local rather than process-global: libtest runs one case
        /// per thread, so a global value would widen every other case's waits — the leak
        /// `future_host::cli_tool`'s guard avoids for the same reason.
        #[test]
        fn the_budget_is_per_thread_so_one_case_cannot_widen_anothers_waits() {
            let _armed = WaitBudgetGuard::armed(Duration::from_secs(7));
            assert_eq!(wait_budget(DEFAULT), Duration::from_secs(7));
            let elsewhere = std::thread::spawn(|| wait_budget(DEFAULT))
                .join()
                .expect("the probe thread runs");
            assert_eq!(
                elsewhere, DEFAULT,
                "a budget armed on one thread must not reach another"
            );
        }
    }
}
