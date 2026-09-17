//! Default-shell resolution and the shell list offered to the client.
//!
//! Resolution order (design §5.1 / opencode `Shell.preferred`):
//!   unix    : account login shell → `$SHELL` → `/bin/bash` → `/bin/sh`
//!   windows : `pwsh.exe` → `powershell.exe` → `cmd.exe`, each resolved to an
//!             absolute path that exists (see `resolve_windows_program`)
//!
//! Known POSIX shells are started as interactive login shells so the user's own
//! profile is loaded; anything else (fish, nushell, an unknown binary) is
//! started bare rather than handed flags it may not understand.
//!
//! Nothing here rewrites `.zshrc`/`.bashrc`/PowerShell profiles — the terminal
//! runs the user's real shell with the user's real environment.

#[cfg(windows)]
use std::ffi::OsStr;
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
    // Bare names on purpose: `resolve_shell` turns each one into an absolute
    // path that exists before it is offered (`resolve_windows_program`), so a
    // missing PowerShell 7 falls through to 5.1 instead of failing the spawn.
    &["pwsh.exe", "powershell.exe", "cmd.exe"]
}

/// The file names Windows considers for `name`: `pwsh` → `pwsh.exe`. A name
/// that already carries an extension is used verbatim.
#[cfg(windows)]
fn windows_launch_names(name: &str) -> Vec<String> {
    if Path::new(name).extension().is_some() {
        return vec![name.to_string()];
    }
    [".exe", ".cmd", ".bat", ".com"]
        .iter()
        .map(|extension| format!("{name}{extension}"))
        .collect()
}

/// Absolute locations a Windows shell may live in without being on `PATH`.
///
/// PowerShell 7's own installer leaves `%ProgramFiles%\PowerShell\7` off `PATH`
/// unless the user asks for it, which is the common way `pwsh.exe` goes
/// missing on an otherwise healthy machine. Windows PowerShell 5.1 and
/// `cmd.exe` live in the system directory; the registry's `%COMSPEC%` is the
/// user's authoritative `cmd.exe`.
#[cfg(windows)]
fn known_windows_locations(name: &str) -> Vec<PathBuf> {
    let stem = Path::new(name)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let file_name = windows_launch_names(name)
        .into_iter()
        .next()
        .unwrap_or_else(|| name.to_string());
    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let mut locations = Vec::new();
    match stem.as_str() {
        "pwsh" => {
            for variable in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
                let Some(root) = std::env::var_os(variable) else {
                    continue;
                };
                for version in ["7", "7-preview", "6"] {
                    locations.push(
                        PathBuf::from(&root)
                            .join("PowerShell")
                            .join(version)
                            .join(&file_name),
                    );
                }
            }
        }
        "powershell" => locations.push(
            system_root
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join(&file_name),
        ),
        "cmd" => {
            locations.push(system_root.join("System32").join(&file_name));
            if let Some(comspec) = std::env::var_os("COMSPEC") {
                locations.push(PathBuf::from(comspec));
            }
        }
        _ => {}
    }
    locations
}

/// Candidate paths for a bare Windows program name: `PATH` entries in the
/// OS's own order.
#[cfg(windows)]
fn path_candidates(name: &str, path_env: Option<&OsStr>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let Some(path_env) = path_env else {
        return candidates;
    };
    for directory in std::env::split_paths(path_env) {
        // Hand-edited `PATH` entries are often quoted or padded; the OS trims
        // the quotes, so the lookup has to as well.
        let directory = directory.to_string_lossy();
        let directory = directory.trim().trim_matches('"');
        if directory.is_empty() {
            continue;
        }
        for name in windows_launch_names(name) {
            candidates.push(Path::new(directory).join(name));
        }
    }
    candidates
}

/// Resolve a Windows shell candidate to an absolute path that exists.
///
/// This is not cosmetic. `portable-pty` passes the program to `CreateProcessW`
/// as `lpApplicationName`, and Win32 does **not** search `PATH` for that
/// parameter: a bare `pwsh.exe` fails with `os error 2` ("The system cannot
/// find the file specified") even when PowerShell is installed, which the panel
/// then reports as `terminal.json`'s `createFailed`.
#[cfg(windows)]
fn resolve_windows_program(name: &str) -> Option<PathBuf> {
    let name = name.trim().trim_matches('"');
    // The well-known locations describe the plain shell names only: a name
    // that names a directory (`bin\pwsh.exe`) must not silently become
    // `%ProgramFiles%\PowerShell\7\pwsh.exe`.
    let locations = if Path::new(name).components().count() > 1 {
        Vec::new()
    } else {
        known_windows_locations(name)
    };
    resolve_windows_program_in(name, std::env::var_os("PATH").as_deref(), &locations)
}

