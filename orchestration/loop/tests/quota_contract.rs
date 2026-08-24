//! P1 contract tests — G-7 quota subdomain: spend-source classification
//! (run/agent/heartbeat), per-goal usage summaries (24h/7d), and the stall
//! repair delivery guard. These replace the P0 "constant + history.len()"
//! accounting with observable quota.

use std::time::Duration;

use future_loop::quota::slot_accounting::{
    account_history, classify_mode, slot_spend, SlotSpendSource, QUOTA_ALLOWED_SLOTS,
};
use future_loop::quota::stall_repair::detect_stall;
use future_loop::quota::usage_summary::{build_usage_summary, build_usage_summary_for_goals};
use future_loop::state::{Goal, RunRecord, Todo};

fn record(recorded_at: u64, source: &str, tools: bool, evidence: bool) -> RunRecord {
    RunRecord {
        turn: 1,
        todo_id: "T1".into(),
        run_id: "run-1".into(),
        terminal_state: if source == "heartbeat" {
            "polled"
        } else {
            "completed"
        }
        .into(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: if tools { vec!["shell".into()] } else { vec![] },
        evidence: if evidence {
            "artifact validated".into()
        } else {
            String::new()
        },
        recorded_at,
        spend_source: Some(source.into()),
        validation: None,
        failure_kind: None,
    }
}

// ── Spend-source classification (plan taxonomy: run/agent/heartbeat) ───────
#[test]
fn modes_classify_into_three_sources() {
    assert_eq!(
        classify_mode(future_loop::contract::TurnMode::BoundedDelivery),
        SlotSpendSource::Run
    );
    assert_eq!(
        classify_mode(future_loop::contract::TurnMode::MonitorPoll),
        SlotSpendSource::Heartbeat
    );
    assert_eq!(
        classify_mode(future_loop::contract::TurnMode::WaitMonitor),
        SlotSpendSource::Heartbeat
    );
    assert_eq!(
        classify_mode(future_loop::contract::TurnMode::Replan),
        SlotSpendSource::Heartbeat
    );
    assert_eq!(
        classify_mode(future_loop::contract::TurnMode::Terminal),
        SlotSpendSource::Heartbeat
    );
}

#[test]
fn heartbeat_polls_never_spend_but_runs_do() {
    assert_eq!(slot_spend(&record(0, "heartbeat", false, false)), 0);
    assert_eq!(slot_spend(&record(0, "run", true, true)), 1);
    assert_eq!(slot_spend(&record(0, "agent", true, true)), 1);
}

// ── Breakdown replaces the raw history.len() count ─────────────────────────
#[test]
fn account_history_splits_spend_by_source() {
    let history = vec![
        record(1_784_000_000, "run", true, true),
        record(1_784_000_100, "run", true, true),
        record(1_784_000_200, "agent", true, true),
        record(1_784_000_300, "heartbeat", false, false),
        record(1_784_000_400, "heartbeat", false, false),
    ];
    let b = account_history(&history);
    assert_eq!(b.allowed_slots, QUOTA_ALLOWED_SLOTS);
    assert_eq!(b.spent_slots, 3, "only run+agent count");
    assert_eq!(b.source_count(SlotSpendSource::Run), 2);
    assert_eq!(b.source_count(SlotSpendSource::Agent), 1);
    assert_eq!(b.source_count(SlotSpendSource::Heartbeat), 0);
}

// ── Legacy ledger lines without a source default to completed-run ──────────
#[test]
fn legacy_records_default_to_completed_run_classification() {
    let legacy = RunRecord {
        spend_source: None,
        validation: None,
        terminal_state: "completed".into(),
        failure_kind: None,
        ..record(0, "run", false, false)
    };
    assert_eq!(slot_spend(&legacy), 1, "legacy completed runs still spend");
    let legacy_error = RunRecord {
        spend_source: None,
        validation: None,
        terminal_state: "error".into(),
        failure_kind: None,
        ..record(0, "run", false, false)
    };
    assert_eq!(
        slot_spend(&legacy_error),
        0,
        "legacy errored runs are quota-neutral"
    );
}

// ── Usage summary buckets 24h/7d and aggregates across goals ───────────────
#[test]
fn usage_summary_buckets_24h_and_7d() {
    let now = 1_784_000_000u64;
    let history = vec![
        record(now, "run", true, true),               // 24h + 7d, spend 1
        record(now - 12 * 3600, "run", false, false), // 24h + 7d, spend 1, no progress signal
        record(now - 3 * 86400, "heartbeat", false, false), // 7d only, no spend
        record(now - 10 * 86400, "run", true, true),  // outside both windows
    ];
    let s = build_usage_summary("g1", &history, now);
    assert_eq!(s.sample_run_count, 4);
    assert_eq!(s.totals.runs_24h, 2);
    assert_eq!(s.totals.runs_7d, 3);
    assert_eq!(s.totals.quota_spend_slots_24h, 2);
    assert_eq!(s.totals.quota_spend_slots_7d, 2);
    assert_eq!(s.totals.automation_run_count_24h, 2);
    assert_eq!(s.totals.progress_signal_run_count_24h, 1);
    assert_eq!(s.goals[0].goal_id, "g1");
}

#[test]
fn usage_summary_aggregates_across_goals() {
    let now = 1_784_000_000u64;
    let g1 = vec![
        record(now, "run", true, true),
        record(now, "heartbeat", false, false),
    ];
    let g2 = vec![record(now, "agent", true, true)];
    let s = build_usage_summary_for_goals(&[("g1", &g1), ("g2", &g2)], now);
    assert_eq!(s.totals.runs_24h, 3);
    assert_eq!(s.totals.quota_spend_slots_24h, 2);
    assert_eq!(s.goals.len(), 2);
    // g1 has 2 runs in 24h (run + heartbeat), g2 has 1 → descending sort.
    assert_eq!(s.goals[0].goal_id, "g1", "sorted by runs_24h desc");
    assert!((s.goals[0].project_share_24h - 2.0 / 3.0).abs() < 0.001);
}

// ── Stall repair: the delivery guard ───────────────────────────────────────
#[test]
fn stall_repair_detects_monitor_stall() {
    let mut goal = Goal::new("g", "o", "/tmp");
    goal.add(Todo::monitor("M1", "watch", Duration::from_secs(60)));
    goal.todo_mut("M1").unwrap().consecutive_no_change = 3; // threshold
    let hint = detect_stall(&goal).expect("stall");
    assert_eq!(hint.kind, "monitor_stalled");
    assert_eq!(hint.blocked_action_scope.as_deref(), Some("monitor_poll"));
    assert!(hint.replan_hint.contains("replan"));
}

#[test]
fn stall_repair_detects_outcome_floor_and_repair_exhaustion() {
    let mut goal = Goal::new("g", "o", "/tmp");
    goal.add(Todo::advancement("T1", "work"));
    goal.execution_profile.outcome_floor_streak_threshold = 2;
    goal.outcome_streak = 2;
    assert_eq!(detect_stall(&goal).unwrap().kind, "outcome_floor");
    goal.execution_profile.outcome_floor_streak_threshold = 0;
    goal.todo_mut("T1").unwrap().failed_attempts = 2; // > MAX_REPAIR_ATTEMPTS=1
    assert_eq!(detect_stall(&goal).unwrap().kind, "repair_budget_exhausted");
}

#[test]
fn healthy_goal_has_no_stall() {
    let mut goal = Goal::new("g", "o", "/tmp");
    goal.add(Todo::advancement("T1", "work"));
    assert_eq!(detect_stall(&goal), None);
}

// ── Packet parity: quota packet fields unchanged by the subdomain split ────
#[test]
fn quota_packet_still_reports_allowed_and_spent() {
    let mut goal = Goal::new("g", "o", "/tmp");
    goal.add(Todo::advancement("T1", "work"));
    let p = future_loop::decision::decide(&goal, std::time::SystemTime::now());
    assert_eq!(p.quota.allowed_slots, QUOTA_ALLOWED_SLOTS);
    assert_eq!(p.quota.spent_slots, goal.history.len() as u64);
}
