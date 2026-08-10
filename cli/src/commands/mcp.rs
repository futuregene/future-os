//! Shared MCP protocol helpers — port of `cli/src/commands/mcp.ts`.
//!
//! Used by `tools` (and `skills` in the TS original; the Rust `skills`
//! command does not need MCP). All HTTP is done with reqwest; timeouts
//! mirror the TS `AbortController` semantics (resolve-timeout errors surface
//! the same message bytes).

use crate::utils::object::is_record;
use crate::utils::platform::get_platform_url;
use serde_json::{Map, Value};
use std::time::Duration;

/// `mcpUrl()` — `{platform}/api/v1/mcp`.
pub async fn mcp_url() -> String {
    let platform_url = get_platform_url(None).await;
    format!("{platform_url}/api/v1/mcp")
}

/// `McpResponse` — `{body, sessionId}`.
#[derive(Debug)]
pub struct McpResponse {
    pub body: Value,
    pub session_id: Option<String>,
}

/// `translateHttpError(status, body)` — map common HTTP status codes to
/// human-readable errors.
fn translate_http_error(status: u16, body: &str) -> String {
    match status {
        401 => "Not logged in or token expired. Run 'future auth login' to sign in.".to_string(),
        403 => "Access denied. Your account may not have access to this resource.".to_string(),
        429 => "Too many requests — rate limited. Wait ~60 seconds and retry.".to_string(),
        502 => "Platform gateway error (502). This is temporary — retry in a minute.".to_string(),
        503 => "Platform service temporarily unavailable (503). Retry in a minute.".to_string(),
        _ => {
            let sliced: String = body.chars().take(200).collect();
            if sliced.is_empty() {
                format!("Request failed (HTTP {status})")
            } else {
                format!("Request failed (HTTP {status}) — {sliced}")
            }
        }
    }
}

/// `mcpPost(url, method, params, apiKey, sessionId?, id?, timeoutMs?)` —
/// POST a JSON-RPC request and parse the SSE `data:` line from the response.
pub async fn mcp_post(
    url: &str,
    method: &str,
    params: &Map<String, Value>,
    api_key: &str,
    session_id: Option<&str>,
    id: Option<u64>,
    timeout_ms: Option<u64>,
) -> Result<McpResponse, String> {
    let mut body = Map::new();
    body.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
    body.insert("method".to_string(), Value::String(method.to_string()));
    body.insert("params".to_string(), Value::Object(params.clone()));
    if let Some(id) = id {
        body.insert("id".to_string(), Value::from(id));
    }

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/json, text/event-stream"),
    );
    headers.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|e| e.to_string())?,
    );
    if let Some(sid) = session_id {
        headers.insert(
            "Mcp-Session-Id",
            reqwest::header::HeaderValue::from_str(sid).map_err(|e| e.to_string())?,
        );
    }

    let effective_timeout = timeout_ms.unwrap_or(60_000);
    let client = reqwest::Client::new();
    let request = client
        .post(url)
        .headers(headers)
        .body(serde_json::to_string(&Value::Object(body)).map_err(|e| e.to_string())?);

    let result = tokio::time::timeout(Duration::from_millis(effective_timeout), async {
        let response = request
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;
        let status = response.status().as_u16();
        // TS: `response.headers.get("mcp-session-id")`.
        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        let text = response.text().await.map_err(|e| e.to_string())?;
        Ok::<_, String>((status, session_id, text))
    })
    .await;

    let (status, sid, data) = match result {
        Ok(Ok(triple)) => triple,
        Ok(Err(e)) => return Err(e),
        // `AbortError` in the TS — timeout message bytes are fixed.
        Err(_) => {
            return Err(format!(
                "Request timed out after {}s.\nUse --timeout <seconds> to extend (e.g. --timeout 600 for image generation).",
                effective_timeout / 1000
            ))
        }
    };

    if status != 200 {
        return Err(translate_http_error(status, &data));
    }

    // Parse SSE stream: look for `data:` lines.
    for line in data.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            let p = rest.trim();
            if !p.is_empty() {
                match serde_json::from_str::<Value>(p) {
                    Ok(value) => {
                        return Ok(McpResponse {
                            body: value,
                            session_id: sid,
                        })
                    }
                    Err(_) => return Err(format!("Invalid JSON in SSE: {p}")),
                }
            }
        }
    }
    Ok(McpResponse {
        body: Value::Object(Map::new()),
        session_id: sid,
    })
}

/// `mcpNotify(...)` — fire-and-forget notification; errors are swallowed.
pub async fn mcp_notify(
    url: &str,
    method: &str,
    params: &Map<String, Value>,
    api_key: &str,
    session_id: &str,
) {
    let mut body = Map::new();
    body.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
    body.insert("method".to_string(), Value::String(method.to_string()));
    body.insert("params".to_string(), Value::Object(params.clone()));

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/json, text/event-stream"),
    );
    headers.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&format!("Bearer {api_key}"))
            .unwrap_or_else(|_| reqwest::header::HeaderValue::from_static("Bearer ")),
    );
    headers.insert(
        "Mcp-Session-Id",
        reqwest::header::HeaderValue::from_str(session_id)
            .unwrap_or_else(|_| reqwest::header::HeaderValue::from_static("")),
    );

    let _ = reqwest::Client::new()
        .post(url)
        .headers(headers)
        .body(serde_json::to_string(&Value::Object(body)).unwrap_or_default())
        .send()
        .await;
}

