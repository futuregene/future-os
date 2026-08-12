//! Oscillation detection (LoopX 对比改进项 ③) — the projection-layer
//! sliding-window signature-pair guard against the A→V→A→V incident
//! pattern (action/verify flip-flopping: each delivery is rejected, the
//! retry is accepted, the next delivery is rejected again — spend burns
//! without the frontier ever converging).
//!
//! The read model is the goal's run-history projection (`Goal::history`,
//! folded from `RunRecorded` events): every delivery-class record projects
//! to one outcome signature — `Accepted` (A) or `Rejected` (V). Over the
//! post-ack signature stream the detector slides a window of
//! [`OSCILLATION_PATTERN_LEN`] and fires when the newest window strictly
//! alternates (A→V→A→V or V→A→V→A): two consecutive repetitions of the
//! same accept/reject pair.
//!
//! Why per-todo guards don't catch it: the repair budget
//! (`MAX_REPAIR_ATTEMPTS`) trips on *consecutive* failures of one todo, and
//! the outcome floor trips on *surface-only* turns. The oscillation pattern
//! completes every todo on retry — no budget exhausts, outcomes are
//! material — yet every first-pass delivery is rejected, doubling cost per
//! unit of progress (the $47K-class runaway this guard exists to stop).
//!
//! Response: the decision kernel converts the next delivery into a replan
//! (same family as outcome-floor / monitor-stall / repair-budget triggers),
//! forcing a frontier-changing delta instead of the next rejected cycle.
//! LoopX alignment: this is the actuation behind the seven-predicate
//! contract's delivery lanes — while oscillating, `normal_delivery_allowed`
//! reads false and only the repair lane (`self_repair_allowed`) stays open.
//!
//! Liveness: the detector only observes records *after* the last replan
//! ACK (`Goal::replan_ack.at`). Without that reset the alternating tail
//! would re-fire immediately after every ACK while delivery stays blocked,
//! deadlocking the goal. Post-ACK the pattern must re-emerge across four
//! fresh delivery outcomes before the guard fires again — a single ACK
//! buys exactly one chance to break the loop, not a permanent exemption.

use crate::quota::slot_accounting::{classify_record, SlotSpendSource};
use crate::state::{Goal, RunRecord};

/// Sliding-window size: two full A→V pairs (A→V→A→V) establish the
/// alternation. Tight enough to intervene before the burn compounds,
/// loose enough that a lone flaky validation (A→V→A) never trips it.
pub const OSCILLATION_PATTERN_LEN: usize = 4;

/// The outcome signature of one delivery turn: the projection symbol the
/// sliding window runs over. (`A` / `V` in the incident vocabulary.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum RunSignature {
    /// A — an accepted delivery action: the turn completed and, when an
    /// independent validator ran, passed.
    Accepted,
    /// V — the verify phase rejected the delivery: the turn completed but
    /// the independent validator failed (`--verify` exit != 0).
    Rejected,
}

impl RunSignature {
    fn symbol(self) -> &'static str {
        match self {
            RunSignature::Accepted => "A",
            RunSignature::Rejected => "V",
        }
    }
}

/// What the detector found: the strictly-alternating suffix of the
/// post-ACK signature stream.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OscillationReport {
    /// Length of the alternating suffix (>= [`OSCILLATION_PATTERN_LEN`]).
    pub alternating_len: usize,
    /// The suffix rendered in A/V symbols, oldest first (e.g. "A→V→A→V").
    pub pattern: String,
    /// `recorded_at` of the newest record in the suffix (diagnostics).
    pub newest_recorded_at: u64,
}

/// Project one run record into its outcome signature. Returns `None` for
/// records that carry no delivery verdict:
///
/// - quota-neutral records (monitor polls, replan slices, heartbeats —
///   [`SlotSpendSource::Heartbeat`]) never enter the stream, so background
///   activity neither fabricates nor breaks an alternation;
/// - turns that did not complete produced no verify verdict (a crashed
///   turn is the repair budget's concern, not an action/verify flip-flop).
pub fn signature_of(record: &RunRecord) -> Option<RunSignature> {
    if classify_record(record) == SlotSpendSource::Heartbeat {
        return None;
    }
    if record.terminal_state != "completed" {
        return None;
    }
    Some(match &record.validation {
        Some(v) if !v.ok => RunSignature::Rejected,
        _ => RunSignature::Accepted,
    })
}

