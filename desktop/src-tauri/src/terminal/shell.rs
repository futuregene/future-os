//! Default-shell resolution and the shell list offered to the client.
//!
//! Resolution order (design §5.1 / opencode `Shell.preferred`):
//!   unix    : account login shell → `$SHELL` → `/bin/bash` → `/bin/sh`
//!   windows : `pwsh.exe` → `powershell.exe` → `cmd.exe`
//!
//! Known POSIX shells are started as interactive login shells so the user's own
//! profile is loaded; anything else (fish, nushell, an unknown binary) is
//! started bare rather than handed flags it may not understand.
//!
//! Nothing here rewrites `.zshrc`/`.bashrc`/PowerShell profiles — the terminal
//! runs the user's real shell with the user's real environment.

use std::path::{Path, PathBuf};

/// Where a resolved shell came from. Reported for diagnostics only; it never
/// justifies a silent substitution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellSource {
    /// Account database (`getpwuid`), i.e. the real login shell.
    AccountLoginShell,
    /// The `SHELL` environment variable.
    EnvShell,
    /// A well-known absolute fallback path.
    KnownPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellChoice {
    pub path: PathBuf,
    pub source: ShellSource,
    /// True when the shell should be started with interactive/login flags.
    pub login: bool,
}

impl ShellChoice {
    /// Arguments passed to the shell. `-l -i` only for shells known to accept
    /// them; anything else is started bare.
    pub fn args(&self) -> Vec<String> {
        if self.login {
            vec!["-l".to_string(), "-i".to_string()]
        } else {
            Vec::new()
        }
    }
}

/// One entry of `GET /terminal/shells`; mirrors opencode's `Shell.Item`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellItem {
    pub path: String,
    pub name: String,
    /// False for shells the picker must not offer as a default (e.g. an
    /// experimental shell opencode-style `deny` entries); kept on the wire so
    /// the client can grey them out instead of hiding them.
    pub acceptable: bool,
}

/// Shell name (lowercased basename) used for the login-flag and deny tables.
fn shell_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// POSIX shells we know accept `-l -i`.
fn accepts_login_flags(path: &Path) -> bool {
    matches!(
        shell_name(path).as_str(),
        "bash" | "zsh" | "sh" | "dash" | "ksh" | "ksh93" | "mksh" | "ash" | "yash" | "tcsh" | "csh"
    )
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(unix)]
fn account_login_shell() -> Option<PathBuf> {
    // SAFETY: `getpwuid_r` is reentrant; we pass a caller-owned buffer and only
    // read `pw_shell` before the buffer goes out of scope.
    unsafe {
        let uid = libc::getuid();
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 4096];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc = libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result);
        if rc != 0 || result.is_null() || pwd.pw_shell.is_null() {
            return None;
        }
        let shell = std::ffi::CStr::from_ptr(pwd.pw_shell);
        let shell = shell.to_string_lossy().into_owned();
        if shell.is_empty() {
            None
        } else {
            Some(PathBuf::from(shell))
        }
    }
}

#[cfg(not(unix))]
fn account_login_shell() -> Option<PathBuf> {
    None
}

#[cfg(unix)]
fn known_fallbacks() -> &'static [&'static str] {
    &["/bin/bash", "/bin/sh"]
}

#[cfg(windows)]
fn known_fallbacks() -> &'static [&'static str] {
    // Resolved through PATH by the OS; `CommandBuilder` handles the lookup.
    &["pwsh.exe", "powershell.exe", "cmd.exe"]
}

#[cfg(not(any(unix, windows)))]
fn known_fallbacks() -> &'static [&'static str] {
    &["/bin/sh"]
}

