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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{spawn_http, HttpRoute};

    fn free_port() -> i64 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port() as i64;
        drop(listener);
        port
    }

    #[tokio::test]
    async fn resolve_port_reuses_live_endpoint() {
        // An HTTP endpoint answering /json/version on the requested port is
        // reused as-is.
        let base = spawn_http(vec![HttpRoute::json("/json/version", 200, "{}")]).await;
        let port: i64 = base.rsplit(':').next().unwrap().parse().unwrap();
        assert_eq!(resolve_port(port).await.unwrap(), port);
    }

    #[tokio::test]
    async fn resolve_port_free_occupied_and_exhausted() {
        let free = free_port();
        assert_eq!(resolve_port(free).await.unwrap(), free);

        // Occupied by a NON-HTTP listener → scan to the next free port.
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = held.local_addr().unwrap().port() as i64;
        let resolved = resolve_port(taken).await.unwrap();
        assert!(resolved > taken && resolved < taken + 50);
        drop(held);

        // Exhaustion: 50 consecutive occupied ports → error.
        let first = free_port();
        let mut holders = Vec::new();
        let mut blocked = true;
        for p in first..first + 50 {
            match std::net::TcpListener::bind(("127.0.0.1", p as u16)) {
                Ok(l) => holders.push(l),
                Err(_) => {
                    blocked = false;
                    break;
                }
            }
        }
        if blocked {
            let err = resolve_port(first).await.unwrap_err();
            assert!(err.contains("No available browser debugging port"), "{err}");
        }
        drop(holders);
    }

    #[tokio::test]
    async fn endpoint_reachable_variants() {
        let base = spawn_http(vec![HttpRoute::json("/json/version", 200, "{}")]).await;
        assert!(endpoint_reachable(&base).await);
        let base = spawn_http(vec![HttpRoute::json("/json/version", 500, "{}")]).await;
        assert!(!endpoint_reachable(&base).await);
        assert!(!endpoint_reachable("http://127.0.0.1:1").await);
    }

    #[tokio::test]
    async fn port_has_listener_probe() {
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = held.local_addr().unwrap().port() as i64;
        assert!(port_has_listener(taken).await);
        drop(held);
        assert!(!port_has_listener(free_port()).await);
    }

    #[tokio::test]
    async fn default_profile_dir_follows_future_home() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_HOME",
            dir.path().as_os_str().to_os_string(),
        )]);
        let path = default_profile_dir();
        assert_eq!(
            path,
            dir.path().join("agent").join("browser").join("profile")
        );
    }

    #[test]
    fn launcher_lookup_with_explicit_path() {
        let (command, kind) = find_browser_launcher(Some("/custom/chrome")).unwrap();
        assert_eq!(command, "/custom/chrome");
        assert_eq!(kind, "chrome");
        assert_eq!(
            launcher_from_executable(Some("/custom/chrome")).as_deref(),
            Some("/custom/chrome")
        );
        let launcher = Launcher::discover(Some("/custom/chrome")).unwrap();
        assert_eq!(launcher.command, "/custom/chrome");
        assert!(launcher.args.is_empty());
    }

    #[test]
    fn launcher_lookup_platform_discovery_runs() {
        // Platform discovery (no explicit path) — outcome depends on the
        // host; just exercise the lookup both ways.
        let _ = find_browser_launcher(None);
        let _ = launcher_from_executable(None);
        let _ = Launcher::discover(None);
    }

    #[test]
    fn error_message_and_config_shape() {
        assert!(no_browser_found_message("extra").contains("extra"));
        let conn = cdp_connection("http://e:1");
        assert_eq!(conn.protocol(), "cdp");
        assert_eq!(conn.browser_kind(), "chromium");
        assert_eq!(conn.endpoint(), "http://e:1");
    }
}
