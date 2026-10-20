//! Construction of gRPC `RpcCommand`s and the agent endpoint. This is the thin
//! request-building layer; orchestration and event handling live in the parent
//! module.

#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use future_rpc::proto::{RpcCommand, RpcResponse};

/// Desktop client wrapper that applies the shared per-command deadline while
/// leaving streaming RPCs on the underlying client deadline-free.
#[derive(Clone, Debug)]
pub struct AgentClient {
    inner: future_rpc::connection::VerifiedAgentClient,
}

impl AgentClient {
    fn new(inner: future_rpc::connection::VerifiedAgentClient) -> Self {
        Self { inner }
    }

    pub async fn execute_command(
        &mut self,
        command: RpcCommand,
    ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
        let _admitted = crate::request_scope::admit()?;
        let prompt = command.r#type == "prompt";
        let session = command.session_id.clone();
        let requested = command.requested_run_id.clone();
        if prompt {
            crate::request_scope::own_run(session.clone(), requested.clone(), self.inner.clone());
        }
        let result = self
            .inner
            .execute_command(future_rpc::command_policy::request_with_timeout(command))
            .await;
        if prompt {
            if let Ok(response) = &result {
                if response.get_ref().success {
                    let data = future_rpc::decode::response_data(response.get_ref());
                    if let Some(run) = data.get("run_id").and_then(serde_json::Value::as_str) {
                        if run != requested {
                            crate::request_scope::release_run(&session, &requested);
                            crate::request_scope::own_run(session, run.into(), self.inner.clone());
                        }
                        // The original entry was registered before dispatch. A
                        // terminal observer may already have released it while
                        // the acknowledgment was in flight; do not resurrect it.
                    }
                } else {
                    crate::request_scope::release_run(&session, &requested);
                }
            }
        }
        result
    }
}

