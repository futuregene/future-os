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
use tokio_stream::StreamExt;

/// O5: scripted behavior for ONE `stream_events` attach (consumed in order;
/// the first entry scripts the first attach, and so on).
#[derive(Clone, Copy, Debug)]
pub enum AttachPlan {
    /// Serve the matching events (idx > after_idx), then terminate with a
    /// `DataLoss` "event stream gap" error after `n` served events.
    GapAfter(usize),
    /// Like `GapAfter`, but with a non-recoverable `internal` status (no
    /// retry path).
    HardErrorAfter(usize),
    /// Serve all matching events and close cleanly.
    Complete,
}

#[derive(Default)]
pub struct MockState {
    pub sessions_created: u64,
    pub prompts: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
    /// Live session ids (added by new_session, removed by delete_session) —
    /// lets `session_alive` probe distinguish a live session from a missing one.
    pub live_sessions: HashSet<String>,
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
    /// Replays the scripted `events` first, then never yields — the
    /// budget-truncation path WITH observed tool starts (O3 idle detection).
    pub events_then_hang: bool,
    /// stream_events fails immediately with a tonic status.
    pub stream_error: bool,
    /// After N scripted events the stream yields one tonic error (mid-stream
    /// failure path in run_turn).
    pub stream_fail_after: Option<usize>,
    /// O5: per-attach stream script for gap-recovery tests. Empty = the
    /// legacy single-shot knobs above.
    pub stream_attach_plan: Vec<AttachPlan>,
    /// O5: `after_idx` values seen by stream_events, in attach order.
    pub attach_after_idx: Vec<i64>,
    /// Per-command raw `data` payload override (valid JSON or not).
    pub raw: HashMap<String, String>,
    pub models_payload: Option<String>,
    /// Command types seen, in order (for assertions).
    pub recorded: Vec<String>,
    /// `name` field of every new_session command seen (wire-level title).
    pub new_session_names: Vec<String>,
    /// (session_id, busy_policy) of every prompt command, in order.
    pub prompt_calls: Vec<(String, String)>,
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
                st.new_session_names.push(cmd.name.clone());
                let id = format!("mock-session-{}", st.sessions_created);
                st.live_sessions.insert(id.clone());
                format!("{{\"sessionId\":\"{}\"}}", id)
            }
            "get_state" => {
                // A session that was never created (or was deleted) probes
                // dead — mirror the agent's real get_state behavior so
                // `session_alive` has a meaningful false path. Only enforced
                // once at least one session has been created via new_session
                // (tests that drive `execute_turn` directly with a hardcoded
                // session id predate session tracking and never create one).
                if st.sessions_created > 0 && !st.live_sessions.contains(&cmd.session_id) {
                    return Ok(response(
                        &cmd,
                        false,
                        String::new(),
                        "session not found".to_string(),
                    ));
                }
                format!(
                    "{{\"tokensIn\":{},\"tokensOut\":{},\"totalCost\":{}}}",
                    st.tokens_in, st.tokens_out, st.cost
                )
            }
            "delete_session" => {
                st.live_sessions.remove(&cmd.session_id);
                String::new()
            }
            "prompt" => {
                st.prompts += 1;
                st.prompt_calls
                    .push((cmd.session_id.clone(), cmd.busy_policy.clone()));
                format!("{{\"run_id\":\"mock-run-{}\"}}", st.prompts)
            }
            "list_models" => st.models_payload.clone().unwrap_or_else(|| {
                "{\"models\":[{\"id\":\"k3\",\"provider\":\"future\",\"label\":\"K3\",\
                 \"thinkingLevel\":\"xhigh\",\"contextWindow\":256000,\"recommended\":true,\
                 \"isDefault\":true},{\"id\":\"plain\"}],\"defaultModel\":\"future/k3\"}"
                    .to_string()
            }),
            // Trivial acks (set_model / set_thinking_level / append_system_prompt /
            // delete_session / abort): empty data → Value::Null path.
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
        let req = request.into_inner();
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.attach_after_idx.push(req.after_idx);
        if !st.stream_attach_plan.is_empty() {
            let attach_no = st.attach_after_idx.len();
            let plan = st.stream_attach_plan.remove(0);
            // Like the real agent's atomic attach: replay only events after
            // the client's cursor.
            let matching: Vec<StreamEvent> = st
                .events
                .iter()
                .filter(|e| e.idx > req.after_idx)
                .cloned()
                .collect();
            let mut items: Vec<Result<StreamEvent, tonic::Status>> = Vec::new();
            match plan {
                AttachPlan::Complete => {
                    items.extend(matching.into_iter().map(Ok));
                }
                AttachPlan::GapAfter(n) => {
                    items.extend(matching.into_iter().take(n).map(Ok));
                    items.push(Err(tonic::Status::data_loss(format!(
                        "event stream gap for session sess, run mock; reconnect with \
                         atomic attach (mock gap #{attach_no})"
                    ))));
                }
                AttachPlan::HardErrorAfter(n) => {
                    items.extend(matching.into_iter().take(n).map(Ok));
                    items.push(Err(tonic::Status::internal(format!(
                        "mock hard failure #{attach_no}"
                    ))));
                }
            }
            return Ok(tonic::Response::new(Box::pin(tokio_stream::iter(items))));
        }
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
        if st.events_then_hang {
            let scripted = tokio_stream::iter(items);
            let pending = tokio_stream::pending::<Result<StreamEvent, tonic::Status>>();
            return Ok(tonic::Response::new(Box::pin(scripted.chain(pending))));
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
