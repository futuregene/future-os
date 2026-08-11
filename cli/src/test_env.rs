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

    /// Remove `vars` for the duration of the guard; originals restored on drop.
    pub fn remove(vars: &[&'static str]) -> Self {
        let saved = vars.iter().map(|k| (*k, std::env::var_os(k))).collect();
        for k in vars {
            std::env::remove_var(k);
        }
        EnvGuard { saved }
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

/// Bounded poll: `cond` every 25 ms, up to ~2 s. Returns the final cond.
/// The exhaustion path is covered by `wait_for_exhaustion_returns_false`.
pub async fn wait_for(mut cond: impl FnMut() -> bool) -> bool {
    for _ in 0..80 {
        if cond() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    false
}

/// Reserve `n` consecutive TCP ports (for port-scan exhaustion tests).
/// The first candidate range always contains one deliberately squatted port,
/// so the collision-retry arm runs deterministically; a genuine transient
/// collision simply retries with a fresh base.
pub fn reserve_consecutive_ports(n: i64) -> (i64, Vec<std::net::TcpListener>) {
    fn free_port() -> i64 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind :0");
        let port = listener.local_addr().expect("local_addr").port() as i64;
        drop(listener);
        port
    }

    let mut base = free_port();
    let squat = std::net::TcpListener::bind(("127.0.0.1", (base + 10) as u16));
    let mut holders = Vec::new();
    let result = loop {
        holders.clear();
        let mut complete = true;
        for p in base..base + n {
            match std::net::TcpListener::bind(("127.0.0.1", p as u16)) {
                Ok(l) => holders.push(l),
                Err(_) => {
                    complete = false;
                    break;
                }
            }
        }
        if complete {
            break base;
        }
        base = free_port();
    };
    drop(squat);
    (result, holders)
}

#[cfg(test)]
mod tests {
    #[tokio::test(flavor = "multi_thread")]
    async fn wait_for_exhaustion_returns_false() {
        assert!(!super::wait_for(|| false).await);
        assert!(super::wait_for(|| true).await);
    }
}
