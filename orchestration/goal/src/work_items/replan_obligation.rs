//! Autonomous replan obligation bookkeeping (G-13) — record + query, in
//! minimal form mirroring LoopX `control_plane/work_items/autonomous_replan_obligation.py`
//! (870 lines; we deliberately skip the queue/injection machinery).
//!
//! Obligations are DERIVED from kernel signals (monitor no-change streak,
//! surface-only progress streak, succession gap, exploration gap) and tracked
//! against the ReplanAcked ledger: an ack with a frontier-changing delta
//! recorded after the obligation's trigger clears it. The ack record itself is
//! event-sourced (`ReplanAcked`); the obligation view is a read model.

use serde::Serialize;

use crate::state::{FailureKind, Goal, TaskClass, TodoStatus};

pub const REPLAN_OBLIGATION_SCHEMA_VERSION: &str = "replan_obligation_v0";
/// LoopX `AUTONOMOUS_REPLAN_STALL_THRESHOLD` (2 consecutive stalled turns).
pub const AUTONOMOUS_REPLAN_STALL_THRESHOLD: u32 = 2;
/// Monitor no-change streak that raises an obligation (aligned with
/// `MONITOR_NO_CHANGE_REPLAN_THRESHOLD` in the decision kernel).
pub const MONITOR_NO_CHANGE_OBLIGATION_THRESHOLD: u32 = 3;

/// Obligation kind: the goal owes an EXPLORATION (a direction proposal), not
/// another attempt at the same slice. Raised from ledger signals only; it
/// never creates work — dispatching a scout is the supervisor's decision.
pub const EXPLORATION_GAP_KIND: &str = "exploration_gap";

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
/// streaks, surface-only progress streaks, completion without closure
/// intent, and an exploration gap (the current plan stopped producing
/// outcomes). An obligation is `cleared` when the trigger resolved or a
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

    // 4) Exploration gap: the current approach has stopped producing
    //    outcomes, so the goal owes a direction proposal (see
    //    `exploration_gap_obligations`).
    out.extend(exploration_gap_obligations(goal));

    out
}

/// Consecutive trailing runs of `todo_id` that the verify gate rejected. A
/// different classification (infra retry, hard error, a success) breaks the
/// streak: the signal is the SAME cause repeating, not failures in general.
fn consecutive_science_verify_failures(goal: &Goal, todo_id: &str) -> u32 {
    let mut streak = 0;
    for run in goal.history.iter().rev().filter(|r| r.todo_id == todo_id) {
        if run.failure_kind == Some(FailureKind::ScienceVerifyFailed) {
            streak += 1;
        } else {
            break;
        }
    }
    streak
}

/// The last run recorded for `todo_id` (its timestamp dates the trigger).
fn last_run_at(goal: &Goal, todo_id: &str) -> Option<u64> {
    goal.history
        .iter()
        .rev()
        .find(|r| r.todo_id == todo_id)
        .map(|r| r.recorded_at)
}

