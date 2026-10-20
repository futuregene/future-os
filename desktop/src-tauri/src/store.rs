//! Shared data backend plus the remaining Agent coordination adapter.
pub(crate) use future_host::store::*;

#[cfg(test)]
pub(crate) fn test_schema_home(label: &str) -> crate::auth_store::test_support::HomeGuard {
    let (home, conn) = db::test_support::guarded_conn(label);
    drop(conn);
    home
}

#[cfg(test)]
mod db {
    pub(crate) mod test_support {
        pub(crate) fn guarded_conn(
            label: &str,
        ) -> (
            crate::auth_store::test_support::HomeGuard,
            future_host::store::test_support::PooledConnection,
        ) {
            let home = crate::auth_store::test_support::HomeGuard::new(label);
            let conn = future_host::store::test_support::open_schema_connection();
            (home, conn)
        }
    }
}
