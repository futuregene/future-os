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
pub async fn spawn_mock(agent: MockAgent) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);
    tokio::spawn(
        Server::builder()
            .add_service(FutureAgentServer::new(agent))
            .serve(addr),
    );
    // Give the listener a moment to come up before clients dial.
    tokio::time::sleep(Duration::from_millis(50)).await;
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

/// One canned HTTP route: exact path → (status, body).
pub struct HttpRoute {
    pub path: String,
    pub status: u16,
    pub body: String,
}

impl HttpRoute {
    pub fn json(path: &str, status: u16, body: &str) -> Self {
        HttpRoute {
            path: path.to_string(),
            status,
            body: body.to_string(),
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let routes = Arc::new(routes);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
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
                let path = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_string();
                let route = routes.iter().find(|r| r.path == path);
                let (status, body) = match route {
                    Some(r) => (r.status, r.body.clone()),
                    None => (404, "{}".to_string()),
                };
                let reason = match status {
                    200 => "OK",
                    201 => "Created",
                    400 => "Bad Request",
                    401 => "Unauthorized",
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    _ => "Status",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://127.0.0.1:{}", addr.port())
}
