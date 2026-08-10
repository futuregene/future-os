//! In-process mock of the FutureAgent gRPC service (ExecuteCommand +
//! StreamEvents — the only two RPCs the loop control plane consumes).
//!
//! The mock is scripted through `MockState`: per-command failures, invalid
//! JSON payloads, a never-yielding event stream (for wall-clock timeout
//! paths), and a scripted event list for `run_turn`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use future_rpc::proto::future_agent_server::{FutureAgent, FutureAgentServer};
use future_rpc::proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};

#[derive(Default)]
pub struct MockState {
    pub sessions_created: u64,
    pub prompts: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
    /// Events replayed by stream_events (any run).
    pub events: Vec<StreamEvent>,
    /// Command types that answer success=false with a mock error.
    pub fail_commands: HashSet<String>,
    /// Command types that answer success=true with a non-JSON payload.
    pub invalid_json: HashSet<String>,
    /// ExecuteCommand fails at the transport level (tonic::Status error).
    pub grpc_error: bool,
    /// stream_events returns a stream that never yields (timeout tests).
    pub hang_stream: bool,
    /// stream_events fails immediately with a tonic status.
    pub stream_error: bool,
    /// After N scripted events the stream yields one tonic error (mid-stream
    /// failure path in run_turn).
    pub stream_fail_after: Option<usize>,
    /// Per-command raw `data` payload override (valid JSON or not).
    pub raw: HashMap<String, String>,
    pub models_payload: Option<String>,
    /// Command types seen, in order (for assertions).
    pub recorded: Vec<String>,
}

impl MockState {
    pub fn fail(cmd: &str) -> Self {
        let mut s = Self::default();
        s.fail_commands.insert(cmd.to_string());
        s
    }
}

pub type SharedState = Arc<Mutex<MockState>>;

pub struct MockAgent {
    state: SharedState,
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
        error_code: if success {
            String::new()
        } else {
            "mock_error".to_string()
        },
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
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.recorded.push(cmd.r#type.clone());
        if st.grpc_error {
            return Err(tonic::Status::unavailable("mock transport failure"));
        }
        if st.fail_commands.contains(&cmd.r#type) {
            return Ok(response(
                &cmd,
                false,
                String::new(),
                format!("mock failure for {}", cmd.r#type),
            ));
        }
        if st.invalid_json.contains(&cmd.r#type) {
            return Ok(response(&cmd, true, "{not-json".to_string(), String::new()));
        }
        if let Some(raw) = st.raw.get(&cmd.r#type) {
            return Ok(response(&cmd, true, raw.clone(), String::new()));
        }
        let data = match cmd.r#type.as_str() {
            "new_session" => {
                st.sessions_created += 1;
                format!("{{\"sessionId\":\"mock-session-{}\"}}", st.sessions_created)
            }
            "get_state" => format!(
                "{{\"tokensIn\":{},\"tokensOut\":{},\"totalCost\":{}}}",
                st.tokens_in, st.tokens_out, st.cost
            ),
            "prompt" => {
                st.prompts += 1;
                format!("{{\"run_id\":\"mock-run-{}\"}}", st.prompts)
            }
            "list_models" => st.models_payload.clone().unwrap_or_else(|| {
                "{\"models\":[{\"id\":\"k3\",\"provider\":\"future\",\"label\":\"K3\",\
                 \"thinkingLevel\":\"xhigh\",\"contextWindow\":256000,\"recommended\":true,\
                 \"isDefault\":true},{\"id\":\"plain\"}],\"defaultModel\":\"future/k3\"}"
                    .to_string()
            }),
            // Trivial acks (set_model / set_thinking_level / append_system_prompt /
            // steer / delete_session / abort): empty data → Value::Null path.
            _ => String::new(),
        };
        Ok(response(&cmd, true, data, String::new()))
    }

    type StreamEventsStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<StreamEvent, tonic::Status>> + Send>,
    >;

    async fn stream_events(
        &self,
        request: tonic::Request<StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        let _ = request.into_inner();
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.stream_error {
            return Err(tonic::Status::internal("mock stream attach failure"));
        }
        if st.hang_stream {
            return Ok(tonic::Response::new(Box::pin(tokio_stream::pending())));
        }
        let mut items: Vec<Result<StreamEvent, tonic::Status>> =
            st.events.iter().cloned().map(Ok).collect();
        if let Some(n) = st.stream_fail_after {
            if items.len() >= n {
                items.insert(n, Err(tonic::Status::data_loss("mock mid-stream failure")));
            }
        }
        Ok(tonic::Response::new(Box::pin(tokio_stream::iter(items))))
    }
}

/// A scripted event for the mock stream. `data` is the JSON payload string.
pub fn ev(run_id: &str, idx: i64, kind: &str, data: &str) -> StreamEvent {
    StreamEvent {
        r#type: kind.to_string(),
        data: data.to_string(),
        run_id: run_id.to_string(),
        idx,
        ..Default::default()
    }
}

/// The standard successful run: one tool, some text, agent_end completed.
pub fn completed_events(run_id: &str) -> Vec<StreamEvent> {
    vec![
        ev(run_id, 0, "agent_start", "{}"),
        ev(run_id, 1, "tool_start", "{\"tool_name\":\"shell\"}"),
        ev(run_id, 2, "text_chunk", "{\"text\":\"artifact written\"}"),
        ev(
            run_id,
            3,
            "agent_end",
            "{\"state\":\"completed\",\"usage\":{\"output_tokens\":5},\"duration_ms\":7}",
        ),
    ]
}

/// Serve the mock on an ephemeral port. Returns (addr, shared state).
pub async fn spawn_mock(state: MockState) -> (String, SharedState) {
    spawn_mock_on("127.0.0.1:0", state)
        .await
        .expect("ephemeral bind always works")
}

/// Serve the mock on a fixed address; None when the port is unavailable
/// (e.g. a real agent already holds 127.0.0.1:50051 — tests must skip then,
/// never talk to the real agent).
pub async fn spawn_mock_on(addr: &str, state: MockState) -> Option<(String, SharedState)> {
    let listener = tokio::net::TcpListener::bind(addr).await.ok()?;
    let local = listener.local_addr().ok()?.to_string();
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
    Some((local, shared))
}
