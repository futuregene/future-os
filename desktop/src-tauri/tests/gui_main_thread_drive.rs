//! **The main-thread driver** (TAU-C8): a `harness = false` test target.
//!
//! Why this target exists — and why it is a *test target*, not a `src/bin/` file:
//! the shell's biggest residual family (`menu.rs`'s `build_macos_menu`, `run_with`'s own
//! body, the `.setup` hook, the window-geometry calls) is unreachable from a libtest body
//! because `muda`/AppKit require the process's **main thread** and libtest runs every test
//! body on a spawned thread (`docs/testing/module-tauri-shell.md` §6.2, measured).
//! `harness = false` gives this file a real `fn main()`, i.e. the main thread, while
//! staying a test target: `cargo test` runs it, so `cargo llvm-cov` instruments it and its
//! profile merges like any other target's, and it is never bundled into the app.
//!
//! How it works:
//!
//! 1. **Isolate first.** `HOME`/`FUTURE_HOME` are redirected to a temp directory before
//!    anything else, so the real `~/.future` (locks, database, logs, the user's agent) is
//!    never touched. That also makes the single-instance socket unique:
//!    `instance::gui_identifier` appends the data root's id whenever `FUTURE_HOME` is
//!    overridden (`lib.rs`'s identifier logic), so this process cannot collide with a real
//!    Desktop on `/tmp/<identifier>_si.sock`.
//! 2. **Run the real entry point** on the main thread: `futureos_lib::run_with`, the same
//!    function `src/main.rs` calls.
//! 3. **Exit through `libc::exit`.** LLVM writes the profile from an `atexit` handler, so
//!    the process must end normally — a `SIGKILL`ed app records nothing (measured; see the
//!    record's Appendix S7). `std::process::exit` runs the `atexit` chain, which is why the
//!    watchdog thread below can end the run from the main thread's event loop.
//! 4. **Print the summary marker** `gui_main_thread_drive: driver completed` as the last
//!    thing before exiting. A `harness = false` target prints no `test result:` line, and
//!    the campaign's harness treats a target without one as truncated; this marker is that
//!    target's summary (see `.future/cov100/measure-final.sh`).
//!
//! Bounded, and it never fails the suite:
//!
//! * it **always** hands the app one single-instance notification once the plugin's listener
//!   is up — that is the coverage this target exists for, and it does not depend on the app
//!   reaching `Ready` (the plugin delivers the callback on its own thread, measured §S9.2);
//! * it waits a **short, tightening** window for the app's `Ready` state before notifying, so
//!   that on a host which delivers `Ready` the callback finds the window (its `unminimize`/
//!   `show`/`set_focus` arms) and the `.setup` hook (menu, dock icon, geometry) has run;
//! * a hard deadline ends the process regardless, so a hang cannot hold the suite;
//! * if `run_with` returns — on a host where the real Tauri/Wry runtime cannot start (a
//!   Linux CI box without a display, for instance) it returns `Err` — the driver reports
//!   that on stderr and exits **0**: the platform's own behaviour is the result, and this
//!   target must not add a red to a job whose subject is not this unit's code.
//!
//! Flavour-sensitive: it needs the `gui` feature (`run_with` only exists there — the
//! headless build exports a different surface), and the `[[test]]` stanza declares
//! `required-features = ["gui"]` so the headless leg skips it instead of failing to build.

// The target is `harness = false`, so a `main` must exist on every platform; the work
// inside is macOS-only (muda's main-thread requirement), which is why it lives in a
// gated module rather than behind an early return - an early return leaves the rest of
// `main` unreachable, and `#![cfg]` at file level would remove `main` altogether.
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("gui_main_thread_drive: macOS only (main-thread menu builder); nothing to drive");
}

#[cfg(target_os = "macos")]
mod drive {
    // The body lives in a SUBDIRECTORY on purpose: Cargo turns every `.rs` directly
    // under `tests/` into its own test target, so a sibling file named
    // `gui_main_thread_drive_body.rs` was compiled as a second binary (which then
    // failed to find `futureos_lib::run_with`, because that target was not built
    // with the gui feature). A subdirectory is not auto-discovered.
    include!("gui_main_thread_drive/body.rs");
}

#[cfg(target_os = "macos")]
fn main() {
    drive::main();
}
