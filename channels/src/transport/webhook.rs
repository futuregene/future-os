//! A minimal inbound webhook server.
//!
//! Several platforms only deliver messages by calling *us*: they need a public
//! HTTPS endpoint, verify it once with a challenge, then POST signed bodies.
//! Answering that needs an HTTP server, and this one is deliberately small:
//! HTTP/1.1, one request per connection, `Content-Length` bodies only, no TLS.
//!
//! The limits are the point. It is enough to receive and verify a webhook, and
//! a deployment that needs more (TLS, keep-alive, HTTP/2) should terminate it
//! on a reverse proxy and forward here — which is also how the public URL gets
//! provisioned in the first place.

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

/// Largest request head we will read (`request line + headers`).
const MAX_HEAD_BYTES: usize = 16 * 1024;
/// Largest body we will read.
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// One inbound request.
#[derive(Debug, Clone)]
pub struct WebhookRequest {
    pub method: String,
    /// Path without the query string, e.g. `/webhooks/whatsapp`.
    pub path: String,
    /// Raw query string (empty when absent).
    pub query: String,
    /// Header names lowercased, values as sent.
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl WebhookRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Parsed body, or `Value::Null` when the body is not JSON.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }

    pub fn query_param(&self, name: &str) -> Option<String> {
        parse_query(&self.query).remove(name)
    }
}

/// One response.
#[derive(Debug, Clone)]
pub struct WebhookResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl WebhookResponse {
    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8".into(),
            body: body.into().into_bytes(),
        }
    }

    pub fn json(status: u16, body: &serde_json::Value) -> Self {
        Self {
            status,
            content_type: "application/json".into(),
            body: body.to_string().into_bytes(),
        }
    }

    pub fn ok() -> Self {
        Self::text(200, "ok")
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::text(400, message)
    }

    pub fn unauthorized() -> Self {
        Self::text(401, "invalid signature")
    }

    pub fn not_found() -> Self {
        Self::text(404, "not found")
    }
}

type HandlerFuture = Pin<Box<dyn Future<Output = WebhookResponse> + Send>>;
type Handler = Arc<dyn Fn(WebhookRequest) -> HandlerFuture + Send + Sync>;

/// A tiny HTTP server with a route table.
pub struct WebhookServer {
    listener: TcpListener,
    routes: HashMap<(String, String), Handler>,
}

impl WebhookServer {
    /// Bind `addr` (`127.0.0.1:0` for an ephemeral port in tests).
    pub async fn bind(addr: &str) -> Result<Self> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|error| anyhow!("webhook server cannot bind {addr}: {error}"))?;
        Ok(Self {
            listener,
            routes: HashMap::new(),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.listener.local_addr()?)
    }

    /// Register a handler for one method + path.
    pub fn on<F, Fut>(mut self, method: &str, path: &str, handler: F) -> Self
    where
        F: Fn(WebhookRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = WebhookResponse> + Send + 'static,
    {
        let handler: Handler = Arc::new(move |request| Box::pin(handler(request)));
        self.routes
            .insert((method.to_ascii_uppercase(), path.to_string()), handler);
        self
    }

    /// Serve until `shutdown` fires.
    pub async fn serve(self, shutdown: Arc<Notify>) -> Result<()> {
        loop {
            let accepted = tokio::select! {
                accepted = self.listener.accept() => accepted,
                _ = shutdown.notified() => {
                    tracing::info!("webhook server stopped");
                    return Ok(());
                }
            };
            let (stream, peer) = match accepted {
                Ok(pair) => pair,
                Err(error) => {
                    tracing::warn!(%error, "webhook accept failed");
                    continue;
                }
            };
            let routes = self.routes.clone();
            tokio::spawn(async move {
                if let Err(error) = handle_connection(stream, routes).await {
                    let message = format!("webhook connection ended with an error: {error}");
                    tracing::debug!(%peer, "{message}");
                }
            });
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    routes: HashMap<(String, String), Handler>,
) -> Result<()> {
    let mut stream = stream;
    let mut buffer: Vec<u8> = Vec::with_capacity(2048);
    let mut head_end = None;
    while head_end.is_none() {
        let mut chunk = [0u8; 2048];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        head_end = find_head_end(&buffer);
        if head_end.is_none() && buffer.len() > MAX_HEAD_BYTES {
            let _ = stream
                .write_all(&serialize(431, "text/plain", b"headers too large"))
                .await;
            return Ok(());
        }
    }
    let head_end = head_end.unwrap_or(0);
    let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
    let (method, path, query, headers) = match parse_head(&head) {
        Some(parsed) => parsed,
        None => {
            let _ = stream
                .write_all(&serialize(400, "text/plain", b"malformed request"))
                .await;
            return Ok(());
        }
    };

    let content_length: usize = headers
        .get("content-length")
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        let _ = stream
            .write_all(&serialize(413, "text/plain", b"body too large"))
            .await;
        return Ok(());
    }
    let mut body = buffer[head_end + 4..].to_vec();
    while body.len() < content_length {
        let mut chunk = [0u8; 8192];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);

    let request = WebhookRequest {
        method: method.to_ascii_uppercase(),
        path: path.clone(),
        query,
        headers,
        body,
    };
    let response = match routes.get(&(request.method.clone(), path)) {
        Some(handler) => handler(request).await,
        None => WebhookResponse::not_found(),
    };
    stream
        .write_all(&serialize(
            response.status,
            &response.content_type,
            &response.body,
        ))
        .await?;
    stream.flush().await?;
    Ok(())
}

fn find_head_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

type ParsedHead = (String, String, String, HashMap<String, String>);

fn parse_head(head: &str) -> Option<ParsedHead> {
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?;
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (target.to_string(), String::new()),
    };
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    Some((method, path, query, headers))
}

