//! User home resolution for the TUI's `~/.future/tui` files (write log, debug
//! redraw log, crash log) and for `~` path expansion in `/cwd`.
//!
//! `$HOME` wins (POSIX, and a redirected/portable home), then `USERPROFILE`
//! (Windows shells set no `HOME`). The platform profile API is deliberately
//! not consulted: it observes neither variable, so a redirected home — a
//! portable install, or the isolated home the tests install — would be ignored
//! on Windows and the files would land in the real user profile.

/// The user's home directory, or `None` when nothing resolves.
pub fn home_dir() -> Option<std::path::PathBuf> {
    ["HOME", "USERPROFILE"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(std::path::PathBuf::from)
        .find(|path| path.is_absolute() && !path.as_os_str().is_empty())
}

/// [`home_dir`], falling back to an empty path (matches the previous
/// `dirs::home_dir().unwrap_or_default()` call sites).
pub fn home_dir_or_default() -> std::path::PathBuf {
    home_dir().unwrap_or_default()
}
