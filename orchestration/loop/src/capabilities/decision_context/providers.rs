//! Replaceable current-authority providers for decision_context (P1-4) —
//! LoopX `capabilities/decision_context/providers.py`, compact set.
//!
//! A provider reads one slice of current-authority state (the replayed
//! goal) and returns its typed section. Pure: no I/O, no clock (the
//! assembler stamps `assembled_at`). The builtin set is the report's P1-4
//! scope — run history / outcome streak / quota status — in a fixed
//! assembly order so packets are deterministic.

use crate::state::Goal;

use super::packets::{OutcomeStreakSection, QuotaStatusSection, RunHistorySection};

/// One provider's contribution to the packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderSection {
    RunHistory(RunHistorySection),
    OutcomeStreak(OutcomeStreakSection),
    QuotaStatus(QuotaStatusSection),
}

impl ProviderSection {
    pub fn provider_id(&self) -> &'static str {
        match self {
            ProviderSection::RunHistory(_) => RunHistoryProvider.id(),
            ProviderSection::OutcomeStreak(_) => OutcomeStreakProvider.id(),
            ProviderSection::QuotaStatus(_) => QuotaStatusProvider.id(),
        }
    }
}

/// A decision-context provider (LoopX `DecisionSourceProvider`, compact):
/// stable id + a pure collect over the goal.
pub trait DecisionContextProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn describe(&self) -> &'static str;
    fn collect(&self, goal: &Goal) -> ProviderSection;
}

/// Provider `run_history`: how the goal's turns have been landing.
pub struct RunHistoryProvider;

impl DecisionContextProvider for RunHistoryProvider {
    fn id(&self) -> &'static str {
        "run_history"
    }
    fn describe(&self) -> &'static str {
        "run count, material-outcome count and recent terminal states from the run history"
    }
    fn collect(&self, goal: &Goal) -> ProviderSection {
        ProviderSection::RunHistory(RunHistorySection {
            run_count: goal.history.len() as u64,
            // Materiality rule shared with the executor writeback: a turn is
            // material when it produced a validated artifact (tools +
            // evidence); surface-only turns accumulate the outcome streak.
            material_runs: goal
                .history
                .iter()
                .filter(|r| !r.tools.is_empty() && !r.evidence.trim().is_empty())
                .count() as u64,
            recent_terminal_states: goal
                .history
                .iter()
                .rev()
                .take(3)
                .map(|r| r.terminal_state.clone())
                .collect(),
            last_run_at: goal.history.last().map(|r| r.recorded_at),
        })
    }
}

/// Provider `outcome_streak`: the surface-only progress loop counter and
/// its configured floor (the kernel replans when the floor is breached).
pub struct OutcomeStreakProvider;

impl DecisionContextProvider for OutcomeStreakProvider {
    fn id(&self) -> &'static str {
        "outcome_streak"
    }
    fn describe(&self) -> &'static str {
        "surface-only turn streak vs the configured outcome floor"
    }
    fn collect(&self, goal: &Goal) -> ProviderSection {
        let threshold = goal.execution_profile.outcome_floor_streak_threshold;
        ProviderSection::OutcomeStreak(OutcomeStreakSection {
            surface_streak: goal.outcome_streak,
            threshold,
            floor_breached: threshold > 0 && goal.outcome_streak >= threshold,
        })
    }
}

/// Provider `quota_status`: the slot budget view (history counter vs the
/// G-3 `QuotaSpent` projection — divergence is read-model drift).
pub struct QuotaStatusProvider;

impl DecisionContextProvider for QuotaStatusProvider {
    fn id(&self) -> &'static str {
        "quota_status"
    }
    fn describe(&self) -> &'static str {
        "allowed/spent quota slots (history counter + QuotaSpent projection)"
    }
    fn collect(&self, goal: &Goal) -> ProviderSection {
        ProviderSection::QuotaStatus(QuotaStatusSection {
            allowed_slots: crate::quota::slot_accounting::QUOTA_ALLOWED_SLOTS,
            spent_slots: goal.history.len() as u64,
            projected_spent_slots: goal.quota_spent_slots,
        })
    }
}

