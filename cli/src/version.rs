//! Build-time version and build identity for `future --version` / `future
//! version --json` — injected by `build.rs`, mirroring `scripts/version.mjs` so
//! the Rust CLI prints exactly what the TS CLI printed.
//!
//! [`VERSION`] is the display string. It is *not* enough to identify the code:
//! a release tag (`1.2.3`) and a coordinated build (`0.0.2-<run>+test`) both
//! omit the commit, and a dev build only carries an abbreviated hash. The
//! structured facts below are what let a support conversation or an agent answer
//! "which commit is this binary, built for which target".

use serde_json::{json, Value};

/// Display version string (e.g. `0.0.2-479c8fee+local`).
pub const VERSION: &str = env!("FUTURE_CLI_VERSION");

/// Full commit this binary was built from, or `unknown` outside a checkout.
const GIT_COMMIT: &str = env!("FUTURE_GIT_COMMIT");
const GIT_COMMIT_SHORT: &str = env!("FUTURE_GIT_COMMIT_SHORT");
/// `"1"`/`"0"` — see `cli/build.rs`.
const GIT_DIRTY: &str = env!("FUTURE_GIT_DIRTY");
const BUILD_TARGET: &str = env!("FUTURE_BUILD_TARGET");
const BUILD_PROFILE: &str = env!("FUTURE_BUILD_PROFILE");

/// A version is a release iff its first component is non-zero (`0.*` is dev —
/// see `scripts/version.mjs`), so this needs no second injected value.
pub fn is_release(version: &str) -> bool {
    !version.starts_with('0')
}

/// Installer/updater version: the plain semver core, since installers reject
/// `-`/`+` suffixes.
pub fn bundle_version(version: &str) -> &str {
    version.split(['-', '+']).next().unwrap_or(version)
}

/// `unknown` means "not built from a git checkout" (a tarball or vendored
/// build), which is a different fact from "no commit recorded" — so it becomes
/// JSON `null` rather than a string a caller could mistake for a real value.
fn known(value: &str) -> Option<&str> {
    (!value.is_empty() && value != "unknown").then_some(value)
}

/// The structured build identity of this binary.
///
/// `commit` is present even for release builds, where the display version has no
/// hash at all — that is what makes "is the binary I am running the commit I am
/// reading?" answerable. `dirty` is only meaningful when `commit` is set: a
/// build without a checkout has no working tree to be dirty.
pub fn build_info() -> Value {
    build_info_from(known(GIT_COMMIT), GIT_COMMIT_SHORT, GIT_DIRTY)
}

/// [`build_info`] over an explicitly supplied commit identity.
///
/// `FUTURE_GIT_COMMIT` and friends are injected by `build.rs` at compile time,
/// so within one build the "no checkout" shape cannot be produced by any input:
/// a build made in a checkout always reports a commit, and the arm that
/// describes its absence would be dead. It is still a shape this projection has
/// to get right (a tarball or vendored build reports it), so the identity is a
/// parameter and both shapes are asserted through this one function.
fn build_info_from(commit: Option<&str>, commit_short: &str, dirty: &str) -> Value {
    json!({
        "version": VERSION,
        "isRelease": is_release(VERSION),
        "bundleVersion": bundle_version(VERSION),
        "gitCommit": commit,
        "gitCommitShort": commit.and(known(commit_short)),
        "gitDirty": commit.map(|_| dirty == "1"),
        "buildTarget": known(BUILD_TARGET),
        "buildProfile": known(BUILD_PROFILE),
        "desktopCapabilities": future_rpc::cli_capabilities::CliCapabilities::default(),
    })
}

/// JSON text of [`build_info`], pretty-printed and newline-terminated.
pub fn build_info_json() -> String {
    let mut text = serde_json::to_string_pretty(&build_info()).unwrap_or_else(|_| "{}".to_string());
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two shapes `gitCommit`/`gitCommitShort`/`gitDirty` may take in a
    /// build identity document. Shared by the two tests below, so a build made
    /// in a checkout measures the other shape as well instead of leaving it to
    /// a build nobody runs in CI.
    fn assert_identity_shape(info: &Value) {
        match info["gitCommit"].as_str() {
            Some(commit) => {
                assert_eq!(commit.len(), 40, "{commit} is not a full object name");
                assert!(
                    commit.chars().all(|c| c.is_ascii_hexdigit()),
                    "{commit} is not hex"
                );
                let short = info["gitCommitShort"].as_str().expect("short commit");
                assert!(
                    commit.starts_with(short),
                    "{short} is not a prefix of {commit}"
                );
                assert!(info["gitDirty"].as_bool().is_some());
            }
            None => {
                assert!(info["gitCommitShort"].is_null());
                // No checkout means no working tree, so dirtiness is unknown —
                // not `false`, which would read as "verified clean".
                assert!(info["gitDirty"].is_null());
            }
        }
    }

    /// The reported commit must be a real object name, since callers compare it
    /// against `git rev-parse HEAD` of a checkout. `unknown` (no checkout) is
    /// the one legitimate absence, and then it is reported as null.
    #[test]
    fn the_commit_is_either_a_real_object_name_or_absent() {
        assert_identity_shape(&build_info());
    }

    /// The no-checkout half of the same contract: a tarball or vendored build
    /// must report an absent commit and *unknown* dirtiness rather than a stale
    /// or empty string. `build.rs` injects the identity at compile time, so the
    /// id this build carries is fixed before any test runs; the other shape is
    /// asserted through the same projection the binary uses.
    #[test]
    fn a_build_without_a_checkout_reports_no_commit_and_unknown_dirtiness() {
        let info = build_info_from(None, "unknown", "0");
        assert_identity_shape(&info);
        assert!(info["gitCommit"].is_null(), "{info}");
        // The injected `short`/`dirty` values must not leak through as fields
        // of their own once there is no commit to qualify.
        assert!(info["gitCommitShort"].is_null(), "{info}");
        assert!(info["gitDirty"].is_null(), "{info}");
    }

    /// The version string and the structured fields describe the same build.
    #[test]
    fn the_structured_fields_agree_with_the_version_string() {
        let info = build_info();
        assert_eq!(info["version"], VERSION);
        assert_eq!(info["isRelease"], is_release(VERSION));
        assert_eq!(info["bundleVersion"], bundle_version(VERSION));
        // No suffix means the version string would be ambiguous about the
        // channel, which the explicit flag removes.
        assert_eq!(
            info["isRelease"],
            !VERSION.starts_with("0"),
            "channel is derived from the version's first component"
        );
    }

    #[test]
    fn release_and_bundle_version_split_the_documented_way() {
        assert!(is_release("1.0.0"));
        assert!(is_release("1.2.3+test"));
        assert!(!is_release("0.0.2-479c8fee+local"));
        assert_eq!(bundle_version("1.2.3+test"), "1.2.3");
        assert_eq!(bundle_version("0.0.2-479c8fee+local.dirty"), "0.0.2");
        assert_eq!(bundle_version("1.2.3"), "1.2.3");
    }

    /// The build script fills these for every build, including out-of-checkout
    /// ones, so they are never an empty string on the wire.
    #[test]
    fn target_and_profile_are_always_populated() {
        let info = build_info();
        for key in ["buildTarget", "buildProfile"] {
            assert!(info[key].is_string(), "{key} missing: {info}");
        }
    }

    #[test]
    fn json_output_is_pretty_and_newline_terminated() {
        let text = build_info_json();
        assert!(text.ends_with("}\n"), "{text}");
        assert!(text.contains("\n  \""), "expected indentation: {text}");
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["version"], VERSION);
    }
}
