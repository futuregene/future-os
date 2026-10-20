//! Desktop entry forwarding to the shared Remote application services.
#[cfg(any(feature = "gui", test))]
pub(crate) use future_host::remote::*;

#[cfg(test)]
pub(crate) mod test_support {
    #[cfg(feature = "gui")]
    pub(crate) use crate::auth_store::test_support::HomeGuard;
    pub(crate) use future_host::remote::test_support::*;

    pub(crate) fn ensure_mock_agent() -> MockAgent {
        crate::agent_bridge::install_channel_runtime();
        crate::runtime::install_test_runtime();
        future_host::remote::test_support::ensure_mock_agent()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn retained_entry_fixtures_keep_the_shared_rpc_lock_order() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        assert!(root.join("commands/agent_mock.rs").is_file());
        let (scanned, violations) = future_host::remote::test_support::scan_lock_order(&root);
        assert!(
            scanned >= 70,
            "the entry scan must see its source tree: {scanned}"
        );
        assert!(
            violations.is_empty(),
            "entry fixture lock inversions: {violations:?}"
        );
    }
}
