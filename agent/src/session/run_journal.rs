//! Run lifecycle markers and unterminated-run recovery helpers.
//!
//! A session's journal records each run's boundary with a `run_started` /
//! `run_terminal` marker pair. These helpers let callers detect a run
//! interrupted by a crash or restart and continue run ordering without
//! persisting queued work.

use super::entry::{SessionEntry, ENTRY_TYPE_RUN_STARTED, ENTRY_TYPE_RUN_TERMINAL};

/// Terminal state recorded on a `run_terminal` marker.
pub const RUN_STATE_COMPLETED: &str = "completed";
pub const RUN_STATE_ERROR: &str = "error";
pub const RUN_STATE_CANCELLED: &str = "cancelled";
pub const RUN_STATE_INCOMPLETE: &str = "incomplete";
/// Recovered terminal state for a run that has a durable `run_started` marker
/// but no `run_terminal` — i.e. the agent crashed or restarted before the run
/// committed. Such a run must never be presented as completed.
pub const RUN_STATE_INTERRUPTED_BY_RESTART: &str = "interrupted_by_restart";

/// True for entry types that are run lifecycle markers rather than
/// conversation content. Forks skip these (they belong to the parent's runs)
/// and every context/display projection filters them out.
pub fn is_run_marker(entry_type: &str) -> bool {
    matches!(entry_type, ENTRY_TYPE_RUN_STARTED | ENTRY_TYPE_RUN_TERMINAL)
}

/// Scan a session's entries for a run that began (has a `run_started` marker)
/// but never committed (no matching `run_terminal`). Returns the run_id of the
/// most recent such unterminated run, if any.
///
/// Runs are sequential per session, so this tracks the currently-open run: set
/// on `run_started`, cleared on the matching `run_terminal`. Anything still open
/// at the end was interrupted — by a crash, an agent restart, or a kill — and
/// must be recovered as `InterruptedByRestart`, never faked as completed. A
/// session rebuilt by a full rewrite carries no markers and yields `None`.
pub fn find_unterminated_run(entries: &[SessionEntry]) -> Option<String> {
    let mut open: Option<String> = None;
    for entry in entries {
        match entry.entry_type.as_str() {
            ENTRY_TYPE_RUN_STARTED => {
                if let Some(run_id) = entry
                    .content
                    .as_ref()
                    .and_then(|c| c.get("run_id"))
                    .and_then(|v| v.as_str())
                {
                    open = Some(run_id.to_string());
                }
            }
            ENTRY_TYPE_RUN_TERMINAL => {
                if let Some(run_id) = entry
                    .content
                    .as_ref()
                    .and_then(|c| c.get("run_id"))
                    .and_then(|v| v.as_str())
                {
                    // A terminal marker closes its own run; only clear the open
                    // run if it matches, so a stray terminal can't mask an older
                    // unterminated run.
                    if open.as_deref() == Some(run_id) {
                        open = None;
                    }
                }
            }
            _ => {}
        }
    }
    open
}

/// Continue run ordering without persisting queued work. New markers carry an
/// explicit sequence; legacy markers contribute their count so an upgraded
/// session never reuses a sequence already visible in its history.
pub fn next_run_sequence(entries: &[SessionEntry]) -> u64 {
    let mut started_count = 0_u64;
    let mut max_sequence = 0_u64;
    for entry in entries {
        if entry.entry_type != ENTRY_TYPE_RUN_STARTED {
            continue;
        }
        started_count = started_count.saturating_add(1);
        if let Some(sequence) = entry
            .content
            .as_ref()
            .and_then(|content| content.get("run_sequence"))
            .and_then(serde_json::Value::as_u64)
        {
            max_sequence = max_sequence.max(sequence);
        }
    }
    max_sequence.max(started_count).saturating_add(1).max(1)
}

