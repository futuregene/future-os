//! Delivery signals (G-15) — LoopX `control_plane/work_items/delivery_*`,
//! natively (compact set): the batch-scale and outcome classifications
//! derived from a run's evidence, plus the two streak counters the delivery
//! contract (G-17) consumes (`small_delivery_batch_scale_streak`,
//! `outcome_gap_streak`). Runs are expected newest-first.

use crate::state::RunRecord;

/// LoopX DeliveryBatchScale values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryBatchScale {
    TestOnly,
    SingleSurface,
    MultiSurface,
    Implementation,
    Unknown,
}

impl DeliveryBatchScale {
    pub fn label(&self) -> &'static str {
        match self {
            DeliveryBatchScale::TestOnly => "test_only",
            DeliveryBatchScale::SingleSurface => "single_surface",
            DeliveryBatchScale::MultiSurface => "multi_surface",
            DeliveryBatchScale::Implementation => "implementation",
            DeliveryBatchScale::Unknown => "unknown",
        }
    }
}

/// LoopX DeliveryOutcome values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    OutcomeProgress,
    SurfaceOnly,
    OutcomeGap,
    NotConfigured,
    Unknown,
}

impl DeliveryOutcome {
    pub fn label(&self) -> &'static str {
        match self {
            DeliveryOutcome::OutcomeProgress => "outcome_progress",
            DeliveryOutcome::SurfaceOnly => "surface_only",
            DeliveryOutcome::OutcomeGap => "outcome_gap",
            DeliveryOutcome::NotConfigured => "not_configured",
            DeliveryOutcome::Unknown => "unknown",
        }
    }
}

/// Outcomes that count as accountable progress (LoopX
/// ACCOUNTABLE_DELIVERY_OUTCOMES: outcome_progress / primary_goal_outcome —
/// surface_only is NOT progress; it breaks the gap streak).
pub fn is_progress_outcome(outcome: DeliveryOutcome) -> bool {
    matches!(outcome, DeliveryOutcome::OutcomeProgress)
}

const TEST_ONLY_HINTS: [&str; 4] = ["test-only", "test_only", "unit test", "isolated test"];
const MULTI_SURFACE_HINTS: [&str; 4] = [
    "multi-surface",
    "multi_surface",
    "multiple surfaces",
    "across surfaces",
];
const IMPLEMENTATION_HINTS: [&str; 2] = ["implementation", "implemented"];

fn evidence_text(run: &RunRecord) -> String {
    // Trim so a run with neither terminal_state nor evidence is genuinely
    // empty (and classifies as Unknown rather than SingleSurface).
    format!("{} {}", run.terminal_state, run.evidence)
        .trim()
        .to_lowercase()
}

/// Classify a run's delivery batch scale from its evidence (LoopX
/// delivery_batch_scale_for_run, declarative hints).
pub fn delivery_batch_scale_for_run(run: &RunRecord) -> DeliveryBatchScale {
    let text = evidence_text(run);
    if TEST_ONLY_HINTS.iter().any(|h| text.contains(h)) {
        DeliveryBatchScale::TestOnly
    } else if MULTI_SURFACE_HINTS.iter().any(|h| text.contains(h)) {
        DeliveryBatchScale::MultiSurface
    } else if IMPLEMENTATION_HINTS.iter().any(|h| text.contains(h)) {
        DeliveryBatchScale::Implementation
    } else if text.is_empty() {
        DeliveryBatchScale::Unknown
    } else {
        DeliveryBatchScale::SingleSurface
    }
}

/// Classify a run's delivery outcome against the outcome-floor markers
/// (LoopX delivery_outcome_for_run): a surface-only hit wins over a progress
/// marker; with no markers configured the floor is not configured.
pub fn delivery_outcome_for_run(
    run: &RunRecord,
    outcome_markers: &[String],
    surface_only_hints: &[String],
) -> DeliveryOutcome {
    let text = evidence_text(run);
    if outcome_markers.is_empty() && surface_only_hints.is_empty() {
        return DeliveryOutcome::NotConfigured;
    }
    if surface_only_hints
        .iter()
        .any(|h| text.contains(&h.to_lowercase()))
    {
        return DeliveryOutcome::SurfaceOnly;
    }
    if outcome_markers
        .iter()
        .any(|h| text.contains(&h.to_lowercase()))
    {
        return DeliveryOutcome::OutcomeProgress;
    }
    DeliveryOutcome::OutcomeGap
}