impl std::ops::Deref for AgentClient {
    type Target = future_rpc::connection::VerifiedAgentClient;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for AgentClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

/// Cap on how long a single connection attempt may take. Without it a hung agent
/// can stall a caller indefinitely — e.g. the GUI's 10s model poll would pile up
/// overlapping calls, and a late failure could clobber fresh state.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Explicit TCP override or `auto` for per-user local IPC. Shared with the
/// bundled-agent supervisor.
pub fn raw_agent_addr() -> String {
    std::env::var("FUTURE_AGENT_GRPC_ADDR")
        .unwrap_or_else(|_| future_rpc::transport::AUTO_ENDPOINT.to_string())
}

/// Classify an RPC failure after connection/readiness verification. An Agent
/// that dies after verification surfaces as a tonic `Unavailable` transport
/// status. Map that to `AppError::AgentUnavailable`
/// so the tolerance sites (`abort_run`'s local cancel, credential reload's
/// "down agent is success") keep working; anything else is an app-level
/// failure and keeps the generic message variant. The agent reports
/// command-level failures inside an OK `RpcResponse` (`success = false`),
/// never as gRPC statuses. Readiness and instance gates may also return status
/// errors before a business command is dispatched.
pub fn map_rpc_error(context: &str, status: tonic::Status) -> crate::Error {
    if status.code() == tonic::Code::Unavailable {
        crate::Error::AgentUnavailable(format!("{context}: {}", status.message()))
    } else if status.code() == tonic::Code::DeadlineExceeded {
        crate::Error::Message(format!(
            "{context}: command timed out; its outcome may be unknown"
        ))
    } else {
        crate::Error::Message(format!("{context}: {status}"))
    }
}

/// Channel cache keyed by endpoint. The GUI talks to a single agent in
/// production, but the test binary hosts more than one in-process mock agent;
/// keying by address lets a changed `FUTURE_AGENT_GRPC_ADDR` reconnect instead
/// of reusing a stale connection to the previous mock.
static AGENT_CHANNEL: tokio::sync::Mutex<
    Option<(String, future_rpc::connection::VerifiedAgentClient)>,
> = tokio::sync::Mutex::const_new(None);

pub fn map_connection_error(error: future_rpc::connection::ConnectionError) -> crate::Error {
    match error {
        future_rpc::connection::ConnectionError::Incompatible {
            agent_version,
            ref reason,
        } => crate::Error::AgentIncompatible {
            agent_version,
            reason: format!("Incompatible Future Agent: {reason}"),
        },
        future_rpc::connection::ConnectionError::InstanceChanged => {
            crate::Error::AgentIncompatible {
                agent_version: None,
                reason: error.to_string(),
            }
        }
        _ => crate::Error::AgentUnavailable(error.to_string()),
    }
}

/// Both the discovery configuration and concrete IPC label participate in the
/// cache key. Readiness must pass even when the channel came from the cache.
#[cfg(any(test, feature = "test-support"))]
static TEST_CONNECTION_GENERATION: AtomicU64 = AtomicU64::new(0);
#[cfg(any(test, feature = "test-support"))]
pub fn reset_test_connection() {
    TEST_CONNECTION_GENERATION.fetch_add(1, Ordering::AcqRel);
}

pub async fn connect_agent() -> Result<AgentClient, crate::Error> {
    let configured = raw_agent_addr();
    let key = format!(
        "{}|{}|{:?}",
        configured,
        future_rpc::transport::connection_plan(Some(&configured))[0].label(),
        future_rpc::connection::CompatibilityRequirements::for_endpoint(
            &configured,
            crate::build_info::VERSION
        )
        .expected_home
    );
    #[cfg(any(test, feature = "test-support"))]
    let key = format!(
        "{key}|{}",
        TEST_CONNECTION_GENERATION.load(Ordering::Acquire)
    );
    let cached = AGENT_CHANNEL
        .lock()
        .await
        .as_ref()
        .filter(|(saved, _)| saved == &key)
        .map(|(_, client)| client.clone());
    if let Some(mut client) = cached {
        let result = agent_channel_runtime()?
            .spawn(async move { client.verify().await.map(|()| client) })
            .await
            .map_err(|error| {
                crate::Error::AgentUnavailable(format!("Agent verification task failed: {error}"))
            })?;
        match result {
            Ok(client) => return Ok(AgentClient::new(client)),
            Err(error) => {
                let mut cached = AGENT_CHANNEL.lock().await;
                if cached.as_ref().is_some_and(|(saved, _)| saved == &key) {
                    *cached = None;
                }
                if !matches!(
                    error,
                    future_rpc::connection::ConnectionError::Transport(_)
                        | future_rpc::connection::ConnectionError::Unverified(_)
                ) {
                    return Err(map_connection_error(error));
                }
            }
        }
    }
    let client = agent_channel_runtime()?
        .spawn(async move {
            future_rpc::connection::connect_verified(
                &configured,
                crate::build_info::VERSION,
                CONNECT_TIMEOUT,
                CONNECT_TIMEOUT,
            )
            .await
        })
        .await
        .map_err(|error| {
            crate::Error::AgentUnavailable(format!("Agent connection task failed: {error}"))
        })?
        .map_err(map_connection_error)?;
    *AGENT_CHANNEL.lock().await = Some((key, client.clone()));
    Ok(AgentClient::new(client))
}

pub async fn get_agent_readiness(
) -> Result<Option<future_rpc::payloads_ext::AgentReadinessPayload>, crate::Error> {
    Ok(Some(connect_agent().await?.readiness().clone()))
}

static CHANNEL_RUNTIME: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();

/// The entry owns the channel driver runtime and installs it before service
/// calls. A transient caller runtime must never own a cached tonic channel.
pub fn configure_channel_runtime(handle: tokio::runtime::Handle) -> Result<(), crate::Error> {
    CHANNEL_RUNTIME
        .set(handle)
        .map_err(|_| "Agent channel runtime already configured".into())
}
pub(crate) fn agent_channel_runtime() -> Result<tokio::runtime::Handle, crate::Error> {
    CHANNEL_RUNTIME.get().cloned().ok_or_else(|| {
        "Entry must configure the Agent channel runtime before starting services".into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn map_rpc_error_distinguishes_unavailable_from_app_failures() {
        let unavailable = map_rpc_error("ctx", tonic::Status::unavailable("connection refused"));
        assert!(matches!(unavailable, crate::Error::AgentUnavailable(_)));
        assert_eq!(unavailable.to_string(), "ctx: connection refused");
        let internal = map_rpc_error("ctx", tonic::Status::internal("boom"));
        assert!(matches!(internal, crate::Error::Message(_)));
        assert!(internal.to_string().starts_with("ctx: "));

        let deadline = map_rpc_error("ctx", tonic::Status::deadline_exceeded("expired"));
        assert!(matches!(deadline, crate::Error::Message(_)));
        assert_eq!(
            deadline.to_string(),
            "ctx: command timed out; its outcome may be unknown"
        );
    }
}

#[cfg(test)]
mod integration_tests {

    use super::*;
    use crate::test_support::agent::{mock_agent, Reply};

    /// Regression test for the startup channel-poisoning bug: the first
    /// `connect_agent` caller of the process runs on a throwaway per-thread
    /// runtime (mirrors lib.rs's session-import startup thread), which is
    /// dropped as soon as the thread returns. The shared channel must stay
    /// usable afterwards — before the fix, every later call failed with
    /// `Service was not ready: transport error` (surfaced as the run
    /// reanimation failure at startup). Runs against the in-process mock.
    #[test]
    fn shared_channel_survives_caller_runtime_drop() {
        let _mock = mock_agent();
        // First caller on a throwaway runtime, dropped on thread exit.
        std::thread::spawn(|| {
            let rt = tokio::runtime::Runtime::new().expect("runtime 1");
            rt.block_on(async {
                let mut client = super::connect_agent()
                    .await
                    .expect("connect on throwaway runtime");
                client
                    .execute_command(crate::agent_commands::list_streaming_sessions_command())
                    .await
                    .expect("first RPC");
            });
        })
        .join()
        .expect("first caller thread");

        // Second caller on a fresh runtime (mirrors the run-reanimation
        // startup thread): with the bug this failed with a transport error.
        let rt = tokio::runtime::Runtime::new().expect("runtime 2");
        rt.block_on(async {
            let mut client = super::connect_agent()
                .await
                .expect("connect after caller runtime drop");
            client
                .execute_command(crate::agent_commands::get_state_command(String::new()))
                .await
                .expect("RPC after caller runtime drop");
        });
    }

    #[test]
    fn raw_agent_addr_defaults_to_auto_discovery() {
        let _mock = mock_agent();
        // The mock sets FUTURE_AGENT_GRPC_ADDR to a bare host:port.
        let raw = raw_agent_addr();
        assert!(!raw.is_empty());
        assert_ne!(raw, future_rpc::transport::AUTO_ENDPOINT);
    }

    #[test]
    fn raw_agent_addr_defaults_to_auto_without_env() {
        let _mock = mock_agent();
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock sets the addr");
        std::env::remove_var("FUTURE_AGENT_GRPC_ADDR");
        assert_eq!(raw_agent_addr(), future_rpc::transport::AUTO_ENDPOINT);
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
    }

    #[tokio::test]
    async fn connect_agent_rejects_an_unparseable_endpoint() {
        let _mock = mock_agent();
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock sets the addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let result = connect_agent().await;
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        let error = result.expect_err("invalid endpoint must fail");
        assert!(
            matches!(error, crate::Error::AgentUnavailable(_)),
            "endpoint parse failure maps to AgentUnavailable: {error}"
        );
    }

    #[tokio::test]
    async fn readiness_failure_maps_to_unavailable_without_business_calls() {
        let mock = mock_agent();
        mock.push(
            "get_agent_readiness",
            Reply::Status(tonic::Code::Unavailable, "transport down"),
        );
        let error = connect_agent()
            .await
            .expect_err("failed readiness must prevent a business client");
        assert!(matches!(error, crate::Error::AgentUnavailable(_)));
        assert!(
            mock.requests().is_empty(),
            "no business calls after failed readiness"
        );
        assert!(!mock.requests_of("get_agent_readiness").is_empty());
    }

    /// A cached connection whose Agent now reports a *different* instance is not
    /// reused: the entry is dropped and the change is surfaced as
    /// `AgentIncompatible`, so no command is sent down a stale channel. An
    /// instance change is not a transport fault, so retrying the cached client
    /// would be wrong (a foreign data root must never be written to).
    #[tokio::test]
    async fn a_cached_connection_that_reports_a_changed_instance_is_dropped() {
        let mock = mock_agent();
        // First connect caches a verified client for this endpoint.
        connect_agent().await.expect("first connect");

        // A restarted/replaced Agent answers the readiness gate with the same
        // version but a new instance identity.
        mock.push_typed_data(
            "get_agent_readiness",
            serde_json::json!({
                "version": crate::build_info::VERSION,
                "agentInstanceId": "agent-after-restart",
                "protocolVersion": future_rpc::connection::AGENT_PROTOCOL_VERSION,
                "dataRootId": "0".repeat(64),
                "storageSchemaVersion": future_rpc::connection::AGENT_STORAGE_SCHEMA_VERSION
            }),
        );
        let error = connect_agent()
            .await
            .expect_err("a changed instance must not be reused");
        assert!(
            matches!(
                error,
                crate::Error::AgentIncompatible {
                    agent_version: None,
                    ..
                }
            ),
            "an instance change maps to AgentIncompatible: {error}"
        );
        assert!(
            mock.requests().is_empty(),
            "no business call may ride a rejected cached channel"
        );
    }

    /// A half-filled readiness handshake is not readiness. Reporting it as ready
    /// would let login persist a credential for a build this Desktop cannot talk
    /// to, so an empty version — or an empty instance id — must be rejected.
    #[tokio::test]
    async fn get_agent_readiness_rejects_a_half_filled_payload() {
        let mock = mock_agent();
        for payload in [
            serde_json::json!({"version": "", "agentInstanceId": "mock-agent"}),
            serde_json::json!({"version": "0.9.0", "agentInstanceId": ""}),
        ] {
            mock.push_typed_data("get_agent_readiness", payload.clone());
            let error = get_agent_readiness()
                .await
                .expect_err("an incomplete readiness payload must not read as ready");
            assert!(
                error.to_string().contains("incomplete readiness response"),
                "{payload} produced {error}"
            );
        }
        // The complete handshake still reads as ready, version and all.
        mock.push_typed_data(
            "get_agent_readiness",
            serde_json::json!({"version": crate::build_info::VERSION, "agentInstanceId": "mock-agent", "protocolVersion": future_rpc::connection::AGENT_PROTOCOL_VERSION, "dataRootId":"0".repeat(64), "storageSchemaVersion": future_rpc::connection::AGENT_STORAGE_SCHEMA_VERSION}),
        );
        let info = get_agent_readiness()
            .await
            .expect("complete payload")
            .expect("an Agent that answers the handshake is ready");
        assert_eq!(info.version, crate::build_info::VERSION);
        assert_eq!(info.agent_instance_id, "mock-agent");
    }
}
