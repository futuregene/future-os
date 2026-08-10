//! Test-only support infrastructure for the channels crate.
//!
//! - [`MockAgent`] — a tonic `FutureAgent` gRPC server with per-command-type
//!   canned responses, failure injection, stateful prompt→run tracking, and a
//!   scripted event stream.
//! - [`spawn_http`] — a minimal HTTP/1.1 responder routing by request path
//!   (query string ignored), draining the request body before responding so
//!   multipart uploads work.
//! - [`spawn_ws`] — a WebSocket server executing a scripted action list per
//!   connection, recording every incoming message.
//! - [`IsolatedHome`] — HOME redirect anchored under target/test-homes so
//!   config/session-store tests never touch the real `~/.future`.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use future_rpc::proto::future_agent_server::{FutureAgent, FutureAgentServer};
use future_rpc::proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};

// ─── Mock FutureAgent gRPC ──────────────────────────────────────────────────

#[derive(Default)]
pub struct MockState {
    /// Command type → `data` JSON returned with success=true.
    pub responses: HashMap<String, String>,
    /// Command type → successive `data` payloads (last one repeats).
    pub sequences: HashMap<String, Vec<String>>,
    /// Types answered success=false with error "mock failure: <type>".
    pub fail_commands: HashSet<String>,
    /// Types answered success=false with an EMPTY error string.
    pub fail_silent: HashSet<String>,
    /// Types that fail the unary call with a tonic transport Status.
    pub status_error: HashSet<String>,
    /// Fail the next N calls of this type with a tonic Status, then succeed
    /// (transient-error retry paths).
    pub fail_times: HashMap<String, u64>,
    /// Canned events for stream_events.
    pub events: Vec<StreamEvent>,
    /// stream_events RPC fails immediately with a tonic Status.
    pub stream_status_error: bool,
    /// After N scripted events the stream yields one tonic error.
    pub stream_mid_error_after: Option<usize>,
    /// stream_events yields nothing, ever (hang).
    pub stream_hang: bool,
    /// Every command received, in arrival order.
    pub recorded: Vec<RpcCommand>,
    /// session_id → active run_id (set by `prompt`).
    pub active_runs: HashMap<String, String>,
    /// Sessions created via new_session.
    pub sessions_created: u64,
    /// Prompts received.
    pub prompts: u64,
}

pub type SharedState = Arc<Mutex<MockState>>;

pub struct MockAgent {
    state: SharedState,
}

fn lock(state: &SharedState) -> MutexGuard<'_, MockState> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

fn response(cmd: &RpcCommand, success: bool, data: String, error: String) -> tonic::Response<RpcResponse> {
    tonic::Response::new(RpcResponse {
        id: cmd.id.clone(),
        r#type: "response".to_string(),
        command: cmd.r#type.clone(),
        success,
        data,
        error,
        error_code: String::new(),
        error_data: String::new(),
        payload: None,
    })
}

