//! Shared in-process test doubles for the remote bridge and provider tests:
//!
//! - [`ensure_mock_agent`]: a scripted gRPC `FutureAgent` server (tonic),
//!   started once per test process on an ephemeral port. `FUTURE_AGENT_GRPC_ADDR`
//!   is pointed at it before the first `connect_agent` — the agent channel is a
//!   process-global `OnceCell`, so every test that may reach the agent must call
//!   this first.
//! - [`FakeNats`]: a minimal core-NATS line-protocol server good enough for
//!   `async_nats` connect / (queue_)subscribe / publish / request-reply.
//! - [`MockPlatform`]: a scripted HTTP/1.1 server for the pairing control plane
//!   (`/client/v1/remote/...`).
//!
//! Scripting discipline (the mock agent is process-global): per-session
//! behavior is keyed by the UNIQUE session ids each test generates, so parallel
//! tests never steal each other's scripts. Session-less commands (`set_auth`,
//! `upsert_provider`, ...) are only scripted by tests that hold the
//! `TEST_HOME_LOCK` (via `HomeGuard`), which serializes them.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

pub(crate) use crate::auth_store::test_support::HomeGuard;

/// Unique-per-process id fragment so parallel tests never share session ids,
/// pair ids, or transfer subjects.
pub(crate) fn unique(tag: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    format!("{tag}-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// Initialize the GUI store (SQLite schema + dirs) under the current
/// (HomeGuard-redirected) HOME.
pub(crate) fn init_store() {
    crate::store::initialize_app_store().expect("store init under test HOME");
}

/// Sign the test HOME in to FutureGene against `platform_url` (a MockPlatform),
/// writing the `future` auth entry the platform-URL resolver reads.
pub(crate) fn sign_in(platform_url: &str) {
    crate::auth_store::set_future_login("test-key", &format!("{platform_url}/api"))
        .expect("write test auth.json");
}

/// A syntactically valid JWT whose payload carries `exp` (unix seconds).
pub(crate) fn jwt(expires_at: i64) -> String {
    let payload = URL_SAFE_NO_PAD.encode(json!({ "exp": expires_at }).to_string());
    format!("hdr.{payload}.sig")
}

/// A v2 pairing code embedding `exp` (unix seconds), as the platform issues.
pub(crate) fn pairing_code(expires_at: i64) -> String {
    URL_SAFE_NO_PAD.encode(json!({ "exp": expires_at }).to_string())
}

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

// ── Mock gRPC FutureAgent ───────────────────────────────────────────────────

#[derive(Clone)]
struct ScriptedResponse {
    success: bool,
    data: String,
    error: String,
}

#[derive(Default)]
struct MockAgentState {
    /// (command, session_id) request log, for assertions.
    requests: Vec<(String, String)>,
    /// One-shot scripted responses keyed by `command` or `command:session_id`.
    scripts: HashMap<String, VecDeque<ScriptedResponse>>,
    /// `get_session_entries` payloads keyed by session id.
    session_entries: HashMap<String, Value>,
    /// Session id → cwd recorded at `new_session` (so `get_state` answers
    /// consistently and the caller doesn't recreate the session).
    session_cwds: HashMap<String, String>,
    session_counter: u64,
}

/// Handle to the process-global mock agent.
#[derive(Clone)]
pub(crate) struct MockAgent {
    state: Arc<Mutex<MockAgentState>>,
}

impl MockAgent {
    /// Enqueue a one-shot response for `command` (any session).
    pub(crate) fn script(&self, command: &str, success: bool, data: Value, error: &str) {
        self.script_for(command, "", success, data, error);
    }

    /// Enqueue a one-shot response for `command` addressed to `session_id`.
    pub(crate) fn script_for(
        &self,
        command: &str,
        session_id: &str,
        success: bool,
        data: Value,
        error: &str,
    ) {
        let key = format!("{command}:{session_id}");
        self.state
            .lock()
            .unwrap()
            .scripts
            .entry(key)
            .or_default()
            .push_back(ScriptedResponse {
                success,
                data: data.to_string(),
                error: error.to_string(),
            });
    }

    /// Set the `get_session_entries` payload for one session.
    pub(crate) fn set_session_entries(&self, session_id: &str, entries: Value) {
        self.state
            .lock()
            .unwrap()
            .session_entries
            .insert(session_id.to_string(), entries);
    }

    /// Snapshot of (command, session_id) pairs the mock has served.
    pub(crate) fn requests(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().requests.clone()
    }

    /// True when the mock served at least one `command` for `session_id`.
    pub(crate) fn served(&self, command: &str, session_id: &str) -> bool {
        self.requests()
            .iter()
            .any(|(c, s)| c == command && s == session_id)
    }
}

struct AgentService {
    state: Arc<Mutex<MockAgentState>>,
}

impl AgentService {
    fn answer(&self, cmd: crate::agent_proto::RpcCommand) -> crate::agent_proto::RpcResponse {
        let (success, data, error) = {
            let mut state = self.state.lock().unwrap();
            state
                .requests
                .push((cmd.r#type.clone(), cmd.session_id.clone()));
            let key = format!("{}:{}", cmd.r#type, cmd.session_id);
            let scripted = state
                .scripts
                .get_mut(&key)
                .and_then(VecDeque::pop_front)
                .or_else(|| {
                    state
                        .scripts
                        .get_mut(cmd.r#type.as_str())
                        .and_then(VecDeque::pop_front)
                });
            match scripted {
                Some(response) => (response.success, response.data, response.error),
                None => default_answer(&cmd, &mut state),
            }
        };
        crate::agent_proto::RpcResponse {
            id: cmd.id.clone(),
            r#type: "response".to_string(),
            command: cmd.r#type.clone(),
            success,
            data,
            error,
            ..Default::default()
        }
    }
}

fn ok(value: Value) -> (bool, String, String) {
    (true, value.to_string(), String::new())
}

fn default_answer(
    cmd: &crate::agent_proto::RpcCommand,
    state: &mut MockAgentState,
) -> (bool, String, String) {
    match cmd.r#type.as_str() {
        "list_streaming_sessions" => ok(json!({ "sessions": [] })),
        "list_models" => ok(json!({
            "models": [],
            "defaultModel": "",
            "isScoped": false,
            "builtinProviders": {
                "deepseek": {
                    "name": "DeepSeek",
                    "modelCount": 3,
                    "baseUrl": "https://api.deepseek.com/v1",
                },
                "azure-openai-responses": {
                    "name": "Azure OpenAI Responses",
                    "modelCount": 1,
                    "baseUrl": "https://YOUR_RESOURCE.openai.azure.com/openai",
                },
                // Filtered out of the GUI catalog: FutureGene is presented
                // separately, and an empty id is invalid.
                "future": {
                    "name": "Future",
                    "modelCount": 9,
                    "baseUrl": "https://future-os.cn/api/v1",
                },
                "": { "name": "", "modelCount": 0, "baseUrl": "" },
                // A pre-display-name agent entry: the GUI falls back to the id.
                "noname": {
                    "name": "",
                    "modelCount": 1,
                    "baseUrl": "https://noname.example.com/v1",
                },
            },
        })),
        "get_messages" => ok(json!({
            "messages": [
                { "role": "user", "content": "hi" },
                { "role": "assistant", "content": "hello" },
            ],
        })),
        "get_session_entries" => {
            let entries = state
                .session_entries
                .get(&cmd.session_id)
                .cloned()
                .unwrap_or_else(|| json!({ "entries": [] }));
            ok(entries)
        }
        "get_events_since" => ok(json!({ "events": [], "hasMore": false })),
        "get_state" => ok(json!({
            "sessionId": cmd.session_id,
            "cwd": state.session_cwds.get(&cmd.session_id).cloned().unwrap_or_default(),
            "isStreaming": false,
        })),
        "new_session" => {
            state.session_counter += 1;
            let session_id = format!("mock-session-{}", state.session_counter);
            state
                .session_cwds
                .insert(session_id.clone(), cmd.cwd.clone());
            ok(json!({ "sessionId": session_id }))
        }
        _ => ok(json!({})),
    }
}

#[tonic::async_trait]
impl crate::agent_proto::future_agent_server::FutureAgent for AgentService {
    async fn execute_command(
        &self,
        request: tonic::Request<crate::agent_proto::RpcCommand>,
    ) -> Result<tonic::Response<crate::agent_proto::RpcResponse>, tonic::Status> {
        Ok(tonic::Response::new(self.answer(request.into_inner())))
    }

    type StreamEventsStream =
        futures::stream::Empty<Result<crate::agent_proto::StreamEvent, tonic::Status>>;

    async fn stream_events(
        &self,
        _request: tonic::Request<crate::agent_proto::StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        // An immediately-ending stream: prompt drivers observe the
        // stream-interrupted path (no agent_end), which is all the remote
        // tests need.
        Ok(tonic::Response::new(futures::stream::empty()))
    }
}

/// Accepted-connection stream for `serve_with_incoming` (avoids depending on
/// tokio-stream feature flags).
struct Incoming {
    listener: tokio::net::TcpListener,
}

impl futures::Stream for Incoming {
    type Item = Result<tokio::net::TcpStream, std::io::Error>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.listener
            .poll_accept(cx)
            .map(|result| Some(result.map(|(stream, _)| stream)))
    }
}

/// Start (once per process) the mock agent and point the GUI at it.
pub(crate) fn ensure_mock_agent() -> MockAgent {
    static MOCK: OnceLock<MockAgent> = OnceLock::new();
    MOCK.get_or_init(|| {
        let state = Arc::new(Mutex::new(MockAgentState::default()));
        let (port_tx, port_rx) = std::sync::mpsc::channel();
        let service_state = state.clone();
        std::thread::Builder::new()
            .name("mock-agent".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .build()
                    .expect("mock agent runtime");
                runtime.spawn(async move {
                    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                        .await
                        .expect("bind mock agent");
                    port_tx
                        .send(listener.local_addr().expect("mock agent addr").port())
                        .expect("report mock agent port");
                    let server = tonic::transport::Server::builder()
                        .add_service(
                            crate::agent_proto::future_agent_server::FutureAgentServer::new(
                                AgentService {
                                    state: service_state,
                                },
                            ),
                        )
                        .serve_with_incoming(Incoming { listener });
                    // Serves until the test process exits; the outcome never
                    // materializes. `map` discards it without a block whose
                    // closing brace would count as an unreached line. Spawn on
                    // the ambient runtime so the outer `runtime` stays borrowable
                    // below (parking it keeps the server task alive).
                    tokio::spawn(futures::FutureExt::map(server, |_| ()));
                });
                // Park forever: dropping the runtime would kill the server task.
                runtime.block_on(std::future::pending::<()>());
            })
            .expect("spawn mock agent thread");
        let port = port_rx.recv().expect("mock agent port");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", format!("127.0.0.1:{port}"));
        MockAgent { state }
    })
    .clone()
}

// ── Fake NATS (core line protocol) ──────────────────────────────────────────

/// One published message observed by the fake server.
#[derive(Clone, Debug)]
pub(crate) struct Published {
    pub subject: String,
    #[allow(dead_code)] // populated for completeness; tests assert on `subject`
    pub reply: Option<String>,
    pub payload: Vec<u8>,
}

impl Published {
    pub(crate) fn json(&self) -> Value {
        serde_json::from_slice(&self.payload).expect("published payload is JSON")
    }
}

#[derive(Clone)]
struct NatsSub {
    pattern: String,
    queue: Option<String>,
    sid: String,
}

enum Out {
    Line(String),
    Msg {
        subject: String,
        sid: String,
        reply: Option<String>,
        payload: Vec<u8>,
    },
}

struct ConnHandle {
    subs: Vec<NatsSub>,
    tx: tokio::sync::mpsc::UnboundedSender<Out>,
}

#[derive(Default)]
struct NatsState {
    conns: HashMap<u64, ConnHandle>,
    next_id: u64,
}

/// A minimal in-process NATS server: INFO handshake, PING/PONG, SUB/UNSUB,
/// PUB with wildcard subject matching and queue-group single delivery.
pub(crate) struct FakeNats {
    url: String,
    state: Arc<Mutex<NatsState>>,
    tap: tokio::sync::broadcast::Sender<Published>,
    accept_task: tokio::task::JoinHandle<()>,
}

impl FakeNats {
    pub(crate) async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind fake nats");
        let port = listener.local_addr().expect("fake nats addr").port();
        let state = Arc::new(Mutex::new(NatsState::default()));
        let (tap, _) = tokio::sync::broadcast::channel(256);
        let accept_state = state.clone();
        let accept_tap = tap.clone();
        let accept_task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { continue };
                let conn_id = {
                    let mut state = accept_state.lock().unwrap();
                    state.next_id += 1;
                    state.next_id
                };
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                accept_state.lock().unwrap().conns.insert(
                    conn_id,
                    ConnHandle {
                        subs: Vec::new(),
                        tx,
                    },
                );
                tokio::spawn(serve_conn(
                    conn_id,
                    stream,
                    rx,
                    accept_state.clone(),
                    accept_tap.clone(),
                ));
            }
        });
        FakeNats {
            url: format!("nats://127.0.0.1:{port}"),
            state,
            tap,
            accept_task,
        }
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    /// Observe every message published through the server (client or injected).
    pub(crate) fn tap(&self) -> tokio::sync::broadcast::Receiver<Published> {
        self.tap.subscribe()
    }

    /// Deliver a message as if published by a client (no sender needed).
    pub(crate) fn inject(&self, subject: &str, reply: Option<&str>, payload: Vec<u8>) {
        deliver(
            &self.state,
            &self.tap,
            subject,
            reply.map(str::to_string),
            payload,
        );
    }

    /// Block until some connection subscribes to `pattern` (or panic).
    pub(crate) async fn wait_for_sub(&self, pattern: &str, timeout: std::time::Duration) {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let found = self
                .state
                .lock()
                .unwrap()
                .conns
                .values()
                .any(|conn| conn.subs.iter().any(|sub| sub.pattern == pattern));
            if found {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for a subscription to {pattern}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    /// Sever every connection and stop accepting (drives client-side
    /// subscribe/stream failure paths).
    pub(crate) fn kill(self) {
        self.accept_task.abort();
        self.state.lock().unwrap().conns.clear();
    }
}

impl Drop for FakeNats {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

/// Wait for a tapped publish on `subject`; panic after `timeout`.
pub(crate) async fn await_publish(
    rx: &mut tokio::sync::broadcast::Receiver<Published>,
    subject: &str,
    timeout: std::time::Duration,
) -> Published {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let outcome = tokio::time::timeout(remaining, rx.recv()).await;
        let published = outcome
            .ok()
            .and_then(Result::ok)
            .filter(|published| published.subject == subject);
        match published {
            Some(published) => return published,
            None if remaining.is_zero() => panic!("timed out waiting for a publish on {subject}"),
            None => continue,
        }
    }
}

/// Assert that no publish on `subject` is tapped within `window`.
pub(crate) async fn assert_no_publish(
    rx: &mut tokio::sync::broadcast::Receiver<Published>,
    subject: &str,
    window: std::time::Duration,
) {
    let deadline = std::time::Instant::now() + window;
    while let Ok(Ok(published)) = tokio::time::timeout(
        deadline.saturating_duration_since(std::time::Instant::now()),
        rx.recv(),
    )
    .await
    {
        assert_ne!(published.subject, subject, "unexpected publish on {subject}");
    }
}

fn subject_matches(pattern: &str, subject: &str) -> bool {
    let mut pattern_tokens = pattern.split('.').peekable();
    let mut subject_tokens = subject.split('.');
    while let Some(pattern_token) = pattern_tokens.next() {
        if pattern_token == ">" {
            return true;
        }
        match subject_tokens.next() {
            Some(token) if pattern_token == "*" || pattern_token == token => {}
            _ => return false,
        }
    }
    subject_tokens.next().is_none()
}

fn deliver(
    state: &Arc<Mutex<NatsState>>,
    tap: &tokio::sync::broadcast::Sender<Published>,
    subject: &str,
    reply: Option<String>,
    payload: Vec<u8>,
) {
    let _ = tap.send(Published {
        subject: subject.to_string(),
        reply: reply.clone(),
        payload: payload.clone(),
    });
    let state = state.lock().unwrap();
    // Non-queue subscribers each receive a copy; queue groups receive exactly
    // one copy (first live member), keyed by (pattern, queue).
    let mut served_groups: Vec<(String, String)> = Vec::new();
    for conn in state.conns.values() {
        for sub in &conn.subs {
            if !subject_matches(&sub.pattern, subject) {
                continue;
            }
            if let Some(queue) = &sub.queue {
                let group = (sub.pattern.clone(), queue.clone());
                if served_groups.contains(&group) {
                    continue;
                }
                served_groups.push(group);
            }
            let _ = conn.tx.send(Out::Msg {
                subject: subject.to_string(),
                sid: sub.sid.clone(),
                reply: reply.clone(),
                payload: payload.clone(),
            });
        }
    }
}

/// Serialize one outbound frame; the caller treats any error as fatal.
async fn write_out(
    write_half: &mut tokio::net::tcp::OwnedWriteHalf,
    out: Out,
) -> Result<(), std::io::Error> {
    match out {
        Out::Line(line) => write_half.write_all(line.as_bytes()).await,
        Out::Msg {
            subject,
            sid,
            reply,
            payload,
        } => {
            let header = match &reply {
                Some(reply) => format!("MSG {subject} {sid} {reply} {}\r\n", payload.len()),
                None => format!("MSG {subject} {sid} {}\r\n", payload.len()),
            };
            let mut frame = header.into_bytes();
            frame.extend_from_slice(&payload);
            frame.extend_from_slice(b"\r\n");
            write_half.write_all(&frame).await
        }
    }
}

async fn serve_conn(
    conn_id: u64,
    stream: tokio::net::TcpStream,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Out>,
    state: Arc<Mutex<NatsState>>,
    tap: tokio::sync::broadcast::Sender<Published>,
) {
    let (read_half, mut write_half) = stream.into_split();
    let writer = tokio::spawn(async move {
        let mut alive = true;
        while alive {
            match rx.recv().await {
                Some(out) => alive = write_out(&mut write_half, out).await.is_ok(),
                None => alive = false,
            }
        }
    });

    let info = concat!(
        "INFO {\"server_id\":\"FAKE\",\"server_name\":\"fake-nats\",\"version\":\"2.10.2\",",
        "\"host\":\"127.0.0.1\",\"port\":0,\"max_payload\":8388608,\"client_id\":1,",
        "\"proto\":1,\"headers\":true,\"auth_required\":false,\"nonce\":\"dGVzdA==\"}\r\n"
    );
    state
        .lock()
        .unwrap()
        .conns
        .get(&conn_id)
        .expect("connection registered while its read loop runs")
        .tx
        .send(Out::Line(info.to_string()))
        .expect("fresh connection channel");

    let mut reader = tokio::io::BufReader::new(read_half);
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).await;
        match read {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&line);
        let tokens: Vec<&str> = text.split_whitespace().collect();
        match tokens.first().copied() {
            Some("PING") => {
                // The connection entry lives until this read loop breaks, so
                // the lookup cannot fail here.
                let tx = state
                    .lock()
                    .unwrap()
                    .conns
                    .get(&conn_id)
                    .map(|conn| conn.tx.clone())
                    .expect("connection registered while its read loop runs");
                let _ = tx.send(Out::Line("PONG\r\n".to_string()));
            }
            Some("CONNECT") | Some("PONG") => {}
            Some("SUB") => {
                // SUB <subject> [queue] <sid>
                let sub = match tokens.as_slice() {
                    [_, subject, sid] => NatsSub {
                        pattern: subject.to_string(),
                        queue: None,
                        sid: sid.to_string(),
                    },
                    [_, subject, queue, sid] => NatsSub {
                        pattern: subject.to_string(),
                        queue: Some(queue.to_string()),
                        sid: sid.to_string(),
                    },
                    _ => continue,
                };
                state
                    .lock()
                    .unwrap()
                    .conns
                    .entry(conn_id)
                    .and_modify(|conn| conn.subs.push(sub));
            }
            Some("UNSUB") => {
                if let Some(sid) = tokens.get(1) {
                    state
                        .lock()
                        .unwrap()
                        .conns
                        .entry(conn_id)
                        .and_modify(|conn| conn.subs.retain(|sub| sub.sid != *sid));
                }
            }
            Some("PUB") => {
                // PUB <subject> [reply] <size>
                let (subject, reply, size) = match tokens.as_slice() {
                    [_, subject, size] => (*subject, None, size.parse::<usize>().unwrap_or(0)),
                    [_, subject, reply, size] => {
                        (*subject, Some(reply.to_string()), size.parse().unwrap_or(0))
                    }
                    _ => continue,
                };
                let mut payload = vec![0_u8; size];
                if reader.read_exact(&mut payload).await.is_err() {
                    break;
                }
                let mut crlf = [0_u8; 2];
                if reader.read_exact(&mut crlf).await.is_err() {
                    break;
                }
                deliver(&state, &tap, subject, reply, payload);
            }
            _ => {}
        }
    }
    state.lock().unwrap().conns.remove(&conn_id);
    writer.abort();
}

