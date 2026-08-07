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
}