#[tonic::async_trait]
impl FutureAgent for MockAgent {
    async fn execute_command(
        &self,
        request: tonic::Request<RpcCommand>,
    ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
        let cmd = request.into_inner();
        let mut st = lock(&self.state);
        st.recorded.push(cmd.clone());
        if st.status_error.contains(&cmd.r#type) {
            return Err(tonic::Status::unavailable("mock transport failure"));
        }
        if let Some(n) = st.fail_times.get_mut(&cmd.r#type) {
            if *n > 0 {
                *n -= 1;
                return Err(tonic::Status::unavailable("mock transient failure"));
            }
        }
        if st.fail_commands.contains(&cmd.r#type) {
            return Ok(response(
                &cmd,
                false,
                String::new(),
                format!("mock failure: {}", cmd.r#type),
            ));
        }
        if st.fail_silent.contains(&cmd.r#type) {
            return Ok(response(&cmd, false, String::new(), String::new()));
        }
        if let Some(seq) = st.sequences.get_mut(&cmd.r#type) {
            let data = if seq.len() > 1 {
                seq.remove(0)
            } else {
                seq[0].clone()
            };
            return Ok(response(&cmd, true, data, String::new()));
        }
        if let Some(data) = st.responses.get(&cmd.r#type) {
            return Ok(response(&cmd, true, data.clone(), String::new()));
        }
        let data = match cmd.r#type.as_str() {
            "new_session" => {
                st.sessions_created += 1;
                format!("{{\"sessionId\":\"mock-session-{}\"}}", st.sessions_created)
            }
            "prompt" => {
                st.prompts += 1;
                let run_id = format!("mock-run-{}", st.prompts);
                st.active_runs.insert(cmd.session_id.clone(), run_id.clone());
                format!("{{\"run_id\":\"{}\"}}", run_id)
            }
            "get_state" => {
                let active = st
                    .active_runs
                    .get(&cmd.session_id)
                    .map(|r| {
                        format!(
                            ",\"activeRun\":{{\"runId\":\"{}\",\"state\":\"running\"}}",
                            r
                        )
                    })
                    .unwrap_or_default();
                format!(
                    "{{\"sessionId\":\"{}\",\"model\":\"future/k3\",\"thinkingLevel\":\"high\",\
                     \"permissionLevel\":\"all\",\"cwd\":\"/tmp\",\"imageSupport\":true,\
                     \"contextTokens\":100,\"contextWindow\":1000,\"tokensIn\":10,\
                     \"tokensOut\":20,\"queryCount\":3,\"totalCost\":0.01,\
                     \"autoCompactionEnabled\":true,\"isStreaming\":false{}}}",
                    cmd.session_id, active
                )
            }
            "list_models" => {
                "{\"models\":[{\"id\":\"k3\",\"provider\":\"future\",\"label\":\"K3\",\
                 \"supportsImages\":true,\"contextWindow\":256000}]}"
                    .to_string()
            }
            _ => String::new(),
        };
        Ok(response(&cmd, true, data, String::new()))
    }

    type StreamEventsStream = std::pin::Pin<
        Box<dyn futures_util::Stream<Item = Result<StreamEvent, tonic::Status>> + Send>,
    >;

    async fn stream_events(
        &self,
        _request: tonic::Request<StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        let st = lock(&self.state);
        if st.stream_status_error {
            return Err(tonic::Status::internal("mock stream attach failure"));
        }
        if st.stream_hang {
            return Ok(tonic::Response::new(Box::pin(futures_util::stream::pending())));
        }
        let mut items: Vec<Result<StreamEvent, tonic::Status>> =
            st.events.iter().cloned().map(Ok).collect();
        if let Some(n) = st.stream_mid_error_after {
            if items.len() >= n {
                items.insert(n, Err(tonic::Status::data_loss("mock mid-stream failure")));
            }
        }
        Ok(tonic::Response::new(Box::pin(futures_util::stream::iter(
            items,
        ))))
    }
}

/// Serve the mock on an ephemeral port. Returns ("127.0.0.1:<port>", state).
pub async fn spawn_mock_grpc(state: MockState) -> (String, SharedState) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("ephemeral bind");
    let local = listener.local_addr().expect("local addr").to_string();
    let shared = Arc::new(Mutex::new(state));
    let svc = MockAgent {
        state: shared.clone(),
    };
    tokio::spawn(async move {
        let _ = tonic::transport::Server::builder()
            .add_service(FutureAgentServer::new(svc))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await;
    });
    (local, shared)
}

/// Build a StreamEvent with a type and JSON data payload.
pub fn ev(run_id: &str, idx: i64, kind: &str, data: &str) -> StreamEvent {
    StreamEvent {
        r#type: kind.to_string(),
        data: data.to_string(),
        run_id: run_id.to_string(),
        idx,
        ..Default::default()
    }
}

