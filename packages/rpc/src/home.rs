//! FutureOS home resolution — the directory that owns this instance's local
//! state (`<home>/agent`, `<home>/run`, ...), normally `~/.future`.
//!
//! `FUTURE_HOME` (set by `future agent --home DIR`) replaces that root for the
//! whole process, which is how several fully isolated Agent instances run side
//! by side: each one gets its own singleton lock, database, sessions, logs and
//! local IPC endpoint. The resolution lives in this crate because the local
//! transport (socket path / named-pipe name) and the Agent's own path helpers
//! must agree on it, and clients that only link `future-rpc` must be able to
//! address the right instance.

use std::path::{Path, PathBuf};

/// Environment variable naming this process's FutureOS home (see
/// [`future_home_override`]).
pub const FUTURE_HOME_ENV: &str = "FUTURE_HOME";

pub const RESTORE_PENDING_FILE: &str = ".restore-incomplete.json";

/// An interrupted multi-file restore must finish or roll back before any writer
/// opens either database. Existence errors are failures, never permission to start.
pub fn ensure_restore_complete(home: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(home.join(RESTORE_PENDING_FILE)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Err(std::io::Error::other(
            "A FutureOS data restore is incomplete. Run `future data resume` or `future data rollback` before starting clients.",
        )),
    }
}

/// The configured FutureOS home override, when the environment names one.
///
/// Empty and relative values are ignored: a relative root would make every
/// process resolve a different directory depending on its working directory,
/// so credentials, sandbox state and IPC endpoints would silently diverge.
pub fn future_home_override() -> Option<PathBuf> {
    future_home_override_from(std::env::var_os(FUTURE_HOME_ENV))
}

/// [`future_home_override`] with the raw environment value injected, so the
/// rejection rules are testable without mutating the process environment.
pub fn future_home_override_from(raw: Option<std::ffi::OsString>) -> Option<PathBuf> {
    raw.map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty() && path.is_absolute())
}

/// The data root shared by Agent and clients. Reading it never creates state.
pub fn future_home() -> PathBuf {
    future_home_override().unwrap_or_else(|| {
        user_home_from(
            std::env::var_os("HOME"),
            std::env::var_os("USERPROFILE"),
            dirs::home_dir(),
        )
        .unwrap_or_else(std::env::temp_dir)
        .join(".future")
    })
}

// Match Agent's established HOME -> USERPROFILE -> platform profile order.
// Windows dirs::home_dir() alone uses the OS profile and ignores a custom HOME,
// which would make the client probe a different lock/data root than its host.
fn user_home_from(
    home: Option<std::ffi::OsString>,
    profile: Option<std::ffi::OsString>,
    system: Option<PathBuf>,
) -> Option<PathBuf> {
    home.into_iter()
        .chain(profile)
        .map(PathBuf::from)
        .find(|path| !path.as_os_str().is_empty() && path.is_absolute())
        .or(system)
}

/// Stable short tag for a non-default FutureOS home, used to give each instance
/// its own Windows named pipe (Unix sockets are distinguished by their path).
/// FNV-1a keeps it dependency-free and identical across the Agent and its
/// clients (unlike `DefaultHasher`, whose output may change between releases);
/// paths are lowercased because Windows filesystems are case-insensitive.
pub fn home_tag(future_home: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in future_home.to_string_lossy().to_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    // 48 bits is far beyond any realistic instance count and keeps the pipe
    // name short.
    format!("{:012x}", hash & 0x0000_ffff_ffff_ffff)
}

