//! Screenshot writer — port of `cli/src/browser/artifacts/screenshot-writer.ts`.
//!
//! Protocol-agnostic path resolution and file writing with a fallback to the
//! artifacts directory when the explicit parent cannot be created.

use std::path::{Path, PathBuf};

/// `ScreenshotWriteResult` — `{ path, filename }`.
#[derive(Debug, Clone)]
pub struct ScreenshotWriteResult {
    pub path: String,
    pub filename: String,
}

/// `FUTURE_HOME`-derived artifacts dir: `~/.future/agent/browser/artifacts`.
pub fn artifacts_dir() -> PathBuf {
    browser_dir().join("artifacts")
}

/// `~/.future/agent/browser` (honors `FUTURE_HOME`).
pub fn browser_dir() -> PathBuf {
    let future_home = std::env::var("FUTURE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".future"));
    future_home.join("agent").join("browser")
}

/// `resolveScreenshotPath(explicitPath?)` — timestamped default in artifacts.
pub fn resolve_screenshot_path(explicit_path: Option<&str>) -> String {
    match explicit_path {
        Some(p) => p.to_string(),
        None => {
            // new Date().toISOString().replace(/[:.]/g, "-")
            let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            let ts = ts.replace([':', '.'], "-");
            artifacts_dir()
                .join(format!("browser-{ts}.png"))
                .display()
                .to_string()
        }
    }
}

/// `writeScreenshot(bytes, resolvedPath)` — try the parent, fall back to
/// the artifacts dir on failure.
pub async fn write_screenshot(
    bytes: &[u8],
    resolved_path: &str,
) -> Result<ScreenshotWriteResult, String> {
    let path = Path::new(resolved_path);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let filename = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();

    let mkdir_ok = tokio::fs::create_dir_all(parent).await.is_ok();
    let write_ok = if mkdir_ok {
        tokio::fs::write(path, bytes).await.is_ok()
    } else {
        false
    };
    if write_ok {
        return Ok(ScreenshotWriteResult {
            path: resolved_path.to_string(),
            filename,
        });
    }

    // Fallback: write to artifacts dir
    let fallback_dir = artifacts_dir();
    tokio::fs::create_dir_all(&fallback_dir)
        .await
        .map_err(|e| e.to_string())?;
    let fallback_path = fallback_dir.join(&filename);
    tokio::fs::write(&fallback_path, bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ScreenshotWriteResult {
        path: fallback_path.display().to_string(),
        filename,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_path_used_as_is() {
        assert_eq!(
            resolve_screenshot_path(Some("/tmp/my-shot.png")),
            "/tmp/my-shot.png"
        );
    }

    #[test]
    fn undefined_generates_timestamped_path() {
        let path = resolve_screenshot_path(None);
        assert!(path.contains("browser-"));
        assert!(path.ends_with(".png"));
    }

    #[test]
    fn generated_path_contains_no_colons() {
        let path = resolve_screenshot_path(None);
        assert!(!path.contains(':'));
    }

    #[tokio::test]
    async fn browser_dir_honors_future_home_env() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_HOME",
            dir.path().as_os_str().to_os_string(),
        )]);
        assert_eq!(browser_dir(), dir.path().join("agent").join("browser"));
        assert_eq!(
            artifacts_dir(),
            dir.path().join("agent").join("browser").join("artifacts")
        );
        drop(_env);
        // Without FUTURE_HOME it derives from the home directory.
        let expected = dirs::home_dir()
            .unwrap_or_default()
            .join(".future")
            .join("agent")
            .join("browser");
        assert_eq!(browser_dir(), expected);
    }

    #[tokio::test]
    async fn write_screenshot_to_explicit_path() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_HOME",
            dir.path().as_os_str().to_os_string(),
        )]);
        let target = dir.path().join("shots").join("one.png");
        let result = write_screenshot(b"png-bytes", target.to_str().expect("utf8"))
            .await
            .expect("write");
        assert_eq!(result.path, target.display().to_string());
        assert_eq!(result.filename, "one.png");
        assert_eq!(tokio::fs::read(&target).await.expect("read"), b"png-bytes");
    }

    #[tokio::test]
    async fn write_screenshot_falls_back_to_artifacts_dir() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_HOME",
            dir.path().as_os_str().to_os_string(),
        )]);
        // Parent is a regular FILE → create_dir_all fails → fallback.
        let blocker = dir.path().join("blocker");
        tokio::fs::write(&blocker, "x").await.expect("write");
        let target = blocker.join("two.png");
        let result = write_screenshot(b"png-bytes", target.to_str().expect("utf8"))
            .await
            .expect("fallback write");
        assert_eq!(result.filename, "two.png");
        assert!(result.path.contains("artifacts"));
        let written = tokio::fs::read(&result.path).await.expect("read");
        assert_eq!(written, b"png-bytes");
    }

    #[tokio::test]
    async fn write_screenshot_fallback_failures_surface() {
        let _guard = crate::test_env::lock_env().await;

        // FUTURE_HOME is a regular FILE → the artifacts dir cannot be
        // created → create_dir_all error propagates.
        let tmp = tempfile::tempdir().expect("tempdir");
        let home_file = tmp.path().join("home-file");
        tokio::fs::write(&home_file, "x").await.expect("write");
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_HOME",
            home_file.as_os_str().to_os_string(),
        )]);
        // Explicit target also unwritable (parent is a file).
        let target = home_file.join("x.png");
        let err = write_screenshot(b"b", target.to_str().expect("utf8"))
            .await
            .unwrap_err();
        assert!(!err.is_empty());
        drop(_env);

        // Artifacts dir exists but the fallback FILE path is a directory.
        let tmp2 = tempfile::tempdir().expect("tempdir");
        let _env2 = crate::test_env::EnvGuard::set(&[(
            "FUTURE_HOME",
            tmp2.path().as_os_str().to_os_string(),
        )]);
        tokio::fs::create_dir_all(artifacts_dir().join("shot.png"))
            .await
            .expect("mkdir");
        let blocker = tmp2.path().join("blocker");
        tokio::fs::write(&blocker, "x").await.expect("write");
        let target = blocker.join("shot.png");
        let err = write_screenshot(b"b", target.to_str().expect("utf8"))
            .await
            .unwrap_err();
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn write_screenshot_filename_defaults_for_rootish_paths() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_HOME",
            dir.path().as_os_str().to_os_string(),
        )]);
        // A bare filename has no parent component → parent "." is used.
        let cwd = dir.path().join("cwd");
        tokio::fs::create_dir_all(&cwd).await.expect("mkdir");
        let _cwd_guard = CwdGuard::enter(&cwd);
        let result = write_screenshot(b"x", "bare.png").await.expect("write");
        assert_eq!(result.filename, "bare.png");
        assert!(cwd.join("bare.png").exists());
    }

    /// Restore the process CWD on drop (tests share one process).
    struct CwdGuard(std::path::PathBuf);

    impl CwdGuard {
        fn enter(dir: &Path) -> Self {
            let original = std::env::current_dir().expect("cwd");
            std::env::set_current_dir(dir).expect("chdir");
            CwdGuard(original)
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.0).expect("restore cwd");
        }
    }
}
