//! The error codes a session-scoped command can fail with, and the wording
//! agents used before the codes existed.
//!
//! This is semantics the Agent and every client must agree on beyond the wire
//! shape: a command naming a session the Agent has no record of is answered
//! *definitively* — the session never existed (a prompt that never reached the
//! Agent) or was deleted. Clients must treat that as "there is nothing there",
//! never as a transient failure to retry: a stale local row pointing at such a
//! session has to be settled or skipped, or it strands the conversation in a
//! state that cannot be resolved by waiting.

/// `RpcResponse.error_code` for a session-scoped command naming a session the
/// Agent has no record of.
///
/// Additive: agents that predate the code report the same condition in `error`
/// only, so clients match the code first and fall back to
/// [`legacy_session_not_found_message`].
pub const SESSION_NOT_FOUND_CODE: &str = "session_not_found";

/// Whether an agent-reported error message is the pre-code wording for
/// [`SESSION_NOT_FOUND_CODE`]. Case-insensitive, because agents and clients
/// have each capitalized it differently.
pub fn legacy_session_not_found_message(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("session not found") || error.contains("no such session")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_legacy_matcher_is_case_insensitive_and_specific() {
        assert!(legacy_session_not_found_message(
            "session not found — pass a valid session_id (new_session creates one)"
        ));
        assert!(legacy_session_not_found_message("No such session"));
        // A failure that must keep its retry meaning is not a missing session.
        assert!(!legacy_session_not_found_message(
            "session_storage_unavailable"
        ));
        assert!(!legacy_session_not_found_message("session_busy"));
    }
}