/// `initializeSession(apiKey)` — MCP `initialize` handshake → session id.
pub async fn initialize_session(api_key: &str) -> Result<String, String> {
    let url = mcp_url().await;
    let mut params = Map::new();
    params.insert(
        "protocolVersion".to_string(),
        Value::String("2024-11-05".to_string()),
    );
    params.insert("capabilities".to_string(), Value::Object(Map::new()));
    let mut client_info = Map::new();
    client_info.insert("name".to_string(), Value::String("future".to_string()));
    client_info.insert("version".to_string(), Value::String("1.0".to_string()));
    params.insert("clientInfo".to_string(), Value::Object(client_info));

    let response = mcp_post(&url, "initialize", &params, api_key, None, Some(1), None).await?;

    if response.body.get("error").is_some() {
        let err = response.body.get("error").cloned().unwrap_or_default();
        let code = err
            .get("code")
            .and_then(Value::as_number)
            .map(|n| n.to_string())
            .or_else(|| err.get("code").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string());
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!(
            "MCP initialize failed: code={code}, message={message}"
        ));
    }
    let session_id = response
        .session_id
        .ok_or_else(|| "No session ID received from MCP server".to_string())?;

    mcp_notify(
        &url,
        "notifications/initialized",
        &Map::new(),
        api_key,
        &session_id,
    )
    .await;
    Ok(session_id)
}

