//! Delivery contract (G-17) — LoopX
//! `control_plane/handoff/delivery_contract.py`, natively (minimal set). When
//! the post-handoff runs degrade (repeated small-scale delivery or an
//! outcome-gap streak past the profile threshold), the handoff carries a
//! delivery contract: the minimum batch scale, the must-include artifacts,
//! and the spend rule the successor must honor. No degradation → no contract.

use crate::state::{Goal, RunRecord};
use crate::work_items::delivery::{
    delivery_outcome_for_run, outcome_gap_streak, small_delivery_batch_scale_streak,
    DeliveryOutcome,
};

/// Default small-scale streak threshold when the profile does not set one
/// (LoopX execution_profile default threshold).
pub const DEFAULT_SMALL_STREAK_THRESHOLD: u32 = 2;

/// The delivery contract for a handoff (None when no degradation is present).
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeliveryContract {
    pub mode: String,
    pub minimum_scale: String,
    pub must_include: Vec<String>,
    pub spend_rule: String,
    pub small_scale_streak_threshold: u32,
    pub outcome_gap_streak_threshold: u32,
    pub if_blocked: String,
    pub post_handoff_small_scale_streak: u32,
    pub post_handoff_outcome_gap_streak: u32,
    pub summary: String,
    pub instruction: String,
}

/// Build the delivery contract from the goal's execution profile and its
/// newest-first run history (LoopX handoff_delivery_contract).
pub fn handoff_delivery_contract(goal: &Goal, runs: &[RunRecord]) -> Option<DeliveryContract> {
    let profile = &goal.execution_profile;
    let threshold = if profile.outcome_floor_streak_threshold > 0 {
        profile.outcome_floor_streak_threshold
    } else {
        DEFAULT_SMALL_STREAK_THRESHOLD
    };
    let small_streak = small_delivery_batch_scale_streak(runs);
    // Outcome floor markers: derive a minimal marker set from the spend rule
    // (an artifact-backed spend rule implies "artifact + validation + writeback"
    // must be present). Declarative outcome markers are a P4 profile concern;
    // the surface-only signal uses the delivery classification itself.
    let markers: Vec<String> = vec![];
    let surfaces: Vec<String> = vec!["surface-only".to_string(), "docs-only".to_string()];
    let outcome_gap = outcome_gap_streak(runs, &markers, &surfaces);
    let outcome_threshold = threshold;

    let small_degraded = small_streak >= threshold;
    let outcome_degraded = outcome_gap >= outcome_threshold
        && runs
            .first()
            .map(|r| {
                delivery_outcome_for_run(r, &markers, &surfaces) != DeliveryOutcome::NotConfigured
            })
            .unwrap_or(false);
    if !small_degraded && !outcome_degraded {
        return None;
    }

    let minimum_scale = "multi_surface_or_implementation";
    let must_include: Vec<String> = vec![
        "coherent_artifact",
        "targeted_validation",
        "state_writeback",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let spend_rule = profile.spend_rule.clone();
    let mode = if outcome_degraded && !small_degraded {
        "expand_after_surface_progress_loop"
    } else {
        "expand_after_repeated_small_delivery"
    };
    let summary = format!(
        "{mode}; minimum_scale={minimum_scale}; include={}; spend_rule={spend_rule}; small_threshold={threshold}; if_blocked=report_blocker_without_spend",
        must_include.join("+")
    );
    let instruction = format!(
        "下一轮回到 active state P0/P1 outcome 做 audit，选连贯段，至少 {minimum_scale}；含真实 {}；禁止 isolated test/surface-only propagation；若只能小步/表面，blocker，不 spend。",
        must_include
            .iter()
            .map(|v| match v.as_str() {
                "coherent_artifact" => "artifact",
                "targeted_validation" => "targeted validation",
                "state_writeback" => "state writeback",
                other => other,
            })
            .collect::<Vec<_>>()
            .join("、")
    );
    Some(DeliveryContract {
        mode: mode.to_string(),
        minimum_scale: minimum_scale.to_string(),
        must_include,
        spend_rule,
        small_scale_streak_threshold: threshold,
        outcome_gap_streak_threshold: outcome_threshold,
        if_blocked: "report_blocker_without_spend".to_string(),
        post_handoff_small_scale_streak: small_streak,
        post_handoff_outcome_gap_streak: outcome_gap,
        summary,
        instruction,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(evidence: &str) -> RunRecord {
        RunRecord {
            turn: 1,
            todo_id: "t1".into(),
            run_id: "r".into(),
            terminal_state: "completed".into(),
            error: None,
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: vec![],
            evidence: evidence.into(),
            recorded_at: 0,
            spend_source: None,
            validation: None,
        }
    }

    fn goal() -> Goal {
        Goal::new("g1", "objective", "/tmp")
    }

    #[test]
    fn no_degradation_no_contract() {
        let runs = vec![run("implemented the fix with tests and writeback")];
        assert!(handoff_delivery_contract(&goal(), &runs).is_none());
    }

    #[test]
    fn repeated_small_delivery_triggers_expand_mode() {
        let runs = vec![run("small tweak"), run("another tweak")];
        let contract = handoff_delivery_contract(&goal(), &runs).unwrap();
        assert_eq!(contract.mode, "expand_after_repeated_small_delivery");
        assert_eq!(contract.post_handoff_small_scale_streak, 2);
        assert!(contract.summary.contains("report_blocker_without_spend"));
        assert!(contract.instruction.contains("不 spend"));
    }

    #[test]
    fn surface_only_loop_triggers_surface_mode() {
        // Multi-surface (not small-scale) runs stuck on surface-only outcomes:
        // the outcome-gap floor triggers expand_after_surface_progress_loop.
        let runs = vec![
            run("multi-surface docs-only change"),
            run("multi-surface surface-only propagation"),
        ];
        let contract = handoff_delivery_contract(&goal(), &runs).unwrap();
        assert_eq!(contract.mode, "expand_after_surface_progress_loop");
        assert_eq!(contract.post_handoff_outcome_gap_streak, 2);
    }

    #[test]
    fn must_include_and_spend_rule_are_carried() {
        let runs = vec![run("small tweak"), run("small tweak")];
        let contract = handoff_delivery_contract(&goal(), &runs).unwrap();
        assert!(contract
            .must_include
            .contains(&"coherent_artifact".to_string()));
        assert!(contract
            .must_include
            .contains(&"state_writeback".to_string()));
        assert!(!contract.spend_rule.is_empty());
    }

    #[test]
    fn is_progress_outcome_classifies_progress() {
        assert!(crate::work_items::delivery::is_progress_outcome(
            crate::work_items::delivery::DeliveryOutcome::OutcomeProgress
        ));
        assert!(!crate::work_items::delivery::is_progress_outcome(
            crate::work_items::delivery::DeliveryOutcome::OutcomeGap
        ));
    }
}
