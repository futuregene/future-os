//! Monitor poll policy executor (P1-3②) — the native subset of LoopX
//! `control_plane/scheduler/monitor_poll_policy.py` +
//! `monitor_poll_writeback.py`, driven by `scheduler tick`.
//!
//! Two halves:
//! - **policy**: classify each open monitor into a poll plan — due vs
//!   waiting vs stalled, with the no-spend-on-unchanged eligibility derived
//!   from the G-12 `monitor_policy` (LoopX `EXTERNAL_MONITOR_POLICIES`).
//!   `scheduler tick` projects the plan so the operator / run loop sees
//!   exactly which read-only polls the cadence has made due.
//! - **writeback**: the cadence-aware next-due derivation shared by the run
//!   path (`executor::writeback`) and event replay (`store::apply`) so a
//!   `1h`-cadence monitor reschedules 1h out instead of the fixed G-8
//!   no-change backoff. The fallback keeps pre-P1-3 replay parity for
//!   monitors without a cadence.

use std::time::SystemTime;

use serde::Serialize;

use crate::state::Goal;

/// Schema version of the poll plan projection.
pub const MONITOR_POLL_PLAN_SCHEMA_VERSION: &str = "monitor_poll_plan_v0";

/// LoopX `EXTERNAL_MONITOR_POLICIES`: the poll policies whose unchanged
/// observations are guaranteed read-only/no-spend.
pub const EXTERNAL_MONITOR_POLICIES: [&str; 2] = [
    "material_transition_only",
    "read_only_observation_then_no_spend_if_unchanged",
];

/// No-spend-on-unchanged eligibility (LoopX
/// `allows_no_spend_external_monitor_poll`, bounded subset): a declared
/// external monitor policy opts in explicitly; an undeclared policy keeps
/// the FO default (monitors are read-only observations and a no-change poll
/// never spends — G-8 quota rule).
pub fn policy_allows_no_spend(policy: Option<&str>) -> bool {
    match policy.map(str::trim) {
        Some(p) if !p.is_empty() => EXTERNAL_MONITOR_POLICIES.contains(&p),
        _ => true,
    }
}

/// Poll interval implied by a cadence declaration, in seconds. Accepts an
/// interval string (`15m` / `1h` / `2d` / `30s`), a cadence class
/// (`hourly` / `daily` / `weekly` — mapped through the G-10 rrule), or a
/// raw MINUTELY rrule. `once` / unknown / absent cadence → `None` (the
/// caller falls back to the fixed no-change backoff).
pub fn cadence_poll_interval_secs(cadence: Option<&str>) -> Option<u64> {
    let cad = cadence?.trim();
    if cad.is_empty() {
        return None;
    }
    if let Some(secs) = super::state::monitor_cadence_secs(cad) {
        return Some(secs);
    }
    super::state::rrule_for_cadence_class(cad)
        .and_then(|r| super::state::scheduler_rrule_interval_minutes(&r))
        .map(|m| (m.max(1)) as u64 * 60)
}

/// Next due epoch after a poll at `polled_at` — cadence-aware with the G-8
/// fixed backoff as fallback. THE single source of truth for monitor
/// rescheduling: the run path and replay both derive the next due through
/// here so a replayed ledger reproduces the live writeback exactly.
pub fn next_poll_due_epoch(polled_at: u64, cadence: Option<&str>) -> u64 {
    polled_at
        + cadence_poll_interval_secs(cadence)
            .unwrap_or(crate::decision::MONITOR_NO_CHANGE_BACKOFF_SECS)
}

/// One due monitor in the poll plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MonitorPollPlanItem {
    pub todo_id: String,
    pub title: String,
    pub target: Option<String>,
    pub policy: Option<String>,
    pub cadence: Option<String>,
    /// Due time (epoch secs).
    pub due_at: u64,
    /// How far past due the monitor is (epoch secs; 0 = exactly due).
    pub overdue_secs: u64,
    pub no_change_count: u32,
    /// No-spend-on-unchanged eligibility (policy classification).
    pub no_spend_if_unchanged: bool,
}

/// The tick-driven poll plan for one goal (LoopX monitor poll policy
/// projection): which monitors are due for their one read-only poll, which
/// stalled (the decision kernel replans those), and when the next poll
/// becomes due.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MonitorPollPlan {
    pub schema_version: String,
    pub goal_id: String,
    pub due_monitors: Vec<MonitorPollPlanItem>,
    /// Ids of monitors past the stall threshold (excluded from `due_monitors`
    /// — the kernel's stall replan owns them).
    pub stalled_monitors: Vec<String>,
    /// Earliest future due time across waiting monitors (epoch secs).
    pub next_due_at: Option<u64>,
}

