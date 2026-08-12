//! Test-only support: a scripted in-process mock of the FutureAgent gRPC
//! service plus HOME-isolated store setup.
//!
//! `connect_agent` caches one process-global channel keyed off
//! `FUTURE_AGENT_GRPC_ADDR`, so the whole test binary shares a single mock
//! server (started lazily on first use). Tests that drive the agent channel
//! hold [`mock_agent`]'s guard for their whole duration — it serializes them
//! and resets the script, so per-test expectations stay deterministic.
//!
//! Tests that touch the SQLite store additionally hold [`TestHome`], which
//! redirects `HOME` (the store resolves its db path per call and the
//! connection pool re-keys on path change). Acquire `TestHome` FIRST and the
//! mock guard second everywhere to keep lock ordering consistent.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use futures::StreamExt;

use crate::agent_proto::future_agent_server::{FutureAgent, FutureAgentServer};
use crate::agent_proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};

/// One scripted `execute_command` reply, consumed FIFO.
pub(crate) enum Reply {
    /// `success = true` with the given JSON `data` string.
    Data(String),
    /// `success = false` with this error message.
    Reject(String),
    /// Transport-level failure (tonic status).
    Status(tonic::Code, &'static str),
}

/// One scripted `stream_events` outcome, consumed FIFO.
pub(crate) enum StreamScript {
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
    replies: VecDeque<(Option<String>, Reply)>,
    streams: VecDeque<StreamScript>,
    requests: Vec<RpcCommand>,
    stream_requests: Vec<StreamRequest>,
}

static STATE: Mutex<MockState> = Mutex::new(MockState {
    replies: VecDeque::new(),
    streams: VecDeque::new(),
    requests: Vec::new(),
    stream_requests: Vec::new(),
});

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

#[tonic::async_trait]
impl FutureAgent for MockAgent {
    async fn execute_command(
        &self,
        request: tonic::Request<RpcCommand>,
    ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
        let cmd = request.into_inner();
        let reply = {
            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            state.requests.push(cmd.clone());
            match state.replies.pop_front() {
                Some((expect, reply)) => {
                    if let Some(expect) = expect {
                        if cmd.r#type != expect {
                            return Err(tonic::Status::internal(format!(
                                "mock script mismatch: expected {expect}, got {}",
                                cmd.r#type
                            )));
                        }
                    }
                    reply
                }
                // Unscripted commands (e.g. the connect health check) get a
                // generic success so first-touch channel init always works.
                None => Reply::Data("{}".to_string()),
            }
        };
        match reply {
            Reply::Data(data) => Ok(tonic::Response::new(rpc_response(&cmd, true, data, String::new()))),
            Reply::Reject(error) => {
                Ok(tonic::Response::new(rpc_response(&cmd, false, String::new(), error)))
            }
            Reply::Status(code, message) => Err(tonic::Status::new(code, message)),
        }
    }

    type StreamEventsStream = std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<StreamEvent, tonic::Status>> + Send>,
    >;

    async fn stream_events(
        &self,
        request: tonic::Request<StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        let req = request.into_inner();
        let script = {
            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            state.stream_requests.push(req);
            state.streams.pop_front()
        };
        match script {
            Some(StreamScript::AttachError(code, message)) => {
                Err(tonic::Status::new(code, message))
            }
            Some(StreamScript::Hang) | None => {
                Ok(tonic::Response::new(Box::pin(futures::stream::pending())))
            }
            Some(StreamScript::Events(events, terminal)) => {
                let mut items: Vec<Result<StreamEvent, tonic::Status>> =
                    events.into_iter().map(Ok).collect();
                if let Some((code, message)) = terminal {
                    items.push(Err(tonic::Status::new(code, message)));
                }
                Ok(tonic::Response::new(Box::pin(futures::stream::iter(items))))
            }
        }
    }
}

fn ensure_mock_server() {
    static START: OnceLock<()> = OnceLock::new();
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
                    let incoming = futures::stream::poll_fn(move |cx| {
                        match listener.poll_accept(cx) {
                            std::task::Poll::Ready(result) => std::task::Poll::Ready(Some(
                                result.map(|(stream, _)| stream),
                            )),
                            std::task::Poll::Pending => std::task::Poll::Pending,
                        }
                    });
                    tonic::transport::Server::builder()
                        .add_service(FutureAgentServer::new(MockAgent))
                        .serve_with_incoming(incoming)
                        .await
                        .expect("serve mock agent");
                });
            })
            .expect("spawn mock agent thread");
        let addr = addr_rx.recv().expect("mock agent address");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", addr.to_string());
    });
}

