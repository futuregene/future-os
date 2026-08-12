//! reward_memory capability (LoopX: reward_memory — deterministic rule translation
//! into finite typed proposals).
//!
//! P1-5 deep implementation, phase 1 (LoopX `capabilities/reward_memory/`
//! ingestion.py + scoped_feedback.py, compact set): cross-run learning
//! infrastructure with exactly two surfaces —
//!
//! ① ingestion — reward signals land in the goal's event ledger as
//!    `RewardSignalRecorded` events. Three sources: `validator` (the run
//!    path auto-records the independent task-validation receipt of every
//!    turn), `delivery_outcome` (a P0-2 delivery resolution —
//!    verified/failed/rework — auto-records its signal), and `evidence`
//!    (an operator/agent scores evidence manually via
//!    `reward-memory record`). Experiment / dogfood / provider sync from
//!    the LoopX surface are deliberately out of scope for phase 1.
//! ② scoped_feedback — [`collect_signals`] + [`summarize`] project the
//!    ledger back out by scope (goal is implicit; agent / todo / source
//!    filter), surfaced as `reward-memory query`.

use std::collections::BTreeMap;

use super::{successor_todo, Capability, TypedProposal};
use crate::state::{TaskValidation, ValidationStatus};
use crate::store::{Event, StoredEvent};

/// Signal source: the run path's independent task-validation receipt.
pub const SOURCE_VALIDATOR: &str = "validator";
/// Signal source: a P0-2 delivery outcome resolution (verified/failed/rework).
pub const SOURCE_DELIVERY_OUTCOME: &str = "delivery_outcome";
/// Signal source: a manual evidence score (`reward-memory record`).
pub const SOURCE_EVIDENCE: &str = "evidence";

/// All ingestion sources (CLI `--source` choices).
pub const REWARD_SOURCE_CHOICES: [&str; 3] =
    [SOURCE_VALIDATOR, SOURCE_DELIVERY_OUTCOME, SOURCE_EVIDENCE];

/// Normalize a source token (case/whitespace-insensitive) to its canonical
/// value; `None` for unknown values.
pub fn normalize_source(value: &str) -> Option<&'static str> {
    match value.trim().to_lowercase().as_str() {
        SOURCE_VALIDATOR => Some(SOURCE_VALIDATOR),
        SOURCE_DELIVERY_OUTCOME => Some(SOURCE_DELIVERY_OUTCOME),
        SOURCE_EVIDENCE => Some(SOURCE_EVIDENCE),
        _ => None,
    }
}

/// Phase-1 validator → signal mapping (deterministic):
/// passed = 1.0, progress = 0.5, failed = 0.0; inconclusive / unavailable
/// carry no score (the validator could not decide); `not_required` means no
/// independent validation happened, so nothing is ingested at all.
pub fn validator_signal(v: &TaskValidation) -> Option<(&'static str, Option<f64>)> {
    match v.status {
        ValidationStatus::Passed => Some(("passed", Some(1.0))),
        ValidationStatus::Progress => Some(("progress", Some(0.5))),
        ValidationStatus::Failed => Some(("failed", Some(0.0))),
        ValidationStatus::Inconclusive => Some(("inconclusive", None)),
        ValidationStatus::Unavailable => Some(("unavailable", None)),
        ValidationStatus::NotRequired => None,
    }
}

/// Phase-1 delivery-outcome → signal mapping (deterministic): the three
/// terminal resolutions — verified = 1.0, rework = 0.5 (delivered value but
/// changes required), failed = 0.0. The pending `delivered` state is not a
/// reward signal (no outcome yet).
pub fn delivery_outcome_signal(outcome: &str) -> Option<(&'static str, Option<f64>)> {
    match outcome {
        crate::work_items::delivery_outcome::OUTCOME_VERIFIED => Some(("verified", Some(1.0))),
        crate::work_items::delivery_outcome::OUTCOME_REWORK => Some(("rework", Some(0.5))),
        crate::work_items::delivery_outcome::OUTCOME_FAILED => Some(("failed", Some(0.0))),
        _ => None,
    }
}

