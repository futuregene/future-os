//! integration_branch capability (LoopX: integration_branch — deterministic rule translation
//! into finite typed proposals).

use super::{successor_todo, Capability, TypedProposal};

pub struct IntegrationBranchCapability;

impl Capability for IntegrationBranchCapability {
    fn name(&self) -> &'static str {
        "integration_branch"
    }
    fn describe(&self) -> &'static str {
        "propose an integration branch for a change set"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup(
                "empty input for integration_branch",
            )];
        }
        let l = text.to_lowercase();
        if l.contains("branch") {
            return vec![TypedProposal::successor(successor_todo("integrationbranch", "Create/refresh the integration branch for the change set and keep it merge-ready."), "rule: marker `branch`")];
        }
        if l.contains("集成") {
            return vec![TypedProposal::successor(
                successor_todo(
                    "integrationbranch",
                    "Create/refresh the integration branch and keep it merge-ready.",
                ),
                "rule: marker `集成`",
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
