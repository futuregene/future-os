//! Test-only support: a scripted in-process mock of the FutureAgent gRPC
//! service plus HOME-isolated store setup.
//!
//! `connect_agent` caches one process-global channel keyed off
//! `FUTURE_AGENT_GRPC_ADDR`, so the whole test binary shares a single mock
//! server (started lazily on first use). Tests that drive the agent channel
//! hold [`mock_agent`]'s guard for their whole duration — it serializes them
//! and resets the script, so per-test expectations stay deterministic.
//!
//! Replies are scripted PER COMMAND TYPE (a FIFO queue each), not globally:
//! `agent_prompt` spawns a session observer whose replay/probe RPCs interleave
//! with the pipeline's own calls, and only per-type queues keep that
//! deterministic. Streams split the same way on `atomic_attach` (collectors
//! and active-run probes attach atomically; idle observers plain-subscribe).
//!
//! Tests that touch the SQLite store additionally hold [`TestHome`], which
//! redirects `HOME` (the store resolves its db path per call and the
//! connection pool re-keys on path change). Acquire `TestHome` FIRST and the
//! mock guard second everywhere to keep lock ordering consistent.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use future_rpc::proto::future_agent_server::{FutureAgent, FutureAgentServer};
use future_rpc::proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};

/// One scripted `execute_command` reply, consumed FIFO per command type.
pub enum Reply {
    /// `success = true` with the given JSON `data` string.
    Data(String),
    /// `success = true` with the production typed payload and empty `data`.
    TypedData(serde_json::Value),
    /// `success = false` with this error message.
    Reject(String),
    /// `success = false` with a stable `error_code` (the shape the production
    /// agent sends for a session it has no record of) and this message.
    RejectCode(&'static str, String),
    /// Transport-level failure (tonic status).
    Status(tonic::Code, &'static str),
    /// Simulate the production instance fence after the endpoint is replaced.
    BoundData {
        instance: &'static str,
        data: serde_json::Value,
    },
    /// Signal request arrival, then wait for a test-controlled JSON response.
    Deferred {
        entered: tokio::sync::oneshot::Sender<()>,
        data: tokio::sync::oneshot::Receiver<serde_json::Value>,
    },
}

/// Production-shaped JSON source for a typed-only `get_state` response.
pub fn get_state_payload(session_id: &str, is_streaming: bool) -> serde_json::Value {
    serde_json::json!({
        "agentInstanceId": "agent-test",
        "model": "future/test",
        "imageSupport": false,
        "thinkingLevel": "medium",
        "isStreaming": is_streaming,
        "isCompacting": false,
        "sessionFile": "",
        "sessionId": session_id,
        "sessionName": "Typed Session",
        "explicitSession": true,
        "autoCompactionEnabled": true,
        "queryCount": 1,
        "version": "test",
        "cwd": "/tmp",
        "skills": [],
        "contextFiles": [],
        "extensions": [],
        "contextWindow": 1000,
        "contextTokens": 10,
        "contextPercent": 1.0,
        "usage":{"inputTokens":1,"outputTokens":2,"cacheReadTokens":0,"cacheWriteTokens":0,"costCny":0.0},
        "permissionLevel": "default",
        "parentSessionId": null,
        "createdBy": "desktop",
        "sourceMeta": null,
        "activeRun": null,
        "queuedRuns": [],
        "recentTerminalAcks": [],
        "queuedCount": 0,
        "interruptedRun": null,
        "requestedRun": null,
        "pendingApprovals": []
    })
}

/// One scripted `stream_events` outcome.
pub enum StreamScript {
    /// The attach itself fails with this status.
    AttachError(tonic::Code, &'static str),
    /// Yield these events in order, then close — or fail mid-stream when the
    /// terminal status is set.
    Events(Vec<StreamEvent>, Option<(tonic::Code, &'static str)>),
    /// A stream that never yields (idle/quiet-window paths).
    Hang,
}

#[derive(Default)]
struct MockState {
    down: bool,
    replies: HashMap<String, VecDeque<Reply>>,
    /// Command types answered with a rejection whenever no scripted reply is
    /// queued. A one-shot `Reply::Reject` cannot express "this call must fail"
    /// when the caller is a *spawned* task: a leftover reply from an earlier
    /// test, or a lingering spawn from one, can consume it first (measured in
    /// this crate — a test that pushed one `new_session` rejection had its
    /// failure arm read `DA:0` for a whole segment because another caller took
    /// the reply). This map is keyed by command type and survives the call.
    reject_defaults: HashMap<String, String>,
    default_data: HashMap<String, String>,
    atomic_streams: VecDeque<StreamScript>,
    plain_streams: VecDeque<StreamScript>,
    requests: Vec<RpcCommand>,
    stream_requests: Vec<StreamRequest>,
}

static STATE: std::sync::LazyLock<Mutex<MockState>> =
    std::sync::LazyLock::new(|| Mutex::new(MockState::default()));

fn rpc_response(cmd: &RpcCommand, success: bool, data: String, error: String) -> RpcResponse {
    RpcResponse {
        id: cmd.id.clone(),
        r#type: "response".to_string(),
        command: cmd.r#type.clone(),
        success,
        data,
        error,
        ..Default::default()
    }
}

struct MockAgent;

/// Default reply payload for an unscripted command: a generic success, so
/// incidental calls (observer replays, health checks) always work. `prompt`
/// echoes the requested run id so tests that let the pipeline generate one
/// still get a consistent acknowledgement.
fn default_reply_data(cmd: &RpcCommand) -> String {
    if cmd.r#type == "get_session_presence" {
        return serde_json::json!({"databaseId":"test-agent-database","restoreGeneration":"test-generation",
            "sessions":cmd.session_ids.iter().map(|id|serde_json::json!({"sessionId":id,"status":"present"})).collect::<Vec<_>>()}).to_string();
    }
    if cmd.r#type == "get_agent_readiness" {
        return serde_json::json!({"version":crate::build_info::VERSION,"agentInstanceId":"agent-test","protocolVersion":future_rpc::connection::AGENT_PROTOCOL_VERSION,"dataRootId":"0".repeat(64),"storageSchemaVersion":future_rpc::connection::AGENT_STORAGE_SCHEMA_VERSION}).to_string();
    }
    if cmd.r#type == "delete_sessions" {
        return serde_json::json!({"results": cmd.session_ids.iter().map(|session_id| {
            serde_json::json!({"sessionId": session_id, "deleted": true,
                "error": "", "errorCode": "", "errorData": null})
        }).collect::<Vec<_>>()})
        .to_string();
    }
    if cmd.r#type == "prompt" {
        let run_id = if cmd.requested_run_id.is_empty() {
            "mock-run"
        } else {
            cmd.requested_run_id.as_str()
        };
        return format!(r#"{{"run_id":"{run_id}"}}"#);
    }
    "{}".to_string()
}

#[tonic::async_trait]
impl FutureAgent for MockAgent {
    async fn execute_command(
        &self,
        request: tonic::Request<RpcCommand>,
    ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
        let expected_instance = request
            .metadata()
            .get(future_rpc::connection::EXPECTED_INSTANCE_HEADER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let cmd = request.into_inner();
        let reply = {
            let mut state = STATE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.requests.push(cmd.clone());
            if state.down && cmd.r#type != "list_streaming_sessions" {
                return Err(tonic::Status::unavailable("mock agent is down"));
            }
            // get_run_state (get_state with a run id) has its own queues keyed
            // by run id: spawned session observers fire plain get_state probes
            // concurrently, and a shared FIFO would let them steal run-scoped
            // replies. A session-scoped queue (`get_state@<session>`) lets a
            // test target one session's probe deterministically; it falls back
            // to the shared `get_state` queue so legacy call sites keep
            // working.
            let mut take = |key: &str| -> Option<Reply> {
                state.replies.get_mut(key).and_then(VecDeque::pop_front)
            };
            let reply = if cmd.r#type == "get_state" && !cmd.run_id.is_empty() {
                take(&format!("get_state#{}", cmd.run_id))
            } else if cmd.r#type == "get_state" && !cmd.session_id.is_empty() {
                take(&format!("get_state@{}", cmd.session_id)).or_else(|| take("get_state"))
            } else {
                take(&cmd.r#type)
            };
            let reject_default = state.reject_defaults.get(&cmd.r#type).cloned();
            reply.unwrap_or_else(|| match reject_default {
                Some(error) => Reply::Reject(error),
                None => Reply::Data(
                    state
                        .default_data
                        .get(&cmd.r#type)
                        .cloned()
                        .unwrap_or_else(|| default_reply_data(&cmd)),
                ),
            })
        };
        match reply {
            Reply::Data(data) => Ok(tonic::Response::new(rpc_response(
                &cmd,
                true,
                data,
                String::new(),
            ))),
            Reply::TypedData(data) => {
                let payload = future_rpc::encode::response_payload(&cmd.r#type, &data)
                    .expect("scripted typed payload must match its command");
                let mut response = rpc_response(&cmd, true, String::new(), String::new());
                response.payload = Some(payload);
                Ok(tonic::Response::new(response))
            }
            Reply::Reject(error) => Ok(tonic::Response::new(rpc_response(
                &cmd,
                false,
                String::new(),
                error,
            ))),
            Reply::RejectCode(code, error) => {
                let mut response = rpc_response(&cmd, false, String::new(), error);
                response.error_code = code.to_string();
                Ok(tonic::Response::new(response))
            }
            Reply::Status(code, message) => Err(tonic::Status::new(code, message)),
            Reply::BoundData { instance, data } => {
                if expected_instance != instance {
                    return Err(tonic::Status::failed_precondition("Agent instance changed"));
                }
                Ok(tonic::Response::new(rpc_response(
                    &cmd,
                    true,
                    data.to_string(),
                    String::new(),
                )))
            }
            Reply::Deferred { entered, data } => {
                let _ = entered.send(());
                let data = data
                    .await
                    .map_err(|_| tonic::Status::cancelled("test reply dropped"))?;
                Ok(tonic::Response::new(rpc_response(
                    &cmd,
                    true,
                    data.to_string(),
                    String::new(),
                )))
            }
        }
    }

    type StreamEventsStream =
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamEvent, tonic::Status>> + Send>>;

    async fn stream_events(
        &self,
        request: tonic::Request<StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        let req = request.into_inner();
        let attach_run_id = req.run_id.clone();
        let script = {
            let mut state = STATE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let atomic = req.atomic_attach;
            state.stream_requests.push(req);
            let queue = if atomic {
                &mut state.atomic_streams
            } else {
                &mut state.plain_streams
            };
            queue.pop_front()
        };
        match script {
            Some(StreamScript::AttachError(code, message)) => {
                Err(tonic::Status::new(code, message))
            }
            // An unscripted stream parks forever: idle observers subscribe
            // this way, and a parked stream must not produce spurious events.
            Some(StreamScript::Hang) | None => {
                Ok(tonic::Response::new(Box::pin(futures::stream::pending())))
            }
            Some(StreamScript::Events(events, terminal)) => {
                // `@attach` is a run-id placeholder for tests where the
                // canonical run id is only chosen once the pipeline runs
                // (e.g. prompt-generated ids): bind it to the attach's
                // requested run id.
                let mut items: Vec<Result<StreamEvent, tonic::Status>> = Vec::new();
                for mut event in events {
                    if event.run_id == "@attach" {
                        event.run_id.clone_from(&attach_run_id);
                    }
                    items.push(Ok(event));
                }
                if let Some((code, message)) = terminal {
                    items.push(Err(tonic::Status::new(code, message)));
                }
                Ok(tonic::Response::new(Box::pin(futures::stream::iter(items))))
            }
        }
    }
}

// This driver belongs only to the test harness. Production services require an
// entry-provided runtime and never construct one as a fallback.
pub(crate) fn install_channel_runtime() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        if crate::agent_client::agent_channel_runtime().is_ok() {
            return;
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(1)
            .build()
            .expect("fixture Agent channel runtime");
        crate::agent_client::configure_channel_runtime(runtime.handle().clone())
            .expect("fixture installs its channel runtime once");
        std::thread::Builder::new()
            .name("fixture-agent-channel".into())
            .spawn(move || runtime.block_on(std::future::pending::<()>()))
            .expect("fixture channel driver");
    });
}

fn cleanup_host() {
    crate::auth_store::test_support::cleanup_host();
}

fn ensure_mock_server() {
    static START: OnceLock<()> = OnceLock::new();
    static MOCK_ADDR: OnceLock<String> = OnceLock::new();
    START.get_or_init(|| {
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("mock-future-agent".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Runtime::new().expect("mock agent runtime");
                runtime.block_on(async move {
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                        .await
                        .expect("bind mock agent listener");
                    let addr = listener.local_addr().expect("mock agent local addr");
                    addr_tx.send(addr).expect("report mock agent addr");
                    let incoming =
                        futures::stream::poll_fn(move |cx| match listener.poll_accept(cx) {
                            std::task::Poll::Ready(result) => {
                                std::task::Poll::Ready(Some(result.map(|(stream, _)| stream)))
                            }
                            std::task::Poll::Pending => std::task::Poll::Pending,
                        });
                    // Drive the server on the runtime's worker pool; it lives
                    // until the process exits, so it is spawned rather than
                    // awaited (awaiting would park this async block forever).
                    let serve = tonic::transport::Server::builder()
                        .add_service(FutureAgentServer::new(MockAgent))
                        .serve_with_incoming(incoming);
                    tokio::spawn(serve);
                });
                // Park this thread: the runtime's worker threads keep serving
                // the mock until the test process exits.
                std::thread::park();
            })
            .expect("spawn mock agent thread");
        let addr = addr_rx.recv().expect("mock agent address");
        let addr = addr.to_string();
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", &addr);
        let _ = MOCK_ADDR.set(addr);
        // Warm the shared-channel OnceCell NOW, while the script is empty and
        // its init health check consumes the default reply. Without this the
        // first test to call connect_agent would lose its first scripted
        // reply to the health check, making outcomes depend on test order.
        std::thread::Builder::new()
            .name("mock-agent-warmup".to_string())
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("warmup runtime");
                runtime
                    .block_on(crate::agent_client::connect_agent())
                    .expect("warmup connect to mock agent");
            })
            .expect("spawn warmup thread")
            .join()
            .expect("warmup thread");
    });
    // Re-assert the env var on every call: the remote/commands mock
    // (`remote::test_support::ensure_mock_agent`) re-points
    // `FUTURE_AGENT_GRPC_ADDR` at ITS server on each call, and the shared
    // channel is keyed by endpoint — so a stale env would make this mock's
    // tests hit the other server. Mirror that re-assert here.
    if let Some(addr) = MOCK_ADDR.get() {
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", addr);
    }
}

