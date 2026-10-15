//! Goal-frontier replan rule selection (G13 ②) — the reference
//! `goal_frontier/replan_rules.py` (197 lines), natively.
//!
//! The disposition→decision interpreter: an ORDERED policy table of replan
//! trigger rules. Given the goal's current facts, the first matching rule
//! (in policy order) is the decision — carrying whether it DERIVES a replan
//! obligation and, when it does, which obligation kind is owed.
//!
//! A goal may carry an explicit rule set (folded from the
//! `ReplanRuleSetUpdated` event; latest wins). Without one the default rule
//! set applies. Selection walks the ACTIVE set's ordered rule ids; unknown
//! ids are skipped and the final rule (`monitor_frontier_exhausted`,
//! unconditionally matching) guarantees a terminal decision.

use serde::{Deserialize, Serialize};

use crate::state::Goal;
use crate::work_items::replan_obligation::MONITOR_NO_CHANGE_OBLIGATION_THRESHOLD;

pub const REPLAN_RULE_DECISION_SCHEMA_VERSION: &str = "goal_frontier_replan_rule_decision_v0";
pub const DEFAULT_REPLAN_RULE_SET_VERSION: &str = "goal_frontier_replan_rules_v0";

/// Rule ids (reference `GoalFrontierReplanRule` — the core subset our
/// native kernel can decide; handoff-gate / long-chain / watch-lane rules
/// are deliberately skipped, matching the existing decision pipeline).
pub const RULE_EXISTING_OBLIGATION: &str = "existing_obligation";
pub const RULE_OPEN_USER_TODO: &str = "open_user_todo";
pub const RULE_TODO_SUCCESSION_GAP: &str = "todo_succession_gap";
pub const RULE_VISION_ACCEPTANCE_GAP: &str = "vision_acceptance_gap";
pub const RULE_MONITOR_NO_CHANGE_STREAK: &str = "monitor_no_change_streak";
pub const RULE_NOT_MONITOR_ONLY: &str = "not_monitor_only";
pub const RULE_NO_OPEN_MONITOR: &str = "no_open_monitor";
pub const RULE_ADVANCEMENT_REMAINS: &str = "advancement_remains";
pub const RULE_MONITOR_FRONTIER_EXHAUSTED: &str = "monitor_frontier_exhausted";

/// The default rule set: the full builtin rule ids in policy order.
pub fn default_rule_set() -> ReplanRuleSet {
    ReplanRuleSet {
        schema_version: DEFAULT_REPLAN_RULE_SET_VERSION.to_string(),
        rule_ids: vec![
            RULE_EXISTING_OBLIGATION.to_string(),
            RULE_OPEN_USER_TODO.to_string(),
            RULE_TODO_SUCCESSION_GAP.to_string(),
            RULE_VISION_ACCEPTANCE_GAP.to_string(),
            RULE_MONITOR_NO_CHANGE_STREAK.to_string(),
            RULE_NOT_MONITOR_ONLY.to_string(),
            RULE_NO_OPEN_MONITOR.to_string(),
            RULE_ADVANCEMENT_REMAINS.to_string(),
            RULE_MONITOR_FRONTIER_EXHAUSTED.to_string(),
        ],
    }
}

/// A goal's active replan rule set (folded from `ReplanRuleSetUpdated`;
/// absent → the default set applies).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplanRuleSet {
    pub schema_version: String,
    /// Rule ids in policy order. Empty on the wire means "reset to the
    /// default set"; the folded state never stores an empty list.
    pub rule_ids: Vec<String>,
}

impl ReplanRuleSet {
    /// The effective ordered rule ids: the declared set AS-IS when it names
    /// at least one known rule (full replace — the caller's order wins);
    /// otherwise the default order (a set of only unknown ids cannot
    /// displace the builtin policy table).
    pub fn effective_rule_ids(&self) -> Vec<String> {
        if self.rule_ids.iter().any(|id| is_known_rule(id)) {
            self.rule_ids.clone()
        } else {
            default_rule_set().rule_ids
        }
    }
}

/// The facts each rule matches on (reference `GoalFrontierReplanFacts`,
/// core subset — handoff/watch-lane/chain facts dropped with their rules).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplanFacts {
    /// An existing scoped obligation stays authoritative: the outcome floor
    /// raised a `surface_only_progress_streak` obligation (the read model in
    /// `work_items::replan_obligation`). The specific trigger rules below
    /// own their own obligation kinds.
    pub existing_replan_required: bool,
    /// Open user gates + open user actions blocking the frontier.
    pub blocking_user_open_count: usize,
    /// Done advancement without successor/no-follow-up.
    pub succession_gap_count: usize,
    /// Unsatisfied acceptance gaps.
    pub acceptance_gap_count: usize,
    /// Runnable (selectable) advancement todos on the frontier.
    pub selectable_frontier_advancement: usize,
    /// Advancement todos still on the frontier (open/blocked/deferred —
    /// selectable or not, e.g. held by another agent's lease).
    pub advancement_open_count: usize,
    /// Open monitors.
    pub monitor_count: usize,
    /// Any open monitor at/above the no-change obligation threshold.
    pub monitor_no_change_streak_triggered: bool,
    /// No runnable advancement remains — only monitor work can be lane work.
    pub monitor_only_lane: bool,
}