/// Build the poll plan for a goal at `now` (pure; the caller persists any
/// writeback). Due monitors are ordered most-overdue-first.
pub fn build_poll_plan(goal: &Goal, now: SystemTime) -> MonitorPollPlan {
    let now_epoch = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut due = vec![];
    let mut stalled = vec![];
    let mut next_due_at: Option<u64> = None;
    for m in goal.open_monitors() {
        if crate::decision::stall::is_monitor_stalled(m) {
            stalled.push(m.id.clone());
            continue;
        }
        let due_at = m
            .resume_when
            .and_then(|d| d.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        match due_at {
            Some(d) if d <= now_epoch => due.push(MonitorPollPlanItem {
                todo_id: m.id.clone(),
                title: m.title.clone(),
                target: m.monitor_target.clone(),
                policy: m.monitor_policy.clone(),
                cadence: m.monitor_cadence.clone(),
                due_at: d,
                overdue_secs: now_epoch - d,
                no_change_count: m.consecutive_no_change,
                no_spend_if_unchanged: policy_allows_no_spend(m.monitor_policy.as_deref()),
            }),
            Some(d) => {
                next_due_at = Some(next_due_at.map_or(d, |n| n.min(d)));
            }
            None => {}
        }
    }
    due.sort_by(|a, b| {
        b.overdue_secs
            .cmp(&a.overdue_secs)
            .then(a.todo_id.cmp(&b.todo_id))
    });
    stalled.sort();
    MonitorPollPlan {
        schema_version: MONITOR_POLL_PLAN_SCHEMA_VERSION.to_string(),
        goal_id: goal.goal_id.clone(),
        due_monitors: due,
        stalled_monitors: stalled,
        next_due_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Todo;
    use std::time::Duration;

    #[test]
    fn policy_classification_matches_loopx_vocabulary() {
        assert!(policy_allows_no_spend(Some("material_transition_only")));
        assert!(policy_allows_no_spend(Some(
            "read_only_observation_then_no_spend_if_unchanged"
        )));
        // Unknown declared policy: NOT classified as a no-spend external poll.
        assert!(!policy_allows_no_spend(Some("custom_webhook")));
        // Undeclared keeps the FO default (read-only observation).
        assert!(policy_allows_no_spend(None));
        assert!(policy_allows_no_spend(Some("  ")));
    }

    #[test]
    fn cadence_interval_parses_strings_classes_and_rrules() {
        assert_eq!(cadence_poll_interval_secs(Some("15m")), Some(900));
        assert_eq!(cadence_poll_interval_secs(Some("2d")), Some(2 * 86400));
        assert_eq!(cadence_poll_interval_secs(Some("hourly")), Some(3600));
        assert_eq!(cadence_poll_interval_secs(Some("daily")), Some(86400));
        assert_eq!(cadence_poll_interval_secs(Some("weekly")), Some(604800));
        assert_eq!(
            cadence_poll_interval_secs(Some("FREQ=MINUTELY;INTERVAL=45")),
            Some(2700)
        );
        // `once` and unknown cadences have no recurrence interval.
        assert_eq!(cadence_poll_interval_secs(Some("once")), None);
        assert_eq!(cadence_poll_interval_secs(Some("fortnightly")), None);
        assert_eq!(cadence_poll_interval_secs(None), None);
        assert_eq!(cadence_poll_interval_secs(Some("")), None);
    }

    #[test]
    fn next_due_is_cadence_aware_with_backoff_fallback() {
        assert_eq!(next_poll_due_epoch(1_000, Some("1h")), 1_000 + 3600);
        // No cadence → the fixed G-8 no-change backoff (replay parity).
        assert_eq!(
            next_poll_due_epoch(1_000, None),
            1_000 + crate::decision::MONITOR_NO_CHANGE_BACKOFF_SECS
        );
    }

    #[test]
    fn poll_plan_classifies_due_waiting_and_stalled() {
        let mut g = Goal::new("g1", "o", "/tmp");
        let now = SystemTime::now();
        // Due, most overdue.
        g.add(Todo::monitor_with(
            "M1",
            "old due",
            Some("https://x"),
            Some("material_transition_only"),
            Some("1h"),
            Duration::from_secs(1),
        ));
        g.todo_mut("M1").unwrap().resume_when = Some(now - Duration::from_secs(300));
        // Due, less overdue, unknown policy.
        g.add(Todo::monitor("M2", "due", Duration::from_secs(1)));
        g.todo_mut("M2").unwrap().monitor_policy = Some("custom_webhook".into());
        g.todo_mut("M2").unwrap().resume_when = Some(now - Duration::from_secs(60));
        // Waiting (due in the future).
        g.add(Todo::monitor("M3", "waiting", Duration::from_secs(3600)));
        // Stalled (counter at threshold) — excluded from due, listed separately.
        g.add(Todo::monitor("M4", "stalled", Duration::from_secs(1)));
        g.todo_mut("M4").unwrap().consecutive_no_change =
            crate::decision::MONITOR_NO_CHANGE_REPLAN_THRESHOLD;

        let plan = build_poll_plan(&g, now);
        assert_eq!(plan.schema_version, MONITOR_POLL_PLAN_SCHEMA_VERSION);
        assert_eq!(plan.stalled_monitors, vec!["M4".to_string()]);
        assert_eq!(plan.due_monitors.len(), 2);
        // Most overdue first.
        assert_eq!(plan.due_monitors[0].todo_id, "M1");
        assert!(plan.due_monitors[0].no_spend_if_unchanged);
        assert_eq!(plan.due_monitors[0].overdue_secs, 300);
        assert_eq!(plan.due_monitors[1].todo_id, "M2");
        assert!(!plan.due_monitors[1].no_spend_if_unchanged);
        // Next due ≈ M3's 3600s-out time.
        let next = plan.next_due_at.expect("waiting monitor sets next due");
        let now_e = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!((now_e + 3599..=now_e + 3600).contains(&next));
    }

    #[test]
    fn empty_goal_plans_empty() {
        let g = Goal::new("g1", "o", "/tmp");
        let plan = build_poll_plan(&g, SystemTime::now());
        assert!(plan.due_monitors.is_empty());
        assert!(plan.stalled_monitors.is_empty());
        assert_eq!(plan.next_due_at, None);
    }
}
