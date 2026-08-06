//! content_ops capability (LoopX: content_ops — deterministic rule translation
//! into finite typed proposals).

use super::{successor_todo, Capability, TypedProposal};

pub struct ContentOpsCapability;

impl Capability for ContentOpsCapability {
    fn name(&self) -> &'static str {
        "content_ops"
    }
    fn describe(&self) -> &'static str {
        "turn source material into draft angles"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup("empty input for content_ops")];
        }
        vec![TypedProposal::successor(successor_todo("contentops", "Draft a concrete content angle from the material; keep source refs, ask for taste before publishing."), "rule: non-empty input")]
    }
}
