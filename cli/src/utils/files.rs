//! File helpers — port of `cli/src/utils/files.ts`.

use std::process::Stdio;

/// `assertReadableFile` — throws `"{label} not found at {path}."` (+ hint).
pub async fn assert_readable_file(
    path: &str,
    label: &str,
    hint: Option<&str>,
) -> Result<(), String> {
    if tokio::fs::try_exists(path).await.unwrap_or(false) {
        Ok(())
    } else {
        match hint {
            Some(hint) => Err(format!("{label} not found at {path}. {hint}")),
            None => Err(format!("{label} not found at {path}.")),
        }
    }
}

/// `assertExecutableFile` — throws `"{label} not found or not executable at {path}."`.
pub async fn assert_executable_file(path: &str, label: &str) -> Result<(), String> {
    if !can_access(path, X_OK).await {
        return Err(format!("{label} not found or not executable at {path}."));
    }
    Ok(())
}

/// Node `fs.constants` access modes (F_OK=0, X_OK=1, W_OK=2, R_OK=4).
pub const F_OK: u32 = 0;
pub const X_OK: u32 = 1;
pub const W_OK: u32 = 2;
pub const R_OK: u32 = 4;

/// `canAccess(path, mode)` — Node `fs.access` semantics. On Windows, Node
/// treats X_OK as F_OK (executability is not a file attribute), so any mode
/// check reduces to existence.
pub async fn can_access(path: &str, mode: u32) -> bool {
    // `meta` is only consumed by the `#[cfg(unix)]` permission check below —
    // on Windows the metadata call doubles as the existence check.
    #[cfg_attr(windows, allow(unused_variables))]
    let meta = match tokio::fs::metadata(path).await {
        Ok(meta) => meta,
        Err(_) => return false,
    };
    if mode == F_OK {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = meta.permissions().mode();
        if mode & X_OK != 0 && perm & 0o111 == 0 {
            return false;
        }
        if mode & W_OK != 0 && perm & 0o222 == 0 {
            return false;
        }
        if mode & R_OK != 0 && perm & 0o444 == 0 {
            return false;
        }
        true
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        true
    }
}

/// Platform lookup command for [`which`]: `where <name>.exe` on Windows,
/// `which <name>` elsewhere. `#[cfg]` (not `cfg!`) so the off-platform branch
/// is not compiled — and thus not counted by coverage — on the build target.
#[cfg(windows)]
fn which_command(name: &str) -> (&'static str, String) {
    ("where", format!("{name}.exe"))
}

/// Unix half of [`which_command`].
#[cfg(not(windows))]
fn which_command(name: &str) -> (&'static str, String) {
    ("which", name.to_string())
}

/// Outcome of a PATH lookup.
///
/// An `Option` loses a distinction its callers depend on: "the executable is
/// not installed" and "the lookup could not be *run*" (no `which`/`where` on
/// PATH, the spawn refused, ...) are different facts. Collapsing them made
/// `future doctor` report a failed probe as "not installed" — and downgrade the
/// missing future-agent from an issue to an optional-component note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathLookup {
    /// The lookup ran and resolved the name to this path.
    Found(String),
    /// The lookup ran and reported no match (`which`/`where` exited non-zero,
    /// or succeeded with empty stdout).
    NotFound,
    /// The lookup command itself could not be run; nothing is known about
    /// whether the name is installed.
    NotLookedUp(String),
}

