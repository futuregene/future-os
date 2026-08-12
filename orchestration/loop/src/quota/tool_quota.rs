//! Per-tool quota at the capability boundary — per-capability invocation
//! counting, ceilings, and trailing windows (LoopX 对比改进项 ②).
//!
//! The capability boundary (`capability propose` and the G-24 per-capability
//! command hooks) is where agents invoke the loop's tools. Every accepted
//! invocation is appended to the goal's event ledger (`CapabilityInvoked`
//! with outcome `accepted`); an invocation that would exceed the tool's
//! quota is refused and the refusal itself is ledgered (outcome
//! `rejected`), so the audit trail shows both usage and enforcement
//! (计数入账本).
//!
//! LoopX alignment: the quota should-run contract carries seven
//! allowed-predicates (`normal_delivery_allowed`,
//! `recovery_delivery_allowed`, `self_repair_allowed`,
//! `capability_repair_allowed`, `workspace_repair_allowed`,
//! `safe_bypass_allowed`, `actionable_by_codex`). This module feeds one of
//! them — [`capability_repair_allowed`]: the capability-repair lane stays
//! allowed only while no capability tool is over its quota.

use crate::state::Goal;

/// Default per-tool invocation ceiling within the trailing window. The
/// boundary tools are deterministic rule proposers (no LLM calls, no side
/// effects — capabilities propose, the kernel decides), so the ceiling
/// exists to stop runaway polling loops and ledger spam rather than to
/// ration cost: 30/hour is generous for legitimate use and tight enough to
/// catch a polling bug.
pub const DEFAULT_TOOL_QUOTA_LIMIT: u64 = 30;

/// Default trailing window for the per-tool quota, in seconds (one hour;
/// sliding by event timestamps, not aligned to wall-clock buckets).
pub const DEFAULT_TOOL_QUOTA_WINDOW_SECS: u64 = 3600;

/// Ledger outcome marker for accepted invocations (counted against quota).
pub const OUTCOME_ACCEPTED: &str = "accepted";
/// Ledger outcome marker for refused invocations (audit only, never counted).
pub const OUTCOME_REJECTED: &str = "rejected";

/// A per-tool quota policy: at most `limit` accepted invocations of `tool`
/// within any trailing window of `window_secs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolQuotaPolicy {
    /// The tool key — the capability id at the capability boundary (a
    /// capability's command verbs share its ceiling).
    pub tool: String,
    pub limit: u64,
    pub window_secs: u64,
}

impl ToolQuotaPolicy {
    /// The conservative default: every boundary tool gets the same ceiling.
    pub fn default_for(tool: &str) -> Self {
        Self {
            tool: tool.to_string(),
            limit: DEFAULT_TOOL_QUOTA_LIMIT,
            window_secs: DEFAULT_TOOL_QUOTA_WINDOW_SECS,
        }
    }
}

/// The evaluation of one tool against its policy at `now`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ToolQuotaDecision {
    pub tool: String,
    pub limit: u64,
    pub window_secs: u64,
    /// Accepted invocations counted inside the current trailing window.
    pub used: u64,
    /// `used < limit` — the next invocation may proceed.
    pub allowed: bool,
}

/// Count accepted invocations of `tool` at or after `window_start`.
/// `invocations` is the goal's (ts, tool) projection folded from accepted
/// `CapabilityInvoked` events.
pub fn count_in_window(invocations: &[(u64, String)], tool: &str, window_start: u64) -> u64 {
    invocations
        .iter()
        .filter(|(ts, t)| *ts >= window_start && t == tool)
        .count() as u64
}

/// Evaluate a policy against the invocation projection at `now` (epoch secs).
pub fn evaluate(
    policy: &ToolQuotaPolicy,
    invocations: &[(u64, String)],
    now: u64,
) -> ToolQuotaDecision {
    let window_start = now.saturating_sub(policy.window_secs);
    let used = count_in_window(invocations, &policy.tool, window_start);
    ToolQuotaDecision {
        tool: policy.tool.clone(),
        limit: policy.limit,
        window_secs: policy.window_secs,
        used,
        allowed: used < policy.limit,
    }
}

/// Evaluate the default policy for `tool`.
pub fn evaluate_default(tool: &str, invocations: &[(u64, String)], now: u64) -> ToolQuotaDecision {
    evaluate(&ToolQuotaPolicy::default_for(tool), invocations, now)
}

/// Per-tool usage rows for every tool present in the projection — the
/// `quota tools` read model. Sorted by tool name for stable output.
pub fn usage_rows(invocations: &[(u64, String)], now: u64) -> Vec<ToolQuotaDecision> {
    let mut tools: Vec<&str> = invocations.iter().map(|(_, t)| t.as_str()).collect();
    tools.sort_unstable();
    tools.dedup();
    tools
        .into_iter()
        .map(|t| evaluate_default(t, invocations, now))
        .collect()
}