/// Length of the strictly-alternating suffix: 1 + the number of backward
/// steps from the tail in which adjacent signatures differ.
fn alternating_suffix_len(signatures: &[RunSignature]) -> usize {
    let mut len = signatures.len().min(1);
    for pair in signatures.windows(2).rev() {
        if pair[0] == pair[1] {
            break;
        }
        len += 1;
    }
    len
}

/// Detect the A→V→A→V oscillation over the goal's run-history projection,
/// observing only delivery records with `recorded_at > since` (`since` is
/// the last replan-ACK timestamp, 0 when the goal never ACKed).
pub fn detect(history: &[RunRecord], since: u64) -> Option<OscillationReport> {
    let signatures: Vec<(u64, RunSignature)> = history
        .iter()
        .filter(|r| r.recorded_at > since)
        .filter_map(|r| signature_of(r).map(|s| (r.recorded_at, s)))
        .collect();
    let len = alternating_suffix_len(&signatures.iter().map(|(_, s)| *s).collect::<Vec<_>>());
    if len < OSCILLATION_PATTERN_LEN {
        return None;
    }
    let suffix = &signatures[signatures.len() - len..];
    Some(OscillationReport {
        alternating_len: len,
        pattern: suffix
            .iter()
            .map(|(_, s)| s.symbol())
            .collect::<Vec<_>>()
            .join("→"),
        newest_recorded_at: suffix.last().map(|(ts, _)| *ts).unwrap_or(0),
    })
}

