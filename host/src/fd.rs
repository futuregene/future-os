//! The Desktop host's view of the process descriptor budget.
//!
//! The limits, the probe and the fields a failed store open reports live in
//! `future-fd`, shared with the Agent's journal writer — this module adds only
//! what belongs to an entry point: the one-time raise at startup, the outcome
//! kept for the host log, and the two-line bridge from this crate's error type.
//!
//! The raise runs from [`crate::environment::prepare_process_for`], which every
//! entry point (`futureos`, `future host`) calls before its first database
//! open, and which is where the TUI's `ensure_fd_headroom`
//! (`tui/src/lib.rs:141`) has its counterpart. The outcome is reported later,
//! from `start_logging`: process preparation runs before the host log's
//! subscriber exists, so a line written there would have nowhere to go.

pub use future_fd::{
    ensure_fd_headroom, limits, open_failure_detail, open_fds, sqlite_code_in_chain, Budget,
    Limits, WANT,
};

/// The SQLite primary result code behind a host error, when it has one.
///
/// The payload is inspected directly instead of walking `source()`: this
/// crate's `Error::Database` is `#[error(transparent)]`, and a transparent
/// wrapper forwards `source()` to the *inner error's* source — the `rusqlite`
/// error itself is never a node of the chain. That distinction is the
/// difference between `code=14` and `code=none` on the line an operator reads,
/// so it has a test below.
pub fn sqlite_code(error: &crate::Error) -> Option<i32> {
    match error {
        crate::Error::Database(error) => future_fd::sqlite_code(error),
        error => future_fd::sqlite_code_in_chain(error),
    }
}

static STARTUP: std::sync::OnceLock<Budget> = std::sync::OnceLock::new();

/// Raise the budget once per process, keeping the outcome for the host log.
pub fn raise_at_startup() {
    let _ = STARTUP.set(ensure_fd_headroom());
}

/// The budget [`raise_at_startup`] recorded, for the log to report once the log
/// is open. `None` before startup has run.
pub fn startup_budget() -> Option<&'static Budget> {
    STARTUP.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store error keeps its SQLite code: the transparent variant is read
    /// directly, because its `source()` skips the payload it wraps.
    #[test]
    fn a_store_error_reports_the_sqlite_code_of_its_cause() {
        let error = crate::Error::Database(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::CannotOpen,
                extended_code: 14,
            },
            Some("unable to open database file".into()),
        ));
        assert_eq!(sqlite_code(&error), Some(14));
        assert_eq!(
            sqlite_code(&crate::Error::Message("no database involved".into())),
            None
        );
    }
}
