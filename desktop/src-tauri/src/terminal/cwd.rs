//! Working-directory resolution for a new terminal tab.
//!
//! Order:
//!   1. the directory of the workspace strictly associated with the thread —
//!      for a standalone chat session that temporary workspace IS the session's
//!      own directory;
//!   2. the user's home directory, when the configured directory is missing or
//!      otherwise unusable.
//!
//! **Why this module validates instead of trusting the PTY layer:**
//! `portable-pty` 0.9.0 builds its command as
//! `cwd.filter(|d| Path::new(d).is_dir()).unwrap_or(home)` — a configured but
//! missing/non-directory cwd is silently replaced with `$HOME`. Verified in the
//! T02 spike (`T02-report.md`, FINDING-1). Here the home fallback is a
//! deliberate choice made before spawn and the resolved directory travels back
//! in the session info, so a skipped configured path stays visible instead of
//! silent.

use std::path::{Path, PathBuf};

use crate::store;

/// Why a directory was rejected. Kept separate from the resolved value so
/// diagnostics can name *which* configured directory is bad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwdError {
    /// The thread does not exist (or was deleted) in the local store.
    ThreadNotFound(String),
    /// The thread is a read-only conversation: no terminal may write to it.
    ThreadNotWritable(String),
    /// The configured path exists but is not a directory.
    NotADirectory(PathBuf),
    /// The configured path does not exist.
    Missing(PathBuf),
    /// The path exists and is a directory, but cannot be entered.
    NotAccessible { path: PathBuf, reason: String },
    /// Nothing usable was configured and even the home directory is unusable.
    NoUsableDirectory { home: Option<PathBuf> },
}

impl CwdError {
    /// Wire code returned to the client. Only the thread errors and
    /// `NoUsableDirectory` still reach a client: a broken configured directory
    /// now resolves to home instead of surfacing as `CWD_INVALID`.
    pub fn code(&self) -> &'static str {
        match self {
            CwdError::ThreadNotFound(_) => "THREAD_NOT_FOUND",
            CwdError::ThreadNotWritable(_) => "THREAD_READONLY",
            _ => "CWD_INVALID",
        }
    }
}

impl std::fmt::Display for CwdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CwdError::ThreadNotFound(id) => {
                write!(f, "THREAD_NOT_FOUND: no conversation {id}")
            }
            CwdError::ThreadNotWritable(id) => write!(
                f,
                "THREAD_READONLY: conversation {id} does not accept terminal tabs"
            ),
            CwdError::NotADirectory(path) => write!(
                f,
                "CWD_INVALID: {} exists but is not a directory",
                path.display()
            ),
            CwdError::Missing(path) => {
                write!(f, "CWD_INVALID: {} does not exist", path.display())
            }
            CwdError::NotAccessible { path, reason } => write!(
                f,
                "CWD_INVALID: {} cannot be entered ({reason})",
                path.display()
            ),
            CwdError::NoUsableDirectory { home } => match home {
                Some(home) => write!(
                    f,
                    "CWD_INVALID: no usable working directory (home {} is unusable)",
                    home.display()
                ),
                None => write!(
                    f,
                    "CWD_INVALID: no usable working directory and no home directory"
                ),
            },
        }
    }
}

/// Where the resolved directory came from. Surfaced in diagnostics so a
/// wrong-directory bug is visible instead of silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CwdSource {
    /// The workspace the thread belongs to (including a chat's temp workspace).
    Workspace,
    /// The user's home directory — the fallback when nothing usable is
    /// configured.
    Home,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCwd {
    pub path: PathBuf,
    pub source: CwdSource,
}

/// Validate a directory that is about to become a PTY cwd.
///
/// Returns the canonicalized path so later `cd`-relative work has a stable
/// base. Rejects a non-existent path, a non-directory, and a directory that
/// cannot be used as a working directory.
pub fn validate_dir(path: &Path) -> Result<PathBuf, CwdError> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(CwdError::Missing(path.to_path_buf()));
        }
        Err(error) => {
            return Err(CwdError::NotAccessible {
                path: path.to_path_buf(),
                reason: error.kind().to_string(),
            });
        }
    };
    if !meta.is_dir() {
        return Err(CwdError::NotADirectory(path.to_path_buf()));
    }
    // `metadata` succeeds on an unreadable directory (search permission is what
    // matters); prove it is actually enterable.
    if let Err(error) = std::fs::read_dir(path) {
        return Err(CwdError::NotAccessible {
            path: path.to_path_buf(),
            reason: error.kind().to_string(),
        });
    }
    // Windows `Path::canonicalize` returns the extended-length spelling
    // (`\\?\C:\...`). That form is fine for the filesystem API but breaks
    // shells: cmd.exe and Windows PowerShell fail to resolve relative paths
    // from a verbatim working directory ("The filename, directory name, or
    // volume label syntax is incorrect"), so the child must get the ordinary
    // spelling. The store already strips the same prefix for the same reason
    // (`store::strip_verbatim_prefix`); POSIX paths never carry it, so this is
    // a no-op off Windows.
    Ok(store::strip_verbatim_prefix(
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
    ))
}

