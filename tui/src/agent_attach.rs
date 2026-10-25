//! TUI Agent policy: attach to a serving Agent, never start or restart one.
//!
//! The Agent's lifetime belongs to its host — `future host`, the desktop app, or
//! an explicit `future agent`. The TUI is a client: it probes for a compatible
//! Agent and, when none answers, names what to start instead of starting it
//! itself.
//!
//! That is a deliberate reversal of the earlier in-process hosting. Starting the
//! Agent here made every terminal a candidate owner of the instance lock, and a
//! supervisor loop in each TUI meant several terminals raced to bring an Agent
//! back after one crash. Keeping the launcher out of the TUI leaves one owner
//! per data root, and one place to look when it is not serving.

use std::time::Duration;

use future_rpc::supervision::{probe_local_agent, LocalAgentState};

/// Per-attempt probe budget. Local IPC answers in milliseconds when it answers.
const PROBE_CONNECT: Duration = Duration::from_millis(400);
const PROBE_HANDSHAKE: Duration = Duration::from_secs(2);

/// What to start, and the boundary the user is relying on.
const ATTACH_ONLY_HINT: &str = "Start Future Host (`future host`), the desktop app, or \
     `future agent`, then start the TUI again. The TUI only attaches to an Agent and never \
     starts or restarts one.";

/// Attach to the Agent serving this data root, or say what to start.
///
/// Every refusal names the state it found and the command that fixes it: the
/// user cannot start the right process from "connection failed".
pub async fn ensure_agent_attached(configured: &str) -> Result<(), String> {
    match probe_local_agent(configured, PROBE_CONNECT, PROBE_HANDSHAKE).await {
        LocalAgentState::Attached => Ok(()),
        other => Err(failure_message(&other)),
    }
}

/// The user-facing sentence for a probe that is not `Attached`.
fn failure_message(state: &LocalAgentState) -> String {
    let situation = match state {
        LocalAgentState::Attached => {
            unreachable!("attaching succeeded; there is no failure to describe")
        }
        LocalAgentState::Startable => "no Agent is serving this data root".to_string(),
        LocalAgentState::Busy(detail) => format!(
            "another process holds this data root's Agent lock but is not serving yet: {detail}"
        ),
        LocalAgentState::Unresponsive(detail) => {
            format!("the Agent endpoint exists but did not answer: {detail}")
        }
        LocalAgentState::Blocked(reason) => reason.clone(),
    };
    format!("{situation}.\n{ATTACH_ONLY_HINT}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of the change: no failure may be answered by the TUI starting
    /// something, and every one of them has to name what the user should start.
    #[test]
    fn every_failure_names_the_host_and_never_promises_a_start() {
        for state in [
            LocalAgentState::Startable,
            LocalAgentState::Busy("held by pid 42".into()),
            LocalAgentState::Unresponsive("handshake timed out".into()),
            LocalAgentState::Blocked("Incompatible Future Agent: protocol 2".into()),
        ] {
            let message = failure_message(&state);
            assert!(message.contains("future host"), "{state:?}: {message}");
            assert!(message.contains("desktop app"), "{state:?}: {message}");
            assert!(
                message.contains("never starts or restarts"),
                "{state:?}: {message}"
            );
        }
    }

    /// The reason the probe found is kept, or the user is left with a generic
    /// "connection failed" that hides a version mismatch or a held lock.
    #[test]
    fn the_probe_reason_survives_the_hint() {
        let message = failure_message(&LocalAgentState::Blocked("protocol 2 refused".into()));
        assert!(message.starts_with("protocol 2 refused."), "{message}");
        let message = failure_message(&LocalAgentState::Busy("held by pid 42".into()));
        assert!(message.contains("held by pid 42"), "{message}");
    }
}