/// Derive the replan facts from goal state (pure; deterministic).
pub fn facts_for_goal(goal: &Goal) -> ReplanFacts {
    let runnable = goal.runnable_advancement().count();
    let advancement_open = goal
        .todos
        .iter()
        .filter(|t| {
            t.class == crate::state::TaskClass::Advancement
                && !matches!(
                    t.status,
                    crate::state::TodoStatus::Done | crate::state::TodoStatus::Superseded
                )
        })
        .count();
    ReplanFacts {
        // The one obligation kind not covered by a specific rule below: the
        // outcome floor's surface-only streak (reference: an existing scoped
        // obligation remains authoritative).
        existing_replan_required: crate::work_items::replan_obligation::unfulfilled_obligations(
            goal,
        )
        .iter()
        .any(|o| o.kind == "surface_only_progress_streak"),
        blocking_user_open_count: goal.open_gates().count()
            + goal.open_of(crate::state::TaskClass::UserAction).count(),
        succession_gap_count: goal.completed_without_closure_intent().len(),
        acceptance_gap_count: goal.unsatisfied_gaps().len(),
        selectable_frontier_advancement: runnable,
        advancement_open_count: advancement_open,
        monitor_count: goal.open_monitors().count(),
        monitor_no_change_streak_triggered: goal
            .open_monitors()
            .any(|m| m.consecutive_no_change >= MONITOR_NO_CHANGE_OBLIGATION_THRESHOLD),
        monitor_only_lane: runnable == 0,
    }
}

/// One replan rule decision (reference `GoalFrontierReplanRuleDecision`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplanRuleDecision {
    pub schema_version: String,
    pub rule: String,
    pub rule_index: u32,
    pub derives_obligation: bool,
    /// The obligation kind this rule owes when `derives_obligation`
    /// (`replan_obligation_v0` vocabulary); `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obligation_kind: Option<String>,
    pub reason: String,
}

/// Whether a rule id is known to the builtin policy table.
pub fn is_known_rule(id: &str) -> bool {
    matches!(
        id,
        RULE_EXISTING_OBLIGATION
            | RULE_OPEN_USER_TODO
            | RULE_TODO_SUCCESSION_GAP
            | RULE_VISION_ACCEPTANCE_GAP
            | RULE_MONITOR_NO_CHANGE_STREAK
            | RULE_NOT_MONITOR_ONLY
            | RULE_NO_OPEN_MONITOR
            | RULE_ADVANCEMENT_REMAINS
            | RULE_MONITOR_FRONTIER_EXHAUSTED
    )
}