/// Whether an outcome floor is configured (markers or surface hints exist).
pub fn outcome_floor_configured(outcome_markers: &[String], surface_only_hints: &[String]) -> bool {
    !outcome_markers.is_empty() || !surface_only_hints.is_empty()
}

/// Consecutive newest runs that are outcome gaps (breaks on progress or a
/// not-configured floor). LoopX outcome_gap_streak.
pub fn outcome_gap_streak(
    runs: &[RunRecord],
    outcome_markers: &[String],
    surface_only_hints: &[String],
) -> u32 {
    if !outcome_floor_configured(outcome_markers, surface_only_hints) {
        return 0;
    }
    let mut streak = 0u32;
    for run in runs {
        let outcome = delivery_outcome_for_run(run, outcome_markers, surface_only_hints);
        if is_progress_outcome(outcome) || outcome == DeliveryOutcome::NotConfigured {
            break;
        }
        streak += 1;
    }
    streak
}

/// Consecutive newest runs delivered at a small batch scale (LoopX
/// small_delivery_batch_scale_streak).
pub fn small_delivery_batch_scale_streak(runs: &[RunRecord]) -> u32 {
    let mut streak = 0u32;
    for run in runs {
        let scale = delivery_batch_scale_for_run(run);
        if !matches!(
            scale,
            DeliveryBatchScale::SingleSurface | DeliveryBatchScale::TestOnly
        ) {
            break;
        }
        streak += 1;
    }
    streak
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
            failure_kind: None,
        }
    }

    #[test]
    fn batch_scale_unknown_when_no_state_or_evidence() {
        let mut r = run("");
        r.terminal_state = String::new();
        assert_eq!(
            delivery_batch_scale_for_run(&r),
            DeliveryBatchScale::Unknown
        );
    }

    #[test]
    fn batch_scale_classification() {
        assert_eq!(
            delivery_batch_scale_for_run(&run("added a unit test only")),
            DeliveryBatchScale::TestOnly
        );
        assert_eq!(
            delivery_batch_scale_for_run(&run("changed docs and code across surfaces")),
            DeliveryBatchScale::MultiSurface
        );
        assert_eq!(
            delivery_batch_scale_for_run(&run("implemented the fix")),
            DeliveryBatchScale::Implementation
        );
        assert_eq!(
            delivery_batch_scale_for_run(&run("small tweak")),
            DeliveryBatchScale::SingleSurface
        );
    }

    #[test]
    fn outcome_classification_prefers_surface_hint() {
        let markers = vec!["merged".to_string()];
        let surfaces = vec!["docs-only".to_string()];
        assert_eq!(
            delivery_outcome_for_run(&run("docs-only change, merged"), &markers, &surfaces),
            DeliveryOutcome::SurfaceOnly
        );
        assert_eq!(
            delivery_outcome_for_run(&run("real fix merged"), &markers, &surfaces),
            DeliveryOutcome::OutcomeProgress
        );
        assert_eq!(
            delivery_outcome_for_run(&run("nothing yet"), &markers, &surfaces),
            DeliveryOutcome::OutcomeGap
        );
    }

    #[test]
    fn unconfigured_floor_never_gaps() {
        let runs = vec![run("nothing"), run("nothing")];
        assert_eq!(outcome_gap_streak(&runs, &[], &[]), 0);
    }

    #[test]
    fn gap_streak_breaks_on_progress() {
        let markers = vec!["merged".to_string()];
        let surfaces = vec![];
        let runs = vec![run("gap"), run("gap"), run("merged"), run("gap")];
        // newest-first: gap, gap, merged → streak 2
        assert_eq!(outcome_gap_streak(&runs, &markers, &surfaces), 2);
    }

    #[test]
    fn surface_only_is_a_gap_not_progress() {
        let markers = vec!["merged".to_string()];
        let surfaces = vec!["docs-only".to_string()];
        let runs = vec![run("docs-only change"), run("docs-only again")];
        assert_eq!(outcome_gap_streak(&runs, &markers, &surfaces), 2);
    }

    #[test]
    fn small_scale_streak_counts_consecutive() {
        let runs = vec![
            run("small tweak"),
            run("unit test only"),
            run("implemented the real feature"),
        ];
        assert_eq!(small_delivery_batch_scale_streak(&runs), 2);
    }
}