/// Resolve the shell to run for a new session.
///
/// Returns `SHELL_UNAVAILABLE` when nothing usable exists instead of silently
/// picking an arbitrary binary.
pub fn resolve_shell() -> Result<ShellChoice, String> {
    let mut tried: Vec<String> = Vec::new();
    let mut candidates: Vec<(PathBuf, ShellSource)> = Vec::new();
    if let Some(shell) = account_login_shell() {
        candidates.push((shell, ShellSource::AccountLoginShell));
    }
    if let Some(shell) = std::env::var_os("SHELL") {
        let shell = PathBuf::from(shell);
        if !shell.as_os_str().is_empty() {
            candidates.push((shell, ShellSource::EnvShell));
        }
    }
    for fallback in known_fallbacks() {
        candidates.push((PathBuf::from(fallback), ShellSource::KnownPath));
    }

    for (path, source) in candidates {
        if path.as_os_str().is_empty() {
            continue;
        }
        // Only absolute paths are validated eagerly; on Windows the fallbacks
        // are bare program names resolved through PATH by the OS.
        let usable = if path.is_absolute() {
            is_executable_file(&path)
        } else {
            true
        };
        if usable {
            return Ok(ShellChoice {
                login: accepts_login_flags(&path),
                path,
                source,
            });
        }
        tried.push(path.display().to_string());
    }

    Err(format!(
        "SHELL_UNAVAILABLE: no usable shell found (tried: {})",
        tried.join(", ")
    ))
}

/// Shells the client may offer. On unix this is `/etc/shells` (opencode reads
/// the same file) with the resolver order prepended so the current default is
/// always present even when it is missing from the file.
pub fn list_shells() -> Vec<ShellItem> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Ok(preferred) = resolve_shell() {
        paths.push(preferred.path);
    }
    #[cfg(unix)]
    if let Ok(text) = std::fs::read_to_string("/etc/shells") {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            paths.push(PathBuf::from(line));
        }
    }
    #[cfg(windows)]
    for name in ["pwsh.exe", "powershell.exe", "cmd.exe"] {
        paths.push(PathBuf::from(name));
    }
    #[cfg(not(any(unix, windows)))]
    for name in known_fallbacks() {
        paths.push(PathBuf::from(name));
    }

    let mut items: Vec<ShellItem> = Vec::new();
    for path in paths {
        let path_text = path.to_string_lossy().into_owned();
        if items.iter().any(|item| item.path == path_text) {
            continue;
        }
        let name = shell_name(&path);
        items.push(ShellItem {
            acceptable: !matches!(name.as_str(), "fish" | "nu"),
            path: path_text,
            name,
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_a_real_shell_on_this_machine() {
        let choice = resolve_shell().expect("a shell must exist on a dev machine");
        assert!(
            choice.path.is_absolute() || cfg!(windows),
            "unix shells must resolve to an absolute path, got {:?}",
            choice.path
        );
        if choice.path.is_absolute() {
            assert!(
                is_executable_file(&choice.path),
                "resolved shell must be executable: {:?}",
                choice.path
            );
        }
    }

    #[test]
    fn known_shells_get_login_flags_and_unknown_ones_do_not() {
        assert!(accepts_login_flags(Path::new("/bin/bash")));
        assert!(accepts_login_flags(Path::new("/usr/bin/zsh")));
        assert!(!accepts_login_flags(Path::new("/usr/bin/fish")));
        assert!(!accepts_login_flags(Path::new("/usr/bin/nu")));
        assert!(!accepts_login_flags(Path::new("/usr/bin/mystery-shell")));
    }

    #[test]
    fn unknown_shell_is_started_without_flags() {
        let choice = ShellChoice {
            path: PathBuf::from("/usr/bin/fish"),
            source: ShellSource::EnvShell,
            login: accepts_login_flags(Path::new("/usr/bin/fish")),
        };
        assert!(choice.args().is_empty());
    }

    #[test]
    fn known_shell_gets_interactive_login_args() {
        let choice = ShellChoice {
            path: PathBuf::from("/bin/bash"),
            source: ShellSource::KnownPath,
            login: accepts_login_flags(Path::new("/bin/bash")),
        };
        assert_eq!(choice.args(), vec!["-l".to_string(), "-i".to_string()]);
    }

    #[test]
    fn non_executable_path_is_rejected() {
        assert!(!is_executable_file(Path::new("/etc/hostname")));
        assert!(!is_executable_file(Path::new("/definitely/not/here")));
    }

    #[test]
    fn shell_list_contains_the_resolved_default_and_no_duplicates() {
        let items = list_shells();
        let preferred = resolve_shell()
            .expect("shell")
            .path
            .to_string_lossy()
            .into_owned();
        assert!(
            items.iter().any(|item| item.path == preferred),
            "the resolved default {preferred} must be offered: {items:?}"
        );
        let mut seen: Vec<&str> = items.iter().map(|item| item.path.as_str()).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "shell list must not repeat a path");
    }
}