static MOCK_LOCK: Mutex<()> = Mutex::new(());

/// Guard serializing every test that drives the shared agent channel. On
/// acquisition it (lazily) starts the mock server, points
/// `FUTURE_AGENT_GRPC_ADDR` at it, and resets the script + request log.
pub struct MockAgentGuard {
    _lock: MutexGuard<'static, ()>,
    timing: crate::test_support::timing::FixtureTiming,
}

pub fn mock_agent() -> MockAgentGuard {
    mock_agent_with_cleanup(crate::agent_bridge::cancel_all_observers)
}

pub fn mock_agent_with_cleanup(cleanup: fn()) -> MockAgentGuard {
    let mut timing = crate::test_support::timing::FixtureTiming::start("MockAgentGuard", "agent");
    let lock = MOCK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    timing.mark("lock_wait");
    // A finished test's observers keep re-attaching on the ambient runtime;
    // drop them now so their get_state/stream_events calls can't consume the
    // replies this test is about to script.
    cleanup();
    timing.mark("pre_cleanup");
    install_channel_runtime();
    ensure_mock_server();
    // Each independent fixture starts with a fresh verification. Otherwise a
    // one-shot readiness failure can be consumed by the previous test's cache
    // probe, followed by a successful fresh handshake from the default script.
    crate::agent_client::reset_test_connection();
    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.down = false;
        state.replies.clear();
        state.reject_defaults.clear();
        state.default_data.clear();
        state.atomic_streams.clear();
        state.plain_streams.clear();
        state.requests.clear();
        state.stream_requests.clear();
    }
    timing.mark("setup");
    MockAgentGuard {
        _lock: lock,
        timing,
    }
}

