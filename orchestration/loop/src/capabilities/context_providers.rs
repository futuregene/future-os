//! context_providers capability (LoopX: context_providers — deterministic rule translation
//! into finite typed proposals).

use super::{successor_todo, Capability, TypedProposal};

pub struct ContextProvidersCapability;

impl Capability for ContextProvidersCapability {
    fn name(&self) -> &'static str {
        "context_providers"
    }
    fn describe(&self) -> &'static str {
        "resolve a context request to a provider"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup(
                "empty input for context_providers",
            )];
        }
        vec![TypedProposal::successor(successor_todo("contextproviders", "Resolve the context request to an available provider and return a bounded observation."), "rule: non-empty input")]
    }
}
