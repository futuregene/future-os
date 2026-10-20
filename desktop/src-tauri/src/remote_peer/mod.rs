//! Desktop entry forwarding to the shared Remote application services.
#[cfg(feature = "gui")]
pub(crate) use future_host::remote_peer::*;

#[cfg(test)]
pub(crate) mod testing {
    use crate::auth_store::test_support::HomeGuard;
    pub(crate) use future_host::remote_peer::testing::*;

    pub(crate) async fn fixture(label: &str) -> (HomeGuard, Fixture) {
        let home = HomeGuard::new(label);
        crate::agent_bridge::install_channel_runtime();
        crate::runtime::install_test_runtime();
        let fixture = future_host::remote_peer::testing::fixture_in_home(home.shared()).await;
        (home, fixture)
    }
}
