//! Shared credentials with a Desktop-only test resource adapter.
pub(crate) use future_host::auth_store::*;

#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) struct HomeGuard {
        _inner: future_host::auth_store::test_support::HomeGuard,
    }
    impl HomeGuard {
        pub(crate) fn new(label: &str) -> Self {
            Self {
                _inner: future_host::auth_store::test_support::HomeGuard::with_cleanup(
                    label,
                    || {
                        crate::runtime::cancel_test_tasks();
                        crate::store::close_pool();
                    },
                ),
            }
        }
    }
}