impl Drop for MockAgentGuard {
    fn drop(&mut self) {
        self.timing.mark("body");
        self.timing.finish();
    }
}

impl MockAgentGuard {
    #[cfg(test)]
    pub(crate) fn set_down(&self, down: bool) {
        crate::agent_client::reset_test_connection();
        STATE.lock().unwrap_or_else(|e| e.into_inner()).down = down;
    }

    #[cfg(test)]
    pub(crate) fn default_data(&self, command: &str, data: serde_json::Value) {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .default_data
            .insert(command.into(), data.to_string());
    }
    #[cfg(test)]
    pub(crate) fn script(
        &self,
        command: &str,
        success: bool,
        data: serde_json::Value,
        error: &str,
    ) {
        self.push(
            command,
            if success {
                Reply::Data(data.to_string())
            } else {
                Reply::Reject(error.into())
            },
        );
    }
    #[cfg(test)]
    pub(crate) fn served(&self, command: &str, session: &str) -> bool {
        self.requests()
            .iter()
            .any(|request| request.r#type == command && request.session_id == session)
    }

    /// Queue a reply for one command type (FIFO within the type).
    pub fn push(&self, command_type: &str, reply: Reply) {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replies
            .entry(command_type.to_string())
            .or_default()
            .push_back(reply);
    }

    /// Make every *unscripted* reply for `command_type` a rejection until
    /// cleared with [`MockAgentGuard::clear_reject_defaults`]. Deterministic for
    /// spawned callers, which a one-shot reply is not.
    pub fn reject_default(&self, command_type: &str, error: &str) {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reject_defaults
            .insert(command_type.to_string(), error.to_string());
    }

    pub fn clear_reject_defaults(&self) {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reject_defaults
            .clear();
    }

    /// Queue a successful reply whose `data` is the JSON rendering of `value`.
    pub fn push_data(&self, command_type: &str, value: serde_json::Value) {
        self.push(command_type, Reply::Data(value.to_string()));
    }

    /// Queue a production-shaped typed-only response (`data` stays empty).
    pub fn push_typed_data(&self, command_type: &str, value: serde_json::Value) {
        self.push(command_type, Reply::TypedData(value));
    }

    /// Queue a `get_run_state` reply (get_state carrying a run id) for one
    /// canonical run id.
    pub fn push_run_state(&self, run_id: &str, value: serde_json::Value) {
        self.push(
            &format!("get_state#{run_id}"),
            Reply::Data(value.to_string()),
        );
    }

    /// Queue a plain `get_state` reply for one specific session. Targets only
    /// that session's probe (the dispatch keys plain get_state by session and
    /// falls back to the shared queue), so concurrently-spawned observers for
    /// other sessions cannot steal it.
    pub fn push_state_for_session(&self, session_id: &str, reply: Reply) {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replies
            .entry(format!("get_state@{session_id}"))
            .or_default()
            .push_back(reply);
    }

    /// Queue one atomic-attach (`atomic_attach: true`) stream outcome — the
    /// kind prompt collectors and active-run probes open.
    pub fn push_stream(&self, script: StreamScript) {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .atomic_streams
            .push_back(script);
    }

    /// Queue one plain-subscribe (`atomic_attach: false`) stream outcome — the
    /// kind idle observers open. Lets observer retry/idle paths be scripted
    /// deterministically.
    pub fn push_plain_stream(&self, script: StreamScript) {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .plain_streams
            .push_back(script);
    }

    /// Business commands since the guard was taken. Readiness is recorded but
    /// queried separately through `requests_of`, since an in-flight diagnostic
    /// on the process-wide channel may finish after its original caller stops.
    pub fn requests(&self) -> Vec<RpcCommand> {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .requests
            .iter()
            .filter(|cmd| cmd.r#type != "get_agent_readiness")
            .cloned()
            .collect()
    }

    /// Recorded commands of one type, in arrival order.
    pub fn requests_of(&self, command_type: &str) -> Vec<RpcCommand> {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .requests
            .iter()
            .filter(|cmd| cmd.r#type == command_type)
            .cloned()
            .collect()
    }

    /// Every `stream_events` request the mock has seen since the guard.
    pub fn stream_requests(&self) -> Vec<StreamRequest> {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stream_requests
            .clone()
    }
}

/// A `StreamEvent` builder matching the fields the bridge reads.
pub fn stream_event(run_id: &str, idx: i64, event_type: &str, data: &str) -> StreamEvent {
    StreamEvent {
        r#type: event_type.to_string(),
        data: data.to_string(),
        run_id: run_id.to_string(),
        idx,
        ..Default::default()
    }
}

/// Arm one synthetic `SQLITE_BUSY` for the NEXT store initialization a
/// [`TestHome`] performs.
///
/// The retry arm in `TestHome::new` exists for a real condition — a task outside
/// the cancel registries still holding the freshly published `app.db`'s write
/// lock — and that condition has no deterministic seam from a test: it needs a
/// peer writer inside the ~µs window between `create_dir_all` and the first
/// `initialize_app_store`. This hook therefore fails the *first* attempt with
/// the exact error shape production produces (`SqliteFailure` with
/// `ErrorCode::DatabaseBusy`), which is what makes the retry body testable —
/// the retried call is the real one, against the real store.
///
/// `test_support` is compiled only under `#[cfg(test)]`, so this is not a
/// production seam.
static BUSY_FAULT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
fn arm_busy_fault() {
    BUSY_FAULT.store(true, Ordering::SeqCst);
}

/// `crate::store::initialize_app_store()` with the one-shot busy fault applied.
fn initialize_test_store() -> Result<(), crate::Error> {
    if BUSY_FAULT.swap(false, Ordering::SeqCst) {
        return Err(crate::Error::Database(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ffi::ErrorCode::DatabaseBusy,
                extended_code: 5,
            },
            Some("database is locked".to_string()),
        )));
    }
    crate::store::initialize_app_store()
}

/// HOME redirect + store initialization for tests that touch the SQLite
/// store. Holds the process-wide `TEST_HOME_LOCK` for its whole lifetime.
pub struct TestHome {
    _lock: MutexGuard<'static, ()>,
    prev_home: Option<String>,
    dir: PathBuf,
    cleanup: fn(),
    timing: crate::test_support::timing::FixtureTiming,
}

impl TestHome {
    pub fn new(tag: &str) -> Self {
        Self::with_cleanup(tag, cleanup_host)
    }