/// Connect an async-nats client to the fake server (no auth).
pub(crate) async fn nats_connect(nats: &FakeNats) -> async_nats::Client {
    async_nats::connect(nats.url())
        .await
        .expect("connect to fake nats")
}

/// Connect with reconnects disabled: when the server dies the connection
/// closes for good, so subscription streams end and publishes fail — the
/// shape the bridge's self-heal loops are built around. Note async-nats maps
/// `max_reconnects(0)` to `None` ("no limit"), so one allowed reconnect is
/// the least that still gives up: the first reconnect attempt trips the
/// MaxReconnects check and the client closes instead of buffering forever.
pub(crate) async fn nats_connect_once(nats: &FakeNats) -> async_nats::Client {
    async_nats::ConnectOptions::new()
        .max_reconnects(1)
        .connect(nats.url())
        .await
        .expect("connect to fake nats")
}

// ── Mock HTTP platform ──────────────────────────────────────────────────────

#[derive(Default)]
struct PlatformState {
    /// Scripted (status, body) responses keyed by path, served in order.
    scripts: HashMap<String, VecDeque<(u16, String)>>,
    /// (method, path, body) request log.
    requests: Vec<(String, String, String)>,
}

/// A scripted HTTP/1.1 server for the pairing control plane. Unscripted paths
/// answer 404 `{"error":"not_found"}`.
pub(crate) struct MockPlatform {
    url: String,
    state: Arc<Mutex<PlatformState>>,
    task: tokio::task::JoinHandle<()>,
}

