//! CLI projection (G-9) — human-readable text rendering of the typed packet,
//! quota usage, and scheduler state, replacing the P0 two-line JSON-ish
//! `quota should-run` output (LoopX `control_plane/quota/cli_projection.py`,
//! 684 lines — we implement the deterministic text projection hosts read).
//!
//! These renderers never write state: they are read-only projections over
//! the packet / ledger / scheduler state.

use crate::contract::ShouldRunPacket;
use crate::quota::slot_accounting::{SlotSpendSource, SpendBreakdown};
use crate::quota::stall_repair::StallRepairHint;
use crate::quota::usage_summary::UsageSummary;
use crate::scheduler::state::{cadence_label, SchedulerState};
use crate::state::Goal;

/// One-line decision banner (kept stable for script consumers that parse the
/// old `quota should-run` two-liner).
pub fn render_decision_line(packet: &ShouldRunPacket) -> String {
    format!(
        "decision: {} | should_run: {} | mode: {}",
        packet.decision,
        packet.should_run,
        packet.interaction_contract.mode.as_str()
    )
}

/// Full text projection for a `quota should-run` packet: decision banner +
/// selected todo + quota (breakdown by spend source) + scheduler hint +
/// stall hint (when the mode is replan) + arbitration record.
pub fn render_quota_projection(
    packet: &ShouldRunPacket,
    breakdown: Option<&SpendBreakdown>,
    stall: Option<&StallRepairHint>,
) -> String {
    let mut out = String::new();
    out.push_str(&render_decision_line(packet));
    out.push('\n');
    out.push_str(&format!("reason: {}\n", packet.reason));
    if let Some(todo) = packet
        .interaction_contract
        .agent_channel
        .selected_todo
        .as_deref()
    {
        out.push_str(&format!("selected todo: {todo}\n"));
    }
    if let Some(arb) = &packet.scheduler_arbitration {
        out.push_str(&format!("arbitration: {}\n", arb.disposition.as_str()));
    }
    if let Some(b) = breakdown {
        out.push_str(&format!(
            "quota: allowed={} spent={} (run={} agent={} heartbeat={})\n",
            b.allowed_slots,
            b.spent_slots,
            b.source_count(SlotSpendSource::Run),
            b.source_count(SlotSpendSource::Agent),
            b.source_count(SlotSpendSource::Heartbeat),
        ));
    }
    out.push_str(&format!(
        "scheduler: action={} cadence_class={}{}\n",
        packet.scheduler_hint.action,
        packet.scheduler_hint.cadence_class,
        packet
            .scheduler_hint
            .next_due_ms
            .map(|ms| format!(" next_due_ms={ms}"))
            .unwrap_or_default()
    ));
    if let Some(s) = stall {
        out.push_str(&format!(
            "stall: {} — {}\n  replan hint: {}\n",
            s.kind, s.reason, s.replan_hint
        ));
        if let Some(scope) = &s.blocked_action_scope {
            out.push_str(&format!("  blocked action scope: {scope}\n"));
        }
    }
    if let Some(tc) = &packet.terminal_closure {
        out.push_str(&format!(
            "terminal closure: kind={} derived={} source={}\n",
            tc.kind, tc.derived, tc.source
        ));
    }
    out
}

/// Render a usage summary (LoopX `cli_projection` quota usage view).
pub fn render_usage_summary(summary: &UsageSummary) -> String {
    let t = &summary.totals;
    let mut out = String::new();
    out.push_str(&format!(
        "usage summary (source={} generated_at={} samples={})\n",
        summary.source, summary.generated_at, summary.sample_run_count
    ));
    out.push_str(&format!(
        "  24h : runs={} spend_slots={} automation={} progress_signal={}\n",
        t.runs_24h,
        t.quota_spend_slots_24h,
        t.automation_run_count_24h,
        t.progress_signal_run_count_24h
    ));
    out.push_str(&format!(
        "  7d  : runs={} spend_slots={} automation={} progress_signal={}\n",
        t.runs_7d,
        t.quota_spend_slots_7d,
        t.automation_run_count_7d,
        t.progress_signal_run_count_7d
    ));
    for g in &summary.goals {
        out.push_str(&format!(
            "  goal {} : runs_24h={} spend_24h={} share_24h={:.3}\n",
            g.goal_id, g.runs_24h, g.quota_spend_slots_24h, g.project_share_24h
        ));
    }
    out.push_str(&format!("  note: {}\n", summary.proxy_note));
    out
}

