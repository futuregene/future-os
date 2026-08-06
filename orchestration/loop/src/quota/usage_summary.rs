//! Usage summary (G-7) — 24h/7d quota usage totals + per-goal rows, mirroring
//! reference `control_plane/quota/usage_summary.py` (`build_usage_summary`).
//!
//! The summary is a read-only projection over the run-history ledger (the
//! `spend_source` stamped by [`crate::quota::slot_accounting`]). It is what
//! makes quota *observable*: `loopx quota usage` renders it, and the P1
//! acceptance gate requires the quota command output to include it.

use crate::quota::slot_accounting::{account_history, slot_spend, SlotSpendSource};
use crate::state::RunRecord;

/// Proxy note (reference `USAGE_PROXY_NOTE`): the summary derives from the run
/// ledger and excludes token counts and raw thread logs.
pub const USAGE_PROXY_NOTE: &str = "run-history proxy; excludes token counts and raw thread logs";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Default)]
pub struct UsageTotals {
    pub runs_24h: u64,
    pub runs_7d: u64,
    pub quota_spend_slots_24h: u64,
    pub quota_spend_slots_7d: u64,
    pub automation_run_count_24h: u64,
    pub automation_run_count_7d: u64,
    pub progress_signal_run_count_24h: u64,
    pub progress_signal_run_count_7d: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UsageGoalRow {
    pub goal_id: String,
    pub runs_24h: u64,
    pub runs_7d: u64,
    pub quota_spend_slots_24h: u64,
    pub quota_spend_slots_7d: u64,
    pub automation_run_count_24h: u64,
    pub automation_run_count_7d: u64,
    pub progress_signal_run_count_24h: u64,
    pub progress_signal_run_count_7d: u64,
    pub project_share_24h: f64,
}

impl UsageGoalRow {
    fn blank(goal_id: &str) -> Self {
        Self {
            goal_id: goal_id.to_string(),
            runs_24h: 0,
            runs_7d: 0,
            quota_spend_slots_24h: 0,
            quota_spend_slots_7d: 0,
            automation_run_count_24h: 0,
            automation_run_count_7d: 0,
            progress_signal_run_count_24h: 0,
            progress_signal_run_count_7d: 0,
            project_share_24h: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UsageSummary {
    pub available: bool,
    pub source: String,
    pub generated_at: String,
    pub sample_run_count: usize,
    pub proxy_note: String,
    pub totals: UsageTotals,
    pub goals: Vec<UsageGoalRow>,
}

/// Build a usage summary over one goal's run history (LoopX
/// `build_usage_summary`). `now_epoch` is injectable for deterministic tests.
pub fn build_usage_summary(goal_id: &str, records: &[RunRecord], now_epoch: u64) -> UsageSummary {
    let cutoff_24h = now_epoch.saturating_sub(24 * 60 * 60);
    let cutoff_7d = now_epoch.saturating_sub(7 * 24 * 60 * 60);
    let mut totals = UsageTotals::default();
    let mut goal = UsageGoalRow::blank(goal_id);
    for record in records {
        let slots = slot_spend(record);
        let automation = matches!(
            crate::quota::slot_accounting::classify_record(record),
            SlotSpendSource::Run | SlotSpendSource::Agent
        );
        let progress_signal = !record.tools.is_empty() && !record.evidence.trim().is_empty();
        if record.recorded_at >= cutoff_7d {
            totals.runs_7d += 1;
            goal.runs_7d += 1;
            totals.quota_spend_slots_7d += slots;
            goal.quota_spend_slots_7d += slots;
            if automation {
                totals.automation_run_count_7d += 1;
                goal.automation_run_count_7d += 1;
            }
            if progress_signal {
                totals.progress_signal_run_count_7d += 1;
                goal.progress_signal_run_count_7d += 1;
            }
        }
        if record.recorded_at >= cutoff_24h {
            totals.runs_24h += 1;
            goal.runs_24h += 1;
            totals.quota_spend_slots_24h += slots;
            goal.quota_spend_slots_24h += slots;
            if automation {
                totals.automation_run_count_24h += 1;
                goal.automation_run_count_24h += 1;
            }
            if progress_signal {
                totals.progress_signal_run_count_24h += 1;
                goal.progress_signal_run_count_24h += 1;
            }
        }
    }
    if totals.runs_24h > 0 {
        goal.project_share_24h = round3(goal.runs_24h as f64 / totals.runs_24h as f64);
    }
    UsageSummary {
        available: true,
        source: "run_history".to_string(),
        generated_at: crate::compat::rfc3339(now_epoch),
        sample_run_count: records.len(),
        proxy_note: USAGE_PROXY_NOTE.to_string(),
        totals,
        goals: vec![goal],
    }
}

/// Aggregate usage across several goals (the CLI `quota usage` view).
pub fn build_usage_summary_for_goals(
    records_by_goal: &[(&str, &[RunRecord])],
    now_epoch: u64,
) -> UsageSummary {
    let mut rows: Vec<UsageGoalRow> = vec![];
    let mut totals = UsageTotals::default();
    for (goal_id, records) in records_by_goal {
        let single = build_usage_summary(goal_id, records, now_epoch);
        totals.runs_24h += single.totals.runs_24h;
        totals.runs_7d += single.totals.runs_7d;
        totals.quota_spend_slots_24h += single.totals.quota_spend_slots_24h;
        totals.quota_spend_slots_7d += single.totals.quota_spend_slots_7d;
        totals.automation_run_count_24h += single.totals.automation_run_count_24h;
        totals.automation_run_count_7d += single.totals.automation_run_count_7d;
        totals.progress_signal_run_count_24h += single.totals.progress_signal_run_count_24h;
        totals.progress_signal_run_count_7d += single.totals.progress_signal_run_count_7d;
        rows.push(
            single
                .goals
                .into_iter()
                .next()
                .unwrap_or_else(|| UsageGoalRow::blank(goal_id)),
        );
    }
    for row in &mut rows {
        if totals.runs_24h > 0 {
            row.project_share_24h = round3(row.runs_24h as f64 / totals.runs_24h as f64);
        }
    }
    rows.sort_by_key(|g| std::cmp::Reverse(g.runs_24h));
    UsageSummary {
        available: true,
        source: "run_history".to_string(),
        generated_at: crate::compat::rfc3339(now_epoch),
        sample_run_count: records_by_goal.iter().map(|(_, r)| r.len()).sum(),
        proxy_note: USAGE_PROXY_NOTE.to_string(),
        totals,
        goals: rows,
    }
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Convenience: the spend breakdown shown next to quota (allowed vs spent by
/// source) reusing the accounting rules.
pub fn breakdown(records: &[RunRecord]) -> crate::quota::slot_accounting::SpendBreakdown {
    account_history(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(recorded_at: u64, tools: bool, evidence: bool, source: &str) -> RunRecord {
        RunRecord {
            turn: 1,
            todo_id: "T1".to_string(),
            run_id: "run-1".to_string(),
            terminal_state: "completed".to_string(),
            error: None,
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: if tools {
                vec!["shell".to_string()]
            } else {
                vec![]
            },
            evidence: if evidence {
                "did the thing".to_string()
            } else {
                String::new()
            },
            recorded_at,
            spend_source: Some(source.to_string()),
            validation: None,
        }
    }

    #[test]
    fn buckets_by_24h_7d_windows() {
        let now = 1_700_000_000u64;
        let records = vec![
            record(now, true, true, "run"),                         // 24h + 7d
            record(now - 12 * 3600, true, false, "run"),            // 24h + 7d, no progress signal
            record(now - 3 * 24 * 3600, false, false, "heartbeat"), // 7d only, no spend
            record(now - 10 * 24 * 3600, true, true, "run"),        // outside both
        ];
        let s = build_usage_summary("g", &records, now);
        assert_eq!(s.sample_run_count, 4);
        assert_eq!(s.totals.runs_24h, 2);
        assert_eq!(s.totals.runs_7d, 3);
        assert_eq!(s.totals.quota_spend_slots_24h, 2);
        assert_eq!(
            s.totals.quota_spend_slots_7d, 2,
            "heartbeat run in 7d window never spends"
        );
        assert_eq!(s.totals.automation_run_count_24h, 2);
        assert_eq!(s.totals.progress_signal_run_count_24h, 1);
        assert_eq!(s.goals[0].goal_id, "g");
        assert_eq!(s.goals[0].project_share_24h, 1.0);
    }

    #[test]
    fn aggregate_sums_across_goals() {
        let now = 1_700_000_000u64;
        let g1 = vec![record(now, true, true, "run")];
        let g2 = vec![
            record(now, true, true, "agent"),
            record(now, false, false, "heartbeat"),
        ];
        let s = build_usage_summary_for_goals(&[("g1", &g1), ("g2", &g2)], now);
        assert_eq!(s.totals.runs_24h, 3);
        assert_eq!(s.totals.quota_spend_slots_24h, 2);
        assert_eq!(s.goals.len(), 2);
        assert_eq!(s.goals[0].goal_id, "g2");
        assert_eq!(s.goals[0].project_share_24h, round3(2.0 / 3.0));
    }

    #[test]
    fn breakdown_visible_from_usage_module() {
        let now = 1_700_000_000u64;
        let records = vec![
            record(now, true, true, "run"),
            record(now, false, false, "heartbeat"),
        ];
        let b = breakdown(&records);
        assert_eq!(b.spent_slots, 1);
        assert_eq!(b.source_count(SlotSpendSource::Run), 1);
    }
}
