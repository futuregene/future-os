//! Build identity — version + release/dev channel, injected at build time.
//!
//! `FUTURE_VERSION` is set by `build.rs` from the `FUTURE_VERSION` env
//! (see `scripts/version.mjs`). Release builds carry a plain `1.X.Y`; dev builds
//! carry `0.0.2[-<hash>]` (the iOS TestFlight variant drops the suffix). The
//! channel is derived from the first version component — `0` means dev, `1`+
//! means release — so there is a single injected value. See the note in
//! `scripts/version.mjs` for the one assumption this makes (release versions
//! must never start with `0`).

/// Display version string for this build.
pub const VERSION: &str = env!("FUTURE_VERSION");

/// A release build starts with `1`+; a dev build starts with `0`.
pub fn is_release() -> bool {
    !VERSION.starts_with('0')
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
}
