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

/// `SafariManager::start(options)` — port of the CLI-visible path.
pub async fn safari_start(
    requested_port: i64,
    _url: Option<&str>,
) -> Result<SafariStartResult, String> {
    if !cfg!(target_os = "macos") {
        return Err("Safari is only available on macOS.".to_string());
    }

    let port = resolve_port(requested_port).await?;
    let driver_endpoint = format!("http://127.0.0.1:{port}");

    // Check if safaridriver is already running on this port.
    if endpoint_reachable(&driver_endpoint).await {
        // Try to create a session — may fail if remote automation is not enabled.
        let session_id = create_session_with_translation(&driver_endpoint).await?;
        return Ok(SafariStartResult {
            connection: BrowserConnectionConfig::Webdriver {
                browser_kind: "safari".to_string(),
                endpoint: driver_endpoint,
                session_id,
                driver_pid: None,
            },
            launcher: SAFARIDRIVER_PATH.to_string(),
            port,
            status: "already_running".to_string(),
        });
    }

    // Launch safaridriver.
    let child = tokio::process::Command::new(SAFARIDRIVER_PATH)
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
