//! Safari browser manager — port of
//! `cli/src/browser/safari/safari-manager.ts`.
//!
//! Launches safaridriver at /usr/bin/safaridriver (macOS), creates WebDriver
//! sessions. Users must first enable remote automation (`safaridriver
//! --enable`); permission errors get a clear remedy.

use super::webdriver_client::WebDriverClient;
use crate::browser::errors::BrowserPermissionError;
use crate::browser::types::BrowserConnectionConfig;
use std::process::Stdio;

const SAFARIDRIVER_PATH: &str = "/usr/bin/safaridriver";

/// Test-only override for the safaridriver binary path (the real const is
/// hard-coded; tests point this at a fake driver script). Under cfg(test)
/// the fallback default is `/bin/sh` so launch-path tests never spawn the
/// REAL safaridriver (`sh --port N` exits immediately and never serves).
#[cfg(test)]
static SAFARIDRIVER_PATH_OVERRIDE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The safaridriver executable path (honors the test override). Single
/// definition so no cfg(not(test))-only copy goes unexecuted in the
/// integration-test (non-cfg-test) build.
fn safaridriver_path() -> String {
    #[cfg(test)]
    if let Some(p) = SAFARIDRIVER_PATH_OVERRIDE.lock().unwrap().clone() {
        return p;
    }
    #[cfg(not(test))]
    const DEFAULT_PATH: &str = SAFARIDRIVER_PATH;
    #[cfg(test)]
    const DEFAULT_PATH: &str = "/bin/sh";
    DEFAULT_PATH.to_string()
}

/// `SafariManager::start(options)` — port of the CLI-visible path.
/// Test-only platform override (the cfg!(macos) gate is otherwise
/// uncoverable on the macOS coverage host).
#[cfg(test)]
static SAFARI_PLATFORM_OVERRIDE: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);

fn is_macos() -> bool {
    #[cfg(test)]
    if let Some(v) = *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() {
        return v;
    }
    cfg!(target_os = "macos")
}

pub async fn safari_start(
    requested_port: i64,
    _url: Option<&str>,
) -> Result<SafariStartResult, String> {
    if !is_macos() {
        return Err("Safari is only available on macOS.".to_string());
    }

    // Check if safaridriver is already running on the REQUESTED port first
    // (after resolve_port the port is free by construction, so a later check
    // could never succeed).
    let requested_endpoint = format!("http://127.0.0.1:{requested_port}");
    if endpoint_reachable(&requested_endpoint).await {
        // Try to create a session — may fail if remote automation is not enabled.
        let session_id = create_session_with_translation(&requested_endpoint).await?;
        return Ok(SafariStartResult {
            connection: BrowserConnectionConfig::Webdriver {
                browser_kind: "safari".to_string(),
                endpoint: requested_endpoint,
                session_id,
                driver_pid: None,
            },
            launcher: SAFARIDRIVER_PATH.to_string(),
            port: requested_port,
            status: "already_running".to_string(),
        });
    }

    let port = resolve_port(requested_port).await?;
    let driver_endpoint = format!("http://127.0.0.1:{port}");

    // Launch safaridriver.
    let child = tokio::process::Command::new(safaridriver_path())
        .args(["--port", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to launch safaridriver: {e}"))?;
    let pid = child.id().unwrap_or(0) as i64;

    // Wait for it to be ready.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if endpoint_reachable(&driver_endpoint).await {
            let session_id = create_session_with_translation(&driver_endpoint).await?;
            return Ok(SafariStartResult {
                connection: BrowserConnectionConfig::Webdriver {
                    browser_kind: "safari".to_string(),
                    endpoint: driver_endpoint,
                    session_id,
                    driver_pid: Some(pid),
                },
                launcher: SAFARIDRIVER_PATH.to_string(),
                port,
                status: "started".to_string(),
            });
        }
        crate::utils::time::sleep(250).await;
    }

    // Started but unreachable.
    Err(format!(
        "safaridriver did not respond at {driver_endpoint} within 10s."
    ))
}

/// `SafariStartResult`.
pub struct SafariStartResult {
    pub connection: BrowserConnectionConfig,
    pub launcher: String,
    pub port: i64,
    pub status: String,
}

/// `SafariManager::status(connection)`.
pub async fn safari_status(
    connection: &BrowserConnectionConfig,
) -> (bool, Option<serde_json::Value>, Option<String>) {
    if connection.protocol() != "webdriver" {
        return (false, None, Some("Not a WebDriver endpoint".to_string()));
    }
    let client = reqwest::Client::new();
    match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client
            .get(format!("{}/status", connection.endpoint()))
            .send(),
    )
    .await
    {
        Ok(Ok(response)) if response.status().is_success() => {
            let data = response.json().await.unwrap_or(serde_json::Value::Null);
            (true, Some(data), None)
        }
        Ok(Ok(response)) => (
            false,
            None,
            Some(format!("HTTP {}", response.status().as_u16())),
        ),
        Ok(Err(e)) => (false, None, Some(e.to_string())),
        Err(_) => (false, None, Some("Timed out".to_string())),
    }
}

