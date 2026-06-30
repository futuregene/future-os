//! Shared run-status vocabulary. Single source of truth for the terminal run
//! states so the SQL `IN (...)` / `NOT IN (...)` lists and the Rust `matches!`
//! checks scattered across `runs.rs` and `cleanup.rs` can't drift apart.

/// Run statuses that are terminal — a run in one of these never transitions
/// again. Use for `contains`-style checks in Rust.
pub(super) const TERMINAL_RUN_STATUSES: &[&str] = &["completed", "failed", "cancelled"];

/// The terminal run statuses pre-rendered as a SQL list for `IN (...)` /
/// `NOT IN (...)` clauses: `'completed', 'failed', 'cancelled'`. Kept in lockstep
/// with [`TERMINAL_RUN_STATUSES`] (see `terminal_status_sql_matches_slice`).
pub(super) const TERMINAL_RUN_STATUSES_SQL: &str = "'completed', 'failed', 'cancelled'";

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