/// Render the persisted scheduler state (LoopX scheduler-state projection).
pub fn render_scheduler_state(state: &SchedulerState) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "scheduler state ({} | goal={} agent={} surface={})\n",
        state.schema_version, state.goal_id, state.agent_id, state.surface
    ));
    out.push_str(&format!("  state_key      : {}\n", state.state_key));
    out.push_str(&format!("  reset_token    : {}\n", state.reset_token));
    out.push_str(&format!(
        "  identity_sig   : {}\n",
        state.identity_signature
    ));
    out.push_str(&format!(
        "  progression    : index={} minutes={:?}\n",
        state.progression_index, state.progression_minutes
    ));
    let current = state
        .progression_minutes
        .get(state.progression_index)
        .copied();
    out.push_str(&format!(
        "  current        : rrule={} interval={}\n",
        state.last_applied_rrule,
        current
            .map(|m| format!("{}m ({})", m, cadence_label(m)))
            .unwrap_or_else(|| "n/a".to_string())
    ));
    if !state.host_update_failures.is_empty() {
        out.push_str(&format!(
            "  host failures  : {} retained\n",
            state.host_update_failures.len()
        ));
        for f in state.host_update_failures.iter().take(4) {
            out.push_str(&format!(
                "    kind={} target={} observed={} count={} at={}\n",
                f.failure_kind, f.target_rrule, f.observed_host_rrule, f.failure_count, f.failed_at
            ));
        }
    } else {
        out.push_str("  host failures  : none\n");
    }
    out.push_str(&format!("  updated_at     : {}\n", state.updated_at));
    out
}

/// The recommended rrule + next progression for the current cadence class,
/// as the scheduler hint projection would expose it (used by `scheduler tick`).
pub fn render_cadence_plan(cadence_class: &str, progression: &[i64], index: usize) -> String {
    let mut out = String::new();
    out.push_str(&format!("cadence class : {cadence_class}\n"));
    let intervals: Vec<i64> = if progression.is_empty() {
        match cadence_class {
            "hourly" => vec![60],
            "daily" => vec![1440],
            "weekly" => vec![10080],
            _ => vec![],
        }
    } else {
        progression.to_vec()
    };
    if intervals.is_empty() {
        out.push_str("  rrule        : none (single execution — `once`)\n");
        return out;
    }
    let i = index.min(intervals.len() - 1);
    let minutes = intervals[i];
    out.push_str(&format!(
        "  rrule        : {}\n",
        crate::scheduler::state::rrule_for_minutes(minutes)
    ));
    out.push_str(&format!(
        "  interval     : {}m ({})\n",
        minutes,
        cadence_label(minutes)
    ));
    if intervals.len() > 1 {
        out.push_str(&format!(
            "  progression  : {} (index {} of {}){}\n",
            intervals
                .iter()
                .map(|m| cadence_label(*m))
                .collect::<Vec<_>>()
                .join(" → "),
            index,
            intervals.len(),
            if i + 1 < intervals.len() {
                format!(" → next {}", cadence_label(intervals[i + 1]))
            } else {
                " → wraps to start".to_string()
            }
        ));
    }
    out
}

/// Compute the initial rrule for a cadence class at a given interval
/// (shared by `scheduler tick` bootstrap and tests).
pub fn initial_rrule_for(cadence_class: &str) -> Option<String> {
    let interval = match cadence_class {
        "hourly" => 60,
        "daily" => 1440,
        "weekly" => 10080,
        _ => return None,
    };
    Some(crate::scheduler::state::rrule_for_minutes(interval))
}