/// Select the first matching rule in the ACTIVE rule set's policy order.
/// `monitor_frontier_exhausted` always matches — the terminal rule.
pub fn select_replan_rule_with_set(facts: &ReplanFacts, rule_ids: &[String]) -> ReplanRuleDecision {
    let ordered: Vec<(u32, &str)> = rule_ids
        .iter()
        .enumerate()
        .filter(|(_, id)| is_known_rule(id))
        .map(|(i, id)| (i as u32, id.as_str()))
        .collect();
    let decision_for = |rule: &str, rule_index: u32, reason: &str| {
        let (derives, obligation_kind) = match rule {
            RULE_TODO_SUCCESSION_GAP => (true, Some("succession_gap".to_string())),
            RULE_VISION_ACCEPTANCE_GAP => (true, Some("vision_acceptance_gap".to_string())),
            RULE_MONITOR_NO_CHANGE_STREAK => (true, Some("monitor_no_change_streak".to_string())),
            RULE_MONITOR_FRONTIER_EXHAUSTED => {
                (true, Some("surface_only_progress_streak".to_string()))
            }
            _ => (false, None),
        };
        ReplanRuleDecision {
            schema_version: REPLAN_RULE_DECISION_SCHEMA_VERSION.to_string(),
            rule: rule.to_string(),
            rule_index,
            derives_obligation: derives,
            obligation_kind,
            reason: reason.to_string(),
        }
    };
    let matching = |rule: &str| -> Option<&'static str> {
        Some(match rule {
            RULE_EXISTING_OBLIGATION if facts.existing_replan_required => {
                "an existing scoped obligation remains authoritative"
            }
            RULE_OPEN_USER_TODO if facts.blocking_user_open_count > 0 => {
                "open blocking user work owns the frontier"
            }
            RULE_TODO_SUCCESSION_GAP
                if facts.succession_gap_count > 0
                    && facts.selectable_frontier_advancement == 0
                    && facts.advancement_open_count == 0 =>
            {
                "completed advancement work lacks a successor or no-followup rationale"
            }
            RULE_VISION_ACCEPTANCE_GAP
                if facts.acceptance_gap_count > 0 && facts.selectable_frontier_advancement == 0 =>
            {
                "the scoped vision gap lacks a satisfying runnable frontier"
            }
            RULE_MONITOR_NO_CHANGE_STREAK
                if facts.monitor_only_lane
                    && facts.monitor_count > 0
                    && facts.monitor_no_change_streak_triggered =>
            {
                "the current agent monitor crossed the no-change replan threshold"
            }
            RULE_NOT_MONITOR_ONLY if !facts.monitor_only_lane => {
                "the selected lane is not monitor-only"
            }
            RULE_NO_OPEN_MONITOR if facts.monitor_count == 0 => "no open monitor remains",
            RULE_ADVANCEMENT_REMAINS if facts.advancement_open_count > 0 => {
                "advancement work remains on the frontier"
            }
            RULE_MONITOR_FRONTIER_EXHAUSTED => {
                "only monitor work remains on an empty advancement frontier"
            }
            _ => return None,
        })
    };
    for (index, rule) in &ordered {
        if let Some(reason) = matching(rule) {
            return decision_for(rule, *index, reason);
        }
    }
    // Custom set contained only unknown ids: fall back to the default order.
    let default_ids = default_rule_set().rule_ids;
    for (index, rule) in default_ids.iter().enumerate() {
        if let Some(reason) = matching(rule) {
            return decision_for(rule, index as u32, reason);
        }
    }
    unreachable!("monitor_frontier_exhausted is the unconditional terminal rule");
}

/// Select the replan rule for a goal under its ACTIVE rule set.
pub fn select_replan_rule(goal: &Goal) -> ReplanRuleDecision {
    let facts = facts_for_goal(goal);
    let ids = match &goal.replan_rule_set {
        Some(set) => set.effective_rule_ids(),
        None => default_rule_set().rule_ids,
    };
    select_replan_rule_with_set(&facts, &ids)
}

