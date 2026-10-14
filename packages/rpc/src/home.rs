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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn home_tag_is_stable_short_and_case_insensitive() {
        let tag = home_tag(Path::new("/tmp/futureos-home"));
        assert_eq!(tag.len(), 12);
        assert!(tag.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(tag, home_tag(Path::new("/tmp/FutureOS-Home")));
        assert_ne!(tag, home_tag(Path::new("/tmp/futureos-other")));
    }
}
