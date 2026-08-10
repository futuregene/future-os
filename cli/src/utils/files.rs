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

/// `which(name)` — first PATH match for an executable. On Unix shells out to
/// `which`, on Windows to `where <name>.exe`, exactly like the TS version.
pub async fn which(name: &str) -> Option<String> {
    let (cmd, target) = which_command(name);
    let output = tokio::process::Command::new(cmd)
        .arg(&target)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Take the first line (`which` may print multiple matches on Windows).
    trimmed.lines().next().map(|line| line.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn which_finds_shell() {
        // `which` reads PATH — serialize against tests that repoint it.
        let _guard = crate::test_env::lock_env().await;
        let found = which("sh").await;
        assert!(found.is_some(), "`which sh` should resolve on this host");
        assert!(found.as_deref().unwrap_or("").contains("sh"));
    }

    #[tokio::test]
    async fn which_missing_returns_none() {
        let _guard = crate::test_env::lock_env().await;
        assert!(which("definitely-not-a-real-binary-xyz").await.is_none());
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
        assert!(assert_readable_file(path.to_str().expect("utf8"), "Config", None)
            .await
            .is_ok());
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
}