/// Whether a goal has any open monitor metadata worth surfacing in status
/// (G-12 projection).
pub fn monitor_metadata_lines(goal: &Goal) -> Vec<String> {
    goal.open_monitors()
        .map(|m| {
            let target = m
                .monitor_target
                .as_deref()
                .map(|t| format!(" target={t}"))
                .unwrap_or_default();
            let policy = m
                .monitor_policy
                .as_deref()
                .map(|p| format!(" policy={p}"))
                .unwrap_or_default();
            let cadence = m
                .monitor_cadence
                .as_deref()
                .map(|c| format!(" cadence={c}"))
                .unwrap_or_default();
            format!(
                "  monitor {}: no_change={}{}{}{}",
                m.id, m.consecutive_no_change, target, policy, cadence
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::{decide, QUOTA_ALLOWED_SLOTS};
    use crate::quota::slot_accounting::account_history;
    use crate::quota::stall_repair::detect_stall;
    use crate::state::Todo;
    use std::time::Duration;

    #[test]
    fn quota_projection_shows_quota_scheduler_and_arbitration() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        let packet = decide(&g, std::time::SystemTime::now());
        let breakdown = account_history(&g.history);
        let proj = render_quota_projection(&packet, Some(&breakdown), None);
        assert!(proj.contains("decision: run"), "decision banner: {proj}");
        assert!(proj.contains("selected todo: T1"));
        assert!(proj.contains(&format!("allowed={QUOTA_ALLOWED_SLOTS}")));
        assert!(proj.contains("scheduler: action=tick_next"));
        assert!(proj.contains("arbitration: active_work"));
    }

    #[test]
    fn quota_projection_shows_stall_hint_on_replan() {
        let mut g = Goal::new("g", "o", "/tmp");
        let mut t = Todo::advancement("T1", "done without intent");
        t.status = crate::state::TodoStatus::Done;
        g.add(t);
        let packet = decide(&g, std::time::SystemTime::now());
        assert_eq!(
            packet.interaction_contract.mode,
            crate::contract::TurnMode::Replan
        );
        let stall = detect_stall(&g);
        let proj = render_quota_projection(&packet, None, stall.as_ref());
        assert!(proj.contains("stall: succession_obligation"));
        assert!(proj.contains("replan hint:"));
    }

    #[test]
    fn usage_summary_renders_totals() {
        let summary = crate::quota::usage_summary::build_usage_summary("g", &[], 1_700_000_000);
        let text = render_usage_summary(&summary);
        assert!(text.contains("usage summary"));
        assert!(text.contains("runs=0"));
        assert!(text.contains("run-history proxy"));
    }

    #[test]
    fn scheduler_state_renders_progression() {
        let identity = crate::scheduler::state::identity_signature("g", "a", "codex_app");
        let state = crate::scheduler::state::build_scheduler_state(
            "g",
            "a",
            "codex_app",
            crate::scheduler::state::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
            &crate::scheduler::state::reset_token("tick", &identity, "FREQ=MINUTELY;INTERVAL=15"),
            &identity,
            0,
            crate::scheduler::state::MONITOR_WAIT_PROGRESSION_MINUTES.to_vec(),
            "FREQ=MINUTELY;INTERVAL=15",
            1_700_000_000,
            vec![],
        )
        .unwrap();
        let text = render_scheduler_state(&state);
        assert!(text.contains("progression"));
        assert!(text.contains("FREQ=MINUTELY;INTERVAL=15"));
        assert!(text.contains("host failures  : none"));
    }

    #[test]
    fn cadence_plan_shows_progression_sequence() {
        let plan = render_cadence_plan("monitor_backoff", &[15, 30, 60], 1);
        assert!(plan.contains("rrule"));
        assert!(plan.contains("15m → 30m → 1h"));
        assert!(plan.contains("next 1h"));
        let once = render_cadence_plan("once", &[], 0);
        assert!(once.contains("none (single execution"));
    }

    #[test]
    fn monitor_metadata_surfaces_in_status_projection() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::monitor_with(
            "M1",
            "Watch A",
            Some("https://example.com/status"),
            Some("material_transition_only"),
            Some("1h"),
            Duration::from_secs(3600),
        ));
        let lines = monitor_metadata_lines(&g);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("target=https://example.com/status"));
        assert!(lines[0].contains("policy=material_transition_only"));
        assert!(lines[0].contains("cadence=1h"));
    }

    #[test]
    fn initial_rrule_mapping() {
        assert_eq!(
            initial_rrule_for("hourly").as_deref(),
            Some("FREQ=MINUTELY;INTERVAL=60")
        );
        assert_eq!(initial_rrule_for("once"), None);
    }
}
