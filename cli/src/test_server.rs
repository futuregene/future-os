//! Test-only mock servers shared across command/rpc tests.
//!
//! - [`MockAgent`] — a tonic `FutureAgent` gRPC server with per-command-type
//!   canned responses and configurable failure modes.
//! - [`spawn_http`] — a minimal HTTP/1.1 responder for the platform-API
//!   commands (account/auth), routing by request path.

use std::collections::{HashMap, HashSet};
use std::net::TcpListener;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{stream, StreamExt as _};
use tonic::transport::Server;

use future_rpc::proto::future_agent_server::{FutureAgent, FutureAgentServer};
use future_rpc::proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};

/// Configurable mock agent: canned `data` payloads per command type,
/// failure injection, a canned event stream, and a record of every command
/// received (for asserting the client sent the right fields).
#[derive(Clone, Default)]
pub struct MockAgent {
    /// Command type → `data` JSON string returned with success=true.
    pub responses: HashMap<String, String>,
    /// Types answered with success=false and error="boom".
    pub fail_types: HashSet<String>,
    /// Types answered with success=false and a CUSTOM error message.
    pub fail_with: HashMap<String, String>,
    /// Types answered with success=false and an EMPTY error message.
    pub fail_silent_types: HashSet<String>,
    /// Types that fail the unary call with a message-less tonic Status.
    pub status_empty_types: HashSet<String>,
    /// Types that fail the unary call with a message-bearing tonic Status.
    pub status_message_types: HashSet<String>,
    /// Canned events for stream_events.
    pub events: Vec<StreamEvent>,
    /// Emit a stream-level error after the canned events.
    pub stream_error_after: bool,
    /// Fail the stream_events RPC itself with this Status.
    pub stream_status_error: Option<tonic::Status>,
    /// Every command received, in arrival order.
    pub seen: Arc<Mutex<Vec<RpcCommand>>>,
}

