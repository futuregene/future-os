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
    FrontierProjection {
        replan_required,
        current_agent_advancement: goal
            .runnable_advancement()
            .filter(|t| t.failed_attempts > 0)
            .count(),
        unclaimed_advancement: goal.runnable_advancement().count(),
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
}
