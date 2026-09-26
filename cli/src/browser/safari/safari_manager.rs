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

/// Test-only: point `safaridriver_path()` at a path the test owns, with a guard
/// that restores it. Module-level (not inside `mod tests`) so the launch tests
/// below can use it on every platform — the unix-only `set_driver_override`
/// variant exists only because its callers spawn real fixtures.
#[cfg(test)]
fn set_driver_path_for_tests(path: &str) -> DriverPathReset {
    *SAFARIDRIVER_PATH_OVERRIDE.lock().unwrap() = Some(path.to_string());
    DriverPathReset
}

/// Guard returned by [`set_driver_path_for_tests`].
#[cfg(test)]
struct DriverPathReset;

#[cfg(test)]
impl Drop for DriverPathReset {
    fn drop(&mut self) {
        *SAFARIDRIVER_PATH_OVERRIDE.lock().unwrap() = None;
    }
}

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

/// Test-only: force the macOS gate on, so the Safari webdriver path is
/// reachable on **every** platform and from other modules' tests (which cannot
/// see the private static). Restores the real platform when dropped.
///
/// This exists because the gate is a *runtime* value here (`is_macos`), not a
/// compile-time one: the three Safari integration tests in
/// `commands::browser_tools` used to be `#[cfg(target_os = "macos")]` with the
/// note "safari webdriver path errors out pre-network elsewhere" — true only
/// while nothing could flip the gate. With the seam, those tests run on
/// Windows and Linux too.
#[cfg(test)]
pub(crate) fn force_macos_gate() -> MacosGate {
    *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = Some(true);
    MacosGate
}

/// Guard returned by [`force_macos_gate`]; restores the platform on drop.
#[cfg(test)]
pub(crate) struct MacosGate;

