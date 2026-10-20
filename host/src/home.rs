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
