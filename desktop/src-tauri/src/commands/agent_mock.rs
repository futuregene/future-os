//! Persistent-script facade over the canonical process-wide mock agent.
//!
//! There is exactly ONE mock agent gRPC server per test process —
//! [`crate::remote::test_support::ensure_mock_agent`]. Remote tests script it
//! with one-shot responses; the `commands`/`store` tests script persistent
//! per-command answers through this facade ([`MockScript`]). Precedence inside
//! the shared server: one-shot scripts, then this facade's persistent maps,
//! then the built-in default answers.
//!
//! - [`ensure_mock_agent`] starts the shared server (idempotent) and points
//!   `FUTURE_AGENT_GRPC_ADDR` at it on every call, so the latched agent
//!   channel always targets the mock.
//! - [`script_mock_agent`] installs a new persistent script (replacing the
//!   previous one); callers must hold [`mock_agent_lock`], which serializes
//!   script mutations across the commands tests.

use std::collections::HashMap;

/// The cross-family serialization lock lives with the canonical mock in
/// [`crate::remote::test_support`] so remote one-shot-script tests and the
/// `commands`/`store` persistent-script tests share a single serialization
/// point. Re-exported here for the existing `use ...::agent_mock::mock_agent_lock`
/// call sites.
pub(crate) use crate::remote::test_support::mock_agent_lock;

/// What the mock answers. `down` answers every command *except* the
/// connect-time health check with `Code::Unavailable` — indistinguishable from
/// a dead agent for callers (`AppError::AgentUnavailable`), while still
/// letting `connect_agent` latch its channel, so test outcomes don't depend on
/// execution order.
#[derive(Clone, Default)]
pub(crate) struct MockScript {
    /// Every command but the health check answers `Unavailable`.
    pub down: bool,
    /// `list_session_ids` returns `success = false` (enumeration failed).
    pub fail_list_session_ids: bool,
    /// The ids a successful `list_session_ids` returns.
    pub session_ids: Vec<String>,
    /// The sessionIds a successful `list_streaming_sessions` returns.
    pub streaming_ids: Vec<String>,
    /// Per-command canned JSON `data` payloads (success = true).
    pub data: HashMap<String, String>,
    /// Per-command canned rejections (success = false, message as given).
    pub errors: HashMap<String, String>,
    /// Per-command transport-level failures (tonic `Unavailable`), for the
    /// RPC-error arms that `errors` (a success=false reply) cannot reach.
    pub transport_fail: std::collections::HashSet<String>,
}

/// Start the shared mock (idempotent) and point `FUTURE_AGENT_GRPC_ADDR` at
/// it. The canonical server also re-asserts the env var, so a test that
/// briefly redirected it (see [`with_broken_endpoint`]) is restored to the
/// live mock. Returns the shared handle for scripting.
pub(crate) fn ensure_mock_agent() -> crate::remote::test_support::MockAgent {
    crate::remote::test_support::ensure_mock_agent()
}

/// Point the mock at a new persistent script. Caller must hold
/// [`mock_agent_lock`].
pub(crate) fn script_mock_agent(script: MockScript) {
    let mut data = script.data;
    let mut errors = script.errors;
    if script.fail_list_session_ids {
        errors
            .entry("list_session_ids".to_string())
            .or_insert_with(|| "mock enumeration failure".to_string());
    } else {
        data.entry("list_session_ids".to_string())
            .or_insert_with(|| serde_json::json!({ "ids": script.session_ids }).to_string());
    }
    // The health check always answers Ok (even in down mode), reporting the
    // scripted streaming sessions.
    data.entry("list_streaming_sessions".to_string())
        .or_insert_with(|| serde_json::json!({ "sessionIds": script.streaming_ids }).to_string());
    let agent = ensure_mock_agent();
    // One-shot scripts take precedence over this facade's persistent maps, so
    // clear any a previous (remote) test left behind before installing the new
    // script; otherwise a stale one-shot response would answer a caller that
    // expected the persistent canned answer.
    agent.clear_scripts();
    agent.set_persistent_script(data, errors, script.down, script.transport_fail);
}