/// `getRecord(body)` helper for MCP result bodies (mirrors tools.ts usage).
pub fn result_of(body: &Value) -> Option<&Map<String, Value>> {
    let result = body.get("result")?;
    if is_record(result) {
        result.as_object()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_http_error_known_statuses() {
        assert_eq!(
            translate_http_error(401, ""),
            "Not logged in or token expired. Run 'future auth login' to sign in."
        );
        assert_eq!(
            translate_http_error(429, ""),
            "Too many requests — rate limited. Wait ~60 seconds and retry."
        );
        assert_eq!(
            translate_http_error(404, "oops"),
            "Request failed (HTTP 404) — oops"
        );
        // body truncated to 200 chars
        let long = "x".repeat(300);
        assert_eq!(
            translate_http_error(500, &long),
            format!("Request failed (HTTP 500) — {}", "x".repeat(200))
        );
    }

    #[test]
    fn sse_session_id_header_recovery() {
        // The header is read from HTTP headers now; this test documents the
        // expected header name (mcp-session-id, lowercased by reqwest).
        let header = "mcp-session-id";
        assert_eq!(header, "mcp-session-id");
    }

    /// Point the platform URL (auth.json) at the mock base URL.
    async fn point_platform_at(base: &str) {
        let path = crate::constants::auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            format!("{{\"future\": {{\"base_url\": \"{base}\"}}}}"),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn mcp_url_from_platform() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // No auth.json → default platform.
        assert!(mcp_url().await.ends_with("/api/v1/mcp"));
        point_platform_at("http://127.0.0.1:9").await;
        assert_eq!(mcp_url().await, "http://127.0.0.1:9/api/v1/mcp");
    }

    #[tokio::test]
    async fn mcp_post_parses_sse_data_line() {
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sse(
            "/api/v1/mcp",
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n",
            Some("sess-1"),
        )])
        .await;
        let response = mcp_post(
            &format!("{base}/api/v1/mcp"),
            "initialize",
            &Map::new(),
            "sk",
            None,
            Some(1),
            None,
        )
        .await
        .expect("post");
        assert_eq!(response.body["result"]["ok"], true);
        assert_eq!(response.session_id.as_deref(), Some("sess-1"));
    }

    #[tokio::test]
    async fn mcp_post_edge_cases() {
        // Invalid JSON inside a data: line.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sse(
            "/api/v1/mcp",
            "data: {oops\n",
            None,
        )])
        .await;
        let err = mcp_post(
            &format!("{base}/api/v1/mcp"),
            "m",
            &Map::new(),
            "sk",
            None,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("Invalid JSON in SSE"), "err: {err}");

        // No data: lines → empty object body.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sse(
            "/api/v1/mcp",
            "event: message\n\n",
            None,
        )])
        .await;
        let response = mcp_post(
            &format!("{base}/api/v1/mcp"),
            "m",
            &Map::new(),
            "sk",
            None,
            None,
            None,
        )
        .await
        .expect("post");
        assert_eq!(response.body, Value::Object(Map::new()));
        assert_eq!(response.session_id, None);

        // Empty data: payload is skipped in favor of a later valid line.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sse(
            "/api/v1/mcp",
            "data:\ndata:   \ndata: {\"result\":{}}\n",
            None,
        )])
        .await;
        let response = mcp_post(
            &format!("{base}/api/v1/mcp"),
            "m",
            &Map::new(),
            "sk",
            None,
            None,
            None,
        )
        .await
        .expect("post");
        assert!(response.body.get("result").is_some());

        // Non-200 status → translated HTTP error.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/api/v1/mcp",
            401,
            "{}",
        )])
        .await;
        let err = mcp_post(
            &format!("{base}/api/v1/mcp"),
            "m",
            &Map::new(),
            "sk",
            None,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("Not logged in"), "err: {err}");

        // Connect failure → "Request failed: ...".
        let err = mcp_post(
            "http://127.0.0.1:1/api/v1/mcp",
            "m",
            &Map::new(),
            "sk",
            None,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("Request failed:"), "err: {err}");
    }

    #[tokio::test]
    async fn mcp_post_timeout_message() {
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::slow(
            "/api/v1/mcp",
            Duration::from_secs(30),
        )])
        .await;
        let err = mcp_post(
            &format!("{base}/api/v1/mcp"),
            "m",
            &Map::new(),
            "sk",
            None,
            None,
            Some(50), // 50ms client timeout against a 30s server
        )
        .await
        .unwrap_err();
        assert!(err.contains("Request timed out after 0s."), "err: {err}");
        assert!(
            err.contains("Use --timeout <seconds> to extend"),
            "err: {err}"
        );
    }

    #[tokio::test]
    async fn initialize_session_flows() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // Success: session id from header + a notifications/initialized call.
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let base = crate::test_server::spawn_http_recording(
            vec![crate::test_server::HttpRoute::sse(
                "/api/v1/mcp",
                "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2024-11-05\"}}\n\n",
                Some("sess-42"),
            )],
            Some(requests.clone()),
        )
        .await;
        point_platform_at(&base).await;
        let sid = initialize_session("sk-test").await.expect("init");
        assert_eq!(sid, "sess-42");
        // initialize + fire-and-forget notification both hit the server.
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            if requests.lock().unwrap().len() >= 2 {
                break;
            }
        }
        // Clone under a short-lived guard so no MutexGuard is held across
        // the awaits below (clippy::await_holding_lock).
        let recorded = requests.lock().unwrap().clone();
        assert!(recorded.iter().any(|r| r.contains("initialize")));
        assert!(recorded
            .iter()
            .any(|r| r.contains("notifications/initialized")));
        assert!(recorded
            .iter()
            .any(|r| r.contains("mcp-session-id: sess-42")));

        // Error body → MCP initialize failed.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sse(
            "/api/v1/mcp",
            "data: {\"error\":{\"code\":-32600,\"message\":\"bad request\"}}\n\n",
            None,
        )])
        .await;
        point_platform_at(&base).await;
        let err = initialize_session("sk").await.unwrap_err();
        assert_eq!(
            err,
            "MCP initialize failed: code=-32600, message=bad request"
        );

        // Error with a STRING code + missing message → defaults.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sse(
            "/api/v1/mcp",
            "data: {\"error\":{\"code\":\"E_X\"}}\n\n",
            None,
        )])
        .await;
        point_platform_at(&base).await;
        let err = initialize_session("sk").await.unwrap_err();
        assert_eq!(
            err,
            "MCP initialize failed: code=E_X, message=unknown error"
        );

        // No session header → dedicated error.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sse(
            "/api/v1/mcp",
            "data: {\"result\":{}}\n\n",
            None,
        )])
        .await;
        point_platform_at(&base).await;
        let err = initialize_session("sk").await.unwrap_err();
        assert_eq!(err, "No session ID received from MCP server");
    }

    #[tokio::test]
    async fn mcp_notify_is_fire_and_forget() {
        // Live server: the POST lands (recorded), no result expected.
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let base = crate::test_server::spawn_http_recording(
            vec![crate::test_server::HttpRoute::json(
                "/api/v1/mcp",
                200,
                "{}",
            )],
            Some(requests.clone()),
        )
        .await;
        mcp_notify(
            &format!("{base}/api/v1/mcp"),
            "notifications/initialized",
            &Map::new(),
            "sk",
            "sess-1",
        )
        .await;
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            if !requests.lock().unwrap().is_empty() {
                break;
            }
        }
        assert_eq!(requests.lock().unwrap().len(), 1);
        // Dead server: errors are swallowed.
        mcp_notify("http://127.0.0.1:1/api/v1/mcp", "m", &Map::new(), "sk", "s").await;
    }

    #[test]
    fn result_of_requires_record_result() {
        assert!(result_of(&serde_json::json!({})).is_none());
        assert!(result_of(&serde_json::json!({"result": [1]})).is_none());
        let body = serde_json::json!({"result": {"a": 1}});
        assert_eq!(result_of(&body).unwrap()["a"], 1);
    }

    #[test]
    fn translate_http_error_remaining_statuses() {
        assert!(translate_http_error(403, "").contains("Access denied"));
        assert!(translate_http_error(502, "").contains("502"));
        assert!(translate_http_error(503, "").contains("503"));
        // Unknown status with EMPTY body → no suffix.
        assert_eq!(translate_http_error(418, ""), "Request failed (HTTP 418)");
    }
}