#[cfg(test)]
impl Drop for MacosGate {
    fn drop(&mut self) {
        *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = None;
    }
}

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
            .timeout(std::time::Duration::from_secs(2))
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
        Ok(Err(e)) => (
            false,
            None,
            Some(if e.is_timeout() {
                "Timed out".into()
            } else {
                e.to_string()
            }),
        ),
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

    /// Reset the driver-path override after a test. Its users (the
    /// safaridriver launch tests) are unix-only.
    #[cfg(unix)]
    struct OverrideReset;
    #[cfg(unix)]
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

    #[cfg(unix)]
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

    /// `safari_start`'s success path: the driver is spawned, does not answer
    /// immediately, and the CLI's readiness poll then finds it and creates a
    /// WebDriver session.
    ///
    /// The driver is a script that records that it ran; the mock WebDriver
    /// endpoint is brought up only once that marker appears, which proves the
    /// spawn step is behind us. An endpoint that answered earlier would take
    /// the already-running branch instead (the endpoint is probed before the
    /// driver is launched), so the marker — not a sleep — is what puts the
    /// appearance inside the poll's 10 s / 250 ms window.
    #[tokio::test(flavor = "multi_thread")]
    async fn launch_then_become_reachable_reports_started() {
        let _guard = crate::test_env::lock_env().await;
        let _macos = force_macos_gate();
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("driver-ran.txt");
        let script = driver_script(dir.path(), &marker);
        let _driver = set_driver_path_for_tests(&script.display().to_string());

        let port = free_port().await;
        let started = tokio::spawn(async move { safari_start(port, None).await });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !marker.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the driver was never spawned, so the poll was never reached"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let server = serve_webdriver(port).await;
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), started)
            .await
            .expect("safari_start is bounded by its own 10 s window")
            .expect("the task does not panic")
            .expect("a driver that answers in the window is a success");
        server.abort();

        assert_eq!(result.status, "started");
        assert_eq!(result.port, port);
        assert_eq!(result.launcher, SAFARIDRIVER_PATH);
        assert_eq!(result.connection.protocol(), "webdriver");
        assert_eq!(result.connection.session_id(), Some("sid-start"));
        assert!(
            matches!(
                result.connection,
                BrowserConnectionConfig::Webdriver {
                    driver_pid: Some(pid),
                    ..
                } if pid > 0
            ),
            "the spawned driver's pid is reported for cleanup"
        );
    }

    /// `safari_status`'s non-timeout transport error: a peer that accepts and
    /// closes without a complete response is reported as the transport error
    /// itself, not as "Timed out" (the deadline arm has its own slow-server
    /// test). A refused port would not do: this host's loopback silently drops
    /// a connect to a closed port, which the 2 s deadline turns into a timeout
    /// and would test the wrong arm.
    #[tokio::test(flavor = "multi_thread")]
    async fn status_reports_a_non_timeout_transport_error() {
        let _guard = crate::test_env::lock_env().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind closing peer");
        let port = listener.local_addr().expect("addr").port();
        let peer = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("accept");
            // Half a response, then gone: not a deadline, a broken exchange.
            drop(socket);
        });
        let config = BrowserConnectionConfig::Webdriver {
            browser_kind: "safari".to_string(),
            endpoint: format!("http://127.0.0.1:{port}"),
            session_id: "s1".to_string(),
            driver_pid: None,
        };
        let (ok, data, err) = safari_status(&config).await;
        peer.await.expect("peer task");
        assert!(!ok);
        assert!(data.is_none());
        let err = err.expect("a broken exchange is reported");
        assert_ne!(err, "Timed out", "a closed peer is not a deadline");
        assert!(!err.is_empty());
    }

    /// `safari_status`'s outer deadline: a peer that accepts and then never
    /// answers must be reported as "Timed out" rather than hanging the command.
    #[tokio::test(flavor = "multi_thread")]
    async fn status_times_out_when_the_peer_never_answers() {
        let _guard = crate::test_env::lock_env().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind silent peer");
        let port = listener.local_addr().expect("addr").port();
        // Accept and hold: the request is read but never answered.
        let held = std::sync::Arc::new(tokio::sync::Notify::new());
        let peer = tokio::spawn({
            let held = held.clone();
            async move {
                let (socket, _) = listener.accept().await.expect("accept");
                held.notify_one();
                // Hold the socket open well past the client's 2 s deadline.
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                drop(socket);
            }
        });
        let config = BrowserConnectionConfig::Webdriver {
            browser_kind: "safari".to_string(),
            endpoint: format!("http://127.0.0.1:{port}"),
            session_id: "s1".to_string(),
            driver_pid: None,
        };
        let (ok, data, err) = safari_status(&config).await;
        // Let the peer run to its end (it drops the socket after its hold)
        // rather than aborting it: its completion is what proves nothing was
        // left writing to a half-read response.
        peer.await.expect("peer task");
        assert!(!ok);
        assert!(data.is_none());
        assert_eq!(err.as_deref(), Some("Timed out"));
    }

    /// A launcher/driver script that records that it ran and exits.
    ///
    /// Windows cannot start a `.sh` and this host cannot create a symlink, so
    /// the script is written in whatever dialect the platform executes — a
    /// `.cmd` (started through `cmd.exe` by both `Start-Process` and Rust's
    /// `Command`, both probed on this host) or a POSIX shell script.
    fn driver_script(dir: &std::path::Path, marker: &std::path::Path) -> std::path::PathBuf {
        #[cfg(windows)]
        {
            let script = dir.join("fake-driver.cmd");
            std::fs::write(
                &script,
                format!(
                    "@echo off\r\n> \"{}\" echo ran\r\nexit /b 0\r\n",
                    marker.display()
                ),
            )
            .expect("write driver script");
            script
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = dir.join("fake-driver.sh");
            std::fs::write(
                &script,
                format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
            )
            .expect("write driver script");
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("chmod driver script");
            script
        }
    }

    /// A minimal WebDriver endpoint on `port`: `/status` and `/session` both
    /// answer 200, which is what the readiness poll and
    /// `create_session_with_translation` need.
    async fn serve_webdriver(port: i64) -> tokio::task::JoinHandle<()> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port as u16))
            .await
            .expect("the resolved port is still free");
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 2048];
                    let read = socket.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..read]);
                    let body = if request.contains("/session") {
                        r#"{"sessionId":"sid-start","value":{}}"#
                    } else {
                        r#"{"ready":true}"#
                    };
                    let mut response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    response.extend_from_slice(body.as_bytes());
                    let _ = socket.write_all(&response).await;
                    let _ = socket.shutdown().await;
                });
            }
        })
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

    // ── The launch path on Windows ──────────────────────────────────────

    /// The overrides themselves, on any host: the driver-path override wins
    /// when set, and without it the `cfg(test)` default (`/bin/sh`) is used so
    /// no test can ever spawn the real safaridriver.
    #[tokio::test]
    async fn the_driver_path_override_wins_over_the_test_default() {
        let _guard = crate::test_env::lock_env().await;
        // Reset first: the override is process-global, so a value left by an
        // earlier test in this binary is not this test's starting state.
        *SAFARIDRIVER_PATH_OVERRIDE.lock().unwrap() = None;
        assert_eq!(safaridriver_path(), "/bin/sh");
        *SAFARIDRIVER_PATH_OVERRIDE.lock().unwrap() = Some("X".to_string());
        assert_eq!(safaridriver_path(), "X");
        *SAFARIDRIVER_PATH_OVERRIDE.lock().unwrap() = None;
        // The launcher reported on success is the real constant, not the
        // test default — one is the binary that runs, the other what the
        // caller is told about it.
        assert_eq!(SAFARIDRIVER_PATH, "/usr/bin/safaridriver");
    }

    /// The macOS gate is driven by the override, so both answers are reachable
    /// on a non-macOS host — and with no override it is this host's own
    /// `cfg!(target_os = "macos")`.
    #[tokio::test]
    async fn the_platform_override_decides_the_macos_gate() {
        let _guard = crate::test_env::lock_env().await;
        *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = Some(true);
        assert!(is_macos());
        *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = Some(false);
        assert!(!is_macos());
        *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = None;
        assert_eq!(is_macos(), cfg!(target_os = "macos"));
    }

    /// A safaridriver that cannot be started is reported as a launch failure
    /// (the spawn error is the message), and one that starts but never serves
    /// is reported as the readiness timeout rather than hanging forever.
    ///
    /// Windows-only because the unix tests use a fake driver that *does* serve
    /// (a python HTTP server); this host has no safaridriver, so the same two
    /// arms are reached by making the process fail instead. Neither case starts
    /// a browser: the first path does not exist and the second is `where.exe`,
    /// which prints a usage error and exits.
    #[cfg(windows)]
    #[tokio::test]
    async fn launch_failure_and_readiness_timeout_are_both_reported() {
        let _guard = crate::test_env::lock_env().await;
        *SAFARI_PLATFORM_OVERRIDE.lock().unwrap() = Some(true);
        let _reset = PlatformReset;

        // 1. The driver does not exist → the spawn error is reported. (The
        //    message is the OS error; Windows' `CreateProcess` failure does not
        //    carry the missing path, so only the launcher is named.)
        *SAFARIDRIVER_PATH_OVERRIDE.lock().unwrap() =
            Some("C:\\future-clitui-no-such-driver.exe".to_string());
        let err = safari_start(free_port().await, None)
            .await
            .map(|_| ())
            .expect_err("a missing driver cannot start");
        assert!(err.contains("Failed to launch safaridriver"), "{err}");

        // 2. The driver starts but never serves → the 10 s readiness budget
        //    expires with the endpoint named. `where` exists on every Windows
        //    host and exits immediately on an unknown option.
        *SAFARIDRIVER_PATH_OVERRIDE.lock().unwrap() = Some("where".to_string());
        let port = free_port().await;
        let err = safari_start(port, None)
            .await
            .map(|_| ())
            .expect_err("a driver that never serves must time out");
        assert!(err.contains("did not respond"), "{err}");
        assert!(err.contains(&format!("127.0.0.1:{port}")), "{err}");
        assert!(err.contains("within 10s"), "{err}");
        *SAFARIDRIVER_PATH_OVERRIDE.lock().unwrap() = None;
    }
}
