//! Process-wide mock Future Agent gRPC server for tests.
//!
//! The real agent channel (`agent_bridge::client::connect_agent`) latches a
//! process-lifetime `OnceCell`, so every test that needs "the agent" must share
//! one server. It is therefore:
//!
//! - started lazily on first use and leaked for the rest of the process;
//! - scriptable through [`MockScript`] — including a `Down` mode that answers
//!   every RPC with `Code::Unavailable`, which clients map to
//!   `AppError::AgentUnavailable` exactly as a dead agent does, so tests see
//!   identical behavior whether the mock pre-dates their first connect or not;
//! - serialized through [`mock_agent_lock`] while a test mutates the script.
//!
//! Unknown commands always answer `Unavailable`, so a test that accidentally
//! reaches the mock observes the same fallback behavior as with no agent.

use std::sync::{Mutex, MutexGuard, OnceLock};

use tonic::transport::Server;
use tonic::Code;

use crate::agent_proto::future_agent_server::{FutureAgent, FutureAgentServer};
use crate::agent_proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};

/// What the mock answers. `down` answers every command *except* the
/// connect-time health check with `Code::Unavailable` — indistinguishable from
/// a dead agent for callers (`AppError::AgentUnavailable`), while still
/// letting `connect_agent` latch its channel, so test outcomes don't depend on
/// execution order.
#[derive(Clone, Default)]
pub(crate) struct MockScript {
    /// Every command but the health check answers `Unavailable`.
    pub down: bool,
    /// `list_session_ids` returns `success = false` (enumeration failed).
    pub fail_list_session_ids: bool,
    /// The ids a successful `list_session_ids` returns.
    pub session_ids: Vec<String>,
}

static SCRIPT: Mutex<MockScript> = Mutex::new(MockScript {
    down: false,
    fail_list_session_ids: false,
    session_ids: Vec::new(),
});

static MOCK_LOCK: Mutex<()> = Mutex::new(());

/// Serialize tests that re-script the shared mock.
pub(crate) fn mock_agent_lock() -> MutexGuard<'static, ()> {
    MOCK_LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Point the mock at a new script. Caller must hold [`mock_agent_lock`].
pub(crate) fn script_mock_agent(script: MockScript) {
    *SCRIPT.lock().unwrap_or_else(|poison| poison.into_inner()) = script;
}

/// The current script, cloned out from under the lock (sync helper — the
/// async_trait method below gets mangled region spans, which strands the
/// poison-recovery closure's zero-count region on the lock line).
fn current_script() -> MockScript {
    SCRIPT
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

struct MockAgent;

#[tonic::async_trait]
impl FutureAgent for MockAgent {
    async fn execute_command(
        &self,
        request: tonic::Request<RpcCommand>,
    ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
        let command = request.into_inner();
        let script = current_script();
        // The health check stays up even in down mode (see MockScript docs).
        if script.down && command.r#type != "list_streaming_sessions" {
            return Err(tonic::Status::new(Code::Unavailable, "mock agent is down"));
        }
        let response = match command.r#type.as_str() {
            // The connect-time health check: any Ok response passes it.
            "list_streaming_sessions" => RpcResponse {
                success: true,
                ..Default::default()
            },
            "list_session_ids" if script.fail_list_session_ids => RpcResponse {
                success: false,
                error: "mock enumeration failure".to_string(),
                ..Default::default()
            },
            "list_session_ids" => RpcResponse {
                success: true,
                data: serde_json::json!({ "ids": script.session_ids }).to_string(),
                ..Default::default()
            },
            _ => {
                return Err(tonic::Status::new(
                    Code::Unavailable,
                    "mock agent does not serve this command",
                ))
            }
        };
        Ok(tonic::Response::new(response))
    }

    type StreamEventsStream = futures::stream::Empty<Result<StreamEvent, tonic::Status>>;

    async fn stream_events(
        &self,
        _request: tonic::Request<StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        Err(tonic::Status::new(
            Code::Unimplemented,
            "mock agent has no event stream",
        ))
    }
}

/// Start the shared mock (idempotent) and point `FUTURE_AGENT_GRPC_ADDR` at it.
/// The server runs on its own thread/runtime for the rest of the process; the
/// environment override stays set so the latched agent channel always targets
/// the mock.
pub(crate) fn ensure_mock_agent() {
    static START: OnceLock<()> = OnceLock::new();
    START.get_or_init(|| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock agent");
        let port = listener.local_addr().expect("mock agent addr").port();
        listener
            .set_nonblocking(true)
            .expect("mock agent listener nonblocking");
        std::thread::spawn(move || {
            // Leaked: the runtime's workers keep driving the server for the
            // rest of the process (the mock is process-wide by design).
            let runtime = Box::leak(Box::new(
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("mock agent runtime"),
            ));
            runtime.spawn(async move {
                let listener =
                    tokio::net::TcpListener::from_std(listener).expect("mock agent listener");
                let incoming = tonic::codegen::tokio_stream::wrappers::TcpListenerStream::new(
                    listener,
                );
                let service = FutureAgentServer::new(MockAgent);
                let server = Server::builder().add_service(service);
                // Polled once (starts listening), never completes — by design.
                server.serve_with_incoming(incoming).await.expect("serve mock")
            });
        });
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", format!("127.0.0.1:{port}"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bare command with just a type — all the mock reads.
    fn typed_command(r#type: &str) -> RpcCommand {
        RpcCommand {
            r#type: r#type.to_string(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn mock_answers_the_scripted_commands() {
        let _lock = mock_agent_lock();
        ensure_mock_agent();
        script_mock_agent(MockScript {
            down: false,
            fail_list_session_ids: false,
            session_ids: vec!["sess_a".to_string()],
        });

        let mut client = crate::agent_bridge::connect_agent()
            .await
            .expect("connect to mock agent");
        let response = client
            .execute_command(typed_command("list_session_ids"))
            .await
            .expect("list_session_ids")
            .into_inner();
        assert!(response.success);
        assert!(response.data.contains("sess_a"));

        // Unknown commands surface as Unavailable → AgentUnavailable downstream.
        let unknown = client.execute_command(typed_command("get_state")).await;
        assert_eq!(unknown.expect_err("unknown command").code(), Code::Unavailable);

        // The stream RPC is unimplemented by the mock.
        let stream = client.stream_events(StreamRequest::default()).await;
        assert_eq!(stream.expect_err("no stream").code(), Code::Unimplemented);

        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn mock_down_mode_is_indistinguishable_from_a_dead_agent() {
        let _lock = mock_agent_lock();
        ensure_mock_agent();
        // Latch the process-wide agent channel while the mock is up (the
        // health check succeeds even in down mode, so this cannot fail once
        // the mock is started).
        script_mock_agent(MockScript::default());
        let mut client = crate::agent_bridge::connect_agent()
            .await
            .expect("connect to mock agent");
        // …then take it down: RPCs on the latched channel fail Unavailable,
        // exactly like a dead agent.
        script_mock_agent(MockScript {
            down: true,
            ..Default::default()
        });

        let first = client.execute_command(typed_command("list_session_ids")).await;
        assert_eq!(first.expect_err("down mock").code(), Code::Unavailable);

        // A fresh connect reuses the latched channel — same failure.
        let mut again = crate::agent_bridge::connect_agent()
            .await
            .expect("latched channel");
        let second = again.execute_command(typed_command("list_session_ids")).await;
        assert_eq!(second.expect_err("down mock").code(), Code::Unavailable);

        script_mock_agent(MockScript::default());
    }
}
