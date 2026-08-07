//! Deterministic mock `FutureAgent` gRPC server for the tmux screen-consistency
//! tests (P4). Serves fixed `get_state` / `list_models` / `prompt` responses
//! and streams a fixed reply event sequence on `prompt`, so the TypeScript TUI
//! and the Rust TUI render byte-identical screens.
//!
//! Usage:
//!   cargo build -p future-tui --example mock_agent
//!   target/debug/examples/mock_agent --port 50051
//!
//! The harness (tests/tmux-diff.sh) starts one instance per TUI on different
//! ports; both instances are deterministic and identical.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream;
use futures_util::Stream;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use future_rpc::proto::future_agent_server::{FutureAgent, FutureAgentServer};
use future_rpc::proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};

// ─── Deterministic payloads (identical on the wire for both TUIs) ───────────

/// Fixed session state (`get_state` data). CamelCase keys per
/// `RpcSessionState` (`serde(rename_all = "camelCase")`).
const STATE_JSON: &str = r#"{
  "agentInstanceId": "mock-agent-instance",
  "model": "mock-model",
  "thinkingLevel": "off",
  "isStreaming": false,
  "isCompacting": false,
  "sessionFile": "/mock/session.jsonl",
  "sessionId": "mock-session-1",
  "sessionName": "Mock Session One",
  "explicitSession": false,
  "autoCompactionEnabled": false,
  "queryCount": 3,
  "version": "0.0.0-mock",
  "cwd": "/mock",
  "permissionLevel": "all",
  "skills": ["future-web"],
  "contextFiles": [],
  "extensions": [],
  "contextTokens": 1234,
  "contextWindow": 128000,
  "contextPercent": 1.0,
  "tokensIn": 100,
  "tokensOut": 50,
  "tokensCacheR": 0,
  "tokensCacheW": 0,
  "totalCost": 0.0012,
  "activeRun": null,
  "queuedRuns": []
}"#;

/// Fixed model list (`list_models` / `get_available_models` data).
const MODELS_JSON: &str = r#"{
  "models": [
    {"id": "mock-model", "label": "Mock Model", "provider": "future", "supportsImages": true, "thinkingLevel": "off", "contextWindow": 128000, "isDefault": true},
    {"id": "mock-pro", "label": "Mock Pro", "provider": "future", "supportsImages": true, "thinkingLevel": "high", "contextWindow": 256000, "isDefault": false},
    {"id": "mock-fast", "label": "Mock Fast", "provider": "future", "supportsImages": false, "thinkingLevel": "off", "contextWindow": 64000, "isDefault": false}
  ]
}"#;

/// Fixed session list (`list_sessions` data) for the /sessions overlay.
/// Matches the real agent's list_sessions summaries (`id/cwd/model/updatedAt`
/// — no session_name), so both TUIs parse identical wire data.
const SESSIONS_JSON: &str = r#"{
  "sessions": [
    {"id": "mock-session-1", "cwd": "/mock", "model": "mock-model", "updatedAt": "2026-08-07T00:00:00Z"},
    {"id": "mock-session-2", "cwd": "/mock", "model": "mock-model", "updatedAt": "2026-08-06T00:00:00Z"}
  ]
}"#;

/// `prompt` RunAck (snake_case per the `RunAck` wire contract).
const PROMPT_ACK_JSON: &str = r#"{
  "run_id": "run_mock_1",
  "run_epoch": 1,
  "accepted_state": "running",
  "run_sequence": 1,
  "queue_position": 0
}"#;

const RUN_ID: &str = "run_mock_1";
const SESSION_ID: &str = "mock-session-1";

/// Fixed assistant reply (markdown exercises the renderer on both sides).
const REPLY_TEXT: &str =
    "Hello from the mock agent!\n\nThis is a **deterministic** reply with `code` and a [link](https://example.com).\n";

#[derive(Clone)]
struct MockAgent {
    /// Active event subscribers (one channel per stream_events connection).
    subs: Arc<std::sync::Mutex<Vec<mpsc::UnboundedSender<StreamEvent>>>>,
}

impl MockAgent {
    fn ok(&self, cmd: &RpcCommand, data: &str) -> RpcResponse {
        RpcResponse {
            id: cmd.id.clone(),
            r#type: "response".into(),
            command: cmd.r#type.clone(),
            success: true,
            data: data.into(),
            error: String::new(),
            error_code: String::new(),
            error_data: String::new(),
            payload: None,
        }
    }

