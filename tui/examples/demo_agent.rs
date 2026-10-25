//! A scripted agent for screenshots: a real gRPC server answering with a fixed
//! session, so `future tui` can be captured showing real transcript content
//! without a model, a network or a cent of credit.
//!
//! This is the TUI's equivalent of the desktop and mobile mock backends: the
//! *application* is the real binary, rendering real rows through the real code
//! path (`get_state` → `Live` → `WorkbenchData` → the frame; `get_session_entries`
//! → `session_rows::rows_to_messages` → `ChatArea`); only the data is scripted.
//! That is what makes a screenshot reproducible — the same command produces the
//! same picture on any machine — which a live model cannot.
//!
//! Usage (the screenshots harness starts it for you):
//!
//! ```text
//! cargo run -p future-tui --example demo_agent -- --port 5501
//! future tui --grpc-addr 127.0.0.1:5501
//! ```

use std::pin::Pin;

use futures_util::stream;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::transport::server::TcpIncoming;
use tonic::transport::Server;

use future_rpc::proto::future_agent_server::{FutureAgent, FutureAgentServer};
use future_rpc::proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};

/// The session the demo agent pretends to have.
const SESSION_ID: &str = "demo-session";
const SESSION_CWD: &str = "/Users/you/future-os";
const MODEL: &str = "future/deepseek-flash";

/// The scripted conversation: one exchange that reasons, reads a file, runs the
/// tests and answers. Written in the display projection's shape (`blocks`), so
/// it travels the same mapper a real session's journal does.
fn journal() -> Value {
    json!({"entries": [
        {
            "id": "e1", "kind": "user", "role": "user",
            "blocks": [{"kind": "text",
                "text": "parser 里有个 off-by-one，找出来并补一个回归测试"}],
        },
        {
            "id": "e2", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "reasoning",
                "text": "先看 parse_expr 的边界条件，怀疑是最后一个 token 的处理。"}],
        },
        {
            "id": "e3", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "tool_call", "toolCallId": "call_read", "name": "read",
                        "arguments": {"path": "src/parser.rs"}}],
        },
        {
            "id": "e4", "kind": "tool", "role": "tool",
            "blocks": [{"kind": "tool_result", "toolCallId": "call_read", "isError": false,
                "text": "142  fn parse_expr(&mut self) -> Result<Expr> {\n143      while self.peek() <= self.end { ... }\n"}],
        },
        {
            "id": "e5", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "tool_call", "toolCallId": "call_test", "name": "shell",
                        "arguments": {"command": "cargo test -p parser"}}],
        },
        {
            "id": "e6", "kind": "tool", "role": "tool",
            "blocks": [{"kind": "tool_result", "toolCallId": "call_test", "isError": false,
                "text": "test result: ok. 40 passed; 0 failed\n"}],
        },
        {
            "id": "e7", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "text",
                "text": "`while self.peek() <= self.end` 在最后一个 token 上多读一次。改成 `<` 之后，41 个用例全过。"}],
        }
    ]})
}

/// `get_state`: what the header, the status bar and the session list draw.
fn state() -> Value {
    json!({
        "sessionId": SESSION_ID,
        "sessionName": "parser 的 off-by-one",
        "cwd": SESSION_CWD,
        "model": MODEL,
        "thinkingLevel": "medium",
        "sandboxTier": "manual",
        "permissionLevel": "workspace",
        "isStreaming": false,
        "version": future_rpc::connection::EXPECTED_AGENT_VERSION,
        "skills": ["future-versioning", "future-peer-review", "future-web"],
        "contextFiles": ["AGENTS.md"],
        "contextTokens": 12_400,
        "contextWindow": 200_000,
        "contextPercent": 6.2,
        "usage": {
            "inputTokens": 9_100, "outputTokens": 3_300,
            "cacheReadTokens": 88_000, "cacheWriteTokens": 2_100,
            "costCny": 0.0137
        }
    })
}

fn sessions() -> Value {
    json!({"sessions": [
        {"id": SESSION_ID, "sessionName": "parser 的 off-by-one", "cwd": SESSION_CWD,
         "model": MODEL, "updatedAtMs": now_ms(), "queryCount": 1, "isStreaming": false},
        {"id": "demo-older", "sessionName": "workbench 布局草图",
         "cwd": "/Users/you/future-os/.worktrees/ui", "model": MODEL,
         "updatedAtMs": now_ms() - 3_600_000, "queryCount": 12, "isStreaming": false},
        {"id": "demo-oldest", "sessionName": "nightly coverage run",
         "cwd": "/Users/you/future-os", "model": MODEL,
         "updatedAtMs": now_ms() - 86_400_000, "queryCount": 4, "isStreaming": false}
    ]})
}