/// One ingested reward signal — the read-model view of a
/// `RewardSignalRecorded` ledger event.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RewardSignal {
    pub todo_id: String,
    pub agent_id: Option<String>,
    pub run_id: Option<String>,
    pub source: String,
    pub signal: String,
    pub score: Option<f64>,
    pub note: Option<String>,
    pub seq: u32,
    pub ts: u64,
}

/// Scope filter for the scoped-feedback query. The goal scope is implicit
/// (the query reads one goal's ledger); agent / todo / source narrow it.
#[derive(Debug, Clone, Default)]
pub struct RewardScope<'a> {
    pub agent_id: Option<&'a str>,
    pub todo_id: Option<&'a str>,
    pub source: Option<&'a str>,
}

impl RewardScope<'_> {
    fn matches(&self, s: &RewardSignal) -> bool {
        if let Some(a) = self.agent_id {
            if s.agent_id.as_deref() != Some(a) {
                return false;
            }
        }
        if let Some(t) = self.todo_id {
            if s.todo_id != t {
                return false;
            }
        }
        if let Some(src) = self.source {
            if s.source != src {
                return false;
            }
        }
        true
    }
}

/// Collect the reward signals from one goal's ledger, filtered by scope.
/// Deterministic order: ledger order (append order).
pub fn collect_signals(events: &[StoredEvent], scope: &RewardScope) -> Vec<RewardSignal> {
    events
        .iter()
        .filter_map(|se| match &se.event {
            Event::RewardSignalRecorded {
                todo_id,
                agent_id,
                run_id,
                source,
                signal,
                score,
                note,
                seq,
                ts,
                ..
            } => Some(RewardSignal {
                todo_id: todo_id.clone(),
                agent_id: agent_id.clone(),
                run_id: run_id.clone(),
                source: source.clone(),
                signal: signal.clone(),
                score: *score,
                note: note.clone(),
                seq: *seq,
                ts: *ts,
            }),
            _ => None,
        })
        .filter(|s| scope.matches(s))
        .collect()
}

/// The next per-todo ingestion sequence (1 + the number of reward signals
/// already in the ledger for this todo) — the G-3 content-id dedupe anchor
/// for otherwise-identical signals appended within the same second.
pub fn next_seq(events: &[StoredEvent], todo_id: &str) -> u32 {
    let count = events
        .iter()
        .filter(|se| matches!(&se.event, Event::RewardSignalRecorded { todo_id: t, .. } if t == todo_id))
        .count();
    count as u32 + 1
}

/// Aggregate view over a scoped signal set (the scoped-feedback summary).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RewardSummary {
    pub total: usize,
    /// Signal count per source (validator / delivery_outcome / evidence).
    pub by_source: BTreeMap<String, usize>,
    /// How many signals carry a score; `avg_score` averages exactly those.
    pub scored: usize,
    pub avg_score: Option<f64>,
    /// Validator-signal outcome counts (the phase-1 learning headline:
    /// does the independent validator keep passing?).
    pub validator_passed: usize,
    pub validator_failed: usize,
}

/// Summarize a scoped signal set (deterministic aggregation).
pub fn summarize(signals: &[RewardSignal]) -> RewardSummary {
    let mut by_source: BTreeMap<String, usize> = BTreeMap::new();
    let mut scored = 0usize;
    let mut total_score = 0.0f64;
    let mut validator_passed = 0usize;
    let mut validator_failed = 0usize;
    for s in signals {
        *by_source.entry(s.source.clone()).or_insert(0) += 1;
        if let Some(score) = s.score {
            scored += 1;
            total_score += score;
        }
        if s.source == SOURCE_VALIDATOR {
            match s.signal.as_str() {
                "passed" => validator_passed += 1,
                "failed" => validator_failed += 1,
                _ => {}
            }
        }
    }
    RewardSummary {
        total: signals.len(),
        by_source,
        scored,
        avg_score: (scored > 0).then_some(total_score / scored as f64),
        validator_passed,
        validator_failed,
    }
}

pub struct RewardMemoryCapability;

