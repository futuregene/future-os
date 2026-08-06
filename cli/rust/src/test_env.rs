//! Test-only helpers for env-mutating tests.
//!
//! Cargo runs tests in threads within one process, and process environment
//! variables (HOME / PATH / FUTURE_AGENT_GRPC_ADDR) are global. Every test
//! that reads or writes them must hold the single shared [`ENV_LOCK`] so
//! they never race each other (e.g. `future doctor` tests repointing PATH
//! while `which` tests look up `sh`).

/// The one lock every env-sensitive test acquires. `tokio::sync::Mutex` is
/// used so the guard can be held across `.await` points inside `#[tokio::test]`
/// bodies without tripping `clippy::await_holding_lock`; it also never
/// poisons (a panicking test can't break every later test).
pub static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Acquire [`ENV_LOCK`] for the duration of a test body.
pub async fn lock_env() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().await
}

/// Save/restore a set of env vars around a test body.
pub struct EnvGuard {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl EnvGuard {
    /// Point `vars` at fresh throwaway values; originals restored on drop.
    pub fn set(vars: &[(&'static str, std::ffi::OsString)]) -> Self {
        let saved = vars
            .iter()
            .map(|(k, _)| (*k, std::env::var_os(k)))
            .collect();
        for (k, v) in vars {
            std::env::set_var(k, v);
        }
        EnvGuard { saved }
    }

    /// Point HOME at a fresh temp dir (and nothing else).
    pub fn temp_home() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = std::env::var_os("HOME");
        std::env::set_var("HOME", dir.path());
        EnvGuard {
            saved: vec![("HOME", home)],
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}
