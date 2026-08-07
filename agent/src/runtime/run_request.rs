use serde::{Deserialize, Serialize};

// The prompt acknowledgement types live in the future-rpc crate (typed-RPC
// milestone) so the wire encode/decode and the agent share one definition.
pub use future_rpc::payloads_ext::{RunAcceptedState, RunAck};

/// Atomic behavior requested when a session already owns an active run.
///
/// Only `RejectIfBusy` is executable until the in-memory session scheduler is wired;
/// the other variants are parsed now so the wire contract can stabilize first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusyPolicy {
    #[default]
    RejectIfBusy,
    EnqueueIfBusy,
    SupersedeSession,
}

impl BusyPolicy {
    pub const VALID_VALUES: [&'static str; 3] =
        ["reject_if_busy", "enqueue_if_busy", "supersede_session"];

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "" | "reject_if_busy" => Ok(Self::RejectIfBusy),
            "enqueue_if_busy" => Ok(Self::EnqueueIfBusy),
            "supersede_session" => Ok(Self::SupersedeSession),
            other => Err(format!(
                "unknown busy policy `{other}`; expected one of: {}",
                Self::VALID_VALUES.join(", ")
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::RejectIfBusy => "reject_if_busy",
            Self::EnqueueIfBusy => "enqueue_if_busy",
            Self::SupersedeSession => "supersede_session",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_busy_policy_is_backward_compatible_reject() {
        assert_eq!(BusyPolicy::parse("").unwrap(), BusyPolicy::RejectIfBusy);
        assert_eq!(
            BusyPolicy::parse("reject_if_busy").unwrap(),
            BusyPolicy::RejectIfBusy
        );
    }

    #[test]
    fn parses_all_frozen_busy_policy_values() {
        assert_eq!(
            BusyPolicy::parse("enqueue_if_busy").unwrap(),
            BusyPolicy::EnqueueIfBusy
        );
        assert_eq!(
            BusyPolicy::parse("supersede_session").unwrap(),
            BusyPolicy::SupersedeSession
        );
    }

    #[test]
    fn rejects_unknown_busy_policy_without_guessing() {
        let error = BusyPolicy::parse("frobnicate").unwrap_err();
        assert!(error.contains("unknown busy policy `frobnicate`"));
        for value in BusyPolicy::VALID_VALUES {
            assert!(error.contains(value));
        }
    }
}
