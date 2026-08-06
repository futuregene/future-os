//! Stall semantics — conditions that force a replan instead of delivery:
//! outcome-floor breaches, exhausted repair budgets, and stalled monitors.

use crate::state::{Goal, TaskClass, Todo};

/// Consecutive no-change monitor polls before the monitor is considered
/// stalled (LoopX replan trigger).
pub const MONITOR_NO_CHANGE_REPLAN_THRESHOLD: u32 = 3;
/// Maximum failed attempts before an advancement todo exhausts its repair
/// budget.
pub const MAX_REPAIR_ATTEMPTS: u32 = 1;

/// A monitor is stalled once its consecutive no-change polls hit the
/// threshold.
pub(crate) fn is_monitor_stalled(m: &Todo) -> bool {
    m.consecutive_no_change >= MONITOR_NO_CHANGE_REPLAN_THRESHOLD
}

/// Outcome-floor breach: `surface_streak` consecutive turns without a
/// material outcome once the configured threshold is reached. Returns the
/// replan reason, or `None` while the floor is not breached.
pub(crate) fn outcome_floor_breach(goal: &Goal) -> Option<String> {
    let threshold = goal.execution_profile.outcome_floor_streak_threshold;
    if threshold > 0 && goal.outcome_streak >= threshold {
        Some(format!(
            "outcome floor: {surface_streak} consecutive turns without a material outcome (threshold {threshold})",
            surface_streak = goal.outcome_streak
        ))
    } else {
        None
    }
}

/// Any open advancement todo has blown through its repair budget.
pub(crate) fn repair_exhausted(goal: &Goal) -> bool {
    goal.open_of(TaskClass::Advancement)
        .any(|t| t.failed_attempts > MAX_REPAIR_ATTEMPTS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Goal, Todo};
    use std::time::Duration;

    #[test]
    fn monitor_stall_threshold_is_inclusive() {
        let mut g = Goal::new("g", "objective", "/tmp");
        g.add(Todo::monitor("M1", "watch", Duration::from_secs(60)));
        let m = g.todo_mut("M1").unwrap();
        m.consecutive_no_change = MONITOR_NO_CHANGE_REPLAN_THRESHOLD - 1;
        assert!(!is_monitor_stalled(m));
        m.consecutive_no_change = MONITOR_NO_CHANGE_REPLAN_THRESHOLD;
        assert!(is_monitor_stalled(m));
    }

    #[test]
    fn outcome_floor_disabled_at_zero_threshold() {
        let mut g = Goal::new("g", "objective", "/tmp");
        g.add(Todo::advancement("T1", "work"));
        g.outcome_streak = 100;
        assert_eq!(outcome_floor_breach(&g), None);
    }

    #[test]
    fn outcome_floor_breaches_at_threshold() {
        let mut g = Goal::new("g", "objective", "/tmp");
        g.add(Todo::advancement("T1", "work"));
        g.execution_profile.outcome_floor_streak_threshold = 3;
        g.outcome_streak = 3;
        let reason = outcome_floor_breach(&g).expect("floor breached");
        assert!(reason.contains("outcome floor"));
        g.outcome_streak = 2;
        assert_eq!(outcome_floor_breach(&g), None);
    }

    #[test]
    fn repair_budget_exhausted_strictly_above_max() {
        let mut g = Goal::new("g", "objective", "/tmp");
        g.add(Todo::advancement("T1", "work"));
        g.todo_mut("T1").unwrap().failed_attempts = MAX_REPAIR_ATTEMPTS;
        assert!(!repair_exhausted(&g));
        g.todo_mut("T1").unwrap().failed_attempts = MAX_REPAIR_ATTEMPTS + 1;
        assert!(repair_exhausted(&g));
    }
}