/// Recorded commands of one type, in order.
pub fn recorded_of(state: &SharedState, r#type: &str) -> Vec<RpcCommand> {
    lock(state)
        .recorded
        .iter()
        .filter(|c| c.r#type == r#type)
        .cloned()
        .collect()
}

// ─── Mock HTTP/1.1 server ───────────────────────────────────────────────────

pub struct HttpResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub delay: Duration,
}

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
                body: body.as_bytes().to_vec(),
                delay: Duration::ZERO,
            },
        )
    }

    pub fn binary(path: &str, status: u16, body: Vec<u8>) -> Self {
        HttpRoute::single(
            path,
            HttpResponse {
                status,
                content_type: "application/octet-stream".to_string(),
                body,
                delay: Duration::ZERO,
            },
        )
    }

    /// Stateful route: successive calls return successive responses; the last
    /// one repeats once exhausted.
    pub fn sequence(path: &str, responses: Vec<(u16, &str)>) -> Self {
        HttpRoute {
            path: path.to_string(),
            responses: responses
                .into_iter()
                .map(|(s, b)| HttpResponse {
                    status: s,
                    content_type: "application/json".to_string(),
                    body: b.as_bytes().to_vec(),
                    delay: Duration::ZERO,
                })
                .collect(),
            index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Hang for `delay` before answering (client-timeout tests).
    pub fn slow(path: &str, delay: Duration) -> Self {
        Self::slow_json(path, "{}", delay)
    }

    /// Hang for `delay`, then answer with this JSON body.
    pub fn slow_json(path: &str, body: &str, delay: Duration) -> Self {
        let mut response = HttpRoute::json(path, 200, body).responses.remove(0);
        response.delay = delay;
        HttpRoute::single(path, response)
    }
}

#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RecordedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn body_string(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub type RecordedRequests = Arc<Mutex<Vec<RecordedRequest>>>;

/// Spawn a minimal HTTP/1.1 server answering `routes` (exact match on the
/// path portion of the request target — query strings are ignored; unmatched
/// paths get 404 with an empty JSON object). Returns the base URL
/// "http://127.0.0.1:<port>" plus the shared request log.
pub async fn spawn_http(routes: Vec<HttpRoute>) -> (String, RecordedRequests) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let routes = Arc::new(routes);
    let recorded: RecordedRequests = Arc::new(Mutex::new(Vec::new()));
    let recorded_task = recorded.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let routes = routes.clone();
            let recorded = recorded_task.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                // Read until end of headers.
                let header_end = loop {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                break pos + 4;
                            }
                            if buf.len() > 256 * 1024 {
                                return;
                            }
                        }
                    }
                };
                let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
                let mut lines = head.lines();
                let request_line = lines.next().unwrap_or("");
                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let target = parts.next().unwrap_or("/").to_string();
                let mut headers = Vec::new();
                let mut content_length = 0usize;
                for line in lines {
                    if let Some((name, value)) = line.split_once(':') {
                        let name = name.trim().to_string();
                        let value = value.trim().to_string();
                        if name.eq_ignore_ascii_case("content-length") {
                            content_length = value.parse().unwrap_or(0);
                        }
                        headers.push((name, value));
                    }
                }
                // Drain the body so reqwest never hits a broken pipe mid-send
                // (multipart uploads in particular).
                let mut body = buf[header_end..].to_vec();
                while body.len() < content_length {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => body.extend_from_slice(&chunk[..n]),
                    }
                }
                recorded
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(RecordedRequest {
                        method,
                        target: target.clone(),
                        headers,
                        body,
                    });
                let path = target.split('?').next().unwrap_or("/");
                let route = routes.iter().find(|r| r.path == path);
                let (status, content_type, body, delay) = match route {
                    Some(r) => {
                        let i = r
                            .index
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                            .min(r.responses.len() - 1);
                        let resp = &r.responses[i];
                        (resp.status, resp.content_type.clone(), resp.body.clone(), resp.delay)
                    }
                    None => (
                        404,
                        "application/json".to_string(),
                        b"{}".to_vec(),
                        Duration::ZERO,
                    ),
                };
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
                    502 => "Bad Gateway",
                    _ => "Status",
                };
                let head = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let mut response = head.into_bytes();
                response.extend_from_slice(&body);
                let _ = socket.write_all(&response).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (format!("http://127.0.0.1:{}", addr.port()), recorded)
}

/// Requests whose path portion equals `path`.
pub fn requests_to(recorded: &RecordedRequests, path: &str) -> Vec<RecordedRequest> {
    recorded
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|r| r.target.split('?').next().unwrap_or("/") == path)
        .cloned()
        .collect()
}

// ─── Mock WebSocket server ──────────────────────────────────────────────────

