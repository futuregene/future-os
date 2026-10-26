//! Entry-owned Agent channel runtime. Shared connection policy, integration
//! tests, requests and exact-run ownership live in future-host.
#[cfg(all(feature = "gui", not(test)))]
pub(crate) use future_host::agent_client::raw_agent_addr;

pub(crate) fn install_channel_runtime() -> future_host::agent_client::RuntimeInstall {
    future_host::agent_client::provide_channel_runtime("agent-channel-runtime", || {
        // Process-lifetime runtime that owns the shared agent channel. tonic
        // spawns the h2 connection's driver task on whatever runtime first
        // touches the channel, and several startup callers (session import, run
        // reanimation) `block_on` a throwaway per-thread `Runtime` that is
        // dropped as soon as the thread returns — which used to kill the driver
        // and poison the cached channel for the rest of the process (every
        // later call failed with `Service was not ready: transport error`).
        // `provide_channel_runtime` parks this runtime on a thread that never
        // returns, so the connection survives regardless of which runtime any
        // caller runs on.
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("agent-channel")
            .build()
            .expect("build agent channel runtime")
    })
}

/// Status/login probes are entry calls too: they can precede any business
/// command. Install the entry-owned channel driver before using shared policy.
#[cfg(feature = "gui")]
pub(crate) async fn get_agent_readiness(
) -> Result<Option<future_rpc::payloads_ext::AgentReadinessPayload>, crate::AppError> {
    install_channel_runtime();
    future_host::agent_client::get_agent_readiness().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use future_host::agent_client::RuntimeInstall;

    /// Stand-in for the other installer a shared process has: the CLI entry in a
    /// process that embeds it, or the shared fixture inside this test binary
    /// (`agent_bridge::test_support::mock_agent`). Whichever of them runs
    /// first, installation must stay idempotent.
    fn other_installer() -> RuntimeInstall {
        future_host::agent_client::provide_channel_runtime("test-agent-channel", || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("test agent channel runtime")
        })
    }

    /// Another installer registered first, then this entry installs twice.
    ///
    /// This is the shape that produced 38 failing tests in one Desktop run: the
    /// entry wrapped "already configured" in `expect`, so the first install here
    /// panicked and poisoned its `Once`, and every later install in the process
    /// failed for the same unrelated reason. Reaching the end of this test *is*
    /// the no-panic assertion; the two `assert_eq!`s pin the reported outcome.
    #[test]
    fn a_second_installer_is_a_normal_outcome_not_a_panic() {
        assert!(matches!(
            other_installer(),
            RuntimeInstall::Installed | RuntimeInstall::AlreadyInstalled
        ));
        assert_eq!(install_channel_runtime(), RuntimeInstall::AlreadyInstalled);
        assert_eq!(install_channel_runtime(), RuntimeInstall::AlreadyInstalled);
    }
}