impl MockPlatform {
    pub(crate) async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind mock platform");
        let port = listener.local_addr().expect("mock platform addr").port();
        let state = Arc::new(Mutex::new(PlatformState::default()));
        let accept_state = state.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { continue };
                tokio::spawn(serve_http(stream, accept_state.clone()));
            }
        });
        MockPlatform {
            url: format!("http://127.0.0.1:{port}"),
            state,
            task,
        }
    }

    /// Platform root URL (no `/api`) for auth.json.
    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    /// Enqueue a one-shot response for `path`.
    pub(crate) fn push(&self, path: &str, status: u16, body: Value) {
        self.state
            .lock()
            .unwrap()
            .scripts
            .entry(path.to_string())
            .or_default()
            .push_back((status, body.to_string()));
    }

    /// Snapshot of (method, path, body) requests served so far.
    pub(crate) fn requests(&self) -> Vec<(String, String, String)> {
        self.state.lock().unwrap().requests.clone()
    }

    /// Script a successful pair-code issuance pointing at `nats_url`.
    pub(crate) fn respond_pair_code(&self, nats_url: &str) -> String {
        let code = pairing_code(now_secs() + 600);
        self.push(
            "/client/v1/remote/pair/code",
            200,
            json!({
                "pair_id": format!("pair_{}", unique("mock")),
                "pairing_code": code,
                "user_jwt": jwt(now_secs() + 3600),
                "nats_url": nats_url,
                "nats_ws_url": nats_url.replace("nats://", "ws://"),
            }),
        );
        code
    }

    /// Script a successful bridge-JWT refresh pointing at `nats_url`.
    pub(crate) fn respond_refresh(&self, nats_url: &str) {
        self.push(
            "/client/v1/remote/auth/token",
            200,
            json!({
                "user_jwt": jwt(now_secs() + 3600),
                "nats_url": nats_url,
                "nats_ws_url": nats_url.replace("nats://", "ws://"),
            }),
        );
    }

    /// Script a revoked-credential refresh failure.
    pub(crate) fn respond_refresh_revoked(&self) {
        self.push(
            "/client/v1/remote/auth/token",
            401,
            json!({
                "error": "invalid_remote_credential",
                "message": "Remote credential is invalid or revoked.",
            }),
        );
    }
}