/// Translate WebDriver/launch errors into user-actionable messages.
async fn create_session_with_translation(driver_endpoint: &str) -> Result<String, String> {
    let client = WebDriverClient::new(driver_endpoint);
    match client.create_session(None).await {
        Ok(sid) => Ok(sid),
        Err(e) => {
            let lower = e.to_lowercase();
            if lower.contains("allow remote automation") || lower.contains("remote automation") {
                return Err(permission_error());
            }
            if lower.contains("session not created") {
                return Err(permission_error());
            }
            Err(e)
        }
    }
}

fn permission_error() -> String {
    let err = BrowserPermissionError {
        error: crate::browser::errors::BrowserError {
            message: "Safari remote automation is disabled.".to_string(),
            code: "browser_permission_error",
        },
        remedy_command: "safaridriver --enable".to_string(),
    };
    err.to_string()
}

async fn endpoint_reachable(url: &str) -> bool {
    let client = reqwest::Client::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        client.get(format!("{url}/status")).send(),
    )
    .await
    .map(|r| r.map(|resp| resp.status().is_success()).unwrap_or(false))
    .unwrap_or(false)
}

async fn resolve_port(requested_port: i64) -> Result<i64, String> {
    // A live driver on the requested port is reused (matches the chromium
    // manager's resolve_port and makes the already-running branch in
    // safari_start reachable).
    if endpoint_reachable(&format!("http://127.0.0.1:{requested_port}")).await {
        return Ok(requested_port);
    }
    if !port_in_use(requested_port).await {
        return Ok(requested_port);
    }
    for port in requested_port + 1..requested_port + 50 {
        if !port_in_use(port).await {
            return Ok(port);
        }
    }
    Err(format!("No available port found near {requested_port}"))
}

