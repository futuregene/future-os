use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunAcceptedState {
    Existing,
    Running,
    Queued,
}

/// Canonical acknowledgement for every accepted prompt request.
///
/// `run_sequence` and `queue_position` remain absent until the session
/// scheduler is the allocator; callers must not invent either value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunAck {
    pub run_id: String,
    pub run_epoch: u64,
    pub accepted_state: RunAcceptedState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_sequence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_position: Option<u64>,
}

impl RunAck {
    pub fn existing(run_id: String, run_epoch: u64) -> Self {
        Self {
            run_id,
            run_epoch,
            accepted_state: RunAcceptedState::Existing,
            run_sequence: None,
            queue_position: None,
        }
    }

    pub fn running(run_id: String, run_epoch: u64) -> Self {
        Self {
            run_id,
            run_epoch,
            accepted_state: RunAcceptedState::Running,
            run_sequence: None,
            queue_position: None,
        }
    }

    pub fn queued(run_id: String, run_sequence: u64, queue_position: u64) -> Self {
        Self {
            run_id,
            run_epoch: 0,
            accepted_state: RunAcceptedState::Queued,
            run_sequence: Some(run_sequence),
            queue_position: Some(queue_position),
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
        let error = BusyPolicy::parse("steer").unwrap_err();
        assert!(error.contains("unknown busy policy `steer`"));
        for value in BusyPolicy::VALID_VALUES {
            assert!(error.contains(value));
        }
    }

    #[test]
    fn run_ack_omits_unallocated_queue_identity() {
        let value = serde_json::to_value(RunAck::running("run-a".into(), 7)).unwrap();
        assert_eq!(value["run_id"], "run-a");
        assert_eq!(value["run_epoch"], 7);
        assert_eq!(value["accepted_state"], "running");
        assert!(value.get("run_sequence").is_none());
        assert!(value.get("queue_position").is_none());
    }
}