fn home_dir() -> Option<PathBuf> {
    let raw = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    if raw.is_empty() {
        return None;
    }
    Some(PathBuf::from(raw))
}

/// Resolve a directory from a candidate source.
///
/// A configured-but-broken directory falls back to home: the shell must start
/// somewhere, and home is the predictable choice. The skipped path is logged so
/// the fallback stays diagnosable, and the resolved directory is reported back
/// in the session info.
pub fn resolve_initial_cwd(configured: Option<&str>) -> Result<ResolvedCwd, CwdError> {
    if let Some(trimmed) = configured.map(str::trim).filter(|value| !value.is_empty()) {
        match validate_dir(Path::new(trimmed)) {
            Ok(path) => {
                return Ok(ResolvedCwd {
                    path,
                    source: CwdSource::Workspace,
                })
            }
            Err(error) => {
                eprintln!(
                    "FutureOS: terminal cwd {trimmed} is unusable ({error}); starting in home"
                );
            }
        }
    }

    let home = home_dir();
    if let Some(home) = home.as_ref() {
        if let Ok(path) = validate_dir(home) {
            return Ok(ResolvedCwd {
                path,
                source: CwdSource::Home,
            });
        }
    }
    Err(CwdError::NoUsableDirectory { home })
}

/// Resolve the working directory for a terminal tab opened from a conversation.
///
/// The client only ever sends a thread id; the directory is derived here from
/// the local store so a stale or hostile client cannot point a shell at an
/// arbitrary path. The workspace is looked up by the thread's own
/// `workspace_id` (never by "the currently active workspace"), so a chat's
/// temporary workspace and a real workspace can never be confused.
pub fn resolve_for_thread(thread_id: &str) -> Result<ResolvedCwd, CwdError> {
    let thread = store::get_thread(thread_id)
        .map_err(|error| CwdError::ThreadNotFound(format!("{thread_id}: {error}")))?
        .ok_or_else(|| CwdError::ThreadNotFound(thread_id.to_string()))?;
    if thread.readonly || thread.deleted_at.is_some() {
        return Err(CwdError::ThreadNotWritable(thread_id.to_string()));
    }
    let workspace_path = store::get_workspace(&thread.workspace_id)
        .ok()
        .flatten()
        .map(|workspace| workspace.path);
    resolve_initial_cwd(workspace_path.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for T02 FINDING-1: a missing cwd must be rejected by
    /// validation, never silently passed through for a `$HOME` substitution.
    #[test]
    fn missing_directory_is_rejected_not_substituted() {
        let error =
            validate_dir(Path::new("/definitely/not/a/real/dir/t02")).expect_err("missing dir");
        assert_eq!(error.code(), "CWD_INVALID");
        assert!(matches!(error, CwdError::Missing(_)));
    }

    #[test]
    fn a_file_is_not_a_valid_working_directory() {
        let file = std::env::current_exe().expect("test binary path");
        let error = validate_dir(&file).expect_err("a file must not validate");
        assert!(matches!(error, CwdError::NotADirectory(_)));
    }

    #[test]
    fn real_directory_validates_and_is_canonicalized() {
        let dir = tempfile::tempdir().expect("tempdir");
        let resolved = validate_dir(dir.path()).expect("tempdir validates");
        assert!(resolved.is_absolute());
        // Canonicalized, but never in the Windows extended-length spelling:
        // a verbatim cwd breaks cmd.exe/PowerShell relative-path resolution,
        // so the child shell must get the ordinary form.
        assert_eq!(
            resolved,
            store::strip_verbatim_prefix(
                std::fs::canonicalize(dir.path()).unwrap_or_else(|_| dir.path().to_path_buf())
            )
        );
        assert!(
            !resolved.to_string_lossy().starts_with(r"\\?\"),
            "the verbatim prefix must not reach the PTY cwd: {}",
            resolved.display()
        );
    }

    #[test]
    fn configured_directory_wins() {
        // `HOME`/`USERPROFILE` are process-global, so this test takes the same
        // lock as the fixture that temporarily blanks or replaces them.
        let _lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let resolved =
            resolve_initial_cwd(Some(&dir.path().to_string_lossy())).expect("valid configured cwd");
        assert_eq!(resolved.source, CwdSource::Workspace);
    }

    /// A configured-but-broken directory starts the shell in home instead of
    /// erroring: the shell must land somewhere predictable.
    #[test]
    fn broken_configured_directory_falls_back_to_home() {
        // `HOME`/`USERPROFILE` are process-global, so this test takes the same
        // lock as the fixture that temporarily blanks or replaces them.
        let _lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let resolved = resolve_initial_cwd(Some("/definitely/not/a/real/dir/t02"))
            .expect("broken configured cwd falls back to home");
        assert_eq!(resolved.source, CwdSource::Home);
        assert!(resolved.path.is_absolute());
    }

    #[test]
    fn nothing_configured_falls_back_to_home() {
        // `HOME`/`USERPROFILE` are process-global, so this test takes the same
        // lock as the fixture that temporarily blanks or replaces them.
        let _lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let resolved = resolve_initial_cwd(None).expect("home fallback");
        assert_eq!(resolved.source, CwdSource::Home);
        assert!(resolved.path.is_absolute());
    }

    #[test]
    fn empty_and_whitespace_config_are_treated_as_absent() {
        // `HOME`/`USERPROFILE` are process-global, so this test takes the same
        // lock as the fixture that temporarily blanks or replaces them.
        let _lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let resolved = resolve_initial_cwd(Some("   ")).expect("blank config falls through");
        assert_eq!(resolved.source, CwdSource::Home);
    }

    /// Browsing the error must not leak anything but the configured path.
    #[test]
    fn error_message_contains_only_the_configured_path() {
        let error = validate_dir(Path::new("/tmp/t02-secret-should-not-appear")).unwrap_err();
        let text = error.to_string();
        assert!(text.starts_with("CWD_INVALID:"));
        assert!(text.contains("/tmp/t02-secret-should-not-appear"));
    }

    /// Every variant carries its own wire code: only the two thread errors and
    /// a home-less machine are distinguished, the rest collapse to
    /// `CWD_INVALID` so the client keeps one branch for a broken directory.
    #[test]
    fn error_codes_are_stable_per_variant() {
        assert_eq!(
            CwdError::ThreadNotFound("t".into()).code(),
            "THREAD_NOT_FOUND"
        );
        assert_eq!(
            CwdError::ThreadNotWritable("t".into()).code(),
            "THREAD_READONLY"
        );
        assert_eq!(
            CwdError::NotADirectory(PathBuf::from("/x")).code(),
            "CWD_INVALID"
        );
        assert_eq!(CwdError::Missing(PathBuf::from("/x")).code(), "CWD_INVALID");
        assert_eq!(
            CwdError::NotAccessible {
                path: PathBuf::from("/x"),
                reason: "denied".into()
            }
            .code(),
            "CWD_INVALID"
        );
        assert_eq!(
            CwdError::NoUsableDirectory { home: None }.code(),
            "CWD_INVALID"
        );
    }

    /// Every variant's message names the offending input (path, conversation or
    /// the OS reason), so a silent fallback stays diagnosable.
    #[test]
    fn error_messages_name_the_offending_input() {
        let cases = [
            (
                CwdError::ThreadNotFound("thread-7".into()),
                "THREAD_NOT_FOUND: no conversation thread-7",
            ),
            (
                CwdError::ThreadNotWritable("thread-7".into()),
                "THREAD_READONLY: conversation thread-7 does not accept terminal tabs",
            ),
            (
                CwdError::NotADirectory(PathBuf::from("/tmp/a-file")),
                "CWD_INVALID: /tmp/a-file exists but is not a directory",
            ),
            (
                CwdError::Missing(PathBuf::from("/tmp/absent")),
                "CWD_INVALID: /tmp/absent does not exist",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
        assert_eq!(
            CwdError::NotAccessible {
                path: PathBuf::from("/root/private"),
                reason: "permission denied".into(),
            }
            .to_string(),
            "CWD_INVALID: /root/private cannot be entered (permission denied)"
        );
        assert_eq!(
            CwdError::NoUsableDirectory {
                home: Some(PathBuf::from("/no-home"))
            }
            .to_string(),
            "CWD_INVALID: no usable working directory (home /no-home is unusable)"
        );
        assert_eq!(
            CwdError::NoUsableDirectory { home: None }.to_string(),
            "CWD_INVALID: no usable working directory and no home directory"
        );
    }

    /// A path the OS layer refuses outright (an interior NUL) is reported as
    /// `NotAccessible` with the OS reason — not as a bogus "missing" and not as
    /// a panic.
    #[test]
    fn an_unrepresentable_path_is_reported_as_not_accessible() {
        let path = Path::new("nul\0dir");
        let error = validate_dir(path).expect_err("an unrepresentable path must not validate");
        assert!(matches!(error, CwdError::NotAccessible { .. }), "{error:?}");
        assert!(error.to_string().contains("cannot be entered"));
    }

    /// A directory that exists but cannot be listed is rejected rather than
    /// handed to the shell as a working directory.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_directory_is_not_accessible() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o000)).unwrap();
        let error = validate_dir(dir.path()).expect_err("an unreadable dir must not validate");
        // Restore before asserting so a failed assertion still cleans up.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(error, CwdError::NotAccessible { .. }), "{error:?}");
    }

    /// `HOME`/`USERPROFILE` are process-global, so this test holds the same lock
    /// the `HomeGuard` fixture uses and restores both variables immediately.
    #[test]
    fn an_unusable_home_falls_through_to_an_error() {
        let _lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let saved_home = std::env::var_os("HOME");
        let saved_profile = std::env::var_os("USERPROFILE");

        // A file is a home that exists but can never be a working directory.
        let file = std::env::current_exe().expect("test binary path");
        std::env::set_var("HOME", &file);
        std::env::set_var("USERPROFILE", &file);
        let error = resolve_initial_cwd(None).expect_err("a file home is unusable");
        assert!(
            matches!(error, CwdError::NoUsableDirectory { home: Some(_) }),
            "{error:?}"
        );
        assert!(error.to_string().contains("is unusable"));

        // An empty home environment counts as absent, not as a relative cwd.
        std::env::set_var("HOME", "");
        std::env::set_var("USERPROFILE", "");
        assert!(home_dir().is_none());
        let error = resolve_initial_cwd(None).expect_err("no home at all");
        assert!(matches!(error, CwdError::NoUsableDirectory { home: None }));
        assert_eq!(error.code(), "CWD_INVALID");

        match saved_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match saved_profile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
    }

    /// The store is the only source of a terminal's directory: the thread's
    /// workspace path wins, and a conversation that no longer accepts tabs is
    /// refused by id (never silently pointed at another directory).
    #[test]
    fn a_thread_resolves_to_its_own_workspace_and_is_refused_once_deleted() {
        use crate::auth_store::test_support::HomeGuard;

        let _home = HomeGuard::new("terminal_cwd_thread");
        crate::store::initialize_app_store().expect("init store");
        let dir = std::env::temp_dir().join(format!("futureos-cwd-thread-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        let workspace = crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("CWD".into()),
            path: dir.display().to_string(),
            description: None,
            create_directory: Some(true),
        })
        .expect("create workspace");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "workspace".into(),
            title: Some("CWD".into()),
            workspace_id: Some(workspace.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create thread");

        let resolved = resolve_for_thread(&thread.id).expect("the workspace is the cwd");
        assert_eq!(resolved.source, CwdSource::Workspace);
        assert_eq!(
            resolved.path,
            crate::store::strip_verbatim_prefix(std::fs::canonicalize(&dir).expect("canonical"))
        );

        // An unknown conversation is a lookup failure, not a home fallback.
        let error = resolve_for_thread("ghost-thread").expect_err("unknown thread");
        assert!(matches!(error, CwdError::ThreadNotFound(_)), "{error:?}");
        assert_eq!(error.code(), "THREAD_NOT_FOUND");

        // A deleted conversation must not accept a new tab. `delete_thread`
        // hard-deletes the row (it does not set `deleted_at`), so the refusal
        // is the lookup failure — the `readonly`/`deleted_at` guard above it
        // belongs to the `ThreadNotWritable` arm, which no store API can reach
        // (nothing ever writes `readonly = 1`; see the waiver ledger).
        crate::store::delete_thread(&thread.id).expect("delete thread");
        let error = resolve_for_thread(&thread.id).expect_err("deleted thread");
        assert!(matches!(error, CwdError::ThreadNotFound(_)), "{error:?}");
        assert_eq!(error.code(), "THREAD_NOT_FOUND");
    }
}
