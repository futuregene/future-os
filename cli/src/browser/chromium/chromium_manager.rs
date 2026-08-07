//! ChromiumManager — BrowserManager implementation for Chrome/Edge/Chromium.
//! Port of `cli/src/browser/chromium/chromium-manager.ts`.
//!
//! The CLI's `browser start` command (browser_tools.rs) reuses the same
//! launch logic inline (1:1 with browser-tools.ts); this manager exists for
//! the factory surface parity. Only the pieces actually reachable from the
//! CLI are exercised.

use crate::browser::discovery::find_browser;
use crate::browser::errors::browser_not_found_error;
use crate::browser::screenshot_writer::browser_dir;
use crate::browser::types::BrowserConnectionConfig;

/// `resolvePort` — requested port if free/reachable, else scan +49.
pub async fn resolve_port(requested_port: i64) -> Result<i64, String> {
    if endpoint_reachable(&format!("http://127.0.0.1:{requested_port}")).await {
        return Ok(requested_port);
    }
    if !port_has_listener(requested_port).await {
        return Ok(requested_port);
    }
    for port in requested_port + 1..requested_port + 50 {
        if !port_has_listener(port).await {
            return Ok(port);
        }
    }
    Err(format!(
        "No available browser debugging port found near {requested_port}."
    ))
}

/// `endpointReachable(endpoint)` — GET /json/version within 1 s.
pub async fn endpoint_reachable(endpoint: &str) -> bool {
    let client = reqwest::Client::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        client.get(format!("{endpoint}/json/version")).send(),
    )
    .await
    .map(|r| r.map(|resp| resp.status().is_success()).unwrap_or(false))
    .unwrap_or(false)
}

/// `portHasListener(port)` — TCP connect probe with a 500 ms bound.
pub async fn port_has_listener(port: i64) -> bool {
    use tokio::io::AsyncWriteExt;
    let mut socket = match tokio::net::TcpStream::connect(("127.0.0.1", port as u16)).await {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = socket.shutdown().await;
    true
}

/// Default profile dir — `~/.future/agent/browser/profile`.
pub fn default_profile_dir() -> std::path::PathBuf {
    browser_dir().join("profile")
}

/// `findBrowserLauncher(executablePath?)` — `{ command, args: [] }`.
pub fn find_browser_launcher(executable_path: Option<&str>) -> Option<(String, String)> {
    let discovered = find_browser(executable_path)?;
    Some((discovered.executable_path, discovered.kind.to_string()))
}

/// Sanity: the launcher-path check used by `browser start`.
pub fn launcher_from_executable(executable_path: Option<&str>) -> Option<String> {
    find_browser_launcher(executable_path).map(|(cmd, _kind)| cmd)
}

/// Error message used when no browser binary is found.
pub fn no_browser_found_message(extra: &str) -> String {
    browser_not_found_error(Some(extra)).to_string()
}

/// A discovered launcher — command + args (the CLI always uses empty args).
pub struct Launcher {
    pub command: String,
    pub args: Vec<String>,
}

impl Launcher {
    pub fn discover(executable_path: Option<&str>) -> Option<Self> {
        let (command, _kind) = find_browser_launcher(executable_path)?;
        Some(Launcher {
            command,
            args: Vec::new(),
        })
    }
}

/// Connection config shape for the already-running / started result.
pub fn cdp_connection(endpoint: &str) -> BrowserConnectionConfig {
    BrowserConnectionConfig::Cdp {
        browser_kind: "chromium".to_string(),
        endpoint: endpoint.to_string(),
    }
}
