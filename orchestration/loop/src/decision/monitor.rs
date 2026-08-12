//! Monitor evaluation — the monitor lane decides between replan (stall),
//! one read-only poll (due), or quiet wait with backoff (none due).

use std::time::SystemTime;

use crate::state::{Goal, Todo};

use super::stall::is_monitor_stalled;

/// Unchanged-poll backoff before the next poll (seconds) — the executor and
/// the `MonitorPolled` event replay share this constant so both stay in sync.
pub const MONITOR_NO_CHANGE_BACKOFF_SECS: u64 = 12;

/// Classify a monitor poll result (G-8): `changed` (material transition —
/// counter resets, monitor may close) or `no_change` (counter advances).
/// Returns `(result, resulting_no_change_count)`.
pub fn monitor_poll_classification(
    changed: bool,
    consecutive_no_change: u32,
) -> (&'static str, u32) {
    if changed {
        ("changed", 0)
    } else {
        ("no_change", consecutive_no_change + 1)
    }
}

/// Monitor lane outcome for the current decision.
pub(crate) enum MonitorOutcome<'a> {
    /// A monitor has stalled: consecutive no-change polls hit the threshold.
    Stalled(&'a Todo),
    /// At least one monitor is due: one read-only poll, no spend on no-change.
    Due(Vec<&'a Todo>),
    /// Monitors are open but none due: quiet wait, carrying the next-due
    /// backoff in milliseconds.
    Waiting(Option<u64>),
    /// No monitors open.
    None,
}

/// Evaluate the monitor lane. Mirrors the original pipeline order:
/// stall → replan, due → one poll, present-but-not-due → quiet wait.
pub(crate) fn monitor_outcome(goal: &Goal, now: SystemTime) -> MonitorOutcome<'_> {
    let monitors: Vec<&Todo> = goal.open_monitors().collect();
    if let Some(stalled) = monitors.iter().find(|m| is_monitor_stalled(m)) {
        return MonitorOutcome::Stalled(stalled);
    }
    let due: Vec<&Todo> = monitors
        .iter()
        .filter(|m| m.resume_when.is_some_and(|d| d <= now))
        .copied()
        .collect();
    if !due.is_empty() {
        return MonitorOutcome::Due(due);
    }
    if !monitors.is_empty() {
        let next_due_ms = monitors
            .iter()
            .filter_map(|m| m.resume_when)
            .min()
            .and_then(|d| d.duration_since(now).ok().map(|x| x.as_millis() as u64));
        return MonitorOutcome::Waiting(next_due_ms);
    }
    MonitorOutcome::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::stall::MONITOR_NO_CHANGE_REPLAN_THRESHOLD;
    use crate::state::{Goal, Todo};
    use std::time::{Duration, SystemTime};

    fn now() -> SystemTime {
        SystemTime::now()
    }

    #[test]
    fn no_monitors_is_none() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T1", "work"));
        assert!(matches!(monitor_outcome(&g, now()), MonitorOutcome::None));
    }

    #[test]
    fn stalled_monitor_wins_over_due() {
        let mut g = Goal::new("g", "o", "/tmp");
        let n = now();
        g.add(Todo::monitor("M1", "watch", Duration::from_secs(3600)));
        g.todo_mut("M1").unwrap().consecutive_no_change = MONITOR_NO_CHANGE_REPLAN_THRESHOLD;
        // Due AND stalled — the stall replan must win (pipeline order).
        g.todo_mut("M1").unwrap().resume_when = Some(n - Duration::from_secs(60));
        assert!(
            matches!(monitor_outcome(&g, n), MonitorOutcome::Stalled(m) if m.id == "M1"),
            "stall must take precedence over due"
        );
    }

    #[test]
    fn due_monitor_polls_once() {
        let mut g = Goal::new("g", "o", "/tmp");
        let n = now();
        g.add(Todo::monitor("M1", "watch", Duration::from_secs(3600)));
        g.todo_mut("M1").unwrap().resume_when = Some(n - Duration::from_secs(60));
        assert!(
            matches!(monitor_outcome(&g, n), MonitorOutcome::Due(ref due) if due.len() == 1 && due[0].id == "M1"),
            "expected Due"
        );
    }

    #[test]
    fn undue_monitor_waits_with_backoff() {
        let mut g = Goal::new("g", "o", "/tmp");
        let n = now();
        g.add(Todo::monitor("M1", "watch", Duration::from_secs(3600)));
        g.todo_mut("M1").unwrap().resume_when = Some(n + Duration::from_secs(3600));
        assert!(
            matches!(monitor_outcome(&g, n), MonitorOutcome::Waiting(Some(ms)) if (3_599_000..=3_600_000).contains(&ms)),
            "expected Waiting with next-due backoff ~3600s"
        );
    }
}
