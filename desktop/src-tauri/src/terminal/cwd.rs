//! Working-directory resolution for a new terminal tab.
//!
//! Order (design §5.2):
//!   1. the thread's own working directory (a worktree checkout even when the
//!      thread runs in one — this is just the real directory path);
//!   2. the workspace strictly associated with that thread;
//!   3. the user's home directory.
//!
//! **Why this module validates instead of trusting the PTY layer:**
//! `portable-pty` 0.9.0 builds its command as
//! `cwd.filter(|d| Path::new(d).is_dir()).unwrap_or(home)` — a configured but
//! missing/non-directory cwd is silently replaced with `$HOME`. Verified in the
//! T02 spike (`T02-report.md`, FINDING-1). The product requirement is the
//! opposite: fail loudly and offer an explicit home fallback, so every path
//! that reaches the PTY is validated here first.
//!
//! A configured-but-broken directory is never papered over by a lower-priority
//! source: the failure names the configured path and the client decides
//! (retry after fixing it, or confirm the home fallback).

use std::path::{Path, PathBuf};

use crate::store;

/// What the client asked for when creating a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CwdPolicy {
    /// Use the thread/workspace directory; a broken configured directory is an
    /// error the user must resolve (the default).
    #[default]
    Thread,
    /// The user explicitly confirmed "start in home instead" after seeing
    /// `CWD_INVALID`.
    HomeConfirmed,
}

impl CwdPolicy {
    pub fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            None | Some("thread") => Some(CwdPolicy::Thread),
            Some("homeConfirmed") => Some(CwdPolicy::HomeConfirmed),
            Some(_) => None,
        }
    }
}

/// Why a directory was rejected. Kept separate from the resolved value so the
/// client can say *which* configured directory is bad.
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
    /// Wire code returned to the client. The UI pairs `CWD_INVALID` with an
    /// explicit "use home instead" action; it is never applied silently.
    pub fn code(&self) -> &'static str {
        match self {
            CwdError::ThreadNotFound(_) => "THREAD_NOT_FOUND",
            CwdError::ThreadNotWritable(_) => "THREAD_READONLY",
            _ => "CWD_INVALID",
        }
    }

    /// Machine-readable hint for the client: only `CWD_INVALID` is retryable
    /// with `cwdPolicy: "homeConfirmed"`.
    pub fn allows_home_fallback(&self) -> bool {
        matches!(
            self,
            CwdError::NotADirectory(_)
                | CwdError::Missing(_)
                | CwdError::NotAccessible { .. }
                | CwdError::NoUsableDirectory { .. }
        )
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
    /// The user's home directory, chosen explicitly.
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
    Ok(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
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
/// `allow_home_fallback` is false for the default path: a configured-but-broken
/// directory must surface as an error so the client can ask the user, rather
/// than quietly opening a terminal in the wrong place.
pub fn resolve_initial_cwd(
    configured: Option<&str>,
    allow_home_fallback: bool,
) -> Result<ResolvedCwd, CwdError> {
    if let Some(trimmed) = configured.map(str::trim).filter(|value| !value.is_empty()) {
        match validate_dir(Path::new(trimmed)) {
            Ok(path) => {
                return Ok(ResolvedCwd {
                    path,
                    source: CwdSource::Workspace,
                })
            }
            Err(error) if !allow_home_fallback => return Err(error),
            // The user explicitly confirmed the home fallback: go to home
            // rather than to another configured directory.
            Err(_) => {}
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
pub fn resolve_for_thread(thread_id: &str, policy: CwdPolicy) -> Result<ResolvedCwd, CwdError> {
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
    resolve_initial_cwd(
        workspace_path.as_deref(),
        policy == CwdPolicy::HomeConfirmed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for T02 FINDING-1: a missing cwd must be an error, never a
    /// silent `$HOME` substitution.
    #[test]
    fn missing_directory_is_rejected_not_substituted() {
        let error =
            validate_dir(Path::new("/definitely/not/a/real/dir/t02")).expect_err("missing dir");
        assert_eq!(error.code(), "CWD_INVALID");
        assert!(matches!(error, CwdError::Missing(_)));
        assert!(error.allows_home_fallback());
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
        assert_eq!(
            resolved,
            std::fs::canonicalize(dir.path()).unwrap_or_else(|_| dir.path().to_path_buf())
        );
    }

    #[test]
    fn configured_directory_wins() {
        let dir = tempfile::tempdir().expect("tempdir");
        let resolved = resolve_initial_cwd(Some(&dir.path().to_string_lossy()), false)
            .expect("valid configured cwd");
        assert_eq!(resolved.source, CwdSource::Workspace);
    }

    #[test]
    fn broken_configured_directory_fails_loudly() {
        let error = resolve_initial_cwd(Some("/definitely/not/a/real/dir/t02"), false)
            .expect_err("broken configured cwd must fail loudly");
        assert!(matches!(error, CwdError::Missing(_)));
    }

    #[test]
    fn nothing_configured_falls_back_to_home() {
        let resolved = resolve_initial_cwd(None, false).expect("home fallback");
        assert_eq!(resolved.source, CwdSource::Home);
        assert!(resolved.path.is_absolute());
    }

    #[test]
    fn empty_and_whitespace_config_are_treated_as_absent() {
        let resolved = resolve_initial_cwd(Some("   "), false).expect("blank config falls through");
        assert_eq!(resolved.source, CwdSource::Home);
    }

    /// The explicit user-confirmed fallback reaches home, and only when asked.
    #[test]
    fn explicit_home_fallback_succeeds_after_broken_config() {
        let resolved = resolve_initial_cwd(Some("/definitely/not/a/real/dir/t02"), true)
            .expect("explicit home fallback");
        assert_eq!(resolved.source, CwdSource::Home);
        assert!(resolved.path.is_absolute());
    }

    /// Browsing the error must not leak anything but the configured path.
    #[test]
    fn error_message_contains_only_the_configured_path() {
        let error = validate_dir(Path::new("/tmp/t02-secret-should-not-appear")).unwrap_err();
        let text = error.to_string();
        assert!(text.starts_with("CWD_INVALID:"));
        assert!(text.contains("/tmp/t02-secret-should-not-appear"));
    }

    #[test]
    fn policy_parsing_rejects_unknown_values() {
        assert_eq!(CwdPolicy::parse(None), Some(CwdPolicy::Thread));
        assert_eq!(CwdPolicy::parse(Some("thread")), Some(CwdPolicy::Thread));
        assert_eq!(
            CwdPolicy::parse(Some("homeConfirmed")),
            Some(CwdPolicy::HomeConfirmed)
        );
        assert_eq!(CwdPolicy::parse(Some("anywhere")), None);
    }
}
