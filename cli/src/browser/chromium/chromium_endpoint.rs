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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_cdp::MockCdp;
    use crate::test_server::{spawn_http, HttpRoute};
    use serde_json::json;

    #[tokio::test]
    async fn resolve_success_full_identity() {
        let mock = MockCdp::start().await;
        let info = resolve_cdp_endpoint(&mock.http_url, 5_000)
            .await
            .expect("resolve");
        assert_eq!(info.web_socket_debugger_url, mock.ws_url);
        assert_eq!(info.browser_kind, "chrome");
        assert_eq!(info.browser_version.as_deref(), Some("Chrome/126.0.0.0"));
        assert_eq!(info.http_endpoint, mock.http_url);
    }

    #[tokio::test]
    async fn resolve_edge_and_lowercase_browser_field() {
        for (body, kind) in [
            (r#"{"Browser":"Edg/120","webSocketDebuggerUrl":"ws://x/ws"}"#, "edge"),
            (
                r#"{"browser":"Chromium/119","webSocketDebuggerUrl":"ws://x/ws"}"#,
                "chromium",
            ),
        ] {
            let base = spawn_http(vec![HttpRoute::json("/json/version", 200, body)]).await;
            let info = resolve_cdp_endpoint(&base, 5_000).await.expect("resolve");
            assert_eq!(info.browser_kind, kind);
        }
    }

    #[tokio::test]
    async fn resolve_timeout_against_slow_server() {
        let base = spawn_http(vec![HttpRoute::slow(
            "/json/version",
            std::time::Duration::from_secs(2),
        )])
        .await;
        let err = resolve_cdp_endpoint(&base, 50).await.unwrap_err();
        assert_eq!(err, "CDP /json/version timed out");
    }

    #[tokio::test]
    async fn resolve_http_error_status() {
        let base = spawn_http(vec![HttpRoute::json("/json/version", 500, "{}")]).await;
        let err = resolve_cdp_endpoint(&base, 5_000).await.unwrap_err();
        assert!(
            err.contains("CDP /json/version returned HTTP 500"),
            "err: {err}"
        );
    }

    #[tokio::test]
    async fn resolve_connection_refused() {
        let err = resolve_cdp_endpoint("http://127.0.0.1:1", 5_000)
            .await
            .unwrap_err();
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn resolve_invalid_json_body() {
        let base = spawn_http(vec![HttpRoute::json("/json/version", 200, "not json")]).await;
        let err = resolve_cdp_endpoint(&base, 5_000).await.unwrap_err();
        assert_eq!(err, "Invalid /json/version response");
    }

    #[tokio::test]
    async fn resolve_missing_or_non_ws_debugger_url() {
        for body in [
            r#"{"Browser":"Chrome/1"}"#,
            r#"{"webSocketDebuggerUrl":"http://x/notws"}"#,
        ] {
            let base = spawn_http(vec![HttpRoute::json("/json/version", 200, body)]).await;
            let err = resolve_cdp_endpoint(&base, 5_000).await.unwrap_err();
            assert!(
                err.contains("Invalid webSocketDebuggerUrl in /json/version"),
                "body={body} err={err}"
            );
        }
    }

    #[test]
    fn identify_browser_from_browser_field() {
        assert_eq!(
            identify_browser(&json!({"Browser": "Microsoft Edge/120"})),
            "edge"
        );
        assert_eq!(identify_browser(&json!({"Browser": "Chrome/126"})), "chrome");
        assert_eq!(
            identify_browser(&json!({"Browser": "Chromium/119"})),
            "chromium"
        );
    }

    #[test]
    fn identify_browser_user_agent_fallbacks() {
        assert_eq!(
            identify_browser(&json!({"User-Agent": "Mozilla/5.0 Edg/120.0"})),
            "edge"
        );
        assert_eq!(
            identify_browser(&json!({"user-agent": "Mozilla/5.0 Chrome/126.0 Safari/537.36"})),
            "chrome"
        );
        assert_eq!(
            identify_browser(&json!({"User-Agent": "Mozilla/5.0 Chromium/119.0"})),
            "chromium"
        );
        // No recognizable marker → default chromium.
        assert_eq!(identify_browser(&json!({})), "chromium");
        assert_eq!(identify_browser(&json!({"Browser": "Safari/17"})), "chromium");
    }
}