/// Decision-facing predicate (stall family): the replan reason when the
/// goal oscillates, `None` while delivery may proceed. `pub(crate)` — the
/// kernel is the only caller; tests go through [`crate::decision::decide`].
pub(crate) fn oscillation_replan_reason(goal: &Goal) -> Option<String> {
    let since = goal.replan_ack.as_ref().map(|a| a.at).unwrap_or(0);
    detect(&goal.history, since).map(|report| {
        format!(
            "oscillation detected: delivery outcomes flip-flop {} ({} consecutive alternating turns) — record a frontier-changing replan delta to break the action/verify loop",
            report.pattern, report.alternating_len
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{task_validation_receipt, RunRecord, ValidationStatus};

    fn record(ts: u64, terminal_state: &str, validation_ok: Option<bool>) -> RunRecord {
        RunRecord {
            turn: ts as u32,
            todo_id: "t".to_string(),
            run_id: format!("run-{ts}"),
            terminal_state: terminal_state.to_string(),
            error: None,
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: vec![],
            evidence: String::new(),
            recorded_at: ts,
            spend_source: Some("run".to_string()),
            validation: validation_ok.map(|ok| {
                if ok {
                    task_validation_receipt(ValidationStatus::Passed, "v", "ok", None, Some(0))
                } else {
                    task_validation_receipt(
                        ValidationStatus::Failed,
                        "v",
                        "rejected",
                        Some(crate::state::RecoveryKind::RepairRequired),
                        Some(1),
                    )
                }
            }),
        }
    }

    fn accepted(ts: u64) -> RunRecord {
        record(ts, "completed", None)
    }

    fn accepted_validated(ts: u64) -> RunRecord {
        record(ts, "completed", Some(true))
    }

    fn rejected(ts: u64) -> RunRecord {
        record(ts, "completed", Some(false))
    }

    #[test]
    fn signature_classification() {
        assert_eq!(signature_of(&accepted(1)), Some(RunSignature::Accepted));
        assert_eq!(
            signature_of(&accepted_validated(1)),
            Some(RunSignature::Accepted)
        );
        assert_eq!(signature_of(&rejected(1)), Some(RunSignature::Rejected));
        // A failed turn carries no verify verdict.
        assert_eq!(signature_of(&record(1, "failed", None)), None);
        // Quota-neutral records never enter the stream — even completed ones.
        let mut poll = accepted(1);
        poll.spend_source = Some("heartbeat".to_string());
        assert_eq!(signature_of(&poll), None);
        // Legacy unstamped completed records classify as delivery turns.
        let mut legacy = accepted(1);
        legacy.spend_source = None;
        assert_eq!(signature_of(&legacy), Some(RunSignature::Accepted));
        // Legacy unstamped non-completed records are heartbeat-classified
        // and stay out of the stream either way.
        let mut legacy_failed = record(1, "failed", None);
        legacy_failed.spend_source = None;
        assert_eq!(signature_of(&legacy_failed), None);
    }

    #[test]
    fn alternating_suffix_lengths() {
        use RunSignature::{Accepted as A, Rejected as V};
        assert_eq!(alternating_suffix_len(&[]), 0);
        assert_eq!(alternating_suffix_len(&[A]), 1);
        assert_eq!(alternating_suffix_len(&[A, A]), 1);
        assert_eq!(alternating_suffix_len(&[A, V]), 2);
        assert_eq!(alternating_suffix_len(&[A, V, A]), 3);
        assert_eq!(alternating_suffix_len(&[A, V, A, V]), 4);
        // A same-symbol pair resets the suffix regardless of what led it.
        assert_eq!(alternating_suffix_len(&[A, V, A, V, V]), 1);
        assert_eq!(alternating_suffix_len(&[V, V, A, V, A]), 4);
    }

    #[test]
    fn detect_fires_on_two_full_pairs() {
        let history = vec![accepted(1), rejected(2), accepted(3), rejected(4)];
        let report = detect(&history, 0).expect("A→V→A→V must fire");
        assert_eq!(report.alternating_len, 4);
        assert_eq!(report.pattern, "A→V→A→V");
        assert_eq!(report.newest_recorded_at, 4);
    }

    #[test]
    fn detect_fires_on_v_first_pattern() {
        let history = vec![rejected(1), accepted(2), rejected(3), accepted(4)];
        let report = detect(&history, 0).expect("V→A→V→A must fire");
        assert_eq!(report.pattern, "V→A→V→A");
    }

    #[test]
    fn detect_ignores_shorter_than_two_pairs() {
        assert!(detect(&[], 0).is_none());
        assert!(detect(&[accepted(1)], 0).is_none());
        assert!(detect(&[accepted(1), rejected(2)], 0).is_none());
        assert!(detect(&[accepted(1), rejected(2), accepted(3)], 0).is_none());
        // Consecutive rejects break the alternation (repair-budget domain).
        let history = vec![
            accepted(1),
            rejected(2),
            rejected(3),
            accepted(4),
            rejected(5),
        ];
        assert!(detect(&history, 0).is_none());
    }

    #[test]
    fn detect_reports_the_suffix_not_the_prefix() {
        // Stabilized start, oscillating tail → fires on the tail.
        let history = vec![
            accepted(1),
            accepted(2),
            rejected(3),
            accepted(4),
            rejected(5),
            accepted(6),
        ];
        let report = detect(&history, 0).expect("oscillating tail must fire");
        assert_eq!(report.alternating_len, 5);
        assert_eq!(report.pattern, "A→V→A→V→A");
        // Oscillating start, stabilized tail → no fire.
        let history = vec![
            accepted(1),
            rejected(2),
            accepted(3),
            rejected(4),
            accepted(5),
            accepted(6),
        ];
        assert!(detect(&history, 0).is_none());
    }

    #[test]
    fn non_delivery_records_neither_fabricate_nor_break_the_pattern() {
        let mut poll = accepted(2);
        poll.spend_source = Some("heartbeat".to_string());
        let failed_turn = record(3, "failed", None);
        let history = vec![
            accepted(1),
            poll,
            failed_turn,
            rejected(4),
            accepted(5),
            rejected(6),
        ];
        let report = detect(&history, 0).expect("heartbeat/failed records must be transparent");
        assert_eq!(report.pattern, "A→V→A→V");
    }

    #[test]
    fn since_filter_scopes_observation_to_post_ack_records() {
        let history = vec![accepted(1), rejected(2), accepted(3), rejected(4)];
        // All four records predate the ACK → the pattern is consumed.
        assert!(detect(&history, 4).is_none());
        assert!(detect(&history, 10).is_none());
        // A partial post-ACK pattern does not fire.
        assert!(detect(&history, 2).is_none());
        let mut fresh = history.clone();
        fresh.push(accepted(5));
        // Only one fresh record post-ACK at 4 → no re-fire yet.
        assert!(detect(&fresh, 4).is_none());
        // The full pattern re-emerging post-ACK re-fires.
        let mut fresh = history.clone();
        fresh.extend([accepted(5), rejected(6), accepted(7), rejected(8)]);
        let report = detect(&fresh, 4).expect("fresh post-ACK alternation must re-fire");
        assert_eq!(report.alternating_len, 4);
        assert_eq!(report.newest_recorded_at, 8);
    }
}