/// Tests in this crate mutate the process environment (`FUTURE_HOME`, `HOME`,
/// `FUTURE_AGENT_SOCKET`) because those are exactly the inputs under test, and
/// the resolved home is process-global: an interleaved set/restore in another
/// test thread would rename this test's instance (and, on this host, could make
/// it bind the endpoint of a running Agent). Every such test takes this lock.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The guard is deliberately held across `await` points by the async tests that
/// resolve an endpoint under a redirected home: the environment has to stay
/// stable for the whole resolution, not just for the mutation. Every such test
/// runs on a current-thread runtime (`#[tokio::test]`), so no other task on that
/// thread can be blocked by it.
#[cfg(test)]
#[allow(clippy::await_holding_lock)]
pub(crate) fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Put one environment variable back exactly as a test found it: a value it
    /// had, or its absence. Extracted so both directions are asserted on their
    /// own (below) instead of depending on whatever the ambient environment
    /// happens to hold for the run — which is also why every test here takes
    /// [`env_guard`].
    fn restore_env(name: &str, previous: Option<std::ffi::OsString>) {
        match previous {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    #[test]
    fn the_environment_restore_helper_puts_back_a_value_and_an_absence() {
        const PROBE: &str = "FUTURE_RPC_RESTORE_PROBE";
        let _guard = env_guard();

        std::env::set_var(PROBE, "set by the test");
        restore_env(PROBE, None);
        assert!(
            std::env::var_os(PROBE).is_none(),
            "a variable that was absent must be absent again"
        );

        std::env::set_var(PROBE, "clobbered");
        restore_env(PROBE, Some("original".into()));
        assert_eq!(std::env::var(PROBE).unwrap(), "original");
        std::env::remove_var(PROBE);
    }

    /// The shared data root: an absolute `FUTURE_HOME` replaces it outright, and
    /// otherwise it is `<HOME>/.future` — read, never created.
    #[test]
    fn the_data_root_follows_the_override_and_then_the_user_home() {
        let _guard = env_guard();
        let previous_future_home = std::env::var_os(FUTURE_HOME_ENV);
        let previous_home = std::env::var_os("HOME");

        let root = std::env::temp_dir().join("rpc-future-home-test");
        std::env::set_var(FUTURE_HOME_ENV, &root);
        assert_eq!(future_home(), root, "an absolute override is the root");

        // Relative overrides are ignored (see `future_home_override_from`), so
        // resolution falls back to the user home and appends the default name.
        std::env::set_var(FUTURE_HOME_ENV, "relative-root");
        let home = std::env::temp_dir().join("rpc-user-home-test");
        std::env::set_var("HOME", &home);
        assert_eq!(future_home(), home.join(".future"));
        assert!(
            !future_home().exists(),
            "resolving the root must not create it"
        );

        restore_env("HOME", previous_home);
        restore_env(FUTURE_HOME_ENV, previous_future_home);
    }

    #[test]
    fn custom_home_precedes_profile_for_agent_client_root_agreement() {
        let home = std::env::temp_dir().join("rpc-home");
        let profile = std::env::temp_dir().join("rpc-profile");
        assert_eq!(
            user_home_from(
                Some(home.clone().into_os_string()),
                Some(profile.into_os_string()),
                None
            ),
            Some(home)
        );
    }

    #[test]
    fn invalid_home_falls_back_to_the_valid_profile() {
        let profile = std::env::temp_dir().join("rpc-profile");
        assert_eq!(
            user_home_from(
                Some("relative-home".into()),
                Some(profile.clone().into_os_string()),
                None
            ),
            Some(profile)
        );
    }

    #[test]
    fn override_accepts_absolute_and_rejects_empty_or_relative() {
        let absolute = std::env::temp_dir().join("futureos-other-home");
        assert_eq!(
            future_home_override_from(Some(absolute.clone().into_os_string())),
            Some(absolute)
        );
        assert_eq!(
            future_home_override_from(Some(std::ffi::OsString::new())),
            None
        );
        assert_eq!(
            future_home_override_from(Some(std::ffi::OsString::from("relative/home"))),
            None
        );
        assert_eq!(future_home_override_from(None), None);
    }

    /// The last resort: with no usable environment and no platform profile the
    /// root still resolves (to the temp directory) instead of panicking.
    #[test]
    fn the_data_root_survives_a_host_with_no_usable_home() {
        assert_eq!(user_home_from(Some("".into()), None, None), None);
        assert_eq!(
            user_home_from(Some("".into()), Some("relative".into()), None),
            None
        );
        let system = std::env::temp_dir().join("rpc-system-home");
        assert_eq!(
            user_home_from(None, None, Some(system.clone())),
            Some(system),
            "the platform profile is the fallback"
        );
    }

    #[test]
    fn home_tag_is_stable_short_and_case_insensitive() {
        let tag = home_tag(Path::new("/tmp/futureos-home"));
        assert_eq!(tag.len(), 12);
        assert!(tag.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(tag, home_tag(Path::new("/tmp/FutureOS-Home")));
        assert_ne!(tag, home_tag(Path::new("/tmp/futureos-other")));
    }
}