/// The builtin provider set in deterministic assembly order (the report's
/// P1-4 compact set: run history / outcome streak / quota status).
pub fn builtin_providers() -> Vec<Box<dyn DecisionContextProvider>> {
    vec![
        Box::new(RunHistoryProvider),
        Box::new(OutcomeStreakProvider),
        Box::new(QuotaStatusProvider),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RunRecord, Todo};

    fn run_record(terminal_state: &str, tools: Vec<String>, evidence: &str, at: u64) -> RunRecord {
        RunRecord {
            turn: 1,
            todo_id: "T1".to_string(),
            run_id: "r1".to_string(),
            validation: None,
            terminal_state: terminal_state.to_string(),
            error: None,
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools,
            evidence: evidence.to_string(),
            recorded_at: at,
            spend_source: None,
        }
    }

    // Extraction helpers: `collect` returns the exact variant each test
    // expects, but the mismatch arm is a real invariant worth pinning.
    fn expect_run_history(section: ProviderSection) -> RunHistorySection {
        match section {
            ProviderSection::RunHistory(s) => s,
            other => panic!("expected run history section, got {}", other.provider_id()),
        }
    }

    fn expect_outcome_streak(section: ProviderSection) -> OutcomeStreakSection {
        match section {
            ProviderSection::OutcomeStreak(s) => s,
            other => panic!(
                "expected outcome streak section, got {}",
                other.provider_id()
            ),
        }
    }

    fn expect_quota_status(section: ProviderSection) -> QuotaStatusSection {
        match section {
            ProviderSection::QuotaStatus(s) => s,
            other => panic!("expected quota status section, got {}", other.provider_id()),
        }
    }

    #[test]
    #[should_panic(expected = "expected run history section")]
    fn run_history_extraction_rejects_wrong_variant() {
        expect_run_history(ProviderSection::QuotaStatus(QuotaStatusSection::default()));
    }

    #[test]
    #[should_panic(expected = "expected outcome streak section")]
    fn outcome_streak_extraction_rejects_wrong_variant() {
        expect_outcome_streak(ProviderSection::RunHistory(RunHistorySection::default()));
    }

    #[test]
    #[should_panic(expected = "expected quota status section")]
    fn quota_status_extraction_rejects_wrong_variant() {
        expect_quota_status(ProviderSection::OutcomeStreak(
            OutcomeStreakSection::default(),
        ));
    }

    #[test]
    fn run_history_provider_counts_and_materiality() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.add(Todo::advancement("T1", "work"));
        g.history.push(run_record(
            "completed",
            vec!["shell".to_string()],
            "artifact",
            10,
        ));
        g.history.push(run_record("continue", vec![], "", 20));
        g.history
            .push(run_record("failed", vec!["shell".to_string()], "", 30));
        let section = RunHistoryProvider.collect(&g);
        let s = expect_run_history(section);
        assert_eq!(s.run_count, 3);
        assert_eq!(s.material_runs, 1, "only tools+evidence turns are material");
        assert_eq!(
            s.recent_terminal_states,
            vec![
                "failed".to_string(),
                "continue".to_string(),
                "completed".to_string()
            ]
        );
        assert_eq!(s.last_run_at, Some(30));
        assert_eq!(ProviderSection::RunHistory(s).provider_id(), "run_history");
    }

    #[test]
    fn outcome_streak_provider_marks_floor_breach() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.execution_profile.outcome_floor_streak_threshold = 3;
        g.outcome_streak = 2;
        let s = expect_outcome_streak(OutcomeStreakProvider.collect(&g));
        assert!(!s.floor_breached);
        assert_eq!(s.surface_streak, 2);
        assert_eq!(s.threshold, 3);
        g.outcome_streak = 3;
        let s = expect_outcome_streak(OutcomeStreakProvider.collect(&g));
        assert!(s.floor_breached);
        // disabled floor never breaches
        g.execution_profile.outcome_floor_streak_threshold = 0;
        let s = expect_outcome_streak(OutcomeStreakProvider.collect(&g));
        assert!(!s.floor_breached);
        assert_eq!(
            ProviderSection::OutcomeStreak(s).provider_id(),
            "outcome_streak"
        );
    }

    #[test]
    fn quota_status_provider_reads_both_counters() {
        let mut g = Goal::new("g", "o", "/tmp");
        g.history.push(run_record("completed", vec![], "e", 10));
        g.quota_spent_slots = 2;
        let section = QuotaStatusProvider.collect(&g);
        assert_eq!(section.provider_id(), "quota_status");
        let s = expect_quota_status(section);
        assert_eq!(
            s.allowed_slots,
            crate::quota::slot_accounting::QUOTA_ALLOWED_SLOTS
        );
        assert_eq!(s.spent_slots, 1);
        assert_eq!(s.projected_spent_slots, 2);
    }

    #[test]
    fn builtin_order_is_deterministic() {
        let ids: Vec<&str> = builtin_providers().iter().map(|p| p.id()).collect();
        assert_eq!(ids, vec!["run_history", "outcome_streak", "quota_status"]);
        assert!(!builtin_providers().iter().any(|p| p.describe().is_empty()));
    }
}