impl MockAgent {
    /// Convenience builder: one canned response.
    pub fn respond(r#type: &str, data: &str) -> Self {
        let mut agent = MockAgent::default();
        agent.responses.insert(r#type.to_string(), data.to_string());
        agent
    }

    /// Recorded commands of the given type.
    pub fn seen_of(&self, r#type: &str) -> Vec<RpcCommand> {
        self.seen
            .lock()
            .expect("seen")
            .iter()
            .filter(|c| c.r#type == r#type)
            .cloned()
            .collect()
    }
}

#[tonic::async_trait]
impl FutureAgent for MockAgent {
    async fn execute_command(
        &self,
        request: tonic::Request<RpcCommand>,
    ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
        let cmd = request.into_inner();
        self.seen.lock().expect("seen").push(cmd.clone());
        if self.status_empty_types.contains(&cmd.r#type) {
            return Err(tonic::Status::new(tonic::Code::Unknown, ""));
        }
        if self.status_message_types.contains(&cmd.r#type) {
            return Err(tonic::Status::unavailable("transport down"));
        }
        let data = self
            .responses
            .get(&cmd.r#type)
            .cloned()
            .unwrap_or_else(|| "{}".to_string());
        let success = !self.fail_types.contains(&cmd.r#type)
            && !self.fail_silent_types.contains(&cmd.r#type)
            && !self.fail_with.contains_key(&cmd.r#type);
        Ok(tonic::Response::new(RpcResponse {
            id: cmd.id,
            r#type: "response".into(),
            command: cmd.r#type.clone(),
            success,
            data,
            error: if success || self.fail_silent_types.contains(&cmd.r#type) {
                String::new()
            } else if let Some(custom) = self.fail_with.get(&cmd.r#type) {
                custom.clone()
            } else {
                "boom".into()
            },
            error_code: String::new(),
            error_data: String::new(),
            payload: None,
        }))
    }

    type StreamEventsStream =
        Pin<Box<dyn futures_util::Stream<Item = Result<StreamEvent, tonic::Status>> + Send>>;

    async fn stream_events(
        &self,
        _request: tonic::Request<StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        if let Some(status) = &self.stream_status_error {
            return Err(status.clone());
        }
        let canned = stream::iter(self.events.clone().into_iter().map(Ok));
        if self.stream_error_after {
            let err = stream::once(async { Err(tonic::Status::internal("mid-stream boom")) });
            return Ok(tonic::Response::new(Box::pin(canned.chain(err))));
        }
        Ok(tonic::Response::new(Box::pin(canned)))
    }
}

/// Spawn the mock on an ephemeral port; returns "127.0.0.1:<port>".
///
/// The socket stays bound and listening across the handover to the tonic
/// server (`serve_with_incoming`), so there is no drop/re-bind window in
/// which a parallel test can steal the port, and clients can connect the
/// moment this returns (the OS backlog holds the handshake until the server
/// task first polls accept) — no fixed startup sleep to race under load.
pub async fn spawn_mock(agent: MockAgent) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    listener.set_nonblocking(true).expect("nonblocking");
    let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener from std");
    tokio::spawn(
        Server::builder()
            .add_service(FutureAgentServer::new(agent))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener)),
    );
    format!("127.0.0.1:{}", addr.port())
}

/// Build a StreamEvent with a type and JSON data payload.
pub fn stream_event(r#type: &str, data: &str) -> StreamEvent {
    StreamEvent {
        r#type: r#type.into(),
        data: data.into(),
        ..Default::default()
    }
}

// ── HTTP mock ───────────────────────────────────────────────────────────────

/// One canned HTTP response.
pub struct HttpResponse {
    pub status: u16,
    pub content_type: String,
    pub extra_headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Sleep this long BEFORE responding (timeout tests).
    pub delay: Duration,
}

/// One canned HTTP route: exact path → one or more responses. With multiple
/// responses they are consumed in order (device-code polling tests); the
/// last one repeats once exhausted.
pub struct HttpRoute {
    pub path: String,
    pub responses: Vec<HttpResponse>,
    pub index: std::sync::atomic::AtomicUsize,
}

impl HttpRoute {
    fn single(path: &str, response: HttpResponse) -> Self {
        HttpRoute {
            path: path.to_string(),
            responses: vec![response],
            index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn json(path: &str, status: u16, body: &str) -> Self {
        HttpRoute::single(
            path,
            HttpResponse {
                status,
                content_type: "application/json".to_string(),
                extra_headers: vec![],
                body: body.as_bytes().to_vec(),
                delay: Duration::ZERO,
            },
        )
    }

    /// Binary body variant (zip downloads).
    pub fn binary(path: &str, status: u16, body: Vec<u8>) -> Self {
        HttpRoute::single(
            path,
            HttpResponse {
                status,
                content_type: "application/octet-stream".to_string(),
                extra_headers: vec![],
                body,
                delay: Duration::ZERO,
            },
        )
    }

    /// SSE variant (MCP): `data: ...` body with an optional session header.
    pub fn sse(path: &str, body: &str, session_id: Option<&str>) -> Self {
        let extra_headers = session_id
            .map(|sid| vec![("mcp-session-id".to_string(), sid.to_string())])
            .unwrap_or_default();
        HttpRoute::single(
            path,
            HttpResponse {
                status: 200,
                content_type: "text/event-stream".to_string(),
                extra_headers,
                body: body.as_bytes().to_vec(),
                delay: Duration::ZERO,
            },
        )
    }

    /// Hang for `delay` before answering (client-timeout tests).
    pub fn slow(path: &str, delay: Duration) -> Self {
        let mut response = HttpRoute::json(path, 200, "{}").responses.remove(0);
        response.delay = delay;
        HttpRoute::single(path, response)
    }

    /// Stateful route: successive calls return successive responses.
    pub fn sequence(path: &str, responses: Vec<(u16, &str)>) -> Self {
        HttpRoute {
            path: path.to_string(),
            responses: responses
                .into_iter()
                .map(|(s, b)| HttpResponse {
                    status: s,
                    content_type: "application/json".to_string(),
                    extra_headers: vec![],
                    body: b.as_bytes().to_vec(),
                    delay: Duration::ZERO,
                })
                .collect(),
            index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Stateful SSE route: (body, session_id) pairs consumed in order —
    /// the MCP initialize → notify → call request chain hits one path.
    pub fn sse_sequence(path: &str, responses: Vec<(&str, Option<&str>)>) -> Self {
        HttpRoute {
            path: path.to_string(),
            responses: responses
                .into_iter()
                .map(|(b, sid)| HttpResponse {
                    status: 200,
                    content_type: "text/event-stream".to_string(),
                    extra_headers: sid
                        .map(|s| vec![("mcp-session-id".to_string(), s.to_string())])
                        .unwrap_or_default(),
                    body: b.as_bytes().to_vec(),
                    delay: Duration::ZERO,
                })
                .collect(),
            index: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

/// Spawn a minimal HTTP/1.1 server answering `routes` (exact path match;
/// unmatched paths get 404 with an empty JSON object). Returns the base URL
/// "http://127.0.0.1:<port>". Each connection gets one response (reqwest
/// opens a fresh connection per request here).
pub async fn spawn_http(routes: Vec<HttpRoute>) -> String {
    spawn_http_recording(routes, None).await
}

/// [`spawn_http`] plus a shared sink receiving the raw request text of every
/// connection (for asserting on Authorization headers / bodies).
pub async fn spawn_http_recording(
    routes: Vec<HttpRoute>,
    requests: Option<Arc<Mutex<Vec<String>>>>,
) -> String {
    spawn_http_impl(routes, requests).await.0
}

/// Handle that stops a [`spawn_http`] server's accept loop, letting the
/// server task run to completion in-test (spawned-task end lines only
/// count when the task finishes).
pub struct HttpShutdown {
    notify: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl HttpShutdown {
    /// Stop accepting new connections and wait for the server task to
    /// finish. In-flight connections are unaffected.
    pub async fn stop(self) {
        self.notify.notify_one();
        self.task.await.expect("accept task completes");
    }
}

/// [`spawn_http`] plus an [`HttpShutdown`] handle for the accept loop.
pub async fn spawn_http_shutdownable(routes: Vec<HttpRoute>) -> (String, HttpShutdown) {
    spawn_http_impl(routes, None).await
}

async fn spawn_http_impl(
    routes: Vec<HttpRoute>,
    requests: Option<Arc<Mutex<Vec<String>>>>,
) -> (String, HttpShutdown) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let routes = Arc::new(routes);
    let notify = Arc::new(tokio::sync::Notify::new());
    let accept_notify = notify.clone();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = accept_notify.notified() => break,
                accepted = listener.accept() => {
                    // Invariant: accept on a live mock listener only fails
                    // on OS-level resource exhaustion.
                    let (mut socket, _) = accepted.expect("mock listener accept");
                    let routes = routes.clone();
                    let requests = requests.clone();
                    tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = Vec::new();
                // Read until end of headers (these requests have no body, or
                // a small JSON one we don't need to parse precisely).
                let mut chunk = [0u8; 4096];
                for _ in 0..16 {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                let request = String::from_utf8_lossy(&buf).into_owned();
                if let Some(sink) = &requests {
                    sink.lock().expect("requests").push(request.clone());
                }
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let route = routes.iter().find(|r| r.path == path);
                let response_data = match route {
                    Some(r) => {
                        let i = r
                            .index
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                            .min(r.responses.len() - 1);
                        let (status, content_type, extra_headers, body, delay) = {
                            let r = &r.responses[i];
                            (
                                r.status,
                                r.content_type.clone(),
                                r.extra_headers.clone(),
                                r.body.clone(),
                                r.delay,
                            )
                        };
                        (status, content_type, extra_headers, body, delay)
                    }
                    None => (
                        404,
                        "application/json".to_string(),
                        vec![],
                        b"{}".to_vec(),
                        Duration::ZERO,
                    ),
                };
                let (status, content_type, extra_headers, body, delay) = response_data;
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                let reason = match status {
                    200 => "OK",
                    201 => "Created",
                    400 => "Bad Request",
                    401 => "Unauthorized",
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    _ => "Status",
                };
                        let mut head = format!(
                            "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
                            body.len()
                        );
                        for (name, value) in &extra_headers {
                            head.push_str(&format!("{name}: {value}\r\n"));
                        }
                        head.push_str("\r\n");
                        let mut response = head.into_bytes();
                        response.extend_from_slice(&body);
                        let _ = socket.write_all(&response).await;
                        let _ = socket.shutdown().await;
                    });
                }
            }
        }
    });
    (
        format!("http://127.0.0.1:{}", addr.port()),
        HttpShutdown { notify, task },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `HttpShutdown::stop` ends the accept loop: the server task observes
    /// the notify, breaks, and runs to completion (its end lines count).
    #[tokio::test(flavor = "multi_thread")]
    async fn http_shutdown_stops_accept_loop() {
        let (base, shutdown) =
            spawn_http_shutdownable(vec![HttpRoute::json("/s", 200, "{}")]).await;
        let resp = reqwest::get(format!("{base}/s")).await.unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        shutdown.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_201_reason_and_aborted_connections() {
        // A 201 route (reason-phrase arm).
        let base = spawn_http(vec![HttpRoute::json("/made", 201, r#"{"ok":true}"#)]).await;
        let body = reqwest::get(format!("{base}/made")).await.unwrap();
        assert_eq!(body.status().as_u16(), 201);

        // A client that connects and closes without writing → Ok(0) read →
        // the handler breaks out cleanly; the mock stays responsive.
        let socket = tokio::net::TcpStream::connect(&base[7..]).await.unwrap();
        drop(socket);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let body = reqwest::get(format!("{base}/made")).await.unwrap();
        assert_eq!(body.status().as_u16(), 201);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_header_read_loop_exhaustion() {
        let base = spawn_http(vec![HttpRoute::json("/x", 200, "{}")]).await;
        // Trickling many tiny writes without the header terminator exhausts
        // the 16-iteration read loop.
        let mut socket = tokio::net::TcpStream::connect(&base[7..]).await.unwrap();
        use tokio::io::AsyncWriteExt;
        for b in std::iter::repeat_n(b'x', 20) {
            // The server may answer/close mid-write → broken pipe is fine.
            let _ = socket.write_all(&[b]).await;
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        drop(socket);
    }
}
