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

use crate::agent_proto::future_agent_server::{FutureAgent, FutureAgentServer};
use crate::agent_proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};

/// One scripted `execute_command` reply, consumed FIFO per command type.
pub(crate) enum Reply {
    /// `success = true` with the given JSON `data` string.
    Data(String),
    /// `success = false` with this error message.
    Reject(String),
    /// Transport-level failure (tonic status).
    Status(tonic::Code, &'static str),
}

/// One scripted `stream_events` outcome.
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
    replies: HashMap<String, VecDeque<Reply>>,
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
        let cmd = request.into_inner();
        let reply = {
            let mut state = STATE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.requests.push(cmd.clone());
            // get_run_state (get_state with a run id) has its own queues keyed
            // by run id: spawned session observers fire plain get_state probes
            // concurrently, and a shared FIFO would let them steal run-scoped
            // replies.
            let key = if cmd.r#type == "get_state" && !cmd.run_id.is_empty() {
                format!("get_state#{}", cmd.run_id)
            } else {
                cmd.r#type.clone()
            };
            state
                .replies
                .get_mut(&key)
                .and_then(VecDeque::pop_front)
                .unwrap_or_else(|| Reply::Data(default_reply_data(&cmd)))
        };
        match reply {
            Reply::Data(data) => Ok(tonic::Response::new(rpc_response(
                &cmd,
                true,
                data,
                String::new(),
            ))),
            Reply::Reject(error) => Ok(tonic::Response::new(rpc_response(
                &cmd,
                false,
                String::new(),
                error,
            ))),
            Reply::Status(code, message) => Err(tonic::Status::new(code, message)),
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
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", addr.to_string());
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
                    .block_on(super::client::connect_agent())
                    .expect("warmup connect to mock agent");
            })
            .expect("spawn warmup thread")
            .join()
            .expect("warmup thread");
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
        state.atomic_streams.clear();
        state.plain_streams.clear();
        state.requests.clear();
        state.stream_requests.clear();
    }
    MockAgentGuard { _lock: lock }
}

impl MockAgentGuard {
    /// Queue a reply for one command type (FIFO within the type).
    pub(crate) fn push(&self, command_type: &str, reply: Reply) {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replies
            .entry(command_type.to_string())
            .or_default()
            .push_back(reply);
    }

    /// Queue a successful reply whose `data` is the JSON rendering of `value`.
    pub(crate) fn push_data(&self, command_type: &str, value: serde_json::Value) {
        self.push(command_type, Reply::Data(value.to_string()));
    }

    /// Queue a `get_run_state` reply (get_state carrying a run id) for one
    /// canonical run id.
    pub(crate) fn push_run_state(&self, run_id: &str, value: serde_json::Value) {
        self.push(
            &format!("get_state#{run_id}"),
            Reply::Data(value.to_string()),
        );
    }

    /// Queue one atomic-attach (`atomic_attach: true`) stream outcome — the
    /// kind prompt collectors and active-run probes open.
    pub(crate) fn push_stream(&self, script: StreamScript) {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .atomic_streams
            .push_back(script);
    }

    /// Queue one plain (`atomic_attach: false`) stream outcome — the kind idle
    /// observers open.
    pub(crate) fn push_plain_stream(&self, script: StreamScript) {
        STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .plain_streams
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
        let lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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

/// Create a pending approval row for the run.
pub(crate) fn seed_approval(
    approval_request_id: &str,
    run_id: &str,
) -> crate::store::ApprovalRequestRecord {
    crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
        approval_request_id: Some(approval_request_id.to_string()),
        run_id: run_id.to_string(),
        tool_call_id: None,
        kind: "tool".to_string(),
        title: "Approve `tool`".to_string(),
        summary: None,
        risk_level: None,
        requested_action: None,
        action_category: None,
        action_payload: None,
        sandbox_boundary: None,
        save_suggestion: None,
        reviewer: None,
    })
    .expect("ensure approval");
    crate::store::get_approval_request(approval_request_id)
        .expect("approval query")
        .expect("approval exists")
}

/// Point HOME at a regular FILE so every store call fails (its directories
/// can never be created) — covers best-effort persistence error arms. The
/// caller must hold the home lock (via [`TestHome`]); pair with
/// [`restore_home`]. Returns the previous HOME value.
pub(crate) fn break_home() -> Option<String> {
    let prev = std::env::var("HOME").ok();
    let file = std::env::temp_dir().join(format!("futureos-broken-home-{}", std::process::id()));
    std::fs::write(&file, "not a directory").expect("write broken-home file");
    std::env::set_var("HOME", &file);
    prev
}

/// Undo [`break_home`].
pub(crate) fn restore_home(prev: Option<String>) {
    if let Some(prev) = prev {
        std::env::set_var("HOME", prev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_proto::RpcCommand;

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
    #[test]
    fn test_home_drop_removes_home_when_unset_before() {
        let saved = std::env::var("HOME").ok();
        let mut home = TestHome::new("no-prev-home");
        home.prev_home = None;
        drop(home);
        if let Some(saved) = saved {
            std::env::set_var("HOME", saved);
        }
    }
}