    pub fn with_cleanup(tag: &str, cleanup: fn()) -> Self {
        let mut timing = crate::test_support::timing::FixtureTiming::start("TestHome", tag);
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        timing.mark("lock_wait");
        // Process-lifetime workers and observers must be gone BEFORE publishing
        // the new HOME and creating its schema, not later when mock_agent is
        // acquired. Otherwise an old task can resolve this new HOME and lock
        // its SQLite database during migration.
        cleanup();
        timing.mark("pre_cleanup");
        let dir = std::env::temp_dir().join(format!(
            "futureos-bridge-test-{}-{}-{}",
            std::process::id(),
            tag,
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test home");
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &dir);
        timing.mark("home_setup");
        // The cancels above stop tracked tasks, but a task outside those
        // registries can still be mid-write on the freshly published HOME and
        // hold its lock for a moment (slow CI runners widen this window).
        // Every step of initialize_app_store is idempotent, so retry the
        // unlucky attempt instead of failing the test.
        let mut result = initialize_test_store();
        for _ in 0..4 {
            match result {
                Err(crate::Error::Database(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error {
                        code: rusqlite::ffi::ErrorCode::DatabaseBusy,
                        ..
                    },
                    _,
                ))) => {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    result = initialize_test_store();
                }
                _ => break,
            }
        }
        result.expect("initialize app store in test home");
        timing.mark("store_setup");
        Self {
            _lock: lock,
            prev_home,
            dir,
            cleanup,
            timing,
        }
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        self.timing.mark("body");
        (self.cleanup)();
        self.timing.mark("cleanup");
        match &self.prev_home {
            Some(prev) => std::env::set_var("HOME", prev),
            None => std::env::remove_var("HOME"),
        }
        // Release pooled connections first: Windows cannot unlink a file that
        // is still open, so deleting while the pool holds this home's database
        // leaves the directory behind on every run.
        crate::store::close_pool();
        let _ = std::fs::remove_dir_all(&self.dir);
        self.timing.finish();
    }
}

pub use crate::store::test_support::{
    seed_approval, seed_run, seed_thread, seed_titleable_thread, seed_workspace,
};

/// Point HOME at a regular FILE so every store call fails (its directories
/// can never be created) — covers best-effort persistence error arms. The
/// caller must hold the home lock (via [`TestHome`]); pair with
/// [`restore_home`]. Returns the previous HOME value.
pub fn break_home() -> Option<String> {
    let prev = std::env::var("HOME").ok();
    let file = std::env::temp_dir().join(format!("futureos-broken-home-{}", std::process::id()));
    std::fs::write(&file, "not a directory").expect("write broken-home file");
    std::env::set_var("HOME", &file);
    prev
}

/// Undo [`break_home`].
pub fn restore_home(prev: Option<String>) {
    if let Some(prev) = prev {
        std::env::set_var("HOME", prev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use future_rpc::proto::RpcCommand;

    /// A `Deferred` reply whose answering end is dropped mid-flight must surface
    /// as a **cancelled RPC**, not as a hang and not as a success with an empty
    /// body: a test that forgets to answer (or a fixture that panics before it
    /// does) has to fail the caller loudly, which is what this arm exists for.
    #[tokio::test]
    async fn a_deferred_reply_whose_answerer_disappears_cancels_the_rpc() {
        let _mock = mock_agent();
        let mut client = crate::agent_client::connect_agent()
            .await
            .expect("connect to the mock");
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel();
        _mock.push(
            "list_session_ids",
            Reply::Deferred {
                entered: entered_tx,
                data: answer_rx,
            },
        );
        // The answering end goes away without ever sending: exactly the state a
        // test leaves behind when it panics between `push` and `send`.
        drop(answer_tx);

        let error = client
            .execute_command(crate::agent_commands::list_session_ids_command())
            .await
            .expect_err("an unanswered deferred reply must cancel the RPC");

        assert_eq!(error.code(), tonic::Code::Cancelled, "{error}");
        assert_eq!(error.message(), "test reply dropped");
        // The mock did reach the deferred branch, so the arm was exercised and
        // not merely unreachable behind an earlier failure.
        entered_rx
            .await
            .expect("the mock entered the deferred reply");
    }

    /// `default_reply_data` echoes the requested run id when present and
    /// falls back to `mock-run` when a prompt carries an empty one (headless
    /// callers that let the pipeline pick the id still get a consistent
    /// acknowledgement).
    #[test]
    fn default_reply_falls_back_to_mock_run_for_empty_requested_id() {
        let with_id = RpcCommand {
            r#type: "prompt".to_string(),
            requested_run_id: "real-run".to_string(),
            ..Default::default()
        };
        assert!(default_reply_data(&with_id).contains("real-run"));

        let empty_id = RpcCommand {
            r#type: "prompt".to_string(),
            requested_run_id: String::new(),
            ..Default::default()
        };
        assert!(default_reply_data(&empty_id).contains("mock-run"));
    }

    /// `TestHome::drop` takes the `None` arm (removes HOME) when no HOME was
    /// set before the fixture was created.
    ///
    /// `HOME` is process-global, so *both* ends of this test take `TEST_HOME_LOCK`:
    /// reading the previous value while another test's `TestHome` is published would
    /// capture that fixture's directory, and restoring it after the lock is gone
    /// would publish a path its owner has already deleted — the next
    /// `HOME`-reading test (a terminal cwd fallback, say) then resolves a directory
    /// that no longer exists. `TestHome::new` takes the same non-reentrant lock
    /// itself, so the guard is only held around the two raw `HOME` touches.
    #[test]
    fn test_home_drop_removes_home_when_unset_before() {
        // The arm under test is the one for a process that never had a HOME (a
        // bare `cargo test`, without this campaign's HOME override). This suite
        // always has one, so the precondition is produced here — under the same
        // lock — and the real value is restored after the arm has run.
        let real_home = std::env::var("HOME").ok();
        let saved = {
            let _lock = crate::TEST_HOME_LOCK
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            std::env::remove_var("HOME");
            std::env::var("HOME").ok()
        };
        let mut home = TestHome::new("no-prev-home");
        home.prev_home = None;
        drop(home);
        let _lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match saved {
            Some(saved) => std::env::set_var("HOME", saved),
            None => std::env::remove_var("HOME"),
        }
        if let Some(real_home) = real_home {
            std::env::set_var("HOME", real_home);
        }
    }

    /// `TestHome::new` retries a `SQLITE_BUSY` from the store initialization and
    /// keeps going, instead of failing the whole test on the first busy attempt.
    ///
    /// The condition is a real one (a task outside the cancel registries holding
    /// the freshly published `app.db`), but it has no deterministic seam, so the
    /// first attempt is failed with the exact production error shape by
    /// [`arm_busy_fault`]; the retry then runs against the real store, and the
    /// assertions below prove the retried initialization actually succeeded.
    #[test]
    fn test_home_retries_a_busy_store_initialization() {
        arm_busy_fault();
        let home = TestHome::new("busy-retry");
        assert!(
            !BUSY_FAULT.load(Ordering::SeqCst),
            "the armed fault is consumed by the first attempt"
        );
        // The retry's own initialization is what left a usable store behind.
        assert!(
            crate::store::get_app_settings().is_ok(),
            "the retried initialization created the store"
        );
        assert!(
            home.path().exists(),
            "the test home survives the retry: {}",
            home.path().display()
        );
    }
}
