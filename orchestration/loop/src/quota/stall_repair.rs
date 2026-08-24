//! Stall repair (G-7) — the delivery guard: detect stalled delivery paths and
//! emit a replan hint, generalizing the P0 `MAX_REPAIR_ATTEMPTS` shortcut.
//!
//! LoopX `control_plane/quota/stall_repair.py` (397 lines) decides when the
//! loop must stop delivering and repair the frontier instead: monitor stalls,
//! outcome-floor breaches, exhausted repair budgets, succession obligations,
//! and acceptance gaps with no runnable work. The hint is a read-only
//! projection for hosts (CLI projection / heartbeat); the decision kernel's
//! `decide_for` remains the single authority that actually flips a packet to
//! replan (packet output unchanged — plan §5.2 G-7 risk note).

use crate::decision::stall::{
    is_monitor_stalled, outcome_floor_breach, repair_exhausted, repair_exhausted_reason,
};
use crate::state::{Goal, TaskClass};

/// A stall-return hint: which guard tripped, why, and what the host should do.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StallRepairHint {
    pub kind: String,
    pub reason: String,
    pub replan_hint: String,
    /// Action kinds that must not be attempted while this stall is open
    /// (LoopX `stall_repair_blocked_action_scope`).
    pub blocked_action_scope: Option<String>,
}

impl StallRepairHint {
    pub fn kind(&self) -> &str {
        &self.kind
    }
}

/// Detect the first open stall (pipeline order mirrors `decide_for`):
/// outcome floor → repair budget → monitor stall → succession obligation →
/// acceptance gap with no runnable work.
pub fn detect_stall(goal: &Goal) -> Option<StallRepairHint> {
    if let Some(reason) = outcome_floor_breach(goal) {
        return Some(StallRepairHint {
            kind: "outcome_floor".to_string(),
            reason,
            replan_hint: "deliver a material outcome (artifact + validation) before the next spend; otherwise record a replan delta".to_string(),
            blocked_action_scope: Some("surface_only_activity".to_string()),
        });
    }
    if repair_exhausted(goal) {
        let reason = repair_exhausted_reason(goal)
            .unwrap_or_else(|| "advancement todo(s) exhausted repair budget".to_string());
        return Some(StallRepairHint {
            kind: "repair_budget_exhausted".to_string(),
            reason,
            replan_hint: "record a frontier-changing replan delta (vision patch / successor / no-follow-up) to clear the stall".to_string(),
            blocked_action_scope: None,
        });
    }
    let stalled_monitor = goal.open_monitors().find(|m| is_monitor_stalled(m));
    if let Some(m) = stalled_monitor {
        return Some(StallRepairHint {
            kind: "monitor_stalled".to_string(),
            reason: format!(
                "monitor {} stalled ({} consecutive no-change polls)",
                m.id, m.consecutive_no_change
            ),
            replan_hint: "replan the monitor: change its target/policy or declare no-follow-up"
                .to_string(),
            blocked_action_scope: Some("monitor_poll".to_string()),
        });
    }
    let unclosed = goal.completed_without_closure_intent();
    if !unclosed.is_empty() {
        return Some(StallRepairHint {
            kind: "succession_obligation".to_string(),
            reason: format!(
                "completed advancement without closure intent: {}",
                unclosed
                    .iter()
                    .map(|t| t.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            replan_hint: "complete must declare --successor or --no-follow-up".to_string(),
            blocked_action_scope: None,
        });
    }
    let gaps = goal.unsatisfied_gaps();
    let has_runnable = goal.open_of(TaskClass::Advancement).next().is_some();
    if !gaps.is_empty() && !has_runnable {
        return Some(StallRepairHint {
            kind: "acceptance_gap".to_string(),
            reason: format!(
                "acceptance gap(s) open with no runnable work: {}",
                gaps.iter()
                    .map(|g| g.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            replan_hint:
                "add a runnable todo that satisfies the gap, or revise the acceptance criteria"
                    .to_string(),
            blocked_action_scope: None,
        });
    }
    None
}

/// Whether the current decision mode would be stalled into a replan (used by
/// the CLI projection to annotate replan packets).
pub fn is_stalled_mode(reason_kind: &str) -> bool {
    matches!(
        reason_kind,
        "outcome_floor"
            | "repair_budget_exhausted"
            | "monitor_stalled"
            | "succession_obligation"
            | "acceptance_gap"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::stall::MONITOR_NO_CHANGE_REPLAN_THRESHOLD;
    use crate::state::{Goal, Todo, TodoStatus};
    use std::time::Duration;

    #[test]
    fn outcome_floor_wins_over_other_stalls() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T1", "work"));
        g.execution_profile.outcome_floor_streak_threshold = 2;
        g.outcome_streak = 2;
        let hint = detect_stall(&g).expect("stall");
        assert_eq!(hint.kind, "outcome_floor");
        assert!(hint.replan_hint.contains("material outcome"));
    }

    #[test]
    fn repair_budget_exhausted_detected() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T1", "work"));
        g.todo_mut("T1").unwrap().failed_attempts = 2; // > MAX_REPAIR_ATTEMPTS=1
        let hint = detect_stall(&g).expect("stall");
        assert_eq!(hint.kind, "repair_budget_exhausted");
    }

    #[test]
    fn monitor_stall_detected() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::monitor("M1", "watch", Duration::from_secs(60)));
        g.todo_mut("M1").unwrap().consecutive_no_change = MONITOR_NO_CHANGE_REPLAN_THRESHOLD;
        let hint = detect_stall(&g).expect("stall");
        assert_eq!(hint.kind, "monitor_stalled");
        assert_eq!(hint.blocked_action_scope.as_deref(), Some("monitor_poll"));
    }

    #[test]
    fn succession_obligation_detected() {
        let mut g = Goal::new("g", "o", "/tmp");
        let mut t = Todo::advancement("T1", "done without intent");
        t.status = TodoStatus::Done;
        g.add(t);
        let hint = detect_stall(&g).expect("stall");
        assert_eq!(hint.kind, "succession_obligation");
    }

    #[test]
    fn healthy_goal_has_no_stall() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T1", "work"));
        assert_eq!(detect_stall(&g), None);
    }

    #[test]
    fn acceptance_gap_only_without_runnable_work() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.acceptance.push(crate::state::AcceptanceGap {
            id: "A1".to_string(),
            description: "thing".to_string(),
            satisfied: false,
        });
        let hint = detect_stall(&g).expect("stall");
        assert_eq!(hint.kind, "acceptance_gap");
        // With runnable work the gap is not a stall (delivery continues).
        g.add(Todo::advancement("T1", "work"));
        assert_eq!(detect_stall(&g), None);
    }
}
