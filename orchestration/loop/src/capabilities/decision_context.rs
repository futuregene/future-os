//! decision_context capability (LoopX: decision_context — deterministic rule translation
//! into finite typed proposals).

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