async fn port_in_use(port: i64) -> bool {
    use tokio::io::AsyncWriteExt;
    let mut socket = match tokio::net::TcpStream::connect(("127.0.0.1", port as u16)).await {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = socket.shutdown().await;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{spawn_http, HttpRoute};

    /// Reset the driver-path override after a test.
    struct OverrideReset;
    impl Drop for OverrideReset {
        fn drop(&mut self) {
            *SAFARIDRIVER_PATH_OVERRIDE.lock().unwrap() = None;
        }
    }

    /// Kill a spawned child on drop (macOS/Linux `kill`). Prevents the
    /// `fake_driver.py` processes launched by tests from leaking as orphans.
    #[cfg(unix)]
    struct KillChild(Option<i64>);
    #[cfg(unix)]
    impl Drop for KillChild {
        fn drop(&mut self) {
            if let Some(pid) = self.0 {
                let _ = std::process::Command::new("kill")
                    .arg(pid.to_string())
                    .status();
            }
        }
    }

    fn set_driver_override(path: &str) -> OverrideReset {
        *SAFARIDRIVER_PATH_OVERRIDE.lock().unwrap() = Some(path.to_string());
        OverrideReset
    }

    async fn free_port() -> i64 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port() as i64;
        drop(listener);
        port
    }

    // ── safari_status ─────────────────────────────────────────────────

    #[tokio::test]
    async fn status_rejects_non_webdriver_config() {
        let config = BrowserConnectionConfig::Cdp {
            browser_kind: "chrome".to_string(),
            endpoint: "http://x".to_string(),
        };
        let (ok, data, err) = safari_status(&config).await;
        assert!(!ok);
        assert!(data.is_none());
        assert_eq!(err.as_deref(), Some("Not a WebDriver endpoint"));
    }

    #[tokio::test]
    async fn status_reachable_and_http_error() {
        let base = spawn_http(vec![HttpRoute::json("/status", 200, r#"{"ready":true}"#)]).await;
        let config = BrowserConnectionConfig::Webdriver {
            browser_kind: "safari".to_string(),
            endpoint: base,
            session_id: "s1".to_string(),
            driver_pid: None,
        };
        let (ok, data, err) = safari_status(&config).await;
        assert!(ok);
        assert_eq!(data, Some(serde_json::json!({"ready": true})));
        assert!(err.is_none());

        let base = spawn_http(vec![HttpRoute::json("/status", 503, "{}")]).await;
        let config = BrowserConnectionConfig::Webdriver {
            browser_kind: "safari".to_string(),
            endpoint: base,
            session_id: "s1".to_string(),
            driver_pid: None,
        };
        let (ok, _, err) = safari_status(&config).await;
        assert!(!ok);
        assert_eq!(err.as_deref(), Some("HTTP 503"));
    }

    #[tokio::test]
    async fn status_unreachable_and_timeout() {
        let config = BrowserConnectionConfig::Webdriver {
            browser_kind: "safari".to_string(),
            endpoint: "http://127.0.0.1:1".to_string(),
            session_id: "s1".to_string(),
            driver_pid: None,
        };
        let (ok, _, err) = safari_status(&config).await;
        assert!(!ok);
        assert!(err.is_some());

        // Slow server → 2 s client timeout.
        let base = spawn_http(vec![HttpRoute::slow(
            "/status",
            std::time::Duration::from_secs(3),
        )])
        .await;
        let config = BrowserConnectionConfig::Webdriver {
            browser_kind: "safari".to_string(),
            endpoint: base,
            session_id: "s1".to_string(),
            driver_pid: None,
        };
        let (ok, _, err) = safari_status(&config).await;
        assert!(!ok);
        assert_eq!(err.as_deref(), Some("Timed out"));
    }

    // ── safari_start: already-running + error translation ─────────────

    #[tokio::test]
    async fn start_already_running_creates_session() {
        let _guard = crate::test_env::lock_env().await;
        *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = Some(true);
        let _reset = PlatformReset;
        let base = spawn_http(vec![
            HttpRoute::json("/status", 200, r#"{"ready":true}"#),
            HttpRoute::json("/session", 200, r#"{"sessionId":"sid-1","value":{}}"#),
        ])
        .await;
        let port: i64 = base.rsplit(':').next().unwrap().parse().unwrap();
        let result = safari_start(port, None).await.expect("start");
        assert_eq!(result.status, "already_running");
        assert_eq!(result.port, port);
        assert_eq!(result.launcher, SAFARIDRIVER_PATH);
        assert_eq!(result.connection.protocol(), "webdriver");
        assert_eq!(result.connection.session_id(), Some("sid-1"));
        assert!(matches!(
            result.connection,
            BrowserConnectionConfig::Webdriver {
                driver_pid: None,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn start_translates_permission_errors() {
        let _guard = crate::test_env::lock_env().await;
        *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = Some(true);
        let _reset = PlatformReset;
        for body in [
            r#"{"value":{"error":"session not created","message":"boom"}}"#,
            r#"{"value":{"error":"unknown error","message":"You must Allow Remote Automation first"}}"#,
        ] {
            let base = spawn_http(vec![
                HttpRoute::json("/status", 200, r#"{"ready":true}"#),
                HttpRoute::json("/session", 500, body),
            ])
            .await;
            let port: i64 = base.rsplit(':').next().unwrap().parse().unwrap();
            let err = safari_start(port, None).await.err().unwrap();
            assert_eq!(err, "Safari remote automation is disabled.", "body={body}");
        }
    }

    #[tokio::test]
    async fn start_passthrough_other_session_errors() {
        let _guard = crate::test_env::lock_env().await;
        *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = Some(true);
        let _reset = PlatformReset;
        let base = spawn_http(vec![
            HttpRoute::json("/status", 200, r#"{"ready":true}"#),
            HttpRoute::json(
                "/session",
                500,
                r#"{"value":{"error":"unknown error","message":"weird driver state"}}"#,
            ),
        ])
        .await;
        let port: i64 = base.rsplit(':').next().unwrap().parse().unwrap();
        let err = safari_start(port, None).await.err().unwrap();
        assert!(err.contains("weird driver state"), "{err}");
    }

    // ── safari_start: launch paths (macOS only) ───────────────────────

    #[cfg(unix)]
    #[tokio::test]
    async fn start_launch_spawn_failure() {
        let _guard = crate::test_env::lock_env().await;
        *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = Some(true);
        let _reset_platform = PlatformReset;
        let _reset = set_driver_override("/nonexistent/safaridriver");
        let err = safari_start(free_port().await, None).await.err().unwrap();
        assert!(err.contains("Failed to launch safaridriver"), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn start_launch_never_ready_times_out() {
        let _guard = crate::test_env::lock_env().await;
        *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = Some(true);
        let _reset = PlatformReset;
        // No driver-path override: the cfg(test) default is /bin/sh, which
        // spawns fine but never serves → 10 s wait → error.
        let started = std::time::Instant::now();
        let err = safari_start(free_port().await, Some("https://x"))
            .await
            .err()
            .unwrap();
        assert!(err.contains("did not respond"), "{err}");
        assert!(started.elapsed().as_secs() >= 10);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn start_launch_success_against_fake_driver() {
        let _guard = crate::test_env::lock_env().await;
        *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = Some(true);
        let _reset_platform = PlatformReset;
        // Fake safaridriver: a shell script launching a tiny python HTTP
        // server that answers /status and POST /session.
        let dir = tempfile::tempdir().unwrap();
        let py = dir.path().join("fake_driver.py");
        std::fs::write(
            &py,
            r#"import http.server, socketserver, sys, json
port = int(sys.argv[1])
class H(http.server.BaseHTTPRequestHandler):
    def _send(self, body):
        b = json.dumps(body).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(b)))
        self.end_headers()
        self.wfile.write(b)
    def do_GET(self):
        self._send({"ready": True})
    def do_POST(self):
        self._send({"sessionId": "fake-sid", "value": {}})
    def log_message(self, *a):
        pass
socketserver.TCPServer(("127.0.0.1", port), H).serve_forever()
"#,
        )
        .unwrap();
        let sh = dir.path().join("safaridriver");
        std::fs::write(
            &sh,
            format!("#!/bin/sh\nexec python3 {} \"$2\"\n", py.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let _reset = set_driver_override(&sh.to_string_lossy());

        // Pre-warm the python interpreter (page-in under full-suite load can
        // otherwise exceed the 10 s readiness window).
        let _ = std::process::Command::new("python3")
            .arg("--version")
            .output();

        let result = safari_start(free_port().await, Some("http://x/"))
            .await
            .expect("start");
        // Reap the fake-driver child this test spawned on exit (even on panic).
        let driver_pid = match &result.connection {
            BrowserConnectionConfig::Webdriver { driver_pid, .. } => *driver_pid,
            _ => None,
        };
        let _kill = KillChild(driver_pid);
        assert_eq!(result.status, "started");
        assert_eq!(result.connection.session_id(), Some("fake-sid"));
        assert!(matches!(
            result.connection,
            BrowserConnectionConfig::Webdriver {
                driver_pid: Some(_),
                ..
            }
        ));
    }

    /// Reset the platform override after a test.
    struct PlatformReset;
    impl Drop for PlatformReset {
        fn drop(&mut self) {
            *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = None;
        }
    }

    #[tokio::test]
    async fn start_rejected_off_macos() {
        // Platform override forces the non-macOS gate on any host.
        let _guard = crate::test_env::lock_env().await;
        *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = Some(false);
        let _reset = PlatformReset;
        let err = safari_start(free_port().await, None).await.err().unwrap();
        assert_eq!(err, "Safari is only available on macOS.");
    }

    #[tokio::test]
    async fn start_propagates_port_exhaustion() {
        // All 50 candidate ports occupied → resolve_port error propagates
        // through safari_start (before any launch attempt).
        let _guard = crate::test_env::lock_env().await;
        *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = Some(true);
        let _reset = PlatformReset;
        let (first, holders) = crate::test_env::reserve_consecutive_ports(50);
        let err = safari_start(first, None).await.err().unwrap();
        assert!(err.contains("No available port found near"), "{err}");
        drop(holders);
    }

    // ── port helpers ──────────────────────────────────────────────────

    #[tokio::test]
    async fn resolve_port_free_occupied_and_exhausted() {
        // Serialize against the other port-exhaustion test (50 consecutive
        // bound ports must not overlap).
        let _guard = crate::test_env::lock_env().await;
        // Free port is returned as-is.
        let free = free_port().await;
        assert_eq!(resolve_port(free).await.unwrap(), free);

        // A live HTTP endpoint on the requested port is reused as-is.
        let base = spawn_http(vec![HttpRoute::json("/status", 200, "{}")]).await;
        let live: i64 = base.rsplit(':').next().unwrap().parse().unwrap();
        assert_eq!(resolve_port(live).await.unwrap(), live);

        // Occupied (but not an HTTP endpoint) → scan to the next free one.
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = held.local_addr().unwrap().port() as i64;
        let resolved = resolve_port(taken).await.unwrap();
        assert!(resolved > taken && resolved < taken + 50);
        drop(held);

        // Exhaustion: 50 consecutive occupied ports → error.
        let (first, holders) = crate::test_env::reserve_consecutive_ports(50);
        let err = resolve_port(first).await.unwrap_err();
        assert!(err.contains("No available port found near"), "{err}");
        drop(holders);
    }

    #[tokio::test]
    async fn port_in_use_probe() {
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = held.local_addr().unwrap().port() as i64;
        assert!(port_in_use(taken).await);
        drop(held);
        let free = free_port().await;
        assert!(!port_in_use(free).await);
    }

    #[tokio::test]
    async fn endpoint_reachable_probe() {
        let base = spawn_http(vec![HttpRoute::json("/status", 200, "{}")]).await;
        assert!(endpoint_reachable(&base).await);
        assert!(!endpoint_reachable("http://127.0.0.1:1").await);
        // Non-2xx is not reachable.
        let base = spawn_http(vec![HttpRoute::json("/status", 500, "{}")]).await;
        assert!(!endpoint_reachable(&base).await);
    }
}
