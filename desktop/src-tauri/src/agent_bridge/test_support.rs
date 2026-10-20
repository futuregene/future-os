//! Desktop test cleanup adapters into the canonical GUI-free RPC/store fixture.
pub(crate) use future_host::test_support::agent::{
    break_home, get_state_payload, restore_home, seed_approval, seed_run, seed_thread,
    seed_titleable_thread, seed_workspace, stream_event, MockAgentGuard, Reply, StreamScript,
};

pub(crate) fn mock_agent() -> MockAgentGuard {
    super::client::install_channel_runtime();
    future_host::test_support::agent::mock_agent_with_cleanup(super::observer::cancel_all_observers)
}

pub(crate) struct TestHome {
    inner: future_host::test_support::agent::TestHome,
}
impl TestHome {
    pub(crate) fn new(tag: &str) -> Self {
        Self {
            inner: future_host::test_support::agent::TestHome::with_cleanup(tag, cleanup),
        }
    }
    pub(crate) fn path(&self) -> &std::path::Path {
        self.inner.path()
    }
}
fn cleanup() {
    crate::runtime::cancel_test_tasks();
    super::observer::cancel_all_observers();
}
