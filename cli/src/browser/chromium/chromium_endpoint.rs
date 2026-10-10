//! CDP endpoint resolver — port of
//! `cli/src/browser/chromium/chromium-endpoint.ts`.
//!
//! GET `<endpoint>/json/version` → webSocketDebuggerUrl + browser identity
//! (chrome / edge / chromium).
//!
//! Two transports reach that GET: an `http(s)` base URL over TCP, and a local
//! socket (see [`crate::browser::target`]). The socket path speaks HTTP/1.1 by
//! hand because the endpoint is the only thing it is ever used for.

use crate::browser::target::{EndpointTarget, SocketSpec};
use serde_json::Value;

/// `CdpEndpointInfo`.
#[derive(Debug, Clone)]
pub struct CdpEndpointInfo {
    pub http_endpoint: String,
    pub web_socket_debugger_url: String,
    pub browser_kind: String,
    pub browser_version: Option<String>,
    /// Set when the endpoint is a local socket.
    ///
    /// The WebSocket must then be dialled over that socket, and
    /// [`Self::web_socket_debugger_url`] holds only a **path** — see
    /// [`resolve_cdp_endpoint`].
    pub socket: Option<SocketSpec>,
    /// The `/json/version` payload as Chrome sent it, for callers that report
    /// browser identity to the user (`status`).
    pub version: Value,
}

/// `resolveCdpEndpoint(httpEndpoint, timeoutMs = 5000)`.
///
/// For a socket endpoint, `web_socket_debugger_url` is reduced to the URL's
/// **path** (`/devtools/browser/<id>`) and the host is dropped. Chrome always
/// reports a loopback host in that field, and for a socket endpoint that host
/// is meaningless: nothing is listening on it, because the whole point of the
/// socket is that no TCP port exists. Keeping it would produce a WebSocket that
/// looks correctly configured and can never connect.
///
/// In other words: **the endpoint decides the transport, `/json/version`
/// decides the path.**
pub async fn resolve_cdp_endpoint(
    endpoint: &str,
    timeout_ms: u64,
) -> Result<CdpEndpointInfo, String> {
    let target = EndpointTarget::parse(endpoint)?;
    let (status, body) = fetch_version(&target, timeout_ms).await?;
    if !(200..300).contains(&status) {
        return Err(format!("CDP /json/version returned HTTP {status}"));
    }
    let data: Value =
        serde_json::from_slice(&body).map_err(|_| "Invalid /json/version response".to_string())?;

    let reported = data
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .map(str::to_string);

    let Some(reported) = reported else {
        return Err(format!(
            "Invalid webSocketDebuggerUrl in /json/version: {}",
            serde_json::to_string(&data).unwrap_or_default()
        ));
    };
    if !reported.starts_with("ws") {
        return Err(format!(
            "Invalid webSocketDebuggerUrl in /json/version: {}",
            serde_json::to_string(&data).unwrap_or_default()
        ));
    }

    let socket = target.socket().cloned();
    // Over a socket the host:port Chrome reports is not reachable; only the
    // path identifies the target on the other end.
    let web_socket_debugger_url = match &socket {
        Some(_) => ws_path_of(&reported).ok_or_else(|| {
            format!("Invalid webSocketDebuggerUrl in /json/version: {reported:?} has no path")
        })?,
        None => reported,
    };

    let browser_kind = identify_browser(&data);
    let browser_version = data
        .get("Browser")
        .and_then(Value::as_str)
        .or_else(|| data.get("browser").and_then(Value::as_str))
        .map(str::to_string);

    Ok(CdpEndpointInfo {
        http_endpoint: endpoint.to_string(),
        web_socket_debugger_url,
        browser_kind,
        browser_version,
        socket,
        version: data,
    })
}

/// `/devtools/browser/<id>` from `ws://host/devtools/browser/<id>`.
fn ws_path_of(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://")?.1;
    after_scheme
        .find('/')
        .map(|index| after_scheme[index..].to_string())
}

