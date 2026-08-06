//! reward_memory capability (LoopX: reward_memory — deterministic rule translation
//! into finite typed proposals).

use super::{successor_todo, Capability, TypedProposal};

pub struct RewardMemoryCapability;

impl Capability for RewardMemoryCapability {
    fn name(&self) -> &'static str {
        "reward_memory"
    }
    fn describe(&self) -> &'static str {
        "capture a run reward as a memory candidate (gated)"
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
