//! Change-Quality capability (LoopX: change-quality — validation of a change
//! before it is accepted; quality signals become repair/no-follow-up
//! proposals, not approvals).

use super::{successor_todo, Capability, TypedProposal};

pub struct ChangeQualityCapability;

impl Capability for ChangeQualityCapability {
    fn name(&self) -> &'static str {
        "change_quality"
    }
    fn describe(&self) -> &'static str {
        "assess a change's validation evidence and propose repair or acceptance"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup("no change evidence provided")];
        }
        let l = text.to_lowercase();
        let tests_pass = l.contains("all pass") || l.contains("tests pass") || l.contains("exit 0");
        let has_artifact = l.contains("diff") || l.contains("patch") || l.contains("written");
        if tests_pass && has_artifact {
            vec![TypedProposal::successor(
                successor_todo(
                    "quality",
                    "Package the validated change for the review/merge surface with the evidence packet.",
                ),
                "change validated (tests pass + artifact present)",
            )]
        } else if !tests_pass {
            vec![TypedProposal::successor(
                successor_todo(
                    "quality",
                    "Repair: run the validation again and fix the failing assertion before reporting success.",
                ),
                "validation evidence is missing or failing",
            )]
        } else {
            vec![TypedProposal::successor(
                successor_todo(
                    "quality",
                    "Record concrete change evidence (paths, diffs, test output) before acceptance.",
                ),
                "artifact evidence is thin",
            )]
        }
    }
}
