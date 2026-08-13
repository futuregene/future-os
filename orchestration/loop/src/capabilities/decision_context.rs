//! decision_context capability (LoopX: decision_context — deterministic rule translation
//! into finite typed proposals).
//!
//! P1-4 deep implementation (LoopX `capabilities/decision_context/`,
//! compact set): the decision-context assembler + providers (run history /
//! outcome streak / quota status) + outcome-feedback writeback live in the
//! submodules. The rule proposer below stays the G-24 capability surface.
//!
//! The assembled packet is also the replay record-time capture: it carries
//! the goal-level decision state (outcome streak, goal status, open
//! acceptance gaps) that the compact todo snapshot cannot, fixing the
//! replay record→run mismatch from incomplete context.

pub mod assembler;
pub mod outcome_feedback;
pub mod packets;
pub mod providers;

use super::{successor_todo, Capability, TypedProposal};

pub struct DecisionContextCapability;

impl Capability for DecisionContextCapability {
    fn name(&self) -> &'static str {
        "decision_context"
    }
    fn describe(&self) -> &'static str {
        "collect context for a pending decision"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup(
                "empty input for decision_context",
            )];
        }
        let l = text.to_lowercase();
        if l.contains("decide") {
            return vec![TypedProposal::successor(successor_todo("decisioncontext", "Collect decision context: options, criteria, evidence refs, and open questions."), "rule: marker `decide`")];
        }
        if l.contains("决策") {
            return vec![TypedProposal::successor(successor_todo("decisioncontext", "Collect decision context: options, criteria, evidence refs, and open questions."), "rule: marker `决策`")];
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
