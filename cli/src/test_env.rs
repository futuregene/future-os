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
pub struct EnvironmentGuard {
    // Release Host first, then the CLI async lock (reverse acquisition order).
    _host: std::sync::MutexGuard<'static, ()>,
    _cli: tokio::sync::MutexGuard<'static, ()>,
}

pub async fn lock_env() -> EnvironmentGuard {
    let cli = ENV_LOCK.lock().await;
    // Shared Host/entry tests use this same process environment after migration.
    let host = future_host::TEST_HOME_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    EnvironmentGuard {
        _host: host,
        _cli: cli,
    }
}

/// Save/restore a set of env vars around a test body.
pub struct EnvGuard {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    home: Option<tempfile::TempDir>,
}

impl EnvGuard {
    /// Point `vars` at fresh throwaway values; originals restored on drop.
    ///
    /// Redirecting `HOME` also redirects the FutureOS root (`FUTURE_HOME`) into
    /// `<that home>/.future`, unless the caller names `FUTURE_HOME` itself.
    /// Without that, an ambient override — a developer whose shell exports
    /// `FUTURE_HOME`, or a second isolated instance — stays in charge while the
    /// test believes it isolated everything: `future_home()` keeps resolving to
    /// the ambient root, so the app database, workspaces and task store a test
    /// just redirected land outside its temp dir, and the local endpoint is the
    /// running Agent's. `<home>/.future` is what a `HOME`-only resolution already
    /// produced, so no path assertion changes — only the ambient value stops
    /// winning.
    pub fn set(vars: &[(&'static str, std::ffi::OsString)]) -> Self {
        let mut saved: Vec<(&'static str, Option<std::ffi::OsString>)> = vars
            .iter()
            .map(|(k, _)| (*k, std::env::var_os(k)))
            .collect();
        let names_future_home = vars.iter().any(|(k, _)| *k == "FUTURE_HOME");
        let redirected_home = vars
            .iter()
            .find(|(k, _)| *k == "HOME")
            .map(|(_, v)| std::path::PathBuf::from(v));
        for (k, v) in vars {
            std::env::set_var(k, v);
        }
        if let (Some(home), false) = (redirected_home, names_future_home) {
            saved.push(("FUTURE_HOME", std::env::var_os("FUTURE_HOME")));
            std::env::set_var("FUTURE_HOME", home.join(".future"));
        }
        EnvGuard { saved, home: None }
    }

    /// Own a fresh home until restoration, with the FutureOS root redirected
    /// into it as well.
    ///
    /// The root has to move with the home, not merely be forgotten: dropping
    /// `FUTURE_HOME` still lands the *files* under `<temp>/.future`, but the
    /// local endpoint is named from the override alone
    /// (`future_home_override()` → the untagged default socket/pipe), so a test
    /// that removed it talked to the developer's running Agent while its files
    /// sat in a temp dir. Pointing the override at `<temp>/.future` keeps the
    /// file paths identical and moves the endpoint with them.
    pub fn temp_home() -> Self {
        Self::set(&[]).with_temp_home()
    }

    pub fn with_temp_home(mut self) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        for key in ["HOME", "USERPROFILE", "FUTURE_HOME"] {
            self.saved.push((key, std::env::var_os(key)));
        }
        std::env::set_var("HOME", dir.path());
        std::env::set_var("USERPROFILE", dir.path());
        // `<temp>/.future` is what `FUTURE_HOME` absent + `<temp>` as HOME
        // already resolved to, so every file assertion is unchanged; the
        // difference is that the endpoint is now this instance's own.
        std::env::set_var("FUTURE_HOME", dir.path().join(".future"));
        self.home = Some(dir);
        self
    }

    /// Remove `vars` for the duration of the guard; originals restored on drop.
    pub fn remove(vars: &[&'static str]) -> Self {
        let saved = vars.iter().map(|k| (*k, std::env::var_os(k))).collect();
        for k in vars {
            std::env::remove_var(k);
        }
        EnvGuard { saved, home: None }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if self.home.is_some() {
            future_host::auth_store::test_support::cleanup_host();
        }
        for (key, value) in self.saved.iter().rev() {
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
    #[tokio::test]
    async fn temporary_home_survives_its_constructor_and_restores_all_roots() {
        let _lock = super::lock_env().await;
        let previous = tempfile::tempdir().unwrap();
        let inherited = tempfile::tempdir().unwrap();
        let _outer = super::EnvGuard::set(&[
            ("HOME", previous.path().as_os_str().to_owned()),
            ("USERPROFILE", previous.path().as_os_str().to_owned()),
            ("FUTURE_HOME", inherited.path().as_os_str().to_owned()),
        ]);
        let guard = super::EnvGuard::temp_home();
        let path = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
        assert!(path.is_dir(), "the returned guard owns the live directory");
        // The ambient override must not outlive the guard: the fixture names
        // its own root (`<home>/.future`), not the caller's.
        assert_eq!(
            std::env::var_os("FUTURE_HOME"),
            Some(path.join(".future").as_os_str().to_owned())
        );
        assert_eq!(
            std::env::var_os("USERPROFILE"),
            Some(path.as_os_str().to_owned())
        );
        future_host::store::initialize_app_store().unwrap();
        assert!(path.join(".future").join("app").join("app.db").is_file());
        drop(guard);
        assert!(
            !path.exists(),
            "the fixture releases storage before deleting its home"
        );
        assert_eq!(
            std::env::var_os("HOME"),
            Some(previous.path().as_os_str().to_owned())
        );
        assert_eq!(
            std::env::var_os("USERPROFILE"),
            Some(previous.path().as_os_str().to_owned())
        );
        assert_eq!(
            std::env::var_os("FUTURE_HOME"),
            Some(inherited.path().as_os_str().to_owned())
        );
    }

    /// Redirecting `HOME` alone must not leave an ambient override in charge.
    /// Asserted in both directions, because the wrong half is the dangerous one:
    /// with the ambient value winning, a test's app database, workspaces and
    /// task store land in the developer's real root and its local endpoint is
    /// the running Agent's — while every path it printed still looks isolated.
    #[tokio::test]
    async fn set_redirects_the_future_home_root_with_the_home_it_redirects() {
        let _lock = super::lock_env().await;
        let ambient = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let explicit = tempfile::tempdir().unwrap();
        let _ambient = super::EnvGuard::remove(&["FUTURE_HOME"]);
        std::env::set_var("FUTURE_HOME", ambient.path());

        {
            let _guard = super::EnvGuard::set(&[("HOME", home.path().as_os_str().to_owned())]);
            assert_eq!(
                std::env::var_os("FUTURE_HOME"),
                Some(home.path().join(".future").as_os_str().to_owned()),
                "an ambient override must not outrank the home the guard set"
            );
        }
        assert_eq!(
            std::env::var_os("FUTURE_HOME"),
            Some(ambient.path().as_os_str().to_owned()),
            "the ambient value is restored on drop"
        );

        // An explicit FUTURE_HOME is the caller's decision, not derived.
        {
            let _guard = super::EnvGuard::set(&[
                ("HOME", home.path().as_os_str().to_owned()),
                ("FUTURE_HOME", explicit.path().as_os_str().to_owned()),
            ]);
            assert_eq!(
                std::env::var_os("FUTURE_HOME"),
                Some(explicit.path().as_os_str().to_owned())
            );
        }
        assert_eq!(
            std::env::var_os("FUTURE_HOME"),
            Some(ambient.path().as_os_str().to_owned())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wait_for_exhaustion_returns_false() {
        assert!(!super::wait_for(|| false).await);
        assert!(super::wait_for(|| true).await);
    }
}
