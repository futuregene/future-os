//! Build identity — version + distribution channel, injected at build time.
//!
//! `FUTURE_VERSION` is set by `build.rs` from the `FUTURE_VERSION` env
//! (see `scripts/version.mjs`). Release builds carry a plain `1.X.Y`; dev builds
//! carry `0.0.2-<hash>` or `0.0.2-<run_number>+<channel>` (the iOS TestFlight
//! variant drops the suffix). The full version is enough to distinguish formal,
//! test, nightly, standalone-dev and local builds, so no second build-time value
//! needs to stay synchronized with it. See the note in `scripts/version.mjs` for
//! the one assumption this makes (release versions must never start with `0`).

/// Display version string for this build.
pub const VERSION: &str = env!("FUTURE_VERSION");

/// Distribution identity used to select the update channel and installation
/// policy. Unknown `0.*` builds are deliberately treated as `Dev`: they may
/// discover a nightly, but never gain automatic-install privileges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildChannel {
    Release,
    Test,
    Nightly,
    Dev,
    Local,
}

pub fn channel_for_version(version: &str) -> BuildChannel {
    if !version.starts_with('0') {
        return BuildChannel::Release;
    }

    let metadata = version.split_once('+').map(|(_, metadata)| metadata);
    match metadata {
        Some("test") => BuildChannel::Test,
        Some("nightly") => BuildChannel::Nightly,
        Some("dev") => BuildChannel::Dev,
        Some("local") | Some("local.dirty") => BuildChannel::Local,
        _ => BuildChannel::Dev,
    }
}

pub fn channel() -> BuildChannel {
    channel_for_version(VERSION)
}

/// A release build starts with `1`+; a dev build starts with `0`.
pub fn is_release() -> bool {
    channel() == BuildChannel::Release
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_channel_derives_from_version_prefix() {
        // The channel is derived from the first version component, so the
        // predicate must always agree with the injected VERSION.
        assert_eq!(is_release(), !VERSION.starts_with('0'));
    }

    #[test]
    fn classifies_every_distribution_channel() {
        assert_eq!(channel_for_version("1.2.3"), BuildChannel::Release);
        assert_eq!(channel_for_version("0.0.2-101+test"), BuildChannel::Test);
        assert_eq!(
            channel_for_version("0.0.2-102+nightly"),
            BuildChannel::Nightly
        );
        assert_eq!(channel_for_version("0.0.2-abcdef+dev"), BuildChannel::Dev);
        assert_eq!(
            channel_for_version("0.0.2-abcdef+local"),
            BuildChannel::Local
        );
        assert_eq!(
            channel_for_version("0.0.2-abcdef+local.dirty"),
            BuildChannel::Local
        );
    }

    #[test]
    fn unknown_zero_versions_are_conservative_dev_builds() {
        assert_eq!(channel_for_version("0.0.2-abcdef"), BuildChannel::Dev);
        assert_eq!(channel_for_version("0.9.0+unexpected"), BuildChannel::Dev);
    }
}
