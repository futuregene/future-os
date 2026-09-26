use serde::{Deserialize, Serialize};

// The prompt acknowledgement types live in the future-rpc crate (typed-RPC
// milestone) so the wire encode/decode and the agent share one definition.
pub use future_rpc::payloads_ext::{RunAcceptedState, RunAck};

/// Atomic behavior requested when a session already owns an active run.
///
/// The default (`EnqueueIfBusy`) appends the new prompt behind any in-progress
/// run — follow-up semantics. `SupersedeSession` interrupts the active run and
/// runs the new prompt next. `EnqueueCoalescing` is the opt-in follow-up: the
/// request joins the queue like `EnqueueIfBusy`, but the run boundary may fold
/// a consecutive run of coalescing requests into ONE run (see
/// `InMemoryRunQueue::drain_coalescing_after_first`), so a burst of supplements
/// is answered together instead of one by one.
///
/// Coalescing is opt-in because a folded request never runs: its id only ever
/// reaches the client as a terminal `merged` acknowledgement. A caller that
/// streams its own run (CLI `--follow-up`, the IM bridges) must therefore keep
/// the plain `EnqueueIfBusy` semantics; only callers that treat the submission
/// as fire-and-forget content (a chat composer, the loop supervisor outbox)
/// opt in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusyPolicy {
    #[default]
    EnqueueIfBusy,
    EnqueueCoalescing,
    SupersedeSession,
}

impl BusyPolicy {
    pub const VALID_VALUES: [&'static str; 3] =
        ["enqueue_if_busy", "enqueue_coalescing", "supersede_session"];

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "" | "enqueue_if_busy" => Ok(Self::EnqueueIfBusy),
            "enqueue_coalescing" => Ok(Self::EnqueueCoalescing),
            "supersede_session" => Ok(Self::SupersedeSession),
            other => Err(format!(
                "unknown busy policy `{other}`; expected one of: {}",
                Self::VALID_VALUES.join(", ")
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::EnqueueIfBusy => "enqueue_if_busy",
            Self::EnqueueCoalescing => "enqueue_coalescing",
            Self::SupersedeSession => "supersede_session",
        }
    }

    /// Whether a queued request with this policy may absorb the requests
    /// queued behind it (and be absorbed by the request ahead of it).
    pub fn coalesces(self) -> bool {
        matches!(self, Self::EnqueueCoalescing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_busy_policy_defaults_to_enqueue() {
        assert_eq!(BusyPolicy::parse("").unwrap(), BusyPolicy::EnqueueIfBusy);
        assert_eq!(
            BusyPolicy::parse("enqueue_if_busy").unwrap(),
            BusyPolicy::EnqueueIfBusy
        );
    }

    #[test]
    fn parses_supersede_session() {
        assert_eq!(
            BusyPolicy::parse("supersede_session").unwrap(),
            BusyPolicy::SupersedeSession
        );
    }

    #[test]
    fn parses_enqueue_coalescing_and_scopes_it_to_opt_in_callers() {
        assert_eq!(
            BusyPolicy::parse("enqueue_coalescing").unwrap(),
            BusyPolicy::EnqueueCoalescing
        );
        assert!(BusyPolicy::EnqueueCoalescing.coalesces());
        assert!(!BusyPolicy::EnqueueIfBusy.coalesces());
        assert!(!BusyPolicy::SupersedeSession.coalesces());
        assert_eq!(BusyPolicy::EnqueueCoalescing.as_str(), "enqueue_coalescing");
        // Surrounding whitespace is tolerated like the other values.
        assert_eq!(
            BusyPolicy::parse(" enqueue_coalescing ").unwrap(),
            BusyPolicy::EnqueueCoalescing
        );
    }

    #[test]
    fn rejects_unknown_busy_policy_without_guessing() {
        let error = BusyPolicy::parse("frobnicate").unwrap_err();
        assert!(error.contains("unknown busy policy `frobnicate`"));
        for value in BusyPolicy::VALID_VALUES {
            assert!(error.contains(value));
        }
        // The removed reject_if_busy value is no longer accepted.
        assert!(BusyPolicy::parse("reject_if_busy").is_err());
    }
}