/// The resolution rule with the environment injected, so the Windows-only
/// behaviour stays testable without touching the process `PATH`.
///
/// `path_env` is searched first (the OS's own order); `extra` holds the
/// well-known install locations and is the last resort. An absolute `name` is
/// never searched for: it either exists or the candidate is dropped.
#[cfg(windows)]
fn resolve_windows_program_in(
    name: &str,
    path_env: Option<&OsStr>,
    extra: &[PathBuf],
) -> Option<PathBuf> {
    let name = name.trim().trim_matches('"');
    if name.is_empty() {
        return None;
    }
    let path = Path::new(name);
    if path.is_absolute() {
        return is_executable_file(path).then(|| path.to_path_buf());
    }
    path_candidates(name, path_env)
        .into_iter()
        .chain(extra.iter().cloned())
        .find(|candidate| is_executable_file(candidate))
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
        // Windows must hand `CreateProcessW` an absolute path that exists, so
        // every candidate is resolved (and validated) first. On unix a bare
        // program name is fine: `std::process` resolves it through `PATH`, and
        // an absolute path is validated eagerly.
        #[cfg(windows)]
        let usable = resolve_windows_program(&path.to_string_lossy());
        #[cfg(not(windows))]
        let usable = if path.is_absolute() {
            is_executable_file(&path).then(|| path.clone())
        } else {
            Some(path.clone())
        };
        match usable {
            Some(path) => {
                return Ok(ShellChoice {
                    login: accepts_login_flags(&path),
                    path,
                    source,
                });
            }
            None => tried.push(path.display().to_string()),
        }
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
        if let Some(path) = resolve_windows_program(name) {
            paths.push(path);
        }
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
            choice.path.is_absolute(),
            "the resolved shell must be an absolute path — a bare program name \
             is what `CreateProcessW` cannot launch on Windows; got {:?}",
            choice.path
        );
        assert!(
            is_executable_file(&choice.path),
            "resolved shell must be executable: {:?}",
            choice.path
        );
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

/// Windows resolution rules. These carry the regression for "无法启动终端":
/// `portable-pty` hands the program name straight to `CreateProcessW`, which
/// never searches `PATH`, so a bare name must never survive resolution.
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::ffi::OsString;

    /// A `PATH` value built from explicit directories, so a test never has to
    /// mutate the process environment.
    fn path_list(directories: &[&Path]) -> OsString {
        std::env::join_paths(directories).expect("join PATH entries")
    }

    fn fake_shell(directory: &Path, name: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, b"").expect("create fake shell");
        path
    }

    #[test]
    fn bare_name_resolves_through_path_with_extension_appended() {
        let dir = tempfile::tempdir().expect("tempdir");
        let expected = fake_shell(dir.path(), "pwsh.exe");
        assert_eq!(
            resolve_windows_program_in("pwsh.exe", Some(&path_list(&[dir.path()])), &[]),
            Some(expected.clone())
        );
        // A name without an extension is what a user's `SHELL` usually holds.
        assert_eq!(
            resolve_windows_program_in("pwsh", Some(&path_list(&[dir.path()])), &[]),
            Some(expected)
        );
    }

    #[test]
    fn path_order_is_respected_and_a_missing_shell_falls_through() {
        let empty = tempfile::tempdir().expect("tempdir");
        let later = tempfile::tempdir().expect("tempdir");
        let expected = fake_shell(later.path(), "powershell.exe");
        assert_eq!(
            resolve_windows_program_in(
                "powershell.exe",
                Some(&path_list(&[empty.path(), later.path()])),
                &[]
            ),
            Some(expected)
        );
    }

    #[test]
    fn a_missing_shell_never_escapes_as_a_bare_name() {
        let empty = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            resolve_windows_program_in("pwsh.exe", Some(&path_list(&[empty.path()])), &[]),
            None,
            "an unresolved name must not reach CreateProcessW as lpApplicationName"
        );
        assert_eq!(resolve_windows_program_in("pwsh.exe", None, &[]), None);
    }

    #[test]
    fn well_known_locations_are_the_last_resort() {
        let dir = tempfile::tempdir().expect("tempdir");
        let installed = fake_shell(dir.path(), "pwsh.exe");
        assert_eq!(
            resolve_windows_program_in("pwsh.exe", None, std::slice::from_ref(&installed)),
            Some(installed)
        );
    }

    #[test]
    fn quoted_and_padded_path_entries_still_match() {
        let dir = tempfile::tempdir().expect("tempdir");
        let expected = fake_shell(dir.path(), "cmd.exe");
        let quoted = OsString::from(format!("\"{}\" ", dir.path().display()));
        assert_eq!(
            resolve_windows_program_in("cmd.exe", Some(&quoted), &[]),
            Some(expected)
        );
    }

    #[test]
    fn absolute_candidates_must_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let present = fake_shell(dir.path(), "shell.exe");
        assert_eq!(
            resolve_windows_program_in(&present.display().to_string(), None, &[]),
            Some(present)
        );
        assert_eq!(
            resolve_windows_program_in(
                &dir.path().join("missing.exe").display().to_string(),
                None,
                &[]
            ),
            None
        );
    }

    #[test]
    fn the_well_known_locations_name_the_installed_shells() {
        let locations = known_windows_locations("pwsh.exe");
        assert!(
            locations
                .iter()
                .any(|path| path.ends_with(r"PowerShell\7\pwsh.exe")),
            "PowerShell 7's default install must be offered: {locations:?}"
        );
        let powershell = known_windows_locations("powershell.exe");
        assert!(
            powershell
                .iter()
                .any(|path| path.ends_with(r"System32\WindowsPowerShell\v1.0\powershell.exe")),
            "Windows PowerShell 5.1 must be offered: {powershell:?}"
        );
        let cmd = known_windows_locations("cmd.exe");
        assert!(
            cmd.iter().any(|path| path.ends_with(r"System32\cmd.exe")),
            "cmd.exe must be offered: {cmd:?}"
        );
    }
}
