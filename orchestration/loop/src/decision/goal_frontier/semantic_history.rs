//! Goal-level bounded semantic history (G13 ③) — the reference
//! `goal_frontier/semantic_history.py` (299 lines; core subset).
//!
//! The reference keeps per-agent semantic slots (latest vision / checkpoint
//! / replan-ack runs) inside `run_history.goals[].semantic_history`. Our
//! native subset is the goal-LEVEL history: a bounded ring of semantic
//! event summaries folded from the event ledger during replay, capped at
//! [`SEMANTIC_HISTORY_CAP`] (oldest dropped) so the projection is a pure,
//! deterministic function of event order.
//!
//! The semantic event history is a standalone goal-level projection
//! (ids/summaries only — summaries are truncated at write time so the
//! packet stays public-safe).

use serde::{Deserialize, Serialize};

use crate::state::Goal;

pub const SEMANTIC_HISTORY_SCHEMA_VERSION: &str = "goal_semantic_history_v0";
/// Bounded history cap (LoopX semantic slots keep the newest N events).
pub const SEMANTIC_HISTORY_CAP: usize = 50;
/// Summaries never exceed this length (public-safe truncation).
pub const SEMANTIC_SUMMARY_MAX_CHARS: usize = 200;

/// One semantic event summary `{kind, todo_id, summary, ts}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticEvent {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo_id: Option<String>,
    pub summary: String,
    pub ts: u64,
}

/// Semantic event kind vocabulary (folded event → kind).
pub const KIND_RUN_LANDED: &str = "run_landed";
pub const KIND_TODO_COMPLETED: &str = "todo_completed";
pub const KIND_TODO_SUPERSEDED: &str = "todo_superseded";
pub const KIND_ACCEPTANCE_GAP_SATISFIED: &str = "acceptance_gap_satisfied";
pub const KIND_REPLAN_ACKED: &str = "replan_acked";
pub const KIND_MONITOR_POLL: &str = "monitor_poll";
pub const KIND_GATE_RESOLVED: &str = "gate_resolved";
pub const KIND_DELIVERY_OUTCOME: &str = "delivery_outcome";
pub const KIND_ROLE_SUCCESSION: &str = "role_succession";
pub const KIND_TURN_NO_PROGRESS: &str = "turn_no_progress";

/// Truncate a summary to the public-safe bound.
pub fn truncate_summary(text: &str) -> String {
    crate::decision::truncate(text, SEMANTIC_SUMMARY_MAX_CHARS)
}

impl Goal {
    /// Record one semantic event summary, keeping the ring bounded
    /// (oldest dropped past [`SEMANTIC_HISTORY_CAP`]). Replay folds events
    /// through this same path, so the projection is deterministic.
    pub fn record_semantic_event(
        &mut self,
        kind: &str,
        todo_id: Option<&str>,
        summary: &str,
        ts: u64,
    ) {
        self.semantic_history.push(SemanticEvent {
            kind: kind.to_string(),
            todo_id: todo_id.map(|s| s.to_string()),
            summary: truncate_summary(summary),
            ts,
        });
        let overflow = self
            .semantic_history
            .len()
            .saturating_sub(SEMANTIC_HISTORY_CAP);
        if overflow > 0 {
            self.semantic_history.drain(..overflow);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_bounded_to_cap_oldest_dropped() {
        let mut g = Goal::new("g", "o", "/tmp");
        for i in 0..(SEMANTIC_HISTORY_CAP + 12) {
            g.record_semantic_event(KIND_RUN_LANDED, Some("T1"), &format!("run {i}"), i as u64);
        }
        assert_eq!(g.semantic_history.len(), SEMANTIC_HISTORY_CAP);
        // Newest 50 kept: first entry is event 12, last is event 61.
        assert_eq!(g.semantic_history.first().unwrap().ts, 12);
        assert_eq!(
            g.semantic_history.last().unwrap().ts,
            (SEMANTIC_HISTORY_CAP + 11) as u64
        );
    }

    #[test]
    fn summaries_are_truncated_to_the_public_safe_bound() {
        let mut g = Goal::new("g", "o", "/tmp");
        let long = "x".repeat(SEMANTIC_SUMMARY_MAX_CHARS + 100);
        g.record_semantic_event(KIND_REPLAN_ACKED, None, &long, 1);
        let event = &g.semantic_history[0];
        assert!(
            event.summary.chars().count() <= SEMANTIC_SUMMARY_MAX_CHARS + 1,
            "summary over the public-safe bound: {} chars",
            event.summary.chars().count()
        );
        assert!(event.summary.ends_with('…'));
        assert_eq!(event.kind, KIND_REPLAN_ACKED);
        assert_eq!(event.todo_id, None);
    }
}
