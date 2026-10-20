//! Entry-owned Agent channel runtime. Shared connection policy, integration
//! tests, requests and exact-run ownership live in future-host.
pub(crate) use future_host::agent_client::raw_agent_addr;
#[cfg(test)]
pub(crate) use future_host::agent_client::reset_test_connection;
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
