//! value_connectors capability (LoopX: value_connectors — deterministic rule translation
//! into finite typed proposals).

use super::{successor_todo, Capability, TypedProposal};

pub struct ValueConnectorsCapability;

impl Capability for ValueConnectorsCapability {
    fn name(&self) -> &'static str {
        "value_connectors"
    }
    fn describe(&self) -> &'static str {
        "plan a value connector"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup(
                "empty input for value_connectors",
            )];
        }
        let l = text.to_lowercase();
        if l.contains("connector") {
            return vec![TypedProposal::successor(successor_todo("valueconnectors", "Plan the value connector: source surface, target surface, mapping, and validation."), "rule: marker `connector`")];
        }
        if l.contains("连接") {
            return vec![TypedProposal::successor(successor_todo("valueconnectors", "Plan the value connector: source surface, target surface, mapping, and validation."), "rule: marker `连接`")];
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