/// Return the durable terminal payload for `run_id`, if the journal contains
/// one. Scans from the end so a later healing rewrite/commit wins over an older
/// marker. The returned value is the marker's `content` object.
pub fn find_run_terminal(entries: &[SessionEntry], run_id: &str) -> Option<serde_json::Value> {
    entries.iter().rev().find_map(|entry| {
        if entry.entry_type != ENTRY_TYPE_RUN_TERMINAL {
            return None;
        }
        let content = entry.content.as_ref()?;
        (content.get("run_id").and_then(|value| value.as_str()) == Some(run_id))
            .then(|| content.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::entry::{
        SessionEntry, ENTRY_TYPE_ASSISTANT, ENTRY_TYPE_RUN_STARTED, ENTRY_TYPE_RUN_TERMINAL,
        ENTRY_TYPE_SESSION_INFO, ENTRY_TYPE_SYSTEM,
    };

    #[test]
    fn run_markers_have_correct_type_and_content() {
        let started = SessionEntry::run_started("run-1", 7);
        assert_eq!(started.entry_type, ENTRY_TYPE_RUN_STARTED);
        assert_eq!(started.role, ENTRY_TYPE_SYSTEM);
        let c = started.content.as_ref().unwrap();
        assert_eq!(c["run_id"], "run-1");
        assert_eq!(c["epoch"], 7);

        let sequenced = SessionEntry::run_started_with_sequence("run-2", 8, Some(42));
        assert_eq!(sequenced.content.as_ref().unwrap()["run_sequence"], 42);
        assert_eq!(
            next_run_sequence(&[started.clone(), sequenced]),
            43,
            "restart must continue after the largest persisted started sequence"
        );

        let terminal = SessionEntry::run_terminal("run-1", RUN_STATE_COMPLETED, 42, 1500, None);
        assert_eq!(terminal.entry_type, ENTRY_TYPE_RUN_TERMINAL);
        let c = terminal.content.as_ref().unwrap();
        assert_eq!(c["run_id"], "run-1");
        assert_eq!(c["state"], "completed");
        assert_eq!(c["run_tokens"], 42);
        assert_eq!(c["run_duration_ms"], 1500);
        assert!(c.get("error").is_none());

        let failed = SessionEntry::run_terminal("run-1", RUN_STATE_ERROR, 0, 10, Some("boom"));
        assert_eq!(failed.content.as_ref().unwrap()["error"], "boom");

        assert!(is_run_marker(ENTRY_TYPE_RUN_STARTED));
        assert!(is_run_marker(ENTRY_TYPE_RUN_TERMINAL));
        assert!(!is_run_marker(ENTRY_TYPE_ASSISTANT));
        assert!(!is_run_marker(ENTRY_TYPE_SESSION_INFO));
    }

    #[test]
    fn find_unterminated_run_detects_missing_terminal() {
        // No markers → nothing open.
        assert_eq!(find_unterminated_run(&[]), None);
        assert_eq!(
            find_unterminated_run(&[SessionEntry::new_user("user", serde_json::json!("hi"))]),
            None
        );

        // A started+terminal pair is closed.
        let closed = vec![
            SessionEntry::run_started("run-1", 1),
            SessionEntry::run_terminal("run-1", RUN_STATE_COMPLETED, 1, 1, None),
        ];
        assert_eq!(find_unterminated_run(&closed), None);

        // A started marker with no terminal is interrupted.
        let open = vec![
            SessionEntry::new_user("user", serde_json::json!("hi")),
            SessionEntry::run_started("run-2", 1),
        ];
        assert_eq!(find_unterminated_run(&open), Some("run-2".to_string()));

        // Multiple runs: only the last, unterminated one is reported.
        let mixed = vec![
            SessionEntry::run_started("run-1", 1),
            SessionEntry::run_terminal("run-1", RUN_STATE_COMPLETED, 1, 1, None),
            SessionEntry::run_started("run-2", 2),
            SessionEntry::run_terminal("run-2", RUN_STATE_ERROR, 0, 1, Some("boom")),
            SessionEntry::run_started("run-3", 3),
        ];
        assert_eq!(find_unterminated_run(&mixed), Some("run-3".to_string()));

        // A terminal for a different run does not mask an older open run.
        let stray = vec![
            SessionEntry::run_started("run-a", 1),
            SessionEntry::run_terminal("run-other", RUN_STATE_COMPLETED, 1, 1, None),
        ];
        assert_eq!(find_unterminated_run(&stray), Some("run-a".to_string()));
    }

    #[test]
    fn find_unterminated_run_ignores_a_closed_run() {
        let entries = vec![
            SessionEntry::run_started("run-a", 1),
            SessionEntry::run_terminal("run-a", RUN_STATE_COMPLETED, 5, 100, None),
        ];
        assert_eq!(find_unterminated_run(&entries), None);

        // A stray terminal for a DIFFERENT run does not mask the open one.
        let entries = vec![
            SessionEntry::run_started("run-a", 1),
            SessionEntry::run_terminal("run-b", RUN_STATE_COMPLETED, 5, 100, None),
        ];
        assert_eq!(find_unterminated_run(&entries).as_deref(), Some("run-a"));

        // Markers without a run_id in their content are ignored.
        let mut bare_terminal =
            SessionEntry::run_terminal("run-a", RUN_STATE_COMPLETED, 0, 0, None);
        bare_terminal.content = None;
        let mut bare_start = SessionEntry::run_started("run-c", 1);
        bare_start.content = None;
        let entries = vec![
            SessionEntry::run_started("run-a", 1),
            bare_terminal,
            bare_start,
        ];
        assert_eq!(find_unterminated_run(&entries).as_deref(), Some("run-a"));
    }
}
