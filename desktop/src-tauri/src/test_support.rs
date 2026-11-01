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
