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
    /// Sleep this long before answering the command (race-window tests).
    pub command_delay: HashMap<String, Duration>,
    /// Canned events for stream_events.
    pub events: Vec<StreamEvent>,
    /// stream_events RPC fails immediately with a tonic Status.
    pub stream_status_error: bool,
    /// After N scripted events the stream yields one tonic error.
    pub stream_mid_error_after: Option<usize>,
    /// stream_events yields nothing, ever (hang).
    pub stream_hang: bool,
    /// Delay before each streamed event (paces the run loop past its flush
    /// throttle deterministically).
    pub stream_event_delay: Option<Duration>,
    /// Every command received, in arrival order.
    pub recorded: Vec<RpcCommand>,
    /// session_id → active run_id (set by `prompt`).
    pub active_runs: HashMap<String, String>,
    /// Override for the imageSupport field in the default get_state payload.
    pub image_support: Option<bool>,
    /// Sessions created via new_session.
    pub sessions_created: u64,
    /// Prompts received.
    pub prompts: u64,
}

pub type SharedState = Arc<Mutex<MockState>>;

pub struct MockAgent {
    state: SharedState,
}

/// Poison-tolerant lock: a panicking holder must not wedge the mocks.
/// Single shared closure so one poison test covers the recovery arm.
fn lock_unpoisoned<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn lock(state: &SharedState) -> MutexGuard<'_, MockState> {
    lock_unpoisoned(state)
}

fn response(
    cmd: &RpcCommand,
    success: bool,
    data: String,
    error: String,
) -> tonic::Response<RpcResponse> {
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
        let delay = {
            let st = lock(&self.state);
            st.command_delay.get(&cmd.r#type).copied()
        };
        if let Some(d) = delay {
            tokio::time::sleep(d).await;
        }
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
                st.active_runs
                    .insert(cmd.session_id.clone(), run_id.clone());
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
                let image_support = st.image_support.unwrap_or(true);
                format!(
                    "{{\"sessionId\":\"{}\",\"model\":\"future/k3\",\"thinkingLevel\":\"high\",\
                     \"permissionLevel\":\"all\",\"cwd\":\"/tmp\",\"imageSupport\":{},\
                     \"contextTokens\":100,\"contextWindow\":1000,\"tokensIn\":10,\
                     \"tokensOut\":20,\"queryCount\":3,\"totalCost\":0.01,\
                     \"autoCompactionEnabled\":true,\"isStreaming\":false{}}}",
                    cmd.session_id, image_support, active
                )
            }
            "list_models" => {
                "{\"models\":[{\"id\":\"future/k3\",\"provider\":\"future\",\"label\":\"K3\",\
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
            return Ok(tonic::Response::new(Box::pin(
                futures_util::stream::pending(),
            )));
        }
        let mut items: Vec<Result<StreamEvent, tonic::Status>> =
            st.events.iter().cloned().map(Ok).collect();
        if let Some(n) = st.stream_mid_error_after {
            if items.len() >= n {
                items.insert(n, Err(tonic::Status::data_loss("mock mid-stream failure")));
            }
        }
        if let Some(d) = st.stream_event_delay {
            use futures_util::StreamExt as _;
            let stream = futures_util::stream::iter(items).then(move |item| async move {
                tokio::time::sleep(d).await;
                item
            });
            return Ok(tonic::Response::new(Box::pin(stream)));
        }
        Ok(tonic::Response::new(Box::pin(futures_util::stream::iter(
            items,
        ))))
    }
}

/// Serve the mock on an ephemeral port. Returns ("127.0.0.1:<port>", state).
pub async fn spawn_mock_grpc(state: MockState) -> (String, SharedState) {
    let (local, shared, shutdown, _handle) = spawn_mock_grpc_inner(state).await;
    // Dropping the sender would resolve the shutdown future and stop the
    // server; forget it instead so the mock serves until runtime teardown.
    std::mem::forget(shutdown);
    (local, shared)
}

