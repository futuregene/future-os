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

/// The FutureOS home root — `~/.future`, or the absolute `FUTURE_HOME` override
/// that replaces it.
///
/// Every TUI file the tool owns lives below it (`<root>/tui/...`), so an
/// isolated instance — which is what `FUTURE_HOME` exists for — keeps its own
/// settings, key bindings and logs instead of sharing the default home's. It
/// is deliberately *not* [`home_dir`]: `~` expansion for /cwd and the footer's
/// path shortening must keep naming the user's real home.
pub fn future_state_dir() -> std::path::PathBuf {
    future_rpc::home::future_home()
}

/// Expand a `/cwd`-style path argument: `~`, `~/x`, and a relative path joined
/// onto `base` when one is given.
///
/// `/cwd` and the new-session screen's workspace field both take a path the
/// user typed, and they must agree on what one means — a leading `~` is the
/// user's home, and a relative path is relative to the caller's base (the
/// session's cwd), never to whatever directory the TUI process happens to be in.
pub fn expand_path(input: &str, base: Option<&str>) -> std::path::PathBuf {
    let mut resolved = input.trim().to_string();
    let home = home_dir_or_default();
    // Both separators name a directory: a path typed on Windows may carry `\`
    // even when the host is POSIX (a pasted Windows path is the usual way).
    let tilde_child = format!("~{}", std::path::MAIN_SEPARATOR);
    let tilde_alternate = if std::path::MAIN_SEPARATOR == '/' {
        "~\\"
    } else {
        ""
    };
    if resolved == "~" {
        resolved = home.display().to_string();
    } else if resolved.starts_with(&tilde_child)
        || (!tilde_alternate.is_empty() && resolved.starts_with(tilde_alternate))
    {
        let rest = resolved
            .strip_prefix(&tilde_child)
            .or_else(|| resolved.strip_prefix(tilde_alternate))
            .unwrap_or_default();
        resolved = home.join(rest).display().to_string();
    } else if !std::path::Path::new(&resolved).is_absolute() {
        let base = match base.filter(|base| !base.is_empty()) {
            Some(base) => std::path::PathBuf::from(base),
            None => home,
        };
        resolved = base.join(&resolved).display().to_string();
    }
    std::path::PathBuf::from(resolved)
}
