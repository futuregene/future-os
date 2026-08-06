//! material_lifecycle capability (LoopX: material_lifecycle — deterministic rule translation
//! into finite typed proposals).

use super::{successor_todo, Capability, TypedProposal};

pub struct MaterialLifecycleCapability;

impl Capability for MaterialLifecycleCapability {
    fn name(&self) -> &'static str {
        "material_lifecycle"
    }
    fn describe(&self) -> &'static str {
        "route material through its lifecycle"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup(
                "empty input for material_lifecycle",
            )];
        }
        let l = text.to_lowercase();
        if l.contains("archive") {
            return vec![TypedProposal::successor(
                successor_todo(
                    "materiallifecycle",
                    "Archive the material with a stable key and a compact fingerprint.",
                ),
                "rule: marker `archive`",
            )];
        }
        if l.contains("归档") {
            return vec![TypedProposal::successor(
                successor_todo(
                    "materiallifecycle",
                    "Archive the material with a stable key and a compact fingerprint.",
                ),
                "rule: marker `归档`",
            )];
        }
        if l.contains("废弃") {
            return vec![TypedProposal::successor(
                successor_todo(
                    "materiallifecycle",
                    "Mark the material retired with the reason; record no-follow-up intent.",
                ),
                "rule: marker `废弃`",
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