/// `exploration_gap` obligations — the goal owes an EXPLORATION, not another
/// attempt at the same slice. The product of a scout is a direction proposal
/// (question / observation / evidence / candidates, each candidate carrying a
/// `kill_when`), which is what this gap asks for.
///
/// Record-and-project only, exactly like the other kinds: raising the gap does
/// NOT create a scout todo. Whether (and where) to dispatch exploration is the
/// supervisor's call — auto-creating work here would let the kernel invent a
/// plan.
///
/// Signals, all read off the existing ledger (no new inputs, no new numbers):
/// 1. a runnable todo whose `failed_attempts` reached the stall threshold, or
///    whose last two runs were both rejected by the verify gate — repeating it
///    is not a plan;
/// 2. the goal's surface-only outcome streak at the configured outcome floor
///    (`0` = disabled, so a goal that never configured a floor owes nothing).
///
/// At most one row per todo and one for the goal: a todo that trips several
/// conditions yields a single obligation with combined evidence, and this is a
/// pure projection of the ledger, so it can never append a fresh row on each
/// wake-up.
///
/// A stalled MONITOR deliberately raises nothing here: the identical
/// `consecutive_no_change` counter already owns `monitor_no_change_streak`
/// above, and a second row for one counter is precisely the duplicate this
/// detector avoids (one row per stalled monitor is pinned by
/// `tests/lease_contract.rs`).
fn exploration_gap_obligations(goal: &Goal) -> Vec<ReplanObligation> {
    let mut out: Vec<ReplanObligation> = vec![];

    for todo in &goal.todos {
        // Finished work owes no exploration for its own failures.
        if matches!(todo.status, TodoStatus::Done | TodoStatus::Superseded) {
            continue;
        }
        let failed = todo.failed_attempts >= AUTONOMOUS_REPLAN_STALL_THRESHOLD;
        let rejections = consecutive_science_verify_failures(goal, &todo.id);
        let rejected = rejections >= AUTONOMOUS_REPLAN_STALL_THRESHOLD;
        if !failed && !rejected {
            continue;
        }
        let mut causes: Vec<String> = vec![];
        if failed {
            causes.push(format!("{} failed attempts", todo.failed_attempts));
        }
        if rejected {
            causes.push(format!(
                "the last {rejections} attempts were rejected by the verify gate"
            ));
        }
        let raised_at = last_run_at(goal, &todo.id).unwrap_or(todo.updated_at);
        let cleared_at = ack_clears(goal, raised_at);
        out.push(ReplanObligation {
            schema_version: REPLAN_OBLIGATION_SCHEMA_VERSION.to_string(),
            kind: EXPLORATION_GAP_KIND.to_string(),
            goal_id: goal.goal_id.clone(),
            todo_id: Some(todo.id.clone()),
            raised_at,
            evidence: format!(
                "todo {} is stuck on one slice ({}): the approach needs a different direction, not another retry",
                todo.id,
                causes.join("; ")
            ),
            cleared: cleared_at.is_some(),
            cleared_reason: cleared_at.map(|_| "replan_ack".to_string()),
            cleared_at,
        });
    }

    // Goal-level: the lane keeps turning without a material outcome.
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
            kind: EXPLORATION_GAP_KIND.to_string(),
            goal_id: goal.goal_id.clone(),
            todo_id: None,
            raised_at,
            evidence: format!(
                "{} consecutive surface-only turns without a material outcome (floor={}): the goal owes a direction proposal",
                goal.outcome_streak, floor
            ),
            cleared: false,
            cleared_reason: None,
            cleared_at: None,
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
        let obligations = detect_obligations(&goal);
        assert!(
            obligations.is_empty(),
            "a monitor below its change threshold must raise nothing: {obligations:?}"
        );
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
        // Below the floor nothing is raised — and the fixture yields an EMPTY
        // list, so `is_empty` is the claim that can fail. (The `.all(kind != X)`
        // this replaces could never fail: with an empty list the closure never
        // ran. Same vacuity as the disabled-floor case below.)
        assert!(
            detect_obligations(&goal).is_empty(),
            "below the floor nothing is raised"
        );
        goal.outcome_streak = 2;
        assert!(detect_obligations(&goal)
            .iter()
            .any(|o| o.kind == "surface_only_progress_streak"));
        // Floor disabled → never raised.
        goal.execution_profile.outcome_floor_streak_threshold = 0;
        // A disabled floor raises nothing at all. Asserting `.all(kind != X)` here
        // was VACUOUS: the list is empty, so the closure never ran and the
        // assertion could not fail. `is_empty` is the claim that can fail.
        let disabled = detect_obligations(&goal);
        assert!(
            disabled.is_empty(),
            "a disabled floor must raise no obligations: {disabled:?}"
        );
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
        let cleared = detect_obligations(&goal2);
        assert!(
            cleared.is_empty(),
            "a goal whose only todo declared no-follow-up owes nothing: {cleared:?}"
        );
    }

    fn run_rec(
        todo_id: &str,
        failure_kind: Option<FailureKind>,
        recorded_at: u64,
    ) -> crate::state::RunRecord {
        use crate::state::RunRecord;
        RunRecord {
            agent_id: None,
            turn: 1,
            todo_id: todo_id.to_string(),
            run_id: format!("run-{todo_id}-{recorded_at}"),
            terminal_state: "completed".to_string(),
            error: None,
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: vec![],
            evidence: String::new(),
            recorded_at,
            spend_source: Some("run".to_string()),
            failure_kind,
            validation: None,
            truncation: None,
        }
    }

    fn exploration_gaps(goal: &Goal) -> Vec<ReplanObligation> {
        detect_obligations(goal)
            .into_iter()
            .filter(|o| o.kind == EXPLORATION_GAP_KIND)
            .collect()
    }

    /// The gap rises when a todo's repair budget is spent — that is the point
    /// where repeating the slice stops being a plan — and stays down one
    /// attempt earlier. Finished work owes no exploration.
    #[test]
    fn exploration_gap_rises_at_the_failed_attempt_threshold() {
        let mut goal = Goal::new("g", "objective", "/tmp");
        goal.add(Todo::advancement("t1", "try to land it"));
        goal.todo_mut("t1").unwrap().failed_attempts = AUTONOMOUS_REPLAN_STALL_THRESHOLD - 1;
        assert!(
            exploration_gaps(&goal).is_empty(),
            "one failed attempt is still a retry, not an exploration: {:?}",
            exploration_gaps(&goal)
        );

        goal.todo_mut("t1").unwrap().failed_attempts = AUTONOMOUS_REPLAN_STALL_THRESHOLD;
        let gaps = exploration_gaps(&goal);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].todo_id.as_deref(), Some("t1"));
        assert!(!gaps[0].cleared);
        assert!(gaps[0].evidence.contains("failed attempts"), "{gaps:?}");
        assert!(has_unfulfilled_obligation(&goal));

        // A completed todo's failure history is settled work, not a gap.
        goal.todo_mut("t1").unwrap().status = TodoStatus::Done;
        assert!(exploration_gaps(&goal).is_empty());
    }

    /// Two consecutive verify-gate rejections are the same cause repeating —
    /// a different classification in between breaks the streak, because that is
    /// a different failure, not a stuck approach.
    #[test]
    fn exploration_gap_rises_on_repeated_verify_gate_rejections() {
        let mut goal = Goal::new("g", "objective", "/tmp");
        goal.add(Todo::advancement("t1", "land the verified output"));
        goal.history
            .push(run_rec("t1", Some(FailureKind::ScienceVerifyFailed), 100));
        assert!(
            exploration_gaps(&goal).is_empty(),
            "a single rejection is a repair, not an exploration"
        );

        goal.history
            .push(run_rec("t1", Some(FailureKind::ScienceVerifyFailed), 200));
        let gaps = exploration_gaps(&goal);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].raised_at, 200, "dated by the last rejection");
        assert!(
            gaps[0].evidence.contains("rejected by the verify gate"),
            "{gaps:?}"
        );

        // An infra retry in between breaks the streak: it is a different cause.
        goal.history
            .push(run_rec("t1", Some(FailureKind::InfraRecoverable), 300));
        assert!(
            exploration_gaps(&goal).is_empty(),
            "a rate-limit retry is not the same failure repeating: {:?}",
            exploration_gaps(&goal)
        );
    }

    /// One row per todo and one per goal: a todo tripping several signals is
    /// still ONE obligation (with combined evidence), and re-deriving the
    /// projection never appends a second row.
    #[test]
    fn exploration_gap_is_deduped_per_todo_and_per_goal() {
        let mut goal = Goal::new("g", "objective", "/tmp");
        goal.add(Todo::advancement("t1", "stuck slice"));
        goal.todo_mut("t1").unwrap().failed_attempts = 5;
        for (at, kind) in [
            (100, FailureKind::ScienceVerifyFailed),
            (200, FailureKind::ScienceVerifyFailed),
            (300, FailureKind::ScienceVerifyFailed),
        ] {
            goal.history.push(run_rec("t1", Some(kind), at));
        }
        let gaps = exploration_gaps(&goal);
        assert_eq!(gaps.len(), 1, "three signals, one obligation: {gaps:?}");
        assert!(
            gaps[0].evidence.contains("failed attempts")
                && gaps[0].evidence.contains("rejected by the verify gate"),
            "the single row must carry both causes: {gaps:?}"
        );
        // Pure projection: re-running on the same ledger cannot grow the list.
        assert_eq!(exploration_gaps(&goal).len(), 1);
    }

    /// The goal-level gap follows the configured outcome floor (0 disables it,
    /// as everywhere else in this detector) and a frontier-changing replan ack
    /// clears the todo-scoped one.
    #[test]
    fn goal_level_exploration_gap_follows_the_floor_and_an_ack_clears_it() {
        let mut goal = Goal::new("g", "objective", "/tmp");
        goal.add(Todo::advancement("t1", "stuck slice"));
        goal.todo_mut("t1").unwrap().failed_attempts = AUTONOMOUS_REPLAN_STALL_THRESHOLD;
        goal.todo_mut("t1").unwrap().updated_at = 500;
        assert!(exploration_gaps(&goal)[0].todo_id.is_some());

        goal.replan_ack = Some(crate::state::ReplanAck {
            recorded: true,
            delta_kinds: vec!["successor_or_supersede".to_string()],
            at: 900,
        });
        assert!(
            exploration_gaps(&goal)[0].cleared,
            "a frontier-changing ack after the raise clears it"
        );
        goal.replan_ack = None;

        // Goal-level: an unconfigured floor never raises.
        goal.outcome_streak = 99;
        assert!(
            exploration_gaps(&goal).iter().all(|o| o.todo_id.is_some()),
            "a floor of 0 must not raise the goal-level gap"
        );
        goal.execution_profile.outcome_floor_streak_threshold = 2;
        let goal_level = exploration_gaps(&goal)
            .into_iter()
            .find(|o| o.todo_id.is_none())
            .expect("a breached floor owes a goal-level exploration gap");
        assert!(
            goal_level.evidence.contains("direction proposal"),
            "{goal_level:?}"
        );
    }

    /// A stalled monitor keeps exactly ONE obligation row: its
    /// `consecutive_no_change` counter is already owned by
    /// `monitor_no_change_streak`, and a second row for the same counter is the
    /// duplicate this detector exists to avoid.
    #[test]
    fn a_stalled_monitor_is_one_obligation_not_two() {
        let mut goal = Goal::new("g", "objective", "/tmp");
        let mut monitor = Todo::monitor("m1", "watch", std::time::Duration::from_secs(60));
        monitor.consecutive_no_change = MONITOR_NO_CHANGE_OBLIGATION_THRESHOLD;
        goal.add(monitor);

        let obligations = detect_obligations(&goal);
        assert_eq!(obligations.len(), 1, "{obligations:?}");
        assert_eq!(obligations[0].kind, "monitor_no_change_streak");
        assert!(exploration_gaps(&goal).is_empty());
    }
}