fn serialize(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status_text(status),
        body.len()
    );
    let mut out = head.into_bytes();
    out.extend_from_slice(body);
    out
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        _ => "Response",
    }
}

/// Decode a query string. `+` means a space; `%XX` is a byte.
pub fn parse_query(query: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (name, value) = match pair.split_once('=') {
            Some((name, value)) => (name, value),
            None => (pair, ""),
        };
        out.insert(percent_decode(name), percent_decode(value));
    }
    out
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
                match hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    None => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_query_decodes_escapes_and_plus() {
        let params = parse_query("hub.mode=subscribe&hub.verify_token=a%2Bb&hub.challenge=12345");
        assert_eq!(
            params.get("hub.mode").map(String::as_str),
            Some("subscribe")
        );
        assert_eq!(
            params.get("hub.verify_token").map(String::as_str),
            Some("a+b")
        );
        assert_eq!(
            params.get("hub.challenge").map(String::as_str),
            Some("12345")
        );
    }

    #[test]
    fn parse_query_handles_empty_and_valueless_pairs() {
        assert!(parse_query("").is_empty());
        let params = parse_query("flag&flag=");
        assert_eq!(params.get("flag").map(String::as_str), Some(""));
    }

    #[test]
    fn parse_query_keeps_invalid_escapes_literal() {
        let params = parse_query("token=%zz%");
        assert_eq!(params.get("token").map(String::as_str), Some("%zz%"));
    }

    #[test]
    fn parse_head_splits_target_and_headers() {
        let head = "POST /hook?a=1 HTTP/1.1\r\nHost: example.test\r\nX-Signature: abc\r\n";
        let (method, path, query, headers) = parse_head(head).unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/hook");
        assert_eq!(query, "a=1");
        assert_eq!(headers.get("x-signature").map(String::as_str), Some("abc"));
    }

    #[test]
    fn parse_head_rejects_an_empty_request_line() {
        assert!(parse_head("").is_none());
    }

    #[test]
    fn serialize_writes_a_content_length_and_closes() {
        let raw = String::from_utf8(serialize(200, "text/plain", b"hi")).unwrap();
        assert!(raw.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(raw.contains("Content-Length: 2\r\n"));
        assert!(raw.contains("Connection: close"));
        assert!(raw.ends_with("\r\n\r\nhi"));
    }

    fn request(method: &str, path: &str) -> String {
        format!("{method} {path} HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\nok")
    }

    #[test]
    fn a_request_exposes_its_headers_body_and_query() {
        let mut headers = HashMap::new();
        headers.insert("x-signature".to_string(), "abc".to_string());
        let request = WebhookRequest {
            method: "POST".into(),
            path: "/hook".into(),
            query: "hub.mode=subscribe&hub.challenge=42".into(),
            headers,
            body: br#"{"event":"message"}"#.to_vec(),
        };
        assert_eq!(request.header("X-Signature"), Some("abc"));
        assert_eq!(request.header("missing"), None);
        assert_eq!(request.json()["event"], "message");
        assert_eq!(request.query_param("hub.challenge").as_deref(), Some("42"));
        assert_eq!(request.query_param("nope"), None);

        // A body that is not JSON is reported as null rather than panicking:
        // webhooks are attacker-reachable, so a bad body is normal input.
        let plain = WebhookRequest {
            body: b"not json".to_vec(),
            ..request
        };
        assert_eq!(plain.json(), serde_json::Value::Null);
    }

    #[test]
    fn the_short_response_constructors_set_their_status_and_body() {
        let ok = WebhookResponse::ok();
        assert_eq!(ok.status, 200);
        assert_eq!(String::from_utf8_lossy(&ok.body), "ok");
        assert!(ok.content_type.starts_with("text/plain"));

        let bad = WebhookResponse::bad_request("no signature");
        assert_eq!(bad.status, 400);
        assert_eq!(String::from_utf8_lossy(&bad.body), "no signature");

        let unauthorized = WebhookResponse::unauthorized();
        assert_eq!(unauthorized.status, 401);
        let missing = WebhookResponse::not_found();
        assert_eq!(missing.status, 404);

        let json = WebhookResponse::json(200, &serde_json::json!({"ok": true}));
        assert_eq!(json.content_type, "application/json");
        assert_eq!(String::from_utf8_lossy(&json.body), r#"{"ok":true}"#);
    }

    async fn round_trip(server: WebhookServer, raw: &str) -> String {
        let (addr, shutdown, serving) = serve(server).await;
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(raw.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        stop(shutdown, serving).await;
        String::from_utf8_lossy(&response).to_string()
    }

    /// Start a server on an ephemeral port and hand back its address plus the
    /// handles needed to stop it.
    async fn serve(
        server: WebhookServer,
    ) -> (SocketAddr, Arc<Notify>, tokio::task::JoinHandle<Result<()>>) {
        let addr = server.local_addr().unwrap();
        let shutdown = Arc::new(Notify::new());
        let serving = {
            let shutdown = shutdown.clone();
            tokio::spawn(async move { server.serve(shutdown).await })
        };
        (addr, shutdown, serving)
    }

    async fn stop(shutdown: Arc<Notify>, serving: tokio::task::JoinHandle<Result<()>>) {
        shutdown.notify_waiters();
        let _ = tokio::time::timeout(Duration::from_secs(2), serving).await;
    }

    use std::time::Duration;

    #[tokio::test]
    async fn a_reqwest_json_post_is_answered() {
        let server = WebhookServer::bind("127.0.0.1:0").await.unwrap().on(
            "POST",
            "/hook",
            |request| async move {
                let body = String::from_utf8_lossy(&request.body).to_string();
                WebhookResponse::json(200, &serde_json::json!({ "seen": body }))
            },
        );
        let (addr, shutdown, serving) = serve(server).await;
        let client = reqwest::Client::builder().http1_only().build().unwrap();
        let payload = serde_json::json!({"pad": "x".repeat(2000)});
        let response = client
            .post(format!("http://{addr}/hook"))
            .json(&payload)
            .send()
            .await
            .expect("post");
        assert_eq!(response.status().as_u16(), 200);
        let text = response.text().await.unwrap();
        assert!(text.contains("pad"), "{text}");
        stop(shutdown, serving).await;
    }

    #[tokio::test]
    async fn a_body_split_across_writes_is_read_in_full() {
        // A real client can deliver the head and the body in separate packets;
        // the server must keep reading until Content-Length is satisfied.
        let server = WebhookServer::bind("127.0.0.1:0").await.unwrap().on(
            "POST",
            "/hook",
            |request| async move {
                WebhookResponse::text(200, String::from_utf8_lossy(&request.body).to_string())
            },
        );
        let (addr, shutdown, serving) = serve(server).await;
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"POST /hook HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\n")
            .await
            .unwrap();
        stream.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        stream.write_all(b"hello").await.unwrap();
        let mut response = Vec::new();
        let _ =
            tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut response)).await;
        stop(shutdown, serving).await;
        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with("hello"), "{response}");
    }

    #[tokio::test]
    async fn a_client_that_stops_mid_body_ends_the_connection_quietly() {
        // Content-Length promises more than the client sends: the server must
        // stop reading and answer, not hang.
        let server =
            WebhookServer::bind("127.0.0.1:0").await.unwrap().on(
                "POST",
                "/hook",
                |request| async move {
                    WebhookResponse::text(200, format!("got {}", request.body.len()))
                },
            );
        let (addr, shutdown, serving) = serve(server).await;
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"POST /hook HTTP/1.1\r\nHost: x\r\nContent-Length: 40\r\n\r\nonly-this")
            .await
            .unwrap();
        stream.flush().await.unwrap();
        // Half-close the write side: the server sees EOF while waiting for the
        // rest of the body.
        stream.shutdown().await.unwrap();
        let mut response = Vec::new();
        let _ =
            tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut response)).await;
        stop(shutdown, serving).await;
        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with("got 9"), "{response}");
    }

    #[tokio::test]
    async fn an_oversized_header_block_is_rejected() {
        // Headers with no end marker: the server must refuse rather than read
        // an unbounded amount of memory.
        let server = WebhookServer::bind("127.0.0.1:0").await.unwrap();
        let (addr, shutdown, serving) = serve(server).await;
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let filler = format!("X-Pad: {}\r\n", "a".repeat(1024));
        for _ in 0..20 {
            stream.write_all(filler.as_bytes()).await.unwrap();
        }
        let mut response = Vec::new();
        let _ =
            tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut response)).await;
        stop(shutdown, serving).await;
        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with("HTTP/1.1 431"), "{response}");
    }

    #[tokio::test]
    async fn a_connection_that_never_sends_a_request_is_dropped_quietly() {
        let server = WebhookServer::bind("127.0.0.1:0").await.unwrap();
        let (addr, shutdown, serving) = serve(server).await;
        let stream = TcpStream::connect(addr).await.unwrap();
        drop(stream);
        tokio::time::sleep(Duration::from_millis(50)).await;
        stop(shutdown, serving).await;
    }

    #[test]
    fn status_text_covers_the_codes_we_emit() {
        assert_eq!(status_text(200), "OK");
        assert_eq!(status_text(400), "Bad Request");
        assert_eq!(status_text(401), "Unauthorized");
        assert_eq!(status_text(403), "Forbidden");
        assert_eq!(status_text(404), "Not Found");
        assert_eq!(status_text(413), "Payload Too Large");
        assert_eq!(status_text(431), "Request Header Fields Too Large");
        assert_eq!(status_text(500), "Internal Server Error");
        assert_eq!(status_text(418), "Response");
    }

    #[test]
    fn percent_decoding_treats_plus_as_a_space() {
        let params = parse_query("token=a+b&other=c%20d");
        assert_eq!(params.get("token").map(String::as_str), Some("a b"));
        assert_eq!(params.get("other").map(String::as_str), Some("c d"));
    }

    #[test]
    fn find_head_end_locates_the_blank_line() {
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\n\r\nrest"), Some(14));
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\n"), None);
    }

    #[tokio::test]
    async fn serves_a_registered_route_and_sees_the_body() {
        let server = WebhookServer::bind("127.0.0.1:0").await.unwrap().on(
            "POST",
            "/hook",
            |request| async move {
                let body = String::from_utf8_lossy(&request.body).to_string();
                WebhookResponse::json(200, &serde_json::json!({ "seen": body }))
            },
        );
        let response = round_trip(server, &request("POST", "/hook")).await;
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains(r#"{"seen":"ok"}"#), "{response}");
    }

    #[tokio::test]
    async fn unknown_routes_and_methods_get_404() {
        let server =
            WebhookServer::bind("127.0.0.1:0")
                .await
                .unwrap()
                .on("GET", "/hook", |_| async { WebhookResponse::ok() });
        let response = round_trip(server, &request("POST", "/hook")).await;
        assert!(response.starts_with("HTTP/1.1 404"), "{response}");
    }

    #[tokio::test]
    async fn a_malformed_request_line_is_rejected() {
        let server = WebhookServer::bind("127.0.0.1:0").await.unwrap();
        let response = round_trip(server, "BROKEN\r\n\r\n").await;
        assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    }

    #[tokio::test]
    async fn an_oversized_declared_body_is_rejected() {
        let server = WebhookServer::bind("127.0.0.1:0").await.unwrap();
        let raw = format!(
            "POST /hook HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_BYTES + 1
        );
        let response = round_trip(server, &raw).await;
        assert!(response.starts_with("HTTP/1.1 413"), "{response}");
    }
}
