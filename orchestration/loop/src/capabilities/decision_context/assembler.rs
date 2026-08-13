//! Decision-context assembler (P1-4) — LoopX
//! `capabilities/decision_context/assembler.py`, compact set.
//!
//! The assembler composes one [`DecisionContextPacket`] from the registered
//! providers: a goal-boundary header (status + open acceptance gap ids)
//! plus each provider's typed section, folded in registration order (a
//! later provider's section replaces an earlier one of the same kind —
//! deterministic). The packet is the record-time capture that fixes the
//! replay record→run mismatch: `replay record` stores it in the case and
//! `goal_from_case` rebuilds the decision-relevant goal state from it.

use crate::state::Goal;

use super::packets::{
    DecisionContextPacket, OutcomeStreakSection, QuotaStatusSection, RunHistorySection,
    DECISION_CONTEXT_PACKET_SCHEMA_VERSION,
};
use super::providers::{builtin_providers, DecisionContextProvider, ProviderSection};

/// Composes decision-context packets from a provider set.
pub struct DecisionContextAssembler {
    providers: Vec<Box<dyn DecisionContextProvider>>,
}

impl Default for DecisionContextAssembler {
    fn default() -> Self {
        Self::with_builtin()
    }
}

impl DecisionContextAssembler {
    pub fn new(providers: Vec<Box<dyn DecisionContextProvider>>) -> Self {
        Self { providers }
    }

    /// The builtin provider set (run history / outcome streak / quota
    /// status), in deterministic assembly order.
    pub fn with_builtin() -> Self {
        Self::new(builtin_providers())
    }

    /// Assemble the packet for `goal` stamped at `now` (epoch secs).
    pub fn assemble(&self, goal: &Goal, now: u64) -> DecisionContextPacket {
        let mut packet = DecisionContextPacket {
            schema_version: DECISION_CONTEXT_PACKET_SCHEMA_VERSION.to_string(),
            goal_id: goal.goal_id.clone(),
            goal_status: goal.status.clone(),
            assembled_at: now,
            providers: Vec::with_capacity(self.providers.len()),
            open_acceptance_gaps: goal
                .unsatisfied_gaps()
                .iter()
                .map(|g| g.id.clone())
                .collect(),
            run_history: RunHistorySection::default(),
            outcome_streak: OutcomeStreakSection::default(),
            quota: QuotaStatusSection::default(),
        };
        for provider in &self.providers {
            packet.providers.push(provider.id().to_string());
            match provider.collect(goal) {
                ProviderSection::RunHistory(section) => packet.run_history = section,
                ProviderSection::OutcomeStreak(section) => packet.outcome_streak = section,
                ProviderSection::QuotaStatus(section) => packet.quota = section,
            }
        }
        packet
    }
}

/// One-shot convenience: assemble with the builtin provider set, stamped at
/// the current time.
pub fn assemble_decision_context(goal: &Goal) -> DecisionContextPacket {
    DecisionContextAssembler::with_builtin().assemble(goal, crate::state::now_epoch())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Todo;

    #[test]
    fn assemble_captures_goal_boundary_and_all_sections() {
        let mut g = Goal::new("g1", "objective", "/tmp").with_acceptance(vec![("A1", "match")]);
        g.add(Todo::advancement("T1", "work"));
        g.execution_profile.outcome_floor_streak_threshold = 2;
        g.outcome_streak = 2;
        let packet = DecisionContextAssembler::with_builtin().assemble(&g, 42);
        assert_eq!(
            packet.schema_version,
            DECISION_CONTEXT_PACKET_SCHEMA_VERSION
        );
        assert_eq!(packet.goal_id, "g1");
        assert_eq!(packet.goal_status, "active");
        assert_eq!(packet.assembled_at, 42);
        assert_eq!(
            packet.providers,
            vec![
                "run_history".to_string(),
                "outcome_streak".to_string(),
                "quota_status".to_string()
            ]
        );
        assert_eq!(packet.open_acceptance_gaps, vec!["A1".to_string()]);
        assert!(packet.outcome_streak.floor_breached);
        assert_eq!(packet.run_history.run_count, 0);
        assert_eq!(
            packet.quota.allowed_slots,
            crate::quota::slot_accounting::QUOTA_ALLOWED_SLOTS
        );
    }

    #[test]
    fn cancelled_goal_status_is_captured() {
        let mut g = Goal::new("g1", "objective", "/tmp");
        g.status = "cancelled".to_string();
        let packet = assemble_decision_context(&g);
        assert_eq!(packet.goal_status, "cancelled");
    }

    struct DuplicateRunHistory;
    impl DecisionContextProvider for DuplicateRunHistory {
        fn id(&self) -> &'static str {
            "run_history"
        }
        fn describe(&self) -> &'static str {
            "test double"
        }
        fn collect(&self, _goal: &Goal) -> ProviderSection {
            ProviderSection::RunHistory(RunHistorySection {
                run_count: 99,
                ..RunHistorySection::default()
            })
        }
    }

    #[test]
    fn later_section_replaces_earlier_of_same_kind() {
        let g = Goal::new("g1", "objective", "/tmp");
        let assembler = DecisionContextAssembler::new(vec![
            Box::new(super::super::providers::RunHistoryProvider),
            Box::new(DuplicateRunHistory),
        ]);
        let packet = assembler.assemble(&g, 1);
        assert_eq!(packet.run_history.run_count, 99, "last writer wins");
        assert_eq!(
            packet.providers,
            vec!["run_history".to_string(), "run_history".to_string()]
        );
    }

    #[test]
    fn default_assembler_uses_builtin_providers_and_describe() {
        let g = Goal::new("g1", "objective", "/tmp");
        let packet = DecisionContextAssembler::default().assemble(&g, 1);
        assert_eq!(
            packet.providers,
            vec![
                "run_history".to_string(),
                "outcome_streak".to_string(),
                "quota_status".to_string()
            ]
        );
        // The duplicate-provider test double exposes its description.
        assert_eq!(DuplicateRunHistory.describe(), "test double");
    }

    #[test]
    fn empty_provider_set_yields_header_only_packet() {
        let g = Goal::new("g1", "objective", "/tmp");
        let packet = DecisionContextAssembler::new(vec![]).assemble(&g, 7);
        assert!(packet.providers.is_empty());
        assert_eq!(packet.run_history, RunHistorySection::default());
        assert_eq!(packet.outcome_streak, OutcomeStreakSection::default());
        assert_eq!(packet.quota, QuotaStatusSection::default());
    }
}