/// Inner spawn wiring a graceful-shutdown trigger: firing the returned
/// sender stops the server so the task completes (used by the self-test to
/// cover the server-exit path; production callers drop it).
async fn spawn_mock_grpc_inner(
    state: MockState,
) -> (
    String,
    SharedState,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("ephemeral bind");
    let local = listener.local_addr().expect("local addr").to_string();
    let shared = Arc::new(Mutex::new(state));
    let svc = MockAgent {
        state: shared.clone(),
    };
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        let _ = tonic::transport::Server::builder()
            .add_service(FutureAgentServer::new(svc))
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async move {
                    let _ = shutdown_rx.await;
                },
            )
            .await;
    });
    (local, shared, shutdown_tx, handle)
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

/// Mark a command type as failing from now on (mid-scenario failure).
pub fn fail_command(state: &SharedState, cmd: &str) {
    lock(state).fail_commands.insert(cmd.to_string());
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
    let ((base, recorded), _handle) = spawn_http_inner(routes, None).await;
    (base, recorded)
}

/// Inner spawn with an optional connection bound: after accepting
/// `max_connections` the accept loop exits so the task completes (used by
/// the self-test to cover the server-exit path; `None` serves forever).
async fn spawn_http_inner(
    routes: Vec<HttpRoute>,
    max_connections: Option<usize>,
) -> ((String, RecordedRequests), tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let routes = Arc::new(routes);
    let recorded: RecordedRequests = Arc::new(Mutex::new(Vec::new()));
    let recorded_task = recorded.clone();
    let handle = tokio::spawn(async move {
        let mut incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
        let mut accepted = 0usize;
        // while-let: the Err/None exit edge lives on this (covered) line, so
        // no unreachable break line is left behind (rustfmt explodes
        // single-line let-else/match forms).
        while let Some(Ok(mut socket)) = futures_util::StreamExt::next(&mut incoming).await {
            accepted += 1;
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
                lock_unpoisoned(&recorded).push(RecordedRequest {
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
                        (
                            resp.status,
                            resp.content_type.clone(),
                            resp.body.clone(),
                            resp.delay,
                        )
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
            if max_connections.is_some_and(|max| accepted >= max) {
                break;
            }
        }
    });
    (
        (format!("http://127.0.0.1:{}", addr.port()), recorded),
        handle,
    )
}

/// Requests whose path portion equals `path`.
pub fn requests_to(recorded: &RecordedRequests, path: &str) -> Vec<RecordedRequest> {
    lock_unpoisoned(recorded)
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
    /// Abortive close (SO_LINGER 0 → RST on unix; plain drop elsewhere) so
    /// the client's next send fails instead of its read.
    ResetTcp,
}

pub type WsReceived = Arc<Mutex<Vec<tokio_tungstenite::tungstenite::Message>>>;

/// Spawn a WS server. Every accepted connection executes a clone of `script`
/// while a background task records all incoming messages. When the script is
/// exhausted the connection is dropped (clean TCP FIN, no WS close frame).
/// Returns the "ws://127.0.0.1:<port>" URL and the received-message log.
pub async fn spawn_ws(script: Vec<WsAction>) -> (String, WsReceived) {
    spawn_ws_per_connection(vec![script]).await
}

/// [`spawn_ws`] with a per-connection script list: connection N runs
/// `scripts[N]` (the last one repeats). Reconnect tests use this to close
/// the first connection and hold later ones open.
pub async fn spawn_ws_per_connection(scripts: Vec<Vec<WsAction>>) -> (String, WsReceived) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMsg;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let received: WsReceived = Arc::new(Mutex::new(Vec::new()));
    let received_task = received.clone();
    let handle = tokio::runtime::Handle::current();
    let conn_counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    std::thread::spawn(move || {
        while let Ok((socket, _)) = listener.accept() {
            let idx = conn_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let script = scripts[idx.min(scripts.len() - 1)].clone();
            let received = received_task.clone();
            // try_clone only fails on OS-level fd errors — no test seam, and
            // silently dropping the connection would hang the test anyway.
            let raw = socket.try_clone().expect("try_clone after accept");
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
                            Ok(m) => lock_unpoisoned(&recv_received).push(m),
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
                        WsAction::ResetTcp => {
                            #[cfg(unix)]
                            {
                                use std::os::unix::io::AsRawFd;
                                let linger = libc::linger {
                                    l_onoff: 1,
                                    l_linger: 0,
                                };
                                unsafe {
                                    libc::setsockopt(
                                        raw.as_raw_fd(),
                                        libc::SOL_SOCKET,
                                        libc::SO_LINGER,
                                        &linger as *const libc::linger as *const _,
                                        std::mem::size_of::<libc::linger>() as _,
                                    );
                                }
                            }
                            break;
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
    lock_unpoisoned(&HOME_LOCK)
}

static HOME_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Fresh per-test directory under target/test-data (no env mutation —
/// safe to use from parallel async tests).
pub fn temp_dir(label: &str) -> std::path::PathBuf {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-data");
    let n = HOME_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = base.join(format!("{}-{}-{}", label, std::process::id(), n));
    std::fs::create_dir_all(&path).expect("create temp dir");
    path
}

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

#[cfg(test)]
mod tests {
    // MockState scaffolding mutates Default::default() instances per-test by
    // design; field-reassign is the readable form for a 15-field mock.
    #![allow(clippy::field_reassign_with_default)]
    //! Self-tests for the mock infrastructure itself: corner arms of the
    //! HTTP/WS servers and helpers that no product test exercises.

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn http_status_reasons_and_sequences() {
        ensure_crypto_provider();
        let routes = vec![
            HttpRoute::json("/made", 201, r#"{"ok":true}"#),
            HttpRoute::json("/bad", 502, "{}"),
            HttpRoute::json("/weird", 499, "{}"), // unmapped reason arm
            HttpRoute::sequence("/seq", vec![(200, r#"{"n":1}"#), (200, r#"{"n":2}"#)]),
            HttpRoute::binary("/bin", 200, b"\x00\x01".to_vec()),
            HttpRoute::slow("/slow", Duration::from_millis(50)),
        ];
        let (base, recorded) = spawn_http(routes).await;
        let client = reqwest::Client::new();
        assert_eq!(
            client
                .get(format!("{base}/made"))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            201
        );
        assert_eq!(
            client
                .get(format!("{base}/bad"))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            502
        );
        assert_eq!(
            client
                .get(format!("{base}/weird"))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            499
        );
        // Sequence advances, then repeats the last response.
        let b1 = client
            .get(format!("{base}/seq"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let b2 = client
            .get(format!("{base}/seq"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let b3 = client
            .get(format!("{base}/seq"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(b1.contains("\"n\":1") && b2.contains("\"n\":2") && b3.contains("\"n\":2"));
        // Binary body.
        let b = client
            .get(format!("{base}/bin"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(&b[..], b"\x00\x01");
        // Slow route responds after its delay.
        assert_eq!(
            client
                .get(format!("{base}/slow"))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            200
        );
        // Unmatched path → 404 {}.
        assert_eq!(
            client
                .get(format!("{base}/nope"))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            404
        );
        // Query strings don't affect routing; the request log keeps them.
        client.get(format!("{base}/made?x=1")).send().await.unwrap();
        let made = requests_to(&recorded, "/made");
        assert!(made.iter().any(|r| r.target.contains("?x=1")));
        assert_eq!(requests_to(&recorded, "/absent").len(), 0);
        // RecordedRequest helpers.
        let r = &made[0];
        assert_eq!(r.method, "GET");
        assert!(r.header("content-type").is_none() || r.header("host").is_some());
        let _ = r.body_string();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_aborted_and_oversized_header_connections() {
        let routes = vec![HttpRoute::json("/x", 200, "{}")];
        let (base, _) = spawn_http(routes).await;
        // Connect then close without writing → Ok(0) read → handler returns.
        let socket = tokio::net::TcpStream::connect(&base[7..]).await.unwrap();
        drop(socket);
        // Headers never terminated → read loop caps out and returns.
        let mut socket = tokio::net::TcpStream::connect(&base[7..]).await.unwrap();
        use tokio::io::AsyncWriteExt;
        socket
            .write_all(b"GET /x HTTP/1.1\r\nX-Long: ")
            .await
            .unwrap();
        let big = vec![b'a'; 300 * 1024];
        socket.write_all(&big).await.unwrap();
        drop(socket);
        // Client closes mid-body (Content-Length larger than what arrives)
        // → the drain loop's break arm.
        let mut socket = tokio::net::TcpStream::connect(&base[7..]).await.unwrap();
        socket
            .write_all(b"POST /x HTTP/1.1\r\nHost: x\r\nContent-Length: 10000\r\n\r\nshort")
            .await
            .unwrap();
        drop(socket);
        // 400/401 reason strings.
        let routes = vec![
            HttpRoute::json("/r400", 400, "{}"),
            HttpRoute::json("/r401", 401, "{}"),
        ];
        let (base2, _) = spawn_http(routes).await;
        assert_eq!(
            reqwest::get(format!("{base2}/r400"))
                .await
                .unwrap()
                .status()
                .as_u16(),
            400
        );
        assert_eq!(
            reqwest::get(format!("{base2}/r401"))
                .await
                .unwrap()
                .status()
                .as_u16(),
            401
        );
        // Server survives all of it.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            reqwest::get(format!("{base}/x"))
                .await
                .unwrap()
                .status()
                .as_u16(),
            200
        );
    }

    #[test]
    fn lock_unpoisoned_recovers_from_poisoned_mutex() {
        let m = Mutex::new(7usize);
        // Poison it: a thread panics while holding the guard.
        let m2 = &m;
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = m2.lock().unwrap();
            panic!("intentional poison");
        }));
        assert!(m.is_poisoned());
        // The recovery arm (`e.into_inner()`) still yields the value.
        assert_eq!(*lock_unpoisoned(&m), 7);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mock_grpc_shutdown_completes_server_task() {
        let (addr, _shared, shutdown, handle) = spawn_mock_grpc_inner(MockState::default()).await;
        assert!(addr.starts_with("127.0.0.1:"));
        shutdown.send(()).expect("shutdown send");
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("server stops promptly on shutdown")
            .expect("server task did not panic");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_server_bounded_accepts_then_exits() {
        let ((base, recorded), handle) = spawn_http_inner(
            vec![HttpRoute::json("/ping", 200, r#"{"ok":true}"#)],
            Some(1),
        )
        .await;
        let status = reqwest::get(format!("{base}/ping")).await.unwrap().status();
        assert_eq!(status.as_u16(), 200);
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("bounded server exits after one connection")
            .expect("server task did not panic");
        assert_eq!(requests_to(&recorded, "/ping").len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mock_grpc_remaining_arms() {
        // fail_silent, fail_times exhaustion, sequences, stream hang, ev().
        let mut state = MockState::default();
        state.fail_silent.insert("abort".into());
        state.fail_times.insert("compact".into(), 1);
        state.sequences.insert(
            "set_cwd".into(),
            vec![r#"{"n":1}"#.into(), r#"{"n":2}"#.into()],
        );
        let (addr, shared) = spawn_mock_grpc(state).await;
        let mut client = crate::grpc_client::AgentClient::connect(&addr)
            .await
            .unwrap();
        // fail_silent → "unknown error".
        let err = client.abort("s").await.err().unwrap();
        assert!(err.to_string().contains("unknown error"));
        // fail_times: first fails at transport, second succeeds.
        assert!(client.compact("s").await.is_err());
        assert!(client.compact("s").await.is_ok());
        // sequences advance then stick.
        client.set_cwd("s", "/a").await.unwrap();
        client.set_cwd("s", "/b").await.unwrap();
        client.set_cwd("s", "/c").await.unwrap();
        assert_eq!(recorded_of(&shared, "set_cwd").len(), 3);
        // fail_command mid-scenario.
        fail_command(&shared, "switch_session");
        assert!(client.switch_session("s").await.is_err());
        // ev() builder.
        let e = ev("r", 3, "text_chunk", "{}");
        assert_eq!(e.run_id, "r");
        assert_eq!(e.idx, 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mock_grpc_hanging_stream() {
        let mut state = MockState::default();
        state.stream_hang = true;
        let (addr, _) = spawn_mock_grpc(state).await;
        let mut client = crate::grpc_client::AgentClient::connect(&addr)
            .await
            .unwrap();
        let mut stream = client.stream_run_events("s", "r").await.unwrap();
        // Never yields → times out, not hangs forever.
        let result = tokio::time::timeout(Duration::from_millis(200), stream.message()).await;
        assert!(result.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ws_server_send_after_close_breaks() {
        // Per-connection scripts: each client vanishes right after the
        // handshake; the delay lets the close propagate so the next scripted
        // send fails and hits its break arm (Text/Binary/Ping/raw variants).
        let d = Duration::from_millis(120);
        let (url, _) = spawn_ws_per_connection(vec![
            vec![
                WsAction::SendText("a".into()),
                WsAction::Delay(d),
                WsAction::SendText("b".into()),
            ],
            vec![
                WsAction::SendBinary(vec![1]),
                WsAction::Delay(d),
                WsAction::SendBinary(vec![2]),
            ],
            vec![
                WsAction::SendPing(vec![1]),
                WsAction::Delay(d),
                WsAction::SendPing(vec![2]),
            ],
            vec![
                WsAction::SendRawBytes(vec![0x88, 0x00]),
                WsAction::Delay(d),
                WsAction::SendRawBytes(vec![0x88, 0x00]),
                WsAction::Delay(d),
                WsAction::SendRawBytes(vec![0x88, 0x00]),
            ],
            vec![WsAction::SendText("tail".into())],
        ])
        .await;
        for _ in 0..4 {
            let (stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            drop(stream); // vanish → the delayed second send breaks
        }
        // Raw TCP connect + immediate drop → the WS handshake fails
        // (Err → return arm; its script slot is never executed).
        let socket = tokio::net::TcpStream::connect(&url[5..]).await.unwrap();
        drop(socket);
        tokio::time::sleep(Duration::from_millis(700)).await;
        // A later connection is still served (last script repeats).
        let (mut stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        use futures_util::StreamExt as _;
        let msg = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(
            msg,
            tokio_tungstenite::tungstenite::Message::Text(_)
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wait_until_times_out() {
        assert!(!wait_until(|| false, Duration::from_millis(60)).await);
        assert!(wait_until(|| true, Duration::from_millis(60)).await);
    }

    #[test]
    fn temp_dir_and_home_helpers() {
        let a = temp_dir("selftest");
        assert!(a.exists());
        let b = temp_dir("selftest");
        assert_ne!(a, b);
        let _guard = home_lock();
        let home = IsolatedHome::new("selftest");
        assert!(home.path.exists());
        assert_eq!(
            std::env::var("HOME").unwrap(),
            home.path.to_string_lossy().to_string()
        );
        drop(home);
        // HOME restored.
        assert_ne!(std::env::var("HOME").unwrap(), "");
    }

    #[test]
    fn isolated_home_without_prior_home_removes_it() {
        let _guard = home_lock();
        let original = std::env::var("HOME").expect("HOME set by test harness");
        std::env::remove_var("HOME");
        let home = IsolatedHome::new("selftest-nohome");
        assert_eq!(
            std::env::var("HOME").unwrap(),
            home.path.to_string_lossy().to_string()
        );
        drop(home);
        // No original → HOME is removed (not restored).
        assert!(std::env::var("HOME").is_err());
        std::env::set_var("HOME", original);
    }
}
