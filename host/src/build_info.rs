//! Build identity — version + distribution channel, injected at build time.
//!
//! `FUTURE_VERSION` is set by `build.rs` from the `FUTURE_VERSION` env
//! (see `scripts/version.mjs`). Release builds carry a plain `1.X.Y`; dev builds
//! carry `0.0.2-<hash>` or `0.0.2-<run_number>+<channel>` (the iOS TestFlight
//! variant drops the suffix). The full version is enough to distinguish formal,
//! test, nightly, standalone-dev and local builds, so no second build-time value
//! needs to stay synchronized with it. See the note in `scripts/version.mjs` for
//! the one assumption this makes (release versions must never start with `0`).

use serde::{Deserialize, Serialize};

/// Display version string for this build.
pub const VERSION: &str = future_agent::utils::VERSION;

/// The commit/target the code compiled into **this process** came from.
///
/// Reported by the running process, which is the only thing that can answer "am
/// I the build you just installed?". `future version` cannot: it describes the
/// file the reader invoked, and the whole failure this exists for is a replaced
/// binary beside a process still running the old one. Asking `future version`
/// then answering with the new commit is exactly the wrong answer — measured
/// while diagnosing a pairing failure on a host whose `/proc/<pid>/exe` had been
/// deleted by an upgrade.
///
/// The values are compile-time constants from `agent/build.rs`
/// (`FUTURE_GIT_COMMIT` and friends), which is compiled into every binary that
/// contains this host — the window and `future host` alike. `None` is "not built
/// from a git checkout" (a tarball or vendored build): a reader comparing the
/// value against `git rev-parse` must not see a literal `unknown` and treat it
/// as a commit name, which is the same rule [`future_agent::utils::build_identity_json`]
/// applies to the Agent's own answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_commit_short: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_dirty: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_profile: Option<String>,
}

impl BuildIdentity {
    /// The identity of the code this process is running.
    pub fn of_this_process() -> Self {
        let commit = known(future_agent::utils::GIT_COMMIT);
        Self {
            git_commit: commit.map(str::to_string),
            // Only meaningful alongside a commit: with no checkout there is no
            // tree whose abbreviated name or dirtiness could be reported.
            git_commit_short: commit
                .and(known(future_agent::utils::GIT_COMMIT_SHORT))
                .map(str::to_string),
            git_dirty: commit.map(|_| future_agent::utils::GIT_DIRTY == "1"),
            build_target: known(future_agent::utils::BUILD_TARGET).map(str::to_string),
            build_profile: known(future_agent::utils::BUILD_PROFILE).map(str::to_string),
        }
    }
}

/// `unknown`/empty is "this build recorded nothing", not a value.
fn known(value: &str) -> Option<&str> {
    (!value.is_empty() && value != "unknown").then_some(value)
}

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

    /// The identity this host reports is the commit it was **compiled** from,
    /// word for word — not a value read at runtime, not a re-derived string.
    ///
    /// The reference is the compile-time constant itself: a hand-written literal
    /// would pass while the two drifted apart, which is the one thing this field
    /// must never do.
    #[test]
    fn the_reported_commit_is_the_compile_time_constant() {
        let identity = BuildIdentity::of_this_process();
        let known =
            |value: &'static str| (!value.is_empty() && value != "unknown").then_some(value);
        assert_eq!(
            identity.git_commit.as_deref(),
            known(future_agent::utils::GIT_COMMIT),
            "the snapshot must name the code in this process"
        );
        assert_eq!(
            identity.git_commit_short.as_deref(),
            known(future_agent::utils::GIT_COMMIT_SHORT)
        );
        assert_eq!(
            identity.build_target.as_deref(),
            known(future_agent::utils::BUILD_TARGET)
        );
        // Builds made in a checkout always carry a commit, so the shape every
        // release has is asserted rather than left to a build nobody runs.
        if known(future_agent::utils::GIT_COMMIT).is_some() {
            assert!(identity.git_dirty.is_some(), "{identity:?}");
        }
    }

    /// One rule for "recorded nothing", shared with the Agent's own identity: an
    /// absent commit is `None`/absent, never the literal string `unknown`, so a
    /// reader comparing against `git rev-parse` cannot mistake it for a name.
    #[test]
    fn the_identity_agrees_with_the_agents_own_answer() {
        let host = serde_json::to_value(BuildIdentity::of_this_process()).unwrap();
        let agent = future_agent::utils::build_identity_json();
        for key in [
            "gitCommit",
            "gitCommitShort",
            "gitDirty",
            "buildTarget",
            "buildProfile",
        ] {
            assert_eq!(
                host.get(key),
                agent.get(key),
                "{key} disagrees: {host} vs {agent}"
            );
        }
    }

    /// A snapshot from a build with no checkout says so by omitting the fields,
    /// not by inventing values.
    #[test]
    fn a_build_without_a_checkout_reports_nothing_rather_than_unknown() {
        let identity = BuildIdentity {
            git_commit: None,
            git_commit_short: None,
            git_dirty: None,
            build_target: known(future_agent::utils::BUILD_TARGET).map(str::to_string),
            build_profile: known(future_agent::utils::BUILD_PROFILE).map(str::to_string),
        };
        let value = serde_json::to_value(&identity).unwrap();
        assert!(value.get("gitCommit").is_none(), "{value}");
        assert!(value.get("gitDirty").is_none(), "{value}");
        // And it reads back as the same thing rather than failing.
        let read: BuildIdentity = serde_json::from_value(value).unwrap();
        assert_eq!(read, identity);
    }
}
