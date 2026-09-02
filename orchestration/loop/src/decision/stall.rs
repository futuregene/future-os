//! Stall semantics — conditions that force a replan instead of delivery:
//! outcome-floor breaches, exhausted repair budgets, and stalled monitors.
//! Also owns the shared SIGNAL TEXT: the advisory strings the decision kernel
//! puts in the delivery reason are the exact strings the turn envelope's
//! context layer recomputes (ARCHITECTURE.md: signals are advisories surfaced
//! in the turn envelope; one detector set, two consumers).

use crate::state::{Goal, TaskClass, Todo};

use super::oscillation::oscillation_replan_reason;
use super::LLM_ZOMBIE_TURN_THRESHOLD;

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

/// The advisory signals for ONE runnable advancement todo, as they appear in
/// the delivery reason and (recomputed from the ledger) in the turn envelope:
/// outcome floor, oscillation, failure count, and the LLM-zombie
/// (no-write-tool) warning. Each is an observation the reading agent acts on;
/// none of them is a kernel-forced replan. Order and wording are part of the
/// surface — both consumers must render these verbatim.
pub fn todo_signals(goal: &Goal, todo: &Todo) -> Vec<String> {
    let mut advisories: Vec<String> = vec![];
    if let Some(signal) = outcome_floor_breach(goal) {
        advisories.push(format!(
            "[signal: {signal} — consider changing strategy or superseding a stale todo]"
        ));
    }
    if let Some(signal) = oscillation_replan_reason(goal) {
        advisories.push(format!(
            "[signal: {signal} — consider a different validator or splitting the todo]"
        ));
    }
    if todo.failed_attempts > 0 {
        advisories.push(format!(
            "[signal: todo {} has {} failed attempt(s) — consider superseding or asking the operator]",
            todo.id, todo.failed_attempts
        ));
    }
    // LLM-zombie signal (was a forced replan in #343; now an advisory —
    // the kernel surfaces it, the agent decides whether to restart with a
    // fresh session).
    let no_progress_turns = goal
        .turn_no_progress
        .iter()
        .filter(|np| np.todo_id == todo.id)
        .count() as u32;
    if no_progress_turns >= LLM_ZOMBIE_TURN_THRESHOLD {
        advisories.push(format!(
            "[signal: {no_progress_turns} turns with no write-class tool (write/edit/shell) — the worker may be stuck; consider restarting with a fresh session]"
        ));
    }
    advisories
}

/// Any open advancement todo has blown through its repair budget.
pub(crate) fn repair_exhausted(goal: &Goal) -> bool {
    goal.open_of(TaskClass::Advancement)
        .any(|t| t.failed_attempts > MAX_REPAIR_ATTEMPTS)
}

/// A diagnostic reason for a validator-carrying todo that exhausted its repair
/// budget: the verify gate failed repeatedly, so the loop must surface the two
/// dominant causes instead of silently retrying — (a) a tautological/always-
/// false validator (e.g. `test -n ""` from an empty `$(...)` expansion), and
/// (b) a gate that asserts a file the agent cannot produce locally because the
/// data is injected at grading time. The operator needs to fix the gate or
/// supersede the todo, not relaunch the same turn.
pub(crate) fn repair_exhausted_reason(goal: &Goal) -> Option<String> {
    let exhausted: Vec<&Todo> = goal
        .open_of(TaskClass::Advancement)
        .filter(|t| t.failed_attempts > MAX_REPAIR_ATTEMPTS)
        .collect();
    if exhausted.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = exhausted
        .iter()
        .map(|t| {
            let gate = t
                .validator
                .as_deref()
                .map(|v| format!(" (--verify `{v}` failed {})", t.failed_attempts))
                .unwrap_or_else(|| {
                    format!(" (no --verify; {} failed attempts)", t.failed_attempts)
                });
            format!("{}{}", t.id, gate)
        })
        .collect();
    parts.push(
        "check (a) is the --verify gate tautological/always-false (e.g. `test -n \"\"` from an empty $() expansion), or (b) does it assert a file only present at grading time (data injected into /app/data)? Fix the gate or supersede the todo — don't relaunch the same turn"
            .to_string(),
    );
    Some(format!(
        "advancement todo(s) exhausted repair budget: {}",
        parts.join("; ")
    ))
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

    #[test]
    fn repair_exhausted_reason_names_todo_and_gate_and_causes() {
        let mut g = Goal::new("g", "objective", "/tmp");
        let mut t = Todo::advancement("T1", "work");
        t.validator = Some("test -n \"\"".to_string());
        t.failed_attempts = MAX_REPAIR_ATTEMPTS + 1;
        g.add(t);
        let reason = repair_exhausted_reason(&g).expect("reason present");
        assert!(reason.contains("T1"), "{reason}");
        assert!(reason.contains("--verify"), "{reason}");
        assert!(
            reason.contains("tautological") || reason.contains("grading time"),
            "{reason}"
        );
    }

    #[test]
    fn repair_exhausted_reason_none_when_no_budget_breached() {
        let mut g = Goal::new("g", "objective", "/tmp");
        g.add(Todo::advancement("T1", "work"));
        assert_eq!(repair_exhausted_reason(&g), None);
    }
}
