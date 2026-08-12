//! Autonomous replan obligation bookkeeping (G-13) — record + query, in
//! minimal form mirroring LoopX `control_plane/work_items/autonomous_replan_obligation.py`
//! (870 lines; we deliberately skip the queue/injection machinery).
//!
//! Obligations are DERIVED from kernel signals (monitor no-change streak,
//! surface-only progress streak, succession gap) and tracked against the
//! ReplanAcked ledger: an ack with a frontier-changing delta recorded after
//! the obligation's trigger clears it. The ack record itself is
//! event-sourced (`ReplanAcked`); the obligation view is a read model.

use serde::Serialize;

use crate::state::{Goal, TaskClass, TodoStatus};

pub const REPLAN_OBLIGATION_SCHEMA_VERSION: &str = "replan_obligation_v0";
/// LoopX `AUTONOMOUS_REPLAN_STALL_THRESHOLD` (2 consecutive stalled turns).
pub const AUTONOMOUS_REPLAN_STALL_THRESHOLD: u32 = 2;
/// Monitor no-change streak that raises an obligation (aligned with
/// `MONITOR_NO_CHANGE_REPLAN_THRESHOLD` in the decision kernel).
pub const MONITOR_NO_CHANGE_OBLIGATION_THRESHOLD: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReplanObligation {
    pub schema_version: String,
    pub kind: String,
    pub goal_id: String,
    pub todo_id: Option<String>,
    pub raised_at: u64,
    pub evidence: String,
    pub cleared: bool,
    pub cleared_reason: Option<String>,
    pub cleared_at: Option<u64>,
}

/// Whether a recorded replan ack clears obligations raised at/before
/// `raised_at` (LoopX: a frontier-changing delta is required to clear).
fn ack_clears(goal: &Goal, raised_at: u64) -> Option<u64> {
    goal.replan_ack
        .as_ref()
        .filter(|ack| ack.has_frontier_delta() && ack.at >= raised_at)
        .map(|ack| ack.at)
}

/// Detect the open replan obligations for a goal (LoopX
/// `build_autonomous_replan_obligation`, minimal): monitor no-change
/// streaks, surface-only progress streaks, and completion without closure
/// intent. An obligation is `cleared` when the trigger resolved or a
/// frontier-delta ack was recorded after it was raised.
pub fn detect_obligations(goal: &Goal) -> Vec<ReplanObligation> {
    let mut out: Vec<ReplanObligation> = vec![];

    // 1) Monitor no-change streak: an open monitor with
    //    consecutive_no_change >= threshold stalled without a material
    //    transition (LoopX `dead_monitor_repeat` / `monitor_no_change_streak`).
    for todo in goal
        .todos
        .iter()
        .filter(|t| t.class == TaskClass::Monitor && t.status == TodoStatus::Open)
    {
        if todo.consecutive_no_change < MONITOR_NO_CHANGE_OBLIGATION_THRESHOLD {
            continue;
        }
        let raised_at = todo.updated_at; // last poll ts (replay-deterministic)
        let cleared_at = ack_clears(goal, raised_at);
        out.push(ReplanObligation {
            schema_version: REPLAN_OBLIGATION_SCHEMA_VERSION.to_string(),
            kind: "monitor_no_change_streak".to_string(),
            goal_id: goal.goal_id.clone(),
            todo_id: Some(todo.id.clone()),
            raised_at,
            evidence: format!(
                "monitor {} recorded {} consecutive unchanged polls without a material transition",
                todo.id, todo.consecutive_no_change
            ),
            cleared: cleared_at.is_some(),
            cleared_reason: cleared_at.map(|_| "replan_ack".to_string()),
            cleared_at,
        });
    }

    // 2) Surface-only progress streak (LoopX outcome floor): the execution
    //    profile requires a material outcome after `threshold` surface-only
    //    turns; the streak is still at/above the floor.
    let floor = goal.execution_profile.outcome_floor_streak_threshold;
    if floor > 0 && goal.outcome_streak >= floor {
        let raised_at = goal
            .history
            .iter()
            .rev()
            .find(|r| !r.tools.is_empty() && !r.evidence.trim().is_empty())
            .map(|r| r.recorded_at)
            .unwrap_or(goal.created_at);
        out.push(ReplanObligation {
            schema_version: REPLAN_OBLIGATION_SCHEMA_VERSION.to_string(),
            kind: "surface_only_progress_streak".to_string(),
            goal_id: goal.goal_id.clone(),
            todo_id: None,
            raised_at,
            evidence: format!(
                "{} consecutive surface-only turns without a material outcome (floor={})",
                goal.outcome_streak, floor
            ),
            cleared: false,
            cleared_reason: None,
            cleared_at: None,
        });
    }

    // 3) Succession gap: completed advancement without declared closure
    //    intent (LoopX `completed_advancement_without_successor`).
    for todo in goal.completed_without_closure_intent() {
        let raised_at = todo.completed_at.unwrap_or(todo.updated_at);
        let cleared_at = ack_clears(goal, raised_at);
        out.push(ReplanObligation {
            schema_version: REPLAN_OBLIGATION_SCHEMA_VERSION.to_string(),
            kind: "succession_gap".to_string(),
            goal_id: goal.goal_id.clone(),
            todo_id: Some(todo.id.clone()),
            raised_at,
            evidence: format!(
                "todo {} completed without declaring a successor or no-follow-up",
                todo.id
            ),
            cleared: cleared_at.is_some(),
            cleared_reason: cleared_at.map(|_| "replan_ack".to_string()),
            cleared_at,
        });
    }

    out
}

