//! Shared run-status vocabulary. Single source of truth for the terminal run
//! states so the SQL `IN (...)` / `NOT IN (...)` lists and the Rust `matches!`
//! checks scattered across `runs.rs` and `cleanup.rs` can't drift apart.

/// Run statuses that are terminal — a run in one of these never transitions
/// again. Use for `contains`-style checks in Rust.
pub(super) const TERMINAL_RUN_STATUSES: &[&str] = &["completed", "failed", "cancelled"];

/// Whether a run status is terminal, for callers outside this module.
///
/// The vocabulary stays here so a new terminal state cannot be added in one
/// place and missed in another.
pub fn is_terminal_run_status(status: &str) -> bool {
    TERMINAL_RUN_STATUSES.contains(&status)
}

/// The terminal run statuses pre-rendered as a SQL list for `IN (...)` /
/// `NOT IN (...)` clauses: `'completed', 'failed', 'cancelled'`. Kept in lockstep
/// with [`TERMINAL_RUN_STATUSES`] (see `terminal_status_sql_matches_slice`).
pub(super) const TERMINAL_RUN_STATUSES_SQL: &str = "'completed', 'failed', 'cancelled'";

/// A run the Agent ACCEPTED but is holding behind the one already executing.
///
/// Not terminal — it still owes a turn, so `has_active_run`, the quit guard and
/// the auto-title gate keep counting it. Not *executing* either, and that is
/// the point of having the state at all: a row in it may never run (an
/// `enqueue_coalescing` burst is folded into the run ahead), so nothing may
/// treat it as the run this conversation is watching stream. While such a row
/// stayed `running` it was also the newest row, so it won every "latest run"
/// read and the view anchored its live preview on a run with no events of its
/// own — the reply then arrived in one lump at the end, or not at all.
pub const QUEUED_RUN_STATUS: &str = "queued";

/// Whether a run status means "accepted, waiting behind another run".
pub fn is_queued_run_status(status: &str) -> bool {
    status == QUEUED_RUN_STATUS
}

/// The statuses that mean a run is executing right now. A view picking the run
/// it watches stream must prefer these over anything merely queued.
pub(super) const EXECUTING_RUN_STATUSES_SQL: &str = "'running', 'waiting_approval'";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_status_sql_matches_slice() {
        let rendered = TERMINAL_RUN_STATUSES
            .iter()
            .map(|status| format!("'{status}'"))
            .collect::<Vec<_>>()
            .join(", ");
        assert_eq!(rendered, TERMINAL_RUN_STATUSES_SQL);
    }
}
