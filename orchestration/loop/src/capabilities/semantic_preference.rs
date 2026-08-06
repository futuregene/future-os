//! semantic_preference capability (LoopX: semantic_preference — deterministic rule translation
//! into finite typed proposals).

use super::{successor_todo, Capability, TypedProposal};

pub struct SemanticPreferenceCapability;

impl Capability for SemanticPreferenceCapability {
    fn name(&self) -> &'static str {
        "semantic_preference"
    }
    fn describe(&self) -> &'static str {
        "capture a soft preference as a gated candidate"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup(
                "empty input for semantic_preference",
            )];
        }
        let l = text.to_lowercase();
        if l.contains("prefer") {
            return vec![TypedProposal::gate("Confirm this preference candidate (source, scope, freshness) before it influences output.", "rule: explicit confirmation required")];
        }
        if l.contains("偏好") {
            return vec![TypedProposal::gate("Confirm this preference candidate (source, scope, freshness) before it influences output.", "rule: explicit confirmation required")];
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
