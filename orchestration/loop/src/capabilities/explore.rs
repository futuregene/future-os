//! explore capability (LoopX: explore — deterministic rule translation
//! into finite typed proposals).

use super::{successor_todo, Capability, TypedProposal};

pub struct ExploreCapability;

impl Capability for ExploreCapability {
    fn name(&self) -> &'static str {
        "explore"
    }
    fn describe(&self) -> &'static str {
        "design an exploration experiment from a hypothesis"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup("empty input for explore")];
        }
        let l = text.to_lowercase();
        if l.contains("hypothesis") {
            return vec![TypedProposal::successor(successor_todo("explore", "Design a cheap exploration experiment to test the hypothesis; record the evidence gate."), "rule: marker `hypothesis`")];
        }
        if l.contains("假设") {
            return vec![TypedProposal::successor(successor_todo("explore", "Design a cheap exploration experiment to test the hypothesis; record the evidence gate."), "rule: marker `假设`")];
        }
        if l.contains("探索") {
            return vec![TypedProposal::successor(
                successor_todo(
                    "explore",
                    "Design a cheap exploration experiment; record the evidence gate.",
                ),
                "rule: marker `探索`",
            )];
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
