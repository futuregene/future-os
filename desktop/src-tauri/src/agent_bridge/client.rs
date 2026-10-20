//! Entry-owned Agent channel runtime plus the remaining connection integration
//! tests. Shared gates, requests and exact-run ownership live in future-host.
#[cfg(test)]
pub(crate) use future_host::agent_client::reset_test_connection;
pub(crate) use future_host::agent_client::{map_connection_error, raw_agent_addr};
pub use future_host::agent_client::{map_rpc_error, AgentClient};
pub use future_host::agent_commands::*;

pub(crate) fn install_channel_runtime() {
    static INITIALIZED: std::sync::Once = std::sync::Once::new();
    INITIALIZED.call_once(|| {
        future_host::agent_client::configure_channel_runtime(agent_channel_runtime())
            .expect("entry configures the Agent channel runtime once");
    });
}

pub async fn connect_agent() -> Result<AgentClient, crate::AppError> {
    install_channel_runtime();
    future_host::agent_client::connect_agent().await
}

pub(crate) async fn get_agent_readiness(
) -> Result<Option<future_rpc::payloads_ext::AgentReadinessPayload>, crate::AppError> {
    install_channel_runtime();
    future_host::agent_client::get_agent_readiness().await
}

/// Process-lifetime runtime that owns the shared agent channel. tonic spawns
/// the h2 connection's driver task on whatever runtime first touches the
/// channel, and several startup callers (session import, run reanimation)
/// `block_on` a throwaway per-thread `Runtime` that is dropped as soon as the
/// thread returns — which used to kill the driver and poison the cached
/// channel for the rest of the process (every later call failed with
/// `Service was not ready: transport error`). Pinning channel creation to a
/// runtime that never shuts down keeps the connection alive regardless of
/// which runtime any caller runs on.
fn agent_channel_runtime() -> tokio::runtime::Handle {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .thread_name("agent-channel")
                .build()
                .expect("build agent channel runtime");
            let handle = runtime.handle().clone();
            std::thread::Builder::new()
                .name("agent-channel-runtime".to_string())
                .spawn(move || {
                    // Park forever: dropping the runtime would kill the shared
                    // connection's driver task.
                    runtime.block_on(std::future::pending::<()>());
                })
                .expect("spawn agent channel runtime thread");
            handle
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{mock_agent, Reply};
    use super::*;

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
                    .execute_command(super::list_streaming_sessions_command())
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
                .execute_command(super::get_state_command(String::new()))
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
            matches!(error, crate::AppError::AgentUnavailable(_)),
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
        assert!(matches!(error, crate::AppError::AgentUnavailable(_)));
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
                crate::AppError::AgentIncompatible {
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