    /// Broadcast the fixed reply event sequence after the prompt ack has been
    /// delivered (small delay so the ack is processed first on both sides).
    fn schedule_prompt_events(&self, prompt_text: &str) {
        let subs = self.subs.clone();
        let prompt_text = prompt_text.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let events = [
                (
                    "user_message",
                    format!(
                        r#"{{"text":{},"sessionId":"{SESSION_ID}"}}"#,
                        serde_json::to_string(&prompt_text).unwrap()
                    ),
                ),
                (
                    "agent_start",
                    format!(r#"{{"started_at_ms":1750000000000,"run_id":"{RUN_ID}"}}"#),
                ),
                (
                    "text_chunk",
                    r#"{"text":"Hello from the mock agent!\n\n","delta":true}"#.to_string(),
                ),
                (
                    "text_chunk",
                    r#"{"text":"This is a **deterministic** reply with `code` and a [link](https://example.com).\n","delta":true}"#
                        .to_string(),
                ),
                (
                    "agent_end",
                    format!(
                        r#"{{"run_id":"{RUN_ID}","duration_ms":500,"usage":{{"prompt_tokens":10,"completion_tokens":20}},"error":null,"text":{}}}"#,
                        serde_json::to_string(REPLY_TEXT).unwrap()
                    ),
                ),
            ];
            let senders: Vec<mpsc::UnboundedSender<StreamEvent>> = subs.lock().unwrap().clone();
            for (idx, (ty, data)) in events.iter().enumerate() {
                let event = StreamEvent {
                    r#type: (*ty).into(),
                    data: data.clone(),
                    run_id: RUN_ID.into(),
                    idx: idx as i64,
                    session_id: SESSION_ID.into(),
                    epoch: 1,
                    event_id: format!("evt_mock_{idx}"),
                    timestamp: "2026-08-07T00:00:00.000Z".into(),
                    ..Default::default()
                };
                for tx in &senders {
                    let _ = tx.send(event.clone());
                }
            }
        });
    }
}

#[tonic::async_trait]
impl FutureAgent for MockAgent {
    async fn execute_command(
        &self,
        request: Request<RpcCommand>,
    ) -> Result<Response<RpcResponse>, Status> {
        let cmd = request.into_inner();
        let data = match cmd.r#type.as_str() {
            "get_state" => STATE_JSON.to_string(),
            "list_models" | "get_available_models" => MODELS_JSON.to_string(),
            "new_session" => r#"{"sessionId":"mock-session-1"}"#.to_string(),
            "reload_config" => r#"{"skills":["future-web"],"contextFiles":[]}"#.to_string(),
            "get_messages" => r#"{"messages":[]}"#.to_string(),
            "list_sessions" => SESSIONS_JSON.to_string(),
            "prompt" => {
                self.schedule_prompt_events(&cmd.message);
                PROMPT_ACK_JSON.to_string()
            }
            _ => "{}".to_string(),
        };
        Ok(Response::new(self.ok(&cmd, &data)))
    }

    type StreamEventsStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, Status>> + Send>>;

    async fn stream_events(
        &self,
        _request: Request<StreamRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let (tx, rx) = mpsc::unbounded_channel::<StreamEvent>();
        self.subs.lock().unwrap().push(tx);
        // Push a first "ping" so the client's connected edge fires promptly
        // (the P3 client clears its 5 s first-data watchdog on first data).
        let first = StreamEvent {
            r#type: "ping".into(),
            session_id: SESSION_ID.into(),
            ..Default::default()
        };
        let stream =
            stream::once(async move { Ok(first) }).chain(UnboundedReceiverStream::new(rx).map(Ok));
        Ok(Response::new(Box::pin(stream)))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut port = 50051u16;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                if let Some(v) = args.next() {
                    port = v.parse().unwrap_or(50051);
                }
            }
            "--help" | "-h" => {
                println!("usage: mock_agent --port <port>");
                return Ok(());
            }
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }

    let addr = format!("127.0.0.1:{port}");
    let agent = MockAgent {
        subs: Arc::new(std::sync::Mutex::new(Vec::new())),
    };
    println!("mock agent listening on {addr}");
    Server::builder()
        .add_service(FutureAgentServer::new(agent))
        .serve(addr.parse()?)
        .await?;
    Ok(())
}
