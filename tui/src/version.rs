//! Build-time version for `future-tui --version` — injected by build.rs,
//! mirroring scripts/version.mjs so the Rust TUI prints exactly what the
//! TypeScript TUI prints (`future-tui v${VERSION}`).

/// Display version string (e.g. `0.0.1568-479c8fee+local`).
pub const VERSION: &str = env!("FUTURE_TUI_VERSION");

/// Whether this build is a release (`version` does not start with `0`).
pub fn is_release() -> bool {
    !VERSION.starts_with('0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_release_matches_version_prefix() {
        assert!(!VERSION.is_empty());
        assert_eq!(is_release(), !VERSION.starts_with('0'));
    }
}