static MOCK_LOCK: Mutex<()> = Mutex::new(());

/// Guard serializing every test that drives the shared agent channel. On
/// acquisition it (lazily) starts the mock server, points
/// `FUTURE_AGENT_GRPC_ADDR` at it, and resets the script + request log.
pub(crate) struct MockAgentGuard {
    _lock: MutexGuard<'static, ()>,
}

pub(crate) fn mock_agent() -> MockAgentGuard {
    let lock = MOCK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ensure_mock_server();
    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.replies.clear();
        state.streams.clear();
        state.requests.clear();
        state.stream_requests.clear();
    }
    MockAgentGuard { _lock: lock }
}

impl MockAgentGuard {
    /// Queue a reply, asserting the command type when `expect_type` is set.
    pub(crate) fn push(&self, expect_type: Option<&str>, reply: Reply) {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replies
            .push_back((expect_type.map(str::to_string), reply));
    }

    /// Queue a successful reply whose `data` is the JSON rendering of `value`.
    pub(crate) fn push_data(&self, expect_type: &str, value: serde_json::Value) {
        self.push(Some(expect_type), Reply::Data(value.to_string()));
    }

    /// Queue one `stream_events` outcome.
    pub(crate) fn push_stream(&self, script: StreamScript) {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .streams
            .push_back(script);
    }

    /// Every `execute_command` the mock has seen since the guard was taken.
    pub(crate) fn requests(&self) -> Vec<RpcCommand> {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .requests
            .clone()
    }

    /// Recorded commands of one type, in arrival order.
    pub(crate) fn requests_of(&self, command_type: &str) -> Vec<RpcCommand> {
        self.requests()
            .into_iter()
            .filter(|cmd| cmd.r#type == command_type)
            .collect()
    }

    /// Every `stream_events` request the mock has seen since the guard.
    pub(crate) fn stream_requests(&self) -> Vec<StreamRequest> {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stream_requests
            .clone()
    }
}

/// A `StreamEvent` builder matching the fields the bridge reads.
pub(crate) fn stream_event(run_id: &str, idx: i64, event_type: &str, data: &str) -> StreamEvent {
    StreamEvent {
        r#type: event_type.to_string(),
        data: data.to_string(),
        run_id: run_id.to_string(),
        idx,
        ..Default::default()
    }
}

/// HOME redirect + store initialization for tests that touch the SQLite
/// store. Holds the process-wide `TEST_HOME_LOCK` for its whole lifetime.
pub(crate) struct TestHome {
    _lock: MutexGuard<'static, ()>,
    prev_home: Option<String>,
    dir: PathBuf,
}

impl TestHome {
    pub(crate) fn new(tag: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let lock = crate::TEST_HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        crate::store::initialize_app_store().expect("initialize app store in test home");
        Self {
            _lock: lock,
            prev_home,
            dir,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        match &self.prev_home {
            Some(prev) => std::env::set_var("HOME", prev),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Create a real on-disk workspace row rooted under `dir` (created on disk so
/// `is_dir()` checks pass).
pub(crate) fn seed_workspace(dir: &Path, name: &str) -> crate::store::WorkspaceRecord {
    let path = dir.join(name);
    std::fs::create_dir_all(&path).expect("create workspace dir");
    crate::store::create_workspace(crate::store::CreateWorkspaceInput {
        name: Some(name.to_string()),
        path: path.display().to_string(),
        description: None,
        create_directory: Some(false),
    })
    .expect("create workspace")
}

/// Create a workspace-mode thread bound to `agent_session_id`.
pub(crate) fn seed_thread(
    workspace_id: &str,
    agent_session_id: Option<&str>,
) -> crate::store::ThreadRecord {
    crate::store::create_thread(crate::store::CreateThreadInput {
        mode: "workspace".to_string(),
        title: Some("test thread".to_string()),
        workspace_id: Some(workspace_id.to_string()),
        workspace_path: None,
        workspace_name: None,
        agent_session_id: agent_session_id.map(str::to_string),
    })
    .expect("create thread")
}

/// Create a (running) run row for the thread.
pub(crate) fn seed_run(thread_id: &str) -> crate::store::RunRecord {
    crate::store::create_run(crate::store::CreateRunInput {
        id: None,
        thread_id: thread_id.to_string(),
        trigger_message_id: None,
        model_provider: None,
        model_id: None,
    })
    .expect("create run")
}