fn models() -> Value {
    json!({"models": [
        {"id": "deepseek-flash", "label": "DeepSeek Flash", "provider": "future",
         "supportsImages": false, "thinkingLevel": "medium", "contextWindow": 200_000,
         "isDefault": true},
        {"id": "claude-sonnet-4", "label": "Claude Sonnet", "provider": "anthropic",
         "supportsImages": true, "thinkingLevel": "medium", "contextWindow": 200_000,
         "isDefault": false}
    ]})
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Clone)]
struct DemoAgent;

#[tonic::async_trait]
impl FutureAgent for DemoAgent {
    async fn execute_command(
        &self,
        request: tonic::Request<RpcCommand>,
    ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
        let cmd = request.into_inner();
        let data: Value = match cmd.r#type.as_str() {
            // The connect handshake: the client refuses an agent whose version
            // or protocol differs, so answer it exactly.
            "get_agent_readiness" => json!({
                "version": future_rpc::connection::EXPECTED_AGENT_VERSION,
                "agentInstanceId": "demo-agent",
                "protocolVersion": future_rpc::connection::AGENT_PROTOCOL_VERSION,
                "dataRootId": "0".repeat(64),
                "storageSchemaVersion": future_rpc::connection::AGENT_STORAGE_SCHEMA_VERSION
            }),
            "get_state" => state(),
            "list_sessions" => sessions(),
            "list_models" => models(),
            "get_session_entries" => journal(),
            // A prompt from the screenshot run would need a model; answer with
            // an ack so the surface shows its "sent" state instead of an error.
            "prompt" => json!({"runId": "demo-run", "runEpoch": 1, "acceptedState": "running"}),
            _ => json!({}),
        };
        Ok(tonic::Response::new(RpcResponse {
            id: cmd.id,
            r#type: "response".into(),
            command: cmd.r#type.clone(),
            success: true,
            data: data.to_string(),
            error: String::new(),
            error_code: String::new(),
            error_data: String::new(),
            payload: None,
        }))
    }

    type StreamEventsStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<StreamEvent, tonic::Status>> + Send>>;

    async fn stream_events(
        &self,
        _request: tonic::Request<StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        // One `ping` so the client's connected edge fires, then stay open: a
        // closed stream would put the surface in its reconnecting state, which
        // is not what a screenshot should show.
        //
        // "Stay open" has to be arranged explicitly. A sender bound in this
        // function is dropped the moment it returns, which closes the receiver
        // the returned stream is reading from — the client then sees a stream
        // that ended right after its first frame, flips to disconnected, and
        // every later call spends its connect wait before running. Measured:
        // the surface never painted at all. So the sender is parked in a task
        // that never finishes instead of living on the stack.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
        tokio::spawn(async move {
            let _held = tx;
            std::future::pending::<()>().await;
        });
        let first = StreamEvent {
            r#type: "ping".into(),
            data: String::new(),
            ..Default::default()
        };
        let stream =
            stream::once(async move { Ok(first) }).chain(UnboundedReceiverStream::new(rx).map(Ok));
        Ok(tonic::Response::new(Box::pin(stream)))
    }
}

#[tokio::main]
async fn main() {
    let mut port: u16 = 5501;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => match args.next().and_then(|value| value.parse().ok()) {
                Some(value) => port = value,
                None => {
                    eprintln!("demo_agent: --port needs a number");
                    std::process::exit(2);
                }
            },
            "--help" | "-h" => {
                println!(
                    "demo_agent — a scripted agent for TUI screenshots\n\
                     \n\
                     usage: demo_agent [--port N]\n\
                     \n\
                     Serves a fixed session on 127.0.0.1:N; point the TUI at it:\n\
                     \x20 future tui --grpc-addr 127.0.0.1:N"
                );
                return;
            }
            other => {
                eprintln!("demo_agent: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    // Port 0 is useful for tests: the OS picks one and the line below reports
    // it, which is how a caller learns where to connect.
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("demo_agent: cannot bind 127.0.0.1:{port}: {error}");
            std::process::exit(1);
        }
    };
    let addr = listener
        .local_addr()
        .expect("bound listener has an address");
    println!("demo_agent listening on {addr}");
    let incoming = TcpIncoming::from_listener(listener, true, None).expect("listener usable");
    let _ = Server::builder()
        .add_service(FutureAgentServer::new(DemoAgent))
        .serve_with_incoming(incoming)
        .await;
}
