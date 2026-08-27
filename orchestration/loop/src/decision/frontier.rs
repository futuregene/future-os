//! Frontier — the runnable work lane: priority-sorted advancement frontier,
//! the work lane selector, and the frontier projection snapshot.

use std::time::SystemTime;

use crate::contract::FrontierProjection;
use crate::state::{Goal, TaskClass, Todo};

/// The runnable advancement frontier for an identity, priority-sorted
/// (P0 before P1 before P2 — LoopX sorts the frontier).
pub(crate) fn sorted_runnable<'a>(goal: &'a Goal, agent_id: Option<&'a str>) -> Vec<&'a Todo> {
    let mut runnable: Vec<&Todo> = goal.runnable_advancement_for(agent_id).collect();
    runnable.sort_by_key(|t| t.priority);
    runnable
}

/// Work lane: the monitor lane when any monitor is open, else the
/// advancement-task lane.
pub(crate) fn lane(goal: &Goal) -> &'static str {
    if goal.open_of(TaskClass::Monitor).next().is_some() {
        "monitor"
    } else {
        "advancement_task"
    }
}

/// Compose the frontier projection snapshot: replan pressure plus the
/// runnable-frontier and monitor counts.
pub(crate) fn frontier_projection(goal: &Goal, replan_required: bool) -> FrontierProjection {
    // Owner-aware pending count: the naive `runnable_advancement().count()`
    // is the shared-pool (agent_id=None) frontier and so drops every
    // owner-scoped todo, reporting `unclaimed_advancement=0` while an
    // all-owner-scoped goal still has real work waiting on its owners.
    let (claimable, owner_scoped) = goal.pending_advancement_owner_aware();
    FrontierProjection {
        replan_required,
        current_agent_advancement: goal
            .runnable_advancement()
            .filter(|t| t.failed_attempts > 0)
            .count(),
        unclaimed_advancement: claimable.len() + owner_scoped.len(),
        acceptance_gaps: goal.unsatisfied_gaps().len(),
        monitors_open: goal.open_monitors().count(),
        monitors_due: goal
            .open_monitors()
            .filter(|m| m.resume_when.is_some_and(|d| d <= SystemTime::now()))
            .count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Goal, Priority, Todo};
    use std::time::{Duration, SystemTime};

    #[test]
    fn sorted_runnable_orders_p0_before_p1_before_p2() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T2", "two"));
        g.add(Todo::advancement("T0", "zero"));
        g.add(Todo::advancement("T1", "one"));
        g.todo_mut("T0").unwrap().priority = Priority::P0;
        g.todo_mut("T2").unwrap().priority = Priority::P2;
        let ids: Vec<&str> = sorted_runnable(&g, None)
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(ids, vec!["T0", "T1", "T2"]);
    }

    #[test]
    fn lane_is_monitor_when_monitors_open() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T1", "work"));
        assert_eq!(lane(&g), "advancement_task");
        g.add(Todo::monitor("M1", "watch", Duration::from_secs(60)));
        assert_eq!(lane(&g), "monitor");
    }

    #[test]
    fn frontier_projection_reports_counts_and_replan_flag() {
        let mut g = Goal::new("g", "o", "/tmp").with_acceptance(vec![("A1", "match")]);
        g.add(Todo::advancement("T1", "work"));
        g.add(Todo::monitor("M1", "watch", Duration::from_secs(60)));
        g.todo_mut("M1").unwrap().resume_when = Some(SystemTime::now() - Duration::from_secs(10));
        let fp = frontier_projection(&g, true);
        assert!(fp.replan_required);
        assert_eq!(fp.unclaimed_advancement, 1);
        assert_eq!(fp.current_agent_advancement, 0);
        assert_eq!(fp.acceptance_gaps, 1);
        assert_eq!(fp.monitors_open, 1);
        assert_eq!(fp.monitors_due, 1);
    }

    #[test]
    fn frontier_projection_counts_owner_scoped_todos_as_pending() {
        // Regression: the naive `runnable_advancement().count()` is the
        // shared-pool (agent_id=None) frontier and dropped owner-scoped
        // todos, reporting unclaimed_advancement=0 while work waited on its
        // owners. The projection must count owner-scoped todos as pending.
        let mut g = Goal::new("g", "o", "/tmp");
        let mut owned = Todo::advancement("T1", "owned work");
        owned.owner = Some("solver-a".to_string());
        g.add(owned);
        g.add(Todo::advancement("T2", "shared work"));
        let fp = frontier_projection(&g, false);
        assert_eq!(
            fp.unclaimed_advancement, 2,
            "owner-scoped + shared todos both count as pending"
        );
    }

    #[test]
    fn pending_advancement_owner_aware_splits_shared_from_owned() {
        let mut g = Goal::new("g", "o", "/tmp");
        let mut a = Todo::advancement("T1", "owned A");
        a.owner = Some("solver-a".to_string());
        let mut b = Todo::advancement("T2", "owned B");
        b.owner = Some("solver-b".to_string());
        g.add(a);
        g.add(b);
        g.add(Todo::advancement("T3", "shared"));
        let (claimable, owner_scoped) = g.pending_advancement_owner_aware();
        let claimable_ids: Vec<&str> = claimable.iter().map(|t| t.id.as_str()).collect();
        let owned_ids: Vec<&str> = owner_scoped.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            claimable_ids,
            vec!["T3"],
            "only the unowned todo is shared-pool claimable"
        );
        assert_eq!(
            owned_ids,
            vec!["T1", "T2"],
            "both owner-scoped todos are pending"
        );
    }

    #[test]
    fn todo_blocks_todo_excluded_until_predecessor_done() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T1", "first"));
        g.add(Todo::advancement("T2", "second").blocking(&["T1"]));
        // T2 blocked while T1 open (todo→todo dependency enforced).
        let ids: Vec<&str> = sorted_runnable(&g, None)
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(ids, vec!["T1"], "T2 must wait for T1");
        // Completing T1 unblocks T2.
        g.todo_mut("T1").unwrap().complete(true, vec![]);
        let ids: Vec<&str> = sorted_runnable(&g, None)
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(ids, vec!["T2"]);
    }

    #[test]
    fn superseded_predecessor_does_not_block() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T1", "obsolete route"));
        g.add(Todo::advancement("T2", "second").blocking(&["T1"]));
        g.todo_mut("T1").unwrap().status = crate::state::TodoStatus::Superseded;
        let ids: Vec<&str> = sorted_runnable(&g, None)
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["T2"],
            "superseded predecessor must not wedge the goal"
        );
    }

    #[test]
    fn unknown_predecessor_does_not_block_frontier() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T2", "second").blocking(&["T-missing"]));
        let ids: Vec<&str> = sorted_runnable(&g, None)
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["T2"],
            "unknown ids are flagged by task-graph, not wedged"
        );
    }

    #[test]
    fn gate_predecessor_blocks_until_resolved() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::user_gate("G1", "approve?", &[]));
        g.add(Todo::advancement("T2", "gated work").blocking(&["G1"]));
        assert!(sorted_runnable(&g, None).is_empty(), "open gate blocks T2");
        g.todo_mut("G1").unwrap().decision = Some("approved".to_string());
        g.todo_mut("G1").unwrap().status = crate::state::TodoStatus::Done;
        let ids: Vec<&str> = sorted_runnable(&g, None)
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(ids, vec!["T2"], "resolved gate releases T2");
    }
}
