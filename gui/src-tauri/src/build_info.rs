//! Build identity — version + release/dev channel, injected at build time.
//!
//! `FUTURE_VERSION` is set by `build.rs` from the `FUTURE_VERSION` env
//! (see `scripts/version.mjs`). Release builds carry a plain `1.X.Y`; dev builds
//! carry `0.0.<commit-count>[-<hash>]` (the iOS TestFlight variant drops the
//! suffix). The channel is derived from the first version component — `0` means
//! dev, `1`+ means release — so there is a single injected value. See the note
//! in `scripts/version.mjs` for the one assumption this makes (release versions
//! must never start with `0`).

/// Display version string for this build.
pub const VERSION: &str = env!("FUTURE_VERSION");

/// A release build starts with `1`+; a dev build starts with `0`.
pub fn is_release() -> bool {
    !VERSION.starts_with('0')
}