/// `which(name)` — first PATH match for an executable. On Unix shells out to
/// `which`, on Windows to `where <name>.exe`, exactly like the TS version.
pub async fn which(name: &str) -> PathLookup {
    let (cmd, target) = which_command(name);
    let output = tokio::process::Command::new(cmd)
        .arg(&target)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;
    let output = match output {
        Ok(output) => output,
        Err(error) => return PathLookup::NotLookedUp(error.to_string()),
    };
    if !output.status.success() {
        return PathLookup::NotFound;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return PathLookup::NotFound;
    }
    // Take the first line (`which` may print multiple matches on Windows).
    PathLookup::Found(
        trimmed
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host shell, one `#[cfg]` test per platform: a `cfg!` pair inside one
    /// body leaves the other platform's tuple as a never-executed line in this
    /// report (the same shape `external_editor::host_dialect_matches_the_platform`
    /// uses). `which` reads PATH — serialize against tests that repoint it.
    #[cfg(windows)]
    #[tokio::test]
    async fn which_finds_the_host_shell() {
        let _guard = crate::test_env::lock_env().await;
        let PathLookup::Found(found) = which("cmd").await else {
            panic!("`which cmd` should resolve on Windows");
        };
        assert!(found.contains("cmd"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn which_finds_the_host_shell() {
        let _guard = crate::test_env::lock_env().await;
        let PathLookup::Found(found) = which("sh").await else {
            panic!("`which sh` should resolve on POSIX");
        };
        assert!(found.contains("sh"));
    }

    #[tokio::test]
    async fn which_missing_is_not_found() {
        let _guard = crate::test_env::lock_env().await;
        assert_eq!(
            which("definitely-not-a-real-binary-xyz").await,
            PathLookup::NotFound
        );
    }

    /// A lookup that could not be *run* is not a missing executable: the two
    /// used to be the same `None`, which is how `future doctor` turned its own
    /// failed probe into "not installed".
    #[cfg(unix)]
    #[tokio::test]
    async fn which_without_a_lookup_command_is_not_looked_up() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        // A real directory, with no `which` in it: the lookup command itself
        // cannot be spawned, so nothing is known about `sh`.
        let empty = dir.path().join("bin");
        std::fs::create_dir_all(&empty).unwrap();
        let _env = crate::test_env::EnvGuard::set(&[("PATH", empty.as_os_str().to_owned())]);
        match which("sh").await {
            PathLookup::NotLookedUp(error) => assert!(!error.is_empty()),
            other => panic!("a lookup that cannot run is not a look-up result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn assert_readable_file_errors() {
        let err = assert_readable_file("/no/such/file", "Config", None)
            .await
            .unwrap_err();
        assert_eq!(err, "Config not found at /no/such/file.");
        let err = assert_readable_file("/no/such/file", "Config", Some("Check the path."))
            .await
            .unwrap_err();
        assert_eq!(err, "Config not found at /no/such/file. Check the path.");
    }

    #[tokio::test]
    async fn assert_readable_file_ok_when_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("present.txt");
        tokio::fs::write(&path, "x").await.expect("write");
        assert!(
            assert_readable_file(path.to_str().expect("utf8"), "Config", None)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn assert_executable_file_checks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tool.sh");
        tokio::fs::write(&path, "#!/bin/sh\n").await.expect("write");
        let missing = dir.path().join("nope.sh");
        let err = assert_executable_file(missing.to_str().expect("utf8"), "Tool")
            .await
            .unwrap_err();
        assert!(err.starts_with("Tool not found or not executable at "));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Not executable yet → error.
            let err = assert_executable_file(path.to_str().expect("utf8"), "Tool")
                .await
                .unwrap_err();
            assert!(err.starts_with("Tool not found or not executable at "));
            // chmod +x → ok.
            let mut perms = std::fs::metadata(&path).expect("meta").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).expect("chmod");
        }
        assert!(assert_executable_file(path.to_str().expect("utf8"), "Tool")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn can_access_modes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("f.txt");
        tokio::fs::write(&path, "x").await.expect("write");
        let path = path.to_str().expect("utf8");
        assert!(can_access(path, F_OK).await);
        assert!(!can_access("/no/such/file", F_OK).await);
        assert!(can_access(path, R_OK).await);
        assert!(can_access(path, W_OK).await);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Strip all permissions → R_OK/W_OK/X_OK all fail, F_OK still ok.
            let mut perms = std::fs::metadata(path).expect("meta").permissions();
            perms.set_mode(0o000);
            std::fs::set_permissions(path, perms).expect("chmod");
            assert!(can_access(path, F_OK).await);
            assert!(!can_access(path, R_OK).await);
            assert!(!can_access(path, W_OK).await);
            assert!(!can_access(path, X_OK).await);
            let mut perms = std::fs::metadata(path).expect("meta").permissions();
            perms.set_mode(0o644);
            std::fs::set_permissions(path, perms).expect("chmod");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn which_empty_output_is_not_found() {
        let _guard = crate::test_env::lock_env().await;
        // A fake `which` that succeeds with EMPTY stdout → None.
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("which");
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut paths = vec![dir.path().to_path_buf()];
        if let Some(p) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&p));
        }
        let _env =
            crate::test_env::EnvGuard::set(&[("PATH", std::env::join_paths(paths).unwrap())]);
        assert_eq!(which("anything").await, PathLookup::NotFound);
    }
}
