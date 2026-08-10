//! Browser discovery — port of `cli/src/browser/browser-discovery.ts`.
//!
//! Find installed Chrome/Edge/Chromium in priority order: Chrome → Edge →
//! Chromium. Accepts an optional user-specified path (executablePath arg).

/// `ChromiumBrowserKind`.
pub type ChromiumBrowserKind = &'static str;

/// `BrowserExecutable`.
pub struct BrowserExecutable {
    pub kind: ChromiumBrowserKind,
    pub executable_path: String,
}

/// `findBrowser(executablePath?)`.
pub fn find_browser(executable_path: Option<&str>) -> Option<BrowserExecutable> {
    if let Some(path) = executable_path {
        let kind = infer_kind(path);
        return Some(BrowserExecutable {
            kind,
            executable_path: path.to_string(),
        });
    }

    #[cfg(target_os = "macos")]
    {
        find_macos_browser()
    }
    #[cfg(target_os = "windows")]
    {
        find_windows_browser()
    }
    #[cfg(target_os = "linux")]
    {
        find_linux_browser()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
fn find_macos_browser() -> Option<BrowserExecutable> {
    const CANDIDATES: [(&str, &str); 3] = [
        (
            "chrome",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        ),
        (
            "edge",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ),
        (
            "chromium",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ),
    ];
    first_existing(&CANDIDATES)
}

/// First candidate whose path exists on disk, in priority order.
#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn first_existing(candidates: &[(&'static str, &str)]) -> Option<BrowserExecutable> {
    for (kind, path) in candidates {
        if std::path::Path::new(path).exists() {
            return Some(BrowserExecutable {
                kind,
                executable_path: (*path).to_string(),
            });
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn find_windows_browser() -> Option<BrowserExecutable> {
    let local = std::env::var("LOCALAPPDATA").ok();
    let prog = std::env::var("PROGRAMFILES").ok();
    let prog_x86 = std::env::var("PROGRAMFILES(X86)").ok();
    let candidates = [
        (
            "chrome",
            local.map(|p| format!("{p}\\Google\\Chrome\\Application\\chrome.exe")),
        ),
        (
            "chrome",
            prog.as_ref()
                .map(|p| format!("{p}\\Google\\Chrome\\Application\\chrome.exe")),
        ),
        (
            "edge",
            prog_x86.map(|p| format!("{p}\\Microsoft\\Edge\\Application\\msedge.exe")),
        ),
        (
            "edge",
            prog.as_ref()
                .map(|p| format!("{p}\\Microsoft\\Edge\\Application\\msedge.exe")),
        ),
    ];
    for (kind, path) in candidates {
        if let Some(path) = path {
            if std::path::Path::new(&path).exists() {
                return Some(BrowserExecutable {
                    kind,
                    executable_path: path,
                });
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn find_linux_browser() -> Option<BrowserExecutable> {
    // CHROME_PATH env var
    if let Ok(env_path) = std::env::var("CHROME_PATH") {
        if std::path::Path::new(&env_path).exists() {
            return Some(BrowserExecutable {
                kind: infer_kind(&env_path),
                executable_path: env_path,
            });
        }
    }
    // Known paths (no Edge on Linux — matches the TS candidates)
    const CANDIDATES: [(&str, &str); 3] = [
        ("chrome", "/usr/bin/google-chrome"),
        ("chromium", "/usr/bin/chromium-browser"),
        ("chromium", "/usr/bin/chromium"),
    ];
    first_existing(&CANDIDATES)
}

/// `inferKind(path)`.
pub fn infer_kind(path: &str) -> ChromiumBrowserKind {
    let lower = path.to_lowercase();
    if lower.contains("edge") || lower.contains("msedge") {
        "edge"
    } else if lower.contains("chrome") {
        "chrome"
    } else if lower.contains("chromium") {
        "chromium"
    } else {
        "chrome"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_kind_matches_ts() {
        assert_eq!(infer_kind("C:\\...\\msedge.exe"), "edge");
        assert_eq!(infer_kind("/Applications/Google Chrome.app/..."), "chrome");
        assert_eq!(infer_kind("chromium-browser"), "chromium");
        assert_eq!(infer_kind("unknown-path"), "chrome");
    }

    #[test]
    fn explicit_path_is_used_as_is() {
        let found = find_browser(Some("/tmp/custom-chrome")).unwrap();
        assert_eq!(found.executable_path, "/tmp/custom-chrome");
        assert_eq!(found.kind, "chrome");
    }

    #[test]
    fn discovery_runs_platform_candidates() {
        // Environment-dependent: returns Some when a browser is installed
        // (dev machines), None on bare CI images. Either way the platform
        // candidate-scan code path executes.
        let _ = find_browser(None);
    }

    #[test]
    fn first_existing_scans_in_order_and_may_miss() {
        let dir = tempfile::tempdir().expect("tempdir");
        let present = dir.path().join("browser-bin");
        std::fs::write(&present, "x").expect("write");
        let present = present.to_str().expect("utf8").to_string();
        // Hit on the second candidate (skips a missing one), then total miss.
        let found = first_existing(&[("chrome", "/no/such/bin"), ("chromium", &present)]);
        assert_eq!(found.map(|b| b.kind), Some("chromium"));
        assert!(first_existing(&[("chrome", "/no/such/bin")]).is_none());
    }
}