/// One scripted server action, executed in order after the WS handshake.
#[derive(Clone)]
pub enum WsAction {
    SendText(String),
    SendBinary(Vec<u8>),
    SendPing(Vec<u8>),
    SendClose,
    Delay(Duration),
    /// Write raw bytes onto the socket underneath the WS framing — used to
    /// inject protocol-level garbage the client's tungstenite rejects.
    SendRawBytes(Vec<u8>),
}

pub type WsReceived = Arc<Mutex<Vec<tokio_tungstenite::tungstenite::Message>>>;

/// Spawn a WS server. Every accepted connection executes a clone of `script`
/// while a background task records all incoming messages. When the script is
/// exhausted the connection is dropped (clean TCP FIN, no WS close frame).
/// Returns the "ws://127.0.0.1:<port>" URL and the received-message log.
pub async fn spawn_ws(script: Vec<WsAction>) -> (String, WsReceived) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMsg;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let received: WsReceived = Arc::new(Mutex::new(Vec::new()));
    let received_task = received.clone();
    let handle = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        while let Ok((socket, _)) = listener.accept() {
            let script = script.clone();
            let received = received_task.clone();
            let raw = match socket.try_clone() {
                Ok(s) => s,
                Err(_) => continue,
            };
            socket.set_nonblocking(true).expect("nonblocking");
            handle.spawn(async move {
                let socket = tokio::net::TcpStream::from_std(socket).expect("from_std");
                let stream = match tokio_tungstenite::accept_async(socket).await {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let (mut sink, mut stream) = stream.split();
                let recv_received = received.clone();
                let reader = tokio::spawn(async move {
                    while let Some(msg) = stream.next().await {
                        match msg {
                            Ok(m) => recv_received
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .push(m),
                            Err(_) => break,
                        }
                    }
                });
                for action in script {
                    match action {
                        WsAction::SendText(t) => {
                            if sink.send(WsMsg::Text(t)).await.is_err() {
                                break;
                            }
                        }
                        WsAction::SendBinary(b) => {
                            if sink.send(WsMsg::Binary(b)).await.is_err() {
                                break;
                            }
                        }
                        WsAction::SendPing(p) => {
                            if sink.send(WsMsg::Ping(p)).await.is_err() {
                                break;
                            }
                        }
                        WsAction::SendClose => {
                            let _ = sink.send(WsMsg::Close(None)).await;
                            break;
                        }
                        WsAction::Delay(d) => tokio::time::sleep(d).await,
                        WsAction::SendRawBytes(bytes) => {
                            use std::io::Write;
                            let mut raw = &raw;
                            if raw.write_all(&bytes).is_err() {
                                break;
                            }
                        }
                    }
                }
                // Script finished: tear the connection down so the client
                // sees EOF (both halves must drop for the socket to close).
                reader.abort();
                drop(sink);
                let _ = reader.await;
            });
        }
    });
    (format!("ws://127.0.0.1:{}", addr.port()), received)
}

// ─── HOME isolation ─────────────────────────────────────────────────────────

static HOME_LOCK: Mutex<()> = Mutex::new(());

/// Poison-tolerant guard serializing HOME-mutating tests.
pub fn home_lock() -> MutexGuard<'static, ()> {
    HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

static HOME_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Redirects $HOME to a fresh directory under target/test-homes. Restores the
/// original value on drop. Hold the [`home_lock`] guard for the whole test.
pub struct IsolatedHome {
    pub path: std::path::PathBuf,
    original: Option<String>,
}

impl IsolatedHome {
    pub fn new(label: &str) -> Self {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target")
            .join("test-homes");
        let n = HOME_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = base.join(format!("{}-{}-{}", label, std::process::id(), n));
        std::fs::create_dir_all(&path).expect("create isolated home");
        let original = std::env::var("HOME").ok();
        std::env::set_var("HOME", &path);
        Self { path, original }
    }
}

impl Drop for IsolatedHome {
    fn drop(&mut self) {
        match &self.original {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}

// ─── Async condition pump ───────────────────────────────────────────────────

/// Install the process-level rustls crypto provider (production does this in
/// `run()`); idempotent — a repeated install returns Err we ignore.
pub fn ensure_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

/// Poll `cond` every 20ms until it holds or `timeout` elapses. Returns the
/// final condition value.
pub async fn wait_until(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    cond()
}
