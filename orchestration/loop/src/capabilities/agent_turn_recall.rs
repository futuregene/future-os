//! agent_turn_recall capability (LoopX: agent_turn_recall — deterministic rule translation
//! into finite typed proposals).

use super::{successor_todo, Capability, TypedProposal};

pub struct AgentTurnRecallCapability;

impl Capability for AgentTurnRecallCapability {
    fn name(&self) -> &'static str {
        "agent_turn_recall"
    }
    fn describe(&self) -> &'static str {
        "recall and refine key evidence from past agent turns"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup(
                "empty input for agent_turn_recall",
            )];
        }
        vec![TypedProposal::successor(successor_todo("agentturnrecall", "Recall and refine the key evidence from the recorded turn; keep only validated facts, drop stale reasoning."), "rule: non-empty input")]
    }
}