/// GET `/json/version` over whichever transport the endpoint names.
async fn fetch_version(target: &EndpointTarget, timeout_ms: u64) -> Result<(u16, Vec<u8>), String> {
    match target {
        EndpointTarget::Http(base) => {
            let client = reqwest::Client::new();
            let response = tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                client
                    .get(format!("{base}/json/version"))
                    .timeout(std::time::Duration::from_millis(timeout_ms))
                    .send(),
            )
            .await
            .map_err(|_| "CDP /json/version timed out".to_string())?
            .map_err(|e| {
                if e.is_timeout() {
                    "CDP /json/version timed out".to_string()
                } else {
                    e.to_string()
                }
            })?;
            let status = response.status().as_u16();
            let body = response
                .bytes()
                .await
                .map_err(|e| format!("CDP /json/version body could not be read: {e}"))?;
            Ok((status, body.to_vec()))
        }
        EndpointTarget::Socket(spec) => {
            let attempt = super::socket_http::get(spec, "/json/version");
            tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), attempt)
                .await
                .map_err(|_| format!("CDP /json/version timed out over socket {spec}"))?
        }
    }
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

    // ── Socket transport ──────────────────────────────────────────────

    /// Serve one HTTP/1.1 response over a bound Unix socket, then close.
    ///
    /// Mirrors what Chrome's DevTools server does: answer the single
    /// `/json/version` request and hang up (`Connection: close`). The listener
    /// is bound by the caller so the test never races the server task.
    async fn serve_one_http_over_socket(listener: tokio::net::UnixListener, response: String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buf = [0u8; 4096];
        let read = stream.read(&mut buf).await.expect("read request");
        let request = String::from_utf8_lossy(&buf[..read]).to_string();
        // The request line proves which path the tool asked for, and the Host
        // header that the handshake's authority is loopback (Chrome's
        // DNS-rebinding guard rejects anything else).
        assert!(
            request.starts_with("GET /json/version HTTP/1.1"),
            "{request}"
        );
        assert!(request.contains("Host: localhost"), "{request}");
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write response");
        stream.flush().await.expect("flush");
    }

    /// Bind a socket under a per-test temp dir, returning the path.
    fn bind_temp_socket(label: &str) -> (std::path::PathBuf, tokio::net::UnixListener) {
        let dir = std::env::temp_dir().join(format!("cdp-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("chrome.sock");
        let _ = std::fs::remove_file(&socket);
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
        (socket, listener)
    }

    fn cleanup(socket: &std::path::Path) {
        let _ = std::fs::remove_file(socket);
        if let Some(dir) = socket.parent() {
            let _ = std::fs::remove_dir(dir);
        }
    }

    fn version_json(ws_url: &str) -> String {
        json!({
            "Browser": "Chrome/154.0.8037.126",
            "User-Agent": "Mozilla/5.0 (Linux; Android 10; K) Chrome/154.0.0.0 Mobile Safari/537.36",
            "Android-Package": "com.android.chrome",
            "webSocketDebuggerUrl": ws_url,
        })
        .to_string()
    }

    fn http_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_socket_endpoint_is_reduced_to_the_web_socket_path() {
        let (socket, listener) = bind_temp_socket("sock");

        // Chrome reports a loopback host even on a socket endpoint. That host
        // is NOT reachable (there is no TCP port at all), so the tool must keep
        // only the path — otherwise the WebSocket step fails with a confusing
        // error while /json/version appeared to succeed.
        let body = version_json("ws://127.0.0.1:9222/devtools/browser/abc-123");
        let server = tokio::spawn(serve_one_http_over_socket(listener, http_response(&body)));

        let endpoint = format!("unix:{}", socket.display());
        let info = resolve_cdp_endpoint(&endpoint, 5_000)
            .await
            .expect("resolve over socket");

        assert_eq!(info.web_socket_debugger_url, "/devtools/browser/abc-123");
        assert_eq!(
            info.socket,
            Some(SocketSpec::Path(socket.display().to_string()))
        );
        assert_eq!(info.browser_kind, "chrome");
        assert_eq!(
            info.browser_version.as_deref(),
            Some("Chrome/154.0.8037.126")
        );

        server.await.unwrap();
        cleanup(&socket);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_chunked_socket_response_is_decoded() {
        let (socket, listener) = bind_temp_socket("chunk");

        let body = version_json("ws://127.0.0.1:9222/devtools/browser/xyz");
        // Split the body across two chunks to prove the framing is real.
        let (first, second) = body.split_at(body.len() / 2);
        let framed = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{first}\r\n{:x}\r\n{second}\r\n0\r\n\r\n",
            first.len(),
            second.len()
        );
        let server = tokio::spawn(serve_one_http_over_socket(listener, framed));

        let info = resolve_cdp_endpoint(&format!("unix:{}", socket.display()), 5_000)
            .await
            .expect("resolve chunked");
        assert_eq!(info.web_socket_debugger_url, "/devtools/browser/xyz");

        server.await.unwrap();
        cleanup(&socket);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_missing_socket_reports_which_socket_failed() {
        let endpoint = "unix:/tmp/definitely-not-there-cdp.sock";
        let err = resolve_cdp_endpoint(endpoint, 1_000).await.unwrap_err();
        // The message names the socket, so the user can tell a missing browser
        // from a wrong path.
        assert!(err.contains("definitely-not-there-cdp.sock"), "{err}");
    }

    /// A complete response is recognised without waiting for the peer to close.
    ///
    /// Chrome's DevTools HTTP server keeps the connection alive even when asked
    /// to close, so "read until EOF" would hang until the timeout. This is the
    /// case that made the real socket path report `reachable: false` while
    /// every mock-based test passed.
    #[test]
    fn ws_path_reduction_handles_urls_without_a_path() {
        assert_eq!(
            ws_path_of("ws://127.0.0.1:9222/devtools/browser/a").as_deref(),
            Some("/devtools/browser/a")
        );
        assert_eq!(ws_path_of("ws://host"), None);
        assert_eq!(ws_path_of("no-scheme/devtools"), None);
    }

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
            (
                r#"{"Browser":"Edg/120","webSocketDebuggerUrl":"ws://x/ws"}"#,
                "edge",
            ),
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
    async fn response_body_is_covered_by_timeout() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n{")
                .await
                .unwrap();
            // Hold the incomplete body past the client's 50 ms deadline, then
            // let the task finish: an abandoned `pending()` future would leave
            // this closure's end (and the socket's drop) for the abort, which
            // is where the line-coverage artifact came from.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            drop(socket);
        });
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            resolve_cdp_endpoint(&format!("http://{address}"), 50),
        )
        .await;
        assert!(result.expect("body read must terminate").is_err());
        // Let the server finish rather than leaving the task suspended: its
        // read loop runs to its end (and closes the socket) only if someone
        // waits for it.
        server.await.expect("server task completes");
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
        assert_eq!(
            identify_browser(&json!({"Browser": "Chrome/126"})),
            "chrome"
        );
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
        assert_eq!(
            identify_browser(&json!({"Browser": "Safari/17"})),
            "chromium"
        );
    }
}
