//! Test-only helpers for env-mutating tests.
//!
//! Cargo runs tests in threads within one process, and process environment
//! variables (HOME / PATH / FUTURE_AGENT_GRPC_ADDR) are global. Every test
//! that reads or writes them must hold the single shared [`ENV_LOCK`] so
//! they never race each other (e.g. `future doctor` tests repointing PATH
//! while `which` tests look up `sh`).

/// How many file descriptors this test binary is allowed to hold.
///
/// `cargo test` runs a binary's tests in threads of **one process**, and this
/// suite's socket tests each hold a listener plus a connection per unary call
/// (a gRPC client opens a fresh one). At the default parallelism of a 32-core
/// machine that peak is a few hundred descriptors, while macOS's soft limit is
/// **256** unless the calling shell raised it — so the whole suite is decided
/// by a shell setting nobody wrote down.
///
/// Measured 2026-11-01, `--test-threads` at the default 32, on a shell whose
/// `ulimit -n` was 256: 1439 of 1447 tests passed and the rest panicked with
/// `Too many open files` while *building a tokio runtime* or opening an app
/// database — never in an assertion. With `ulimit -n 8192` the same tree passed
/// 3 of 3. That is the descriptor cliff, not a race.
///
/// So the suite raises its own budget instead of asking the caller for one.
/// The raise is bounded by the hard limit, only ever upward, and idempotent, so
/// it composes with any test that lowers the limit on purpose to force
/// `EMFILE` (the agent's sandbox helper does): those restore what they found,
/// and the next call here raises again if what they found was the old ceiling.
///
/// `tui::test_env::ensure_fd_headroom` is the same helper for the same reason.
#[cfg(unix)]
pub fn ensure_fd_headroom() {
    // No `Once`: a test may lower the limit and restore it, so a one-shot guard
    // would fire into that window and never fire again.
    const WANT: libc::rlim_t = 4096;
    let mut limits = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: both calls pass a valid `rlimit` for `RLIMIT_NOFILE`; failure is
    // reported by the return value and only means "keep the current budget".
    unsafe {
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) != 0 {
            return;
        }
        if limits.rlim_cur >= WANT {
            return;
        }
        let raised = libc::rlimit {
            rlim_cur: WANT.min(limits.rlim_max),
            rlim_max: limits.rlim_max,
        };
        let _ = libc::setrlimit(libc::RLIMIT_NOFILE, &raised);
    }
}

/// Windows has no `RLIMIT_NOFILE`: a process is limited by its handle table,
/// not by a number a test can raise, so there is nothing to do here.
#[cfg(not(unix))]
pub fn ensure_fd_headroom() {}

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
    // Every env-sensitive test passes through here, so it is also where the
    // suite lifts its own descriptor ceiling before anything opens a socket.
    ensure_fd_headroom();
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

/// A free TCP port that no other `bind("127.0.0.1:0")` in this process (or on
/// this machine) can be handed while the test uses it.
///
/// The `free_port()` idiom — bind `:0`, read the port, drop the listener — only
/// proves the port was free at that instant. Every other `:0` bind in the test
/// binary draws from the same ephemeral pool: mock HTTP servers, CDP fixtures,
/// and the 50-port exhaustion reservation all do. One of them can be handed the
/// port between the probe and its use, and then the test talks to a stranger.
/// That is what reddened CI (`start_stops_polling_when_the_probe_budget_runs_out`
/// took the already-running branch and died on `No sessionId in createSession
/// response: {}`, i.e. a mock with a `/status` route and no `/session` route
/// answered instead of silence) and it reproduces here as `port_in_use_probe`
/// failing on `!port_in_use(free)`.
///
/// The kernel never assigns a `:0` bind a port below its ephemeral range —
/// `net.inet.ip.portrange.first` = 49152 on macOS, `ip_local_port_range` starts
/// at 32768 on Linux, the dynamic range starts at 49152 on Windows — so a port
/// from [`RESERVED_PORT_BAND`] is only ever taken by an explicit bind, which no
/// fixture here does. Ports are handed out once per process (the start is
/// spread by pid so concurrently running test binaries differ) and each
/// candidate is proved free by binding it first.
pub fn reserved_port() -> i64 {
    use std::sync::{Mutex, OnceLock};
    static NEXT: OnceLock<Mutex<u16>> = OnceLock::new();
    let mut next = NEXT
        .get_or_init(|| {
            let span = u32::from(RESERVED_PORT_BAND.1 - RESERVED_PORT_BAND.0);
            let start = RESERVED_PORT_BAND.0 + (std::process::id() % span) as u16;
            Mutex::new(start)
        })
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for _ in 0..(RESERVED_PORT_BAND.1 - RESERVED_PORT_BAND.0) {
        let candidate = *next;
        *next = if candidate + 1 >= RESERVED_PORT_BAND.1 {
            RESERVED_PORT_BAND.0
        } else {
            candidate + 1
        };
        if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", candidate)) {
            drop(listener);
            return i64::from(candidate);
        }
    }
    panic!(
        "no free port in the reserved band {}..{}",
        RESERVED_PORT_BAND.0, RESERVED_PORT_BAND.1
    );
}

/// The band [`reserved_port`] hands out from: entirely below every supported
/// platform's ephemeral range (see that function), so `bind(:0)` never returns
/// one of these ports to anybody else.
pub const RESERVED_PORT_BAND: (u16, u16) = (20_000, 29_000);

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
    /// Three properties the fix depends on: the port is out of `bind(:0)`'s
    /// reach, it is free when handed out, and it is never handed out twice.
    #[test]
    fn reserved_port_is_out_of_reach_free_and_unique() {
        let (low, high) = super::RESERVED_PORT_BAND;
        // A `:0` bind is only ever assigned a port from the OS ephemeral range:
        // 49152+ on macOS, 32768+ on Linux, 49152+ on Windows. A port below the
        // lowest of those floors cannot be taken by another fixture's bind.
        assert!(
            high <= 32_768,
            "the band must end below Linux's 32768 ephemeral floor"
        );
        let first = super::reserved_port();
        assert!(
            (i64::from(low)..i64::from(high)).contains(&first),
            "{first} is outside the reserved band {low}..{high}"
        );
        // Free at hand-out: the same bind a `:0` fixture would attempt succeeds.
        let listener = std::net::TcpListener::bind(("127.0.0.1", first as u16))
            .expect("a reserved port is free when it is handed out");
        drop(listener);
        // Handed out once per process, so two concurrently starting tests never
        // agree on a port.
        let second = super::reserved_port();
        assert!(
            (i64::from(low)..i64::from(high)).contains(&second),
            "{second} is outside the reserved band {low}..{high}"
        );
        assert_ne!(first, second, "a reserved port must not repeat");
    }

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
