//! Outcome continuity (G13 ①) — outcome-streak segmentation, the reference
//! `goal_frontier/outcome_continuity.py` (501 lines; core subset).
//!
//! The kernel's `outcome_streak` counter is a single number; the segment
//! projection restores the *structure* of that streak: a segment is a
//! maximal run of consecutive same-kind turns (surface-only vs material)
//! over one frontier slice. A segment RESETS when:
//!   - the turn kind flips (material ⇄ surface-only), or
//!   - the frontier changed between the two runs — a frontier-changing
//!     event (todo added/completed/superseded, gate resolved, frontier-delta
//!     replan ack, todo archived) landed in `(prev.recorded_at, run.recorded_at]`.
//!
//! Segments are a PROJECTION over goal state (run history + the
//! `Goal::frontier_change_ts` markers folded during replay) — never a
//! second source of truth, same rule as `todo_summary`.

use serde::{Deserialize, Serialize};

use crate::state::Goal;

pub const OUTCOME_SEGMENT_SCHEMA_VERSION: &str = "goal_outcome_segment_v0";

/// Segment kind vocabulary (reference VISION_OUTCOME_CHECKPOINT material vs
/// continuation outcomes, collapsed to the two streak classes).
pub const SEGMENT_KIND_SURFACE_ONLY: &str = "surface_only";
pub const SEGMENT_KIND_MATERIAL: &str = "material";

/// One outcome-continuity segment: `{segment_id, start_turn, length, kind}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeSegment {
    pub segment_id: String,
    pub start_turn: u32,
    pub length: u32,
    pub kind: String,
}

/// Materiality rule shared with the executor writeback and the run-history
/// decision-context provider: a turn is material when it produced a
/// validated artifact (tools invoked + evidence).
pub fn run_is_material(record: &crate::state::RunRecord) -> bool {
    !record.tools.is_empty() && !record.evidence.trim().is_empty()
}

fn kind_of(record: &crate::state::RunRecord) -> &'static str {
    if run_is_material(record) {
        SEGMENT_KIND_MATERIAL
    } else {
        SEGMENT_KIND_SURFACE_ONLY
    }
}

/// Did a frontier-changing event land between `prev_at` (exclusive) and
/// `at` (inclusive)? Markers are the `Goal::frontier_change_ts` timestamps
/// folded from frontier-changing events during replay.
fn frontier_changed_between(goal: &Goal, prev_at: u64, at: u64) -> bool {
    goal.frontier_change_ts
        .iter()
        .any(|ts| *ts > prev_at && *ts <= at)
}

/// Project the outcome-continuity segments over the run history.
/// Deterministic: a pure function of goal state (run records in ledger
/// order + frontier-change markers). Runs outside any segment are dropped
/// from the projection (they are not part of a streak).
pub fn outcome_segments(goal: &Goal) -> Vec<OutcomeSegment> {
    let mut segments: Vec<OutcomeSegment> = vec![];
    let mut current_kind: Option<&'static str> = None;
    let mut prev_at: Option<u64> = None;

    for record in &goal.history {
        let kind = kind_of(record);
        let frontier_reset = prev_at
            .map(|prev| frontier_changed_between(goal, prev, record.recorded_at))
            .unwrap_or(false);
        if current_kind == Some(kind) && !frontier_reset {
            if let Some(last) = segments.last_mut() {
                last.length += 1;
            }
        } else {
            segments.push(OutcomeSegment {
                segment_id: format!("seg_{}", record.turn),
                start_turn: record.turn,
                length: 1,
                kind: kind.to_string(),
            });
            current_kind = Some(kind);
        }
        prev_at = Some(record.recorded_at);
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RunRecord;

    fn run(turn: u32, at: u64, tools: &[&str], evidence: &str) -> RunRecord {
        RunRecord {
            turn,
            todo_id: format!("T{turn}"),
            run_id: format!("r{turn}"),
            validation: None,
            terminal_state: "completed".to_string(),
            error: None,
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: tools.iter().map(|s| s.to_string()).collect(),
            evidence: evidence.to_string(),
            recorded_at: at,
            spend_source: None,
            failure_kind: None,
        }
    }

    #[test]
    fn surface_only_streak_forms_one_segment() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.history.push(run(1, 10, &[], ""));
        g.history.push(run(2, 20, &[], ""));
        g.history.push(run(3, 30, &[], ""));
        let segments = outcome_segments(&g);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].kind, SEGMENT_KIND_SURFACE_ONLY);
        assert_eq!(segments[0].start_turn, 1);
        assert_eq!(segments[0].length, 3);
        assert_eq!(segments[0].segment_id, "seg_1");
    }

    #[test]
    fn material_turn_flips_the_segment_kind() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.history.push(run(1, 10, &[], ""));
        g.history.push(run(2, 20, &["shell"], "artifact"));
        g.history.push(run(3, 30, &[], ""));
        let segments = outcome_segments(&g);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].kind, SEGMENT_KIND_SURFACE_ONLY);
        assert_eq!(segments[1].kind, SEGMENT_KIND_MATERIAL);
        assert_eq!(segments[1].length, 1);
        assert_eq!(segments[2].kind, SEGMENT_KIND_SURFACE_ONLY);
    }

    #[test]
    fn frontier_change_between_runs_resets_the_segment() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.history.push(run(1, 10, &[], ""));
        g.history.push(run(2, 20, &[], ""));
        // A todo completed at ts 25 (between run 2 and run 3) moved the
        // frontier: the streak continuity resets even though run 3 is
        // surface-only like its predecessors.
        g.frontier_change_ts.push(25);
        g.history.push(run(3, 30, &[], ""));
        g.history.push(run(4, 40, &[], ""));
        let segments = outcome_segments(&g);
        assert_eq!(segments.len(), 2, "frontier change resets the segment");
        assert_eq!(segments[0].length, 2);
        assert_eq!(segments[1].length, 2);
        assert_eq!(segments[1].start_turn, 3);
        // A marker at/after the run start also resets (boundary inclusive).
        let mut g2 = Goal::new("g2", "o", "/tmp");
        g2.history.push(run(1, 10, &[], ""));
        g2.frontier_change_ts.push(30);
        g2.history.push(run(2, 30, &[], ""));
        assert_eq!(outcome_segments(&g2).len(), 2);
        // A marker BEFORE both runs does not reset mid-segment.
        let mut g3 = Goal::new("g3", "o", "/tmp");
        g3.frontier_change_ts.push(5);
        g3.history.push(run(1, 10, &[], ""));
        g3.history.push(run(2, 20, &[], ""));
        assert_eq!(outcome_segments(&g3).len(), 1);
    }
}