/// The active rule set for a goal (explicit set or the default).
pub fn active_rule_set(goal: &Goal) -> ReplanRuleSet {
    goal.replan_rule_set
        .clone()
        .unwrap_or_else(default_rule_set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Todo, TodoStatus};
    use std::time::Duration;

    #[test]
    fn default_set_orders_rules_in_policy_order() {
        let set = default_rule_set();
        assert_eq!(set.schema_version, DEFAULT_REPLAN_RULE_SET_VERSION);
        assert_eq!(
            set.rule_ids.first().map(String::as_str),
            Some(RULE_EXISTING_OBLIGATION)
        );
        assert_eq!(
            set.rule_ids.last().map(String::as_str),
            Some(RULE_MONITOR_FRONTIER_EXHAUSTED)
        );
    }

    #[test]
    fn succession_gap_selects_rule_and_derives_obligation() {
        let mut g = Goal::new("g", "o", "/tmp");
        let mut todo = Todo::advancement("T1", "work");
        todo.complete(false, vec![]); // no successor, no no-follow-up
        g.add(todo);
        let d = select_replan_rule(&g);
        assert_eq!(d.rule, RULE_TODO_SUCCESSION_GAP);
        assert!(d.derives_obligation);
        assert_eq!(d.obligation_kind.as_deref(), Some("succession_gap"));
    }

    #[test]
    fn acceptance_gap_without_frontier_selects_vision_rule() {
        let mut g = Goal::new("g", "o", "/tmp").with_acceptance(vec![("A1", "match")]);
        g.add(Todo::advancement("T1", "work"));
        g.todo_mut("T1").unwrap().status = TodoStatus::Done;
        g.todo_mut("T1").unwrap().no_follow_up = true;
        let d = select_replan_rule(&g);
        assert_eq!(d.rule, RULE_VISION_ACCEPTANCE_GAP);
        assert!(d.derives_obligation);
        assert_eq!(d.obligation_kind.as_deref(), Some("vision_acceptance_gap"));
    }

    #[test]
    fn runnable_frontier_selects_not_monitor_only_without_obligation() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T1", "work"));
        let d = select_replan_rule(&g);
        // reference policy order: NOT_MONITOR_ONLY precedes the
        // advancement-remains fallthrough when the lane has selectable work.
        assert_eq!(d.rule, RULE_NOT_MONITOR_ONLY);
        assert!(!d.derives_obligation);
        assert_eq!(d.obligation_kind, None);
    }

    #[test]
    fn monitor_streak_on_monitor_only_lane_derives_obligation() {
        let mut g = Goal::new("g", "o", "/tmp");
        let mut m = Todo::monitor("M1", "watch", Duration::from_secs(60));
        m.consecutive_no_change = MONITOR_NO_CHANGE_OBLIGATION_THRESHOLD;
        g.add(m);
        let d = select_replan_rule(&g);
        assert_eq!(d.rule, RULE_MONITOR_NO_CHANGE_STREAK);
        assert!(d.derives_obligation);
        assert_eq!(
            d.obligation_kind.as_deref(),
            Some("monitor_no_change_streak")
        );
    }

    #[test]
    fn surface_only_streak_obligation_is_authoritative_first() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T1", "work"));
        // Outcome floor breached → the surface-only-streak obligation is on
        // record → EXISTING_OBLIGATION fires even though work is runnable.
        g.execution_profile.outcome_floor_streak_threshold = 2;
        g.outcome_streak = 2;
        let d = select_replan_rule(&g);
        assert_eq!(d.rule, RULE_EXISTING_OBLIGATION);
        assert!(!d.derives_obligation);
        assert!(d.reason.contains("authoritative"));
    }

    #[test]
    fn open_user_todo_owns_the_frontier() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::user_gate("G1", "approve this?", &[]));
        let d = select_replan_rule(&g);
        assert_eq!(d.rule, RULE_OPEN_USER_TODO);
        assert!(!d.derives_obligation);
        assert!(d.reason.contains("blocking user work"));
    }

    #[test]
    fn advancement_remains_on_a_blocked_frontier() {
        let mut g = Goal::new("g", "o", "/tmp");
        // An open EXTERNAL blocker (not a user gate) gates the advancement, so
        // blocking_user_open_count stays 0; the open monitor keeps
        // monitor_count > 0. runnable == 0 → monitor_only_lane, and the
        // open-but-blocked advancement fires RULE_ADVANCEMENT_REMAINS.
        g.add(Todo::blocker("B1", "wait for authority", &["A1"]));
        g.add(Todo::advancement("A1", "work").blocking(&["B1"]));
        g.add(Todo::monitor("M1", "watch", Duration::from_secs(60)));
        let d = select_replan_rule(&g);
        assert_eq!(d.rule, RULE_ADVANCEMENT_REMAINS);
        assert!(!d.derives_obligation);
        assert!(d.reason.contains("advancement work remains"));
    }

    #[test]
    fn unknown_only_rule_set_falls_back_to_default_order() {
        // `effective_rule_ids` folds a set of only-unknown ids back to the
        // default order before `select_replan_rule` is reached; drive the
        // in-function fallback directly with a known-but-non-matching rule.
        let facts = ReplanFacts {
            existing_replan_required: false,
            blocking_user_open_count: 0,
            succession_gap_count: 0,
            acceptance_gap_count: 0,
            selectable_frontier_advancement: 0,
            advancement_open_count: 0,
            monitor_count: 0,
            monitor_no_change_streak_triggered: false,
            monitor_only_lane: true,
        };
        // RULE_OPEN_USER_TODO does not match (no blocking user work); the
        // default-order loop resolves to NO_OPEN_MONITOR (monitor_count == 0).
        let d = select_replan_rule_with_set(&facts, &[RULE_OPEN_USER_TODO.to_string()]);
        assert_eq!(d.rule, RULE_NO_OPEN_MONITOR);
    }

    #[test]
    fn custom_rule_set_respects_declared_order() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T1", "work"));
        // Custom set puts the exhausted rule first: it fires before
        // not_monitor_only (policy order is the caller's).
        g.replan_rule_set = Some(ReplanRuleSet {
            schema_version: DEFAULT_REPLAN_RULE_SET_VERSION.to_string(),
            rule_ids: vec![
                RULE_MONITOR_FRONTIER_EXHAUSTED.to_string(),
                RULE_ADVANCEMENT_REMAINS.to_string(),
            ],
        });
        let d = select_replan_rule(&g);
        assert_eq!(d.rule, RULE_MONITOR_FRONTIER_EXHAUSTED);
        // Unknown ids are skipped; default order still resolves.
        let mut g2 = Goal::new("g2", "o", "/tmp");
        g2.add(Todo::advancement("T1", "work"));
        g2.replan_rule_set = Some(ReplanRuleSet {
            schema_version: DEFAULT_REPLAN_RULE_SET_VERSION.to_string(),
            rule_ids: vec!["custom_unknown_rule".to_string()],
        });
        assert_eq!(select_replan_rule(&g2).rule, RULE_NOT_MONITOR_ONLY);
    }
}
