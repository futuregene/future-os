//! CDP endpoint resolver — port of
//! `cli/src/browser/chromium/chromium-endpoint.ts`.
//!
//! GET `<httpEndpoint>/json/version` → webSocketDebuggerUrl + browser
//! identity (chrome / edge / chromium).

use serde_json::Value;

/// `CdpEndpointInfo`.
#[derive(Debug, Clone)]
pub struct CdpEndpointInfo {
    pub http_endpoint: String,
    pub web_socket_debugger_url: String,
    pub browser_kind: String,
    pub browser_version: Option<String>,
}

/// `resolveCdpEndpoint(httpEndpoint, timeoutMs = 5000)`.
pub async fn resolve_cdp_endpoint(
    http_endpoint: &str,
    timeout_ms: u64,
) -> Result<CdpEndpointInfo, String> {
    let client = reqwest::Client::new();
    let response = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        client.get(format!("{http_endpoint}/json/version")).send(),
    )
    .await
    .map_err(|_| "CDP /json/version timed out".to_string())?
    .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(format!(
            "CDP /json/version returned HTTP {}: {}",
            response.status().as_u16(),
            response.status().canonical_reason().unwrap_or("Unknown")
        ));
    }

    let data: Value = response
        .json()
        .await
        .map_err(|_| "Invalid /json/version response".to_string())?;

    let web_socket_debugger_url = data
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .map(str::to_string);

    let Some(ws_url) = web_socket_debugger_url else {
        return Err(format!(
            "Invalid webSocketDebuggerUrl in /json/version: {}",
            serde_json::to_string(&data).unwrap_or_default()
        ));
    };
    if !ws_url.starts_with("ws") {
        return Err(format!(
            "Invalid webSocketDebuggerUrl in /json/version: {}",
            serde_json::to_string(&data).unwrap_or_default()
        ));
    }

    let browser_kind = identify_browser(&data);
    let browser_version = data
        .get("Browser")
        .and_then(Value::as_str)
        .or_else(|| data.get("browser").and_then(Value::as_str))
        .map(str::to_string);

    Ok(CdpEndpointInfo {
        http_endpoint: http_endpoint.to_string(),
        web_socket_debugger_url: ws_url,
        browser_kind,
        browser_version,
    })
}

/// `identifyBrowser(data)` — from the Browser/User-Agent fields.
pub fn identify_browser(data: &Value) -> String {
    let browser = data
        .get("Browser")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();

    if browser.contains("edg") || browser.contains("edge") {
        return "edge".to_string();
    }
    if browser.contains("chrome") {
        return "chrome".to_string();
    }
    if browser.contains("chromium") {
        return "chromium".to_string();
    }

    // Fallback: check User-Agent style fields.
    let ua = data
        .get("User-Agent")
        .or_else(|| data.get("user-agent"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    if ua.contains("edg/") {
        return "edge".to_string();
    }
    if ua.contains("chrome/") {
        return "chrome".to_string();
    }
    if ua.contains("chromium/") {
        return "chromium".to_string();
    }

    "chromium".to_string()
}