impl Capability for RewardMemoryCapability {
    fn name(&self) -> &'static str {
        "reward_memory"
    }
    fn describe(&self) -> &'static str {
        "ingest reward signals (validator / delivery outcome / evidence) into the ledger and query them by scope"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup("empty input for reward_memory")];
        }
        let l = text.to_lowercase();
        if l.contains("reward") {
            return vec![TypedProposal::gate(
                "Confirm this reward as a reusable memory candidate before it affects future runs.",
                "rule: explicit confirmation required",
            )];
        }
        if l.contains("评价") {
            return vec![TypedProposal::gate("Confirm this evaluation as a reusable memory candidate before it affects future runs.", "rule: explicit confirmation required")];
        }
        vec![TypedProposal::successor(
            successor_todo(
                "clarify",
                "Clarify the request before acting (missing signal).",
            ),
            "rule: no marker matched",
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RecoveryKind;

    fn validation(status: ValidationStatus) -> TaskValidation {
        crate::state::task_validation_receipt(
            status,
            "shell",
            "summary",
            Some(RecoveryKind::RepairRequired),
            Some(1),
        )
    }

    fn stored_signal(todo_id: &str, source: &str, signal: &str, score: Option<f64>) -> StoredEvent {
        StoredEvent {
            event_id: String::new(),
            producer: None,
            source_ref: None,
            source_section: None,
            source_line: None,
            privacy: None,
            event: Event::RewardSignalRecorded {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                agent_id: None,
                run_id: None,
                source: source.into(),
                signal: signal.into(),
                score,
                note: None,
                seq: 1,
                ts: 100,
            },
        }
    }

    #[test]
    fn validator_signal_maps_every_status() {
        assert_eq!(
            validator_signal(&validation(ValidationStatus::Passed)),
            Some(("passed", Some(1.0)))
        );
        assert_eq!(
            validator_signal(&validation(ValidationStatus::Progress)),
            Some(("progress", Some(0.5)))
        );
        assert_eq!(
            validator_signal(&validation(ValidationStatus::Failed)),
            Some(("failed", Some(0.0)))
        );
        assert_eq!(
            validator_signal(&validation(ValidationStatus::Inconclusive)),
            Some(("inconclusive", None))
        );
        assert_eq!(
            validator_signal(&validation(ValidationStatus::Unavailable)),
            Some(("unavailable", None))
        );
        // not_required = no independent validation happened → no ingestion.
        assert_eq!(
            validator_signal(&validation(ValidationStatus::NotRequired)),
            None
        );
    }

    #[test]
    fn delivery_outcome_signal_maps_resolutions_only() {
        use crate::work_items::delivery_outcome as dov;
        assert_eq!(
            delivery_outcome_signal(dov::OUTCOME_VERIFIED),
            Some(("verified", Some(1.0)))
        );
        assert_eq!(
            delivery_outcome_signal(dov::OUTCOME_REWORK),
            Some(("rework", Some(0.5)))
        );
        assert_eq!(
            delivery_outcome_signal(dov::OUTCOME_FAILED),
            Some(("failed", Some(0.0)))
        );
        // The pending state is not a reward signal; neither is garbage.
        assert_eq!(delivery_outcome_signal(dov::OUTCOME_DELIVERED), None);
        assert_eq!(delivery_outcome_signal("bogus"), None);
    }

    #[test]
    fn normalize_source_accepts_canonical_and_rejects_unknown() {
        assert_eq!(normalize_source("validator"), Some(SOURCE_VALIDATOR));
        assert_eq!(normalize_source(" Evidence "), Some(SOURCE_EVIDENCE));
        assert_eq!(
            normalize_source("DELIVERY_OUTCOME"),
            Some(SOURCE_DELIVERY_OUTCOME)
        );
        assert_eq!(normalize_source("nope"), None);
    }

    #[test]
    fn collect_filters_by_agent_todo_and_source_scope() {
        let mut with_agent = stored_signal("t1", SOURCE_VALIDATOR, "passed", Some(1.0));
        if let Event::RewardSignalRecorded { agent_id, .. } = &mut with_agent.event {
            *agent_id = Some("agent-a".into());
        }
        let events = vec![
            with_agent,
            stored_signal("t1", SOURCE_EVIDENCE, "scored", Some(0.7)),
            stored_signal("t2", SOURCE_VALIDATOR, "failed", Some(0.0)),
        ];
        // No scope: everything, in ledger order.
        let all = collect_signals(&events, &RewardScope::default());
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].todo_id, "t1");
        assert_eq!(all[0].agent_id.as_deref(), Some("agent-a"));
        // Agent scope.
        let scoped = collect_signals(
            &events,
            &RewardScope {
                agent_id: Some("agent-a"),
                ..Default::default()
            },
        );
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].source, SOURCE_VALIDATOR);
        // Todo scope.
        let scoped = collect_signals(
            &events,
            &RewardScope {
                todo_id: Some("t2"),
                ..Default::default()
            },
        );
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].signal, "failed");
        // Source scope.
        let scoped = collect_signals(
            &events,
            &RewardScope {
                source: Some(SOURCE_EVIDENCE),
                ..Default::default()
            },
        );
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].score, Some(0.7));
        // Combined scope with no match.
        let scoped = collect_signals(
            &events,
            &RewardScope {
                agent_id: Some("agent-a"),
                todo_id: Some("t2"),
                ..Default::default()
            },
        );
        assert!(scoped.is_empty());
        // Non-reward events are ignored.
        let other = StoredEvent {
            event_id: String::new(),
            producer: None,
            source_ref: None,
            source_section: None,
            source_line: None,
            privacy: None,
            event: Event::GoalStarted {
                goal_id: "g".into(),
                ts: 1,
            },
        };
        assert!(collect_signals(&[other], &RewardScope::default()).is_empty());
    }

    #[test]
    fn next_seq_counts_existing_signals_for_the_todo() {
        let events = vec![
            stored_signal("t1", SOURCE_VALIDATOR, "passed", Some(1.0)),
            stored_signal("t1", SOURCE_EVIDENCE, "scored", Some(0.5)),
            stored_signal("t2", SOURCE_VALIDATOR, "failed", Some(0.0)),
        ];
        assert_eq!(next_seq(&events, "t1"), 3);
        assert_eq!(next_seq(&events, "t2"), 2);
        assert_eq!(next_seq(&events, "t3"), 1);
        assert_eq!(next_seq(&[], "t1"), 1);
    }

    #[test]
    fn summarize_aggregates_sources_scores_and_validator_outcomes() {
        let signals = vec![
            RewardSignal {
                todo_id: "t1".into(),
                agent_id: None,
                run_id: None,
                source: SOURCE_VALIDATOR.into(),
                signal: "passed".into(),
                score: Some(1.0),
                note: None,
                seq: 1,
                ts: 1,
            },
            RewardSignal {
                todo_id: "t1".into(),
                agent_id: None,
                run_id: None,
                source: SOURCE_VALIDATOR.into(),
                signal: "failed".into(),
                score: Some(0.0),
                note: None,
                seq: 2,
                ts: 2,
            },
            RewardSignal {
                todo_id: "t2".into(),
                agent_id: None,
                run_id: None,
                source: SOURCE_EVIDENCE.into(),
                signal: "scored".into(),
                score: Some(0.5),
                note: None,
                seq: 1,
                ts: 3,
            },
            RewardSignal {
                todo_id: "t3".into(),
                agent_id: None,
                run_id: None,
                source: SOURCE_VALIDATOR.into(),
                signal: "inconclusive".into(),
                score: None,
                note: None,
                seq: 1,
                ts: 4,
            },
        ];
        let s = summarize(&signals);
        assert_eq!(s.total, 4);
        assert_eq!(s.by_source.get(SOURCE_VALIDATOR), Some(&3));
        assert_eq!(s.by_source.get(SOURCE_EVIDENCE), Some(&1));
        assert_eq!(s.scored, 3);
        assert_eq!(s.avg_score, Some(0.5));
        assert_eq!(s.validator_passed, 1);
        assert_eq!(s.validator_failed, 1);
        // Empty set: no average.
        let empty = summarize(&[]);
        assert_eq!(empty.total, 0);
        assert_eq!(empty.avg_score, None);
        assert!(empty.by_source.is_empty());
    }
}
