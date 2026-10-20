//! Shared credentials with a Desktop-only test resource adapter.
pub(crate) use future_host::auth_store::*;

#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) struct HomeGuard {
        inner: future_host::auth_store::test_support::HomeGuard,
    }
    impl HomeGuard {
        pub(crate) fn new(label: &str) -> Self {
            Self {
                inner: future_host::auth_store::test_support::HomeGuard::with_cleanup(
                    label,
                    || {
                        crate::runtime::cancel_test_tasks();
                        crate::store::close_pool();
                    },
                ),
            }
        }
        #[cfg(test)]
        fn path(&self) -> &std::path::Path {
            self.inner.path()
        }
    }

    #[test]
    fn home_guard_removes_its_directory_even_with_a_live_pooled_connection() {
        // Regression: the guard used to delete its temp home while the
        // connection pool still held the database open. Windows cannot unlink
        // an open file, so the delete failed silently and every run leaked its
        // home directory (~19k dirs / 33 GB accumulated before this was found),
        // which eventually filled the disk and surfaced as spurious SQLite
        // `DatabaseBusy`/`StorageFull` failures in unrelated tests.
        let dir = {
            let guard = HomeGuard::new("leak-regression");
            let dir = guard.path().to_path_buf();
            // Leave a pooled connection to this home's database open across the
            // guard's drop: that handle is what used to make the delete fail.
            crate::store::initialize_app_store().expect("store");
            dir
        };
        assert!(!dir.exists(), "guard must remove {} on drop", dir.display());
    }
}
