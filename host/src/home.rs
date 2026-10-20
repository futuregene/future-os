/// Cross-platform home directory. Prefers `HOME` (always set on macOS/Linux, and
/// what the test suite overrides to redirect storage) and falls back to
/// `USERPROFILE` on Windows, where `HOME` is normally unset.
pub fn home_dir() -> Option<String> {
    home_dir_from(
        std::env::var("HOME").ok(),
        std::env::var("USERPROFILE").ok(),
    )
}

fn home_dir_from(home: Option<String>, userprofile: Option<String>) -> Option<String> {
    [home, userprofile]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty() && std::path::Path::new(value).is_absolute())
}

/// FutureOS home root (`<home>/agent`, `<home>/tasks`, …), normally `~/.future`.
///
/// `FUTURE_HOME` replaces the whole root (matching the agent); otherwise the
/// desktop app dir's parent is used, so the tasks store and the GUI store can
/// never disagree about which home they belong to.
pub fn future_home_root() -> std::path::PathBuf {
    if let Some(override_dir) = future_rpc::home::future_home_override() {
        return override_dir;
    }
    future_app_settings::app_dir()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
        .unwrap_or_else(|| std::env::temp_dir().join(".future"))
}

#[cfg(test)]
mod home_tests {
    #[test]
    fn rejects_empty_and_relative_home_overrides() {
        let absolute = std::env::temp_dir().to_string_lossy().into_owned();
        for invalid in ["", "relative-home"] {
            assert_eq!(
                super::home_dir_from(Some(invalid.into()), Some(absolute.clone())),
                Some(absolute.clone())
            );
            assert_eq!(
                super::home_dir_from(Some(invalid.into()), Some(invalid.into())),
                None
            );
        }
        assert_eq!(
            super::home_dir_from(Some(absolute.clone()), None),
            Some(absolute)
        );
    }
}
