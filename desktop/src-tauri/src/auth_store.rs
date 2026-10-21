//! Shared credentials with a Desktop-only test resource adapter.
#[cfg(feature = "gui")]
pub(crate) use future_host::auth_store::*;

#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) struct HomeGuard {
        _inner: future_host::auth_store::test_support::HomeGuard,
    }
    impl HomeGuard {
        pub(crate) fn shared(&self) -> &future_host::auth_store::test_support::HomeGuard {
            &self._inner
        }
        pub(crate) fn new(label: &str) -> Self {
            Self {
                _inner: future_host::auth_store::test_support::HomeGuard::with_cleanup(
                    label,
                    || {
                        future_host::auth_store::test_support::cleanup_host();
                        crate::desktop_shutdown::reset_for_test();
                    },
                ),
            }
        }
    }
}
