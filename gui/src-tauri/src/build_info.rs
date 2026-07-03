//! Build identity — version + release/dev channel, injected at build time.
//!
//! `FUTURE_VERSION` is set by `build.rs` from the `FUTURE_VERSION` env
//! (see `scripts/version.mjs`). Release builds carry a plain `X.Y.Z`; dev builds
//! carry a `-dev.<hash>` suffix. The channel is derived from that suffix so there
//! is a single injected value — see the note in `scripts/version.mjs` for the
//! one assumption this makes (release versions must never carry a `-` suffix).

/// Display version string for this build.
pub const VERSION: &str = env!("FUTURE_VERSION");

/// A release build carries a plain `X.Y.Z` with no prerelease suffix; a dev
/// build carries `-dev.<hash>`.
pub fn is_release() -> bool {
    !VERSION.contains('-')
}