/// Run `call` with a deliberately unparseable agent endpoint, then restore
/// the mock's address. `Endpoint::from_shared` runs before the latched
/// channel is consulted, so this makes `connect_agent` fail deterministically
/// regardless of latch state. Caller must hold [`mock_agent_lock`].
pub(crate) async fn with_broken_endpoint<F: std::future::Future>(
    call: impl FnOnce() -> F,
) -> F::Output {
    std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
    let result = call().await;
    ensure_mock_agent();
    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use tonic::Code;

    /// Bare command with just a type — all the mock reads.
    fn typed_command(r#type: &str) -> crate::agent_proto::RpcCommand {
        crate::agent_proto::RpcCommand {
            r#type: r#type.to_string(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn mock_answers_the_scripted_commands() {
        let _lock = mock_agent_lock();
        ensure_mock_agent();
        script_mock_agent(MockScript {
            down: false,
            fail_list_session_ids: false,
            session_ids: vec!["sess_a".to_string()],
            ..Default::default()
        });

        let mut client = crate::agent_bridge::connect_agent()
            .await
            .expect("connect to mock agent");
        let response = client
            .execute_command(typed_command("list_session_ids"))
            .await
            .expect("list_session_ids")
            .into_inner();
        assert!(response.success);
        assert!(response.data.contains("sess_a"));

        // A canned rejection: success = false with the scripted message.
        script_mock_agent(MockScript {
            errors: HashMap::from([("set_session_name".to_string(), "mock rejection".to_string())]),
            ..Default::default()
        });
        let rejected = client
            .execute_command(typed_command("set_session_name"))
            .await
            .expect("set_session_name")
            .into_inner();
        assert!(!rejected.success);
        assert_eq!(rejected.error, "mock rejection");

        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn mock_down_mode_is_indistinguishable_from_a_dead_agent() {
        let _lock = mock_agent_lock();
        ensure_mock_agent();
        // Latch the process-wide agent channel while the mock is up (the
        // health check succeeds even in down mode, so this cannot fail once
        // the mock is started).
        script_mock_agent(MockScript::default());
        let mut client = crate::agent_bridge::connect_agent()
            .await
            .expect("connect to mock agent");
        // …then take it down: RPCs on the latched channel fail Unavailable,
        // exactly like a dead agent.
        script_mock_agent(MockScript {
            down: true,
            ..Default::default()
        });

        let first = client
            .execute_command(typed_command("list_session_ids"))
            .await;
        assert_eq!(first.expect_err("down mock").code(), Code::Unavailable);

        // The health check stays up: the latch/health probe must keep
        // succeeding so callers reach the Unavailable path instead of a
        // connect failure.
        let health = client
            .execute_command(typed_command("list_streaming_sessions"))
            .await
            .expect("health check");
        assert!(health.into_inner().success);

        // A fresh connect reuses the latched channel — same failure.
        let mut again = crate::agent_bridge::connect_agent()
            .await
            .expect("latched channel");
        let second = again
            .execute_command(typed_command("list_session_ids"))
            .await;
        assert_eq!(second.expect_err("down mock").code(), Code::Unavailable);

        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn failed_enumeration_surfaces_as_a_rejection() {
        let _lock = mock_agent_lock();
        ensure_mock_agent();
        script_mock_agent(MockScript {
            fail_list_session_ids: true,
            ..Default::default()
        });

        let mut client = crate::agent_bridge::connect_agent()
            .await
            .expect("connect to mock agent");
        let response = client
            .execute_command(typed_command("list_session_ids"))
            .await
            .expect("list_session_ids")
            .into_inner();
        assert!(!response.success);

        script_mock_agent(MockScript::default());
    }
}