/// The tools currently at or over their default quota (empty = all clear).
pub fn exceeded_tools(invocations: &[(u64, String)], now: u64) -> Vec<String> {
    usage_rows(invocations, now)
        .into_iter()
        .filter(|d| !d.allowed)
        .map(|d| d.tool)
        .collect()
}

/// The packet's `capability_repair_allowed` predicate (one of the seven
/// LoopX allowed-predicates): the capability-repair lane is allowed only
/// while no capability tool is over its quota.
pub fn capability_repair_allowed(goal: &Goal, now: u64) -> bool {
    exceeded_tools(&goal.capability_invocations, now).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invocations(rows: &[(u64, &str)]) -> Vec<(u64, String)> {
        rows.iter().map(|(ts, t)| (*ts, t.to_string())).collect()
    }

    #[test]
    fn default_policy_values() {
        let p = ToolQuotaPolicy::default_for("issue_fix");
        assert_eq!(p.tool, "issue_fix");
        assert_eq!(p.limit, DEFAULT_TOOL_QUOTA_LIMIT);
        assert_eq!(p.window_secs, DEFAULT_TOOL_QUOTA_WINDOW_SECS);
    }

    #[test]
    fn count_filters_by_tool_and_window() {
        let inv = invocations(&[
            (100, "issue_fix"),
            (200, "issue_fix"),
            (300, "explore"),
            (50, "issue_fix"), // before window_start
        ]);
        assert_eq!(count_in_window(&inv, "issue_fix", 100), 2);
        assert_eq!(count_in_window(&inv, "explore", 100), 1);
        assert_eq!(count_in_window(&inv, "issue_fix", 0), 3);
        assert_eq!(count_in_window(&inv, "unknown", 0), 0);
        assert_eq!(count_in_window(&[], "issue_fix", 0), 0);
    }

    #[test]
    fn evaluate_allows_under_limit_and_refuses_at_limit() {
        let policy = ToolQuotaPolicy {
            tool: "issue_fix".to_string(),
            limit: 2,
            window_secs: 100,
        };
        let inv = invocations(&[(950, "issue_fix")]);
        let d = evaluate(&policy, &inv, 1000);
        assert_eq!(d.used, 1);
        assert!(d.allowed);
        let inv = invocations(&[(950, "issue_fix"), (960, "issue_fix")]);
        let d = evaluate(&policy, &inv, 1000);
        assert_eq!(d.used, 2);
        assert!(!d.allowed, "at the limit the next call is refused");
    }

    #[test]
    fn evaluate_window_expiry_frees_quota() {
        let policy = ToolQuotaPolicy {
            tool: "issue_fix".to_string(),
            limit: 1,
            window_secs: 100,
        };
        let inv = invocations(&[(800, "issue_fix")]);
        // 800 is outside the trailing window (900..=1000) → quota free.
        let d = evaluate(&policy, &inv, 1000);
        assert_eq!(d.used, 0);
        assert!(d.allowed);
    }

    #[test]
    fn usage_rows_are_sorted_and_deduplicated() {
        let inv = invocations(&[(10, "explore"), (20, "issue_fix"), (30, "explore")]);
        let rows = usage_rows(&inv, 1000);
        let names: Vec<&str> = rows.iter().map(|d| d.tool.as_str()).collect();
        assert_eq!(names, vec!["explore", "issue_fix"]);
        assert_eq!(rows[0].used, 2);
        assert_eq!(rows[1].used, 1);
        assert!(rows.iter().all(|d| d.allowed));
    }

    #[test]
    fn exceeded_tools_lists_only_saturated_tools() {
        let mut inv = invocations(&[(10, "explore")]);
        for i in 0..DEFAULT_TOOL_QUOTA_LIMIT {
            inv.push((500 + i, "issue_fix".to_string()));
        }
        let exceeded = exceeded_tools(&inv, 1000);
        assert_eq!(exceeded, vec!["issue_fix".to_string()]);
        assert!(exceeded_tools(&[], 1000).is_empty());
    }

    #[test]
    fn capability_repair_allowed_tracks_saturation() {
        let mut goal = Goal::new("g1", "obj", "/tmp");
        assert!(capability_repair_allowed(&goal, 1000));
        for i in 0..DEFAULT_TOOL_QUOTA_LIMIT {
            goal.capability_invocations
                .push((500 + i, "issue_fix".to_string()));
        }
        assert!(
            !capability_repair_allowed(&goal, 1000),
            "a saturated tool closes the capability-repair lane"
        );
        // After the window slides past the invocations the lane reopens.
        assert!(capability_repair_allowed(
            &goal,
            500 + DEFAULT_TOOL_QUOTA_LIMIT + DEFAULT_TOOL_QUOTA_WINDOW_SECS
        ));
    }
}