/// The unfulfilled (open) obligations — the "record + query" surface.
pub fn unfulfilled_obligations(goal: &Goal) -> Vec<ReplanObligation> {
    detect_obligations(goal)
        .into_iter()
        .filter(|o| !o.cleared)
        .collect()
}

/// Convenience: whether the goal currently owes an unfulfilled replan
/// obligation (used by the CLI projection / stall hints).
pub fn has_unfulfilled_obligation(goal: &Goal) -> bool {
    !unfulfilled_obligations(goal).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Todo;

    #[test]
    fn monitor_below_threshold_raises_nothing() {
        let mut goal = Goal::new("g", "objective", "/tmp");
        goal.add(Todo::monitor(
            "m1",
            "watch",
            std::time::Duration::from_secs(60),
        ));
        assert!(detect_obligations(&goal)
            .iter()
            .all(|o| o.kind != "monitor_no_change_streak"));
    }

    #[test]
    fn monitor_streak_raises_obligation_and_ack_clears_it() {
        let mut goal = Goal::new("g", "objective", "/tmp");
        let mut monitor = Todo::monitor("m1", "watch", std::time::Duration::from_secs(60));
        monitor.consecutive_no_change = MONITOR_NO_CHANGE_OBLIGATION_THRESHOLD;
        monitor.updated_at = 1_000;
        goal.add(monitor);

        let obligations = detect_obligations(&goal);
        let monitor_ob = obligations
            .iter()
            .find(|o| o.kind == "monitor_no_change_streak")
            .expect("obligation raised");
        assert!(!monitor_ob.cleared);
        assert!(has_unfulfilled_obligation(&goal));

        // Ack with a frontier delta after the last poll clears it.
        goal.replan_ack = Some(crate::state::ReplanAck {
            recorded: true,
            delta_kinds: vec!["vision_patch".to_string()],
            at: 1_100,
        });
        let obligations = detect_obligations(&goal);
        let monitor_ob = obligations
            .iter()
            .find(|o| o.kind == "monitor_no_change_streak")
            .unwrap();
        assert!(monitor_ob.cleared);
        assert_eq!(monitor_ob.cleared_reason.as_deref(), Some("replan_ack"));
        assert!(!has_unfulfilled_obligation(&goal));
    }

    #[test]
    fn surface_only_streak_obeys_outcome_floor() {
        let mut goal = Goal::new("g", "objective", "/tmp");
        goal.execution_profile.outcome_floor_streak_threshold = 2;
        goal.outcome_streak = 1;
        assert!(detect_obligations(&goal)
            .iter()
            .all(|o| o.kind != "surface_only_progress_streak"));
        goal.outcome_streak = 2;
        assert!(detect_obligations(&goal)
            .iter()
            .any(|o| o.kind == "surface_only_progress_streak"));
        // Floor disabled → never raised.
        goal.execution_profile.outcome_floor_streak_threshold = 0;
        assert!(detect_obligations(&goal)
            .iter()
            .all(|o| o.kind != "surface_only_progress_streak"));
    }

    #[test]
    fn succession_gap_is_tracked() {
        let mut goal = Goal::new("g", "objective", "/tmp");
        let mut todo = Todo::advancement("t1", "work");
        todo.complete(false, vec![]); // no successor, no no-follow-up
        goal.add(todo);
        assert!(detect_obligations(&goal)
            .iter()
            .any(|o| o.kind == "succession_gap"));
        // Declaring no-follow-up removes the gap.
        let mut goal2 = Goal::new("g2", "objective", "/tmp");
        let mut todo2 = Todo::advancement("t1", "work");
        todo2.complete(true, vec![]);
        goal2.add(todo2);
        assert!(detect_obligations(&goal2)
            .iter()
            .all(|o| o.kind != "succession_gap"));
    }
}