impl Drop for MockPlatform {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_http(mut stream: tokio::net::TcpStream, state: Arc<Mutex<PlatformState>>) {
    let (read_half, mut write_half) = stream.split();
    let mut reader = tokio::io::BufReader::new(read_half);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await.is_err() {
        return;
    }
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    let (method, path) = match parts.as_slice() {
        [method, path, ..] => (method.to_string(), path.to_string()),
        _ => return,
    };
    let mut content_length = 0_usize;
    let mut header = String::new();
    loop {
        header.clear();
        match reader.read_line(&mut header).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let trimmed = header.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed
            .to_ascii_lowercase()
            .strip_prefix("content-length:")
            .and_then(|value| value.trim().parse::<usize>().ok())
        {
            content_length = value;
        }
    }
    let mut body = vec![0_u8; content_length];
    if reader.read_exact(&mut body).await.is_err() {
        return;
    }
    let body_text = String::from_utf8_lossy(&body).to_string();
    let (status, response_body) = {
        let mut state = state.lock().unwrap();
        state
            .requests
            .push((method.clone(), path.clone(), body_text));
        state
            .scripts
            .get_mut(&path)
            .and_then(VecDeque::pop_front)
            .unwrap_or((404, json!({ "error": "not_found" }).to_string()))
    };
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
        response_body.len()
    );
    let _ = write_half.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Read one CRLF-terminated line from a raw socket (with a deadline).
    async fn read_line(stream: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let mut byte = [0_u8; 1];
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let read = tokio::time::timeout(remaining, stream.read(&mut byte))
                .await
                .expect("read deadline")
                .expect("read");
            if read == 0 || byte[0] == b'\n' {
                break;
            }
            bytes.push(byte[0]);
        }
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[tokio::test]
    async fn fake_nats_tolerates_malformed_frames() {
        let nats = FakeNats::start().await;
        let addr = nats.url().trim_start_matches("nats://").to_string();
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        // The INFO handshake arrives first; PING gets a PONG.
        assert!(read_line(&mut raw).await.starts_with("INFO"));
        raw.write_all(b"PING\r\n").await.unwrap();
        assert_eq!(read_line(&mut raw).await, "PONG\r");

        // Malformed control lines are skipped, not fatal.
        raw.write_all(b"SUB\r\n").await.unwrap(); // missing args
        raw.write_all(b"SUB only-subject\r\n").await.unwrap(); // missing sid
        raw.write_all(b"UNSUB\r\n").await.unwrap(); // no sid
        raw.write_all(b"HELLO there\r\n").await.unwrap(); // unknown verb
        raw.write_all(b"PUB\r\n").await.unwrap(); // malformed publish
        raw.write_all(b"PING\r\n").await.unwrap();
        assert_eq!(read_line(&mut raw).await, "PONG\r");
        drop(raw);

        // A payload truncated mid-body drops the connection.
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        assert!(read_line(&mut raw).await.starts_with("INFO"));
        raw.write_all(b"PUB s 5\r\nab").await.unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);
    }

    #[tokio::test]
    async fn fake_nats_drops_a_connection_with_a_truncated_payload() {
        let nats = FakeNats::start().await;
        let addr = nats.url().trim_start_matches("nats://").to_string();
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        assert!(read_line(&mut raw).await.starts_with("INFO"));
        // Declare 5 payload bytes but send only 2, then half-close: the payload
        // read fails and the server must drop the connection.
        raw.write_all(b"PUB s 5\r\nab").await.unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);
        // Wait until the server removes the half-read connection, making the
        // payload-read break under test deterministic.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if nats.state.lock().unwrap().conns.is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "truncated-payload connection never dropped"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn fake_nats_drops_a_connection_missing_its_trailing_crlf() {
        let nats = FakeNats::start().await;
        let addr = nats.url().trim_start_matches("nats://").to_string();
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        assert!(read_line(&mut raw).await.starts_with("INFO"));
        // Send a complete payload but omit the trailing CRLF, then half-close:
        // the server must drop the connection instead of hanging.
        raw.write_all(b"PUB s 2\r\nok").await.unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);
        // Wait until the server removes the half-read connection (the trailing
        // CRLF read fails), making the break under test deterministic.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if nats.state.lock().unwrap().conns.is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "missing-CRLF connection never dropped"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn fake_nats_delivers_a_queue_group_to_one_member_only() {
        let nats = FakeNats::start().await;
        let first = nats_connect(&nats).await;
        let second = nats_connect(&nats).await;
        use futures::StreamExt;
        let mut sub_a = first
            .queue_subscribe("q.subject".to_string(), "workers".to_string())
            .await
            .unwrap();
        let mut sub_b = second
            .queue_subscribe("q.subject".to_string(), "workers".to_string())
            .await
            .unwrap();
        // Both subscriptions must be live before the publish, or the dedup
        // branch under test never runs.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let members = nats
                .state
                .lock()
                .unwrap()
                .conns
                .values()
                .flat_map(|conn| conn.subs.iter())
                .filter(|sub| sub.pattern == "q.subject")
                .count();
            if members == 2 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "both queue subscriptions never registered"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        // Both members share one (pattern, queue) group: each publish lands on
        // exactly one of them, never both.
        for _ in 0..2 {
            nats.inject("q.subject", None, b"x".to_vec());
        }
        let got_a = tokio::time::timeout(std::time::Duration::from_millis(200), sub_a.next())
            .await
            .ok()
            .flatten()
            .is_some();
        let got_b = tokio::time::timeout(std::time::Duration::from_millis(200), sub_b.next())
            .await
            .ok()
            .flatten()
            .is_some();
        assert!(got_a || got_b, "the group must receive the publish");
        assert_ne!(got_a, got_b, "only one group member may receive it");
    }

    #[tokio::test]
    #[should_panic(expected = "timed out waiting for a publish")]
    async fn await_publish_times_out_when_nothing_matches() {
        let nats = FakeNats::start().await;
        let mut tap = nats.tap();
        await_publish(&mut tap, "never.published", std::time::Duration::from_millis(30)).await;
    }

    #[tokio::test]
    async fn assert_no_publish_fails_when_one_arrives() {
        let nats = FakeNats::start().await;
        let mut tap = nats.tap();
        nats.inject("some.subject", None, Vec::new());
        let result = tokio::spawn(async move {
            assert_no_publish(&mut tap, "some.subject", std::time::Duration::from_secs(1)).await;
        })
        .await
        .expect_err("a matching publish must fail the assertion");
        assert!(result.is_panic());
    }

    #[tokio::test]
    async fn mock_platform_handles_broken_requests() {
        let platform = MockPlatform::start().await;
        let addr = platform.url().trim_start_matches("http://").to_string();

        // Connect and close without sending anything.
        let raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        drop(raw);

        // Invalid UTF-8 in the request line fails the read outright.
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        raw.write_all(b"\xff\xfe\r\n").await.unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);

        // A request line without a path.
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        raw.write_all(b"GARBAGE\r\n\r\n").await.unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);

        // Headers that never terminate.
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        raw.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n").await.unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);

        // A body shorter than Content-Length.
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        raw.write_all(b"POST /x HTTP/1.1\r\nContent-Length: 100\r\n\r\nshort")
            .await
            .unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);

        // An unscripted path → 404; an unusual scripted status maps its name.
        platform.push("/redirect", 302, serde_json::json!({}));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let response = client
            .get(format!("{}/redirect", platform.url()))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 302);
        let response = client
            .get(format!("{}/nowhere", platform.url()))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 404);
    }
}

