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

/// The checkout the scripted session claims to sit in.
///
/// The fictional path is the default because a frame that names the machine it
/// was captured on is not a baseline. A surface that *reads* the checkout — the
/// git panel is the one — needs a real repository, so a capture can name one:
/// `FUTURE_DEMO_CWD=$PWD tui/scripts/shot.sh git`. Nothing changes unless it is
/// set, so every other scenario stays byte-identical across machines.
fn session_cwd() -> String {
    std::env::var("FUTURE_DEMO_CWD").unwrap_or_else(|_| SESSION_CWD.to_string())
}

/// The scripted conversation: one exchange that reasons, reads a file, runs the
/// tests and answers. Written in the display projection's shape (`blocks`), so
/// it travels the same mapper a real session's journal does.
fn journal() -> Value {
    let mut entries = vec![
        json!({
            "id": "e1", "kind": "user", "role": "user",
            "blocks": [{"kind": "text",
                "text": "parser 里有个 off-by-one，找出来并补一个回归测试"}],
        }),
        json!({
            "id": "e2", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "reasoning",
                "text": "先看 parse_expr 的边界条件，怀疑是最后一个 token 的处理。"}],
        }),
        json!({
            "id": "e3", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "tool_call", "toolCallId": "call_read", "name": "read",
                        "arguments": {"path": "src/parser.rs"}}],
        }),
        json!({
            "id": "e4", "kind": "tool", "role": "tool",
            "blocks": [{"kind": "tool_result", "toolCallId": "call_read", "isError": false,
                "text": "142  fn parse_expr(&mut self) -> Result<Expr> {\n143      while self.peek() <= self.end { ... }\n"}],
        }),
        json!({
            "id": "e5", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "tool_call", "toolCallId": "call_test", "name": "shell",
                        "arguments": {"command": "cargo test -p parser"}}],
        }),
        json!({
            "id": "e6", "kind": "tool", "role": "tool",
            "blocks": [{"kind": "tool_result", "toolCallId": "call_test", "isError": false,
                "text": "test result: ok. 40 passed; 0 failed\n"}],
        }),
        json!({
            "id": "e7", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "text",
                "text": "`while self.peek() <= self.end` 在最后一个 token 上多读一次。改成 `<` 之后，41 个用例全过。"}],
        }),
    ];
    if std::env::var_os("FUTURE_DEMO_LONG").is_some() {
        long_turns(&mut entries);
    }
    json!({ "entries": entries })
}

/// How many extra turns `FUTURE_DEMO_LONG=1` appends: enough rows to outrun the
/// transcript band of a tall terminal, which is what a `scrolled` capture needs.
const LONG_TURNS: usize = 16;

/// More exchanges after the tutorial's, for the one scenario that is *about*
/// scrolling (`scrolled`, which wheels up to reach the `↑ 更早` badge).
///
/// A conversation that fits the band has nowhere to scroll — the ceiling counts
/// the window as well as the rows (see `Workbench::scroll`) — so that shot needs
/// a transcript taller than the band. Opt-in rather than always: the other
/// scenarios' frames are byte-identical pictures, and rows nobody asked for
/// would change them.
fn long_turns(entries: &mut Vec<Value>) {
    for turn in 1..=LONG_TURNS {
        entries.push(json!({
            "id": format!("lu{turn}"), "kind": "user", "role": "user",
            "blocks": [{"kind": "text",
                "text": format!("第 {turn} 轮：这一层重试为什么不直接拿掉？")}],
        }));
        entries.push(json!({
            "id": format!("la{turn}"), "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "text", "text": format!(
                "第 {turn} 轮：上游会在 45 秒静默断开，这一层把它挡在界面之外。\n日志里能查到 `retry attempt {turn}`，超时的那条没有收尾行。")}],
        }));
    }
}

/// `get_state`: what the header, the status bar and the session list draw.
fn state() -> Value {
    json!({
        "sessionId": SESSION_ID,
        "sessionName": "parser 的 off-by-one",
        "cwd": session_cwd(),
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
        // `inputTokens` is the session's `prompt_tokens`, so the cached part is
        // inside it (88.0k + 2.1k of 92.0k → a 96% hit rate). A stand-in that
        // reports more cache than prompt would paint a footer no real session
        // can produce.
        "usage": {
            "inputTokens": 92_000, "outputTokens": 3_300,
            "cacheReadTokens": 88_000, "cacheWriteTokens": 2_100,
            "costCny": 0.0137
        }
    })
}

fn sessions() -> Value {
    json!({"sessions": [
        {"id": SESSION_ID, "sessionName": "parser 的 off-by-one", "cwd": session_cwd(),
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

/// What the scripted *live* turn says, one delta at a time.
///
/// A screenshot of a reply that is still arriving needs the agent to emit
/// events, and a real model cannot: the frame would differ per run and per
/// machine. So the tutorial's answer is a fixed list of deltas — the last one
/// deliberately stops mid-sentence, which is what a frame captured mid-stream
/// looks like. The surface lays it out through its ordinary path
/// (`Live::handle_event` → `ChatArea` → the frame); only the words are scripted.
const STREAM_DELTAS: [&str; 44] = [
    "先看边界：",
    "`parse_expr` 在循环里先消费了运算符，",
    "再检查 `peek()`，",
    "于是最后一个 token 永远没有机会触发收尾分支。\n\n",
    "复现也简单，",
    "输入 `1+2` 是对的，",
    "输入 `1+2+` 就少一个右操作数",
    "——",
    "因为循环在**先**前进后才判断端点。\n\n",
    "修法是把检查提到前进之前：",
    "\n\n```rust\n",
    "while let Some(tok) = lexer.peek() {\n",
    "    if tok.is_eof() { break; }\n",
    "    let op = lexer.next();\n",
    "    let rhs = parse_atom(lexer)?;\n",
    "    lhs = apply(lhs, op, rhs);\n",
    "}\n",
    "```\n\n",
    "回归测试就盯这一件事：",
    "一个以运算符结尾的表达式",
    "必须报 `UnexpectedEof`，",
    "而不是悄悄返回左边那一项。",
    "\n\n",
    "另外还有两处同样的写法，",
    "分别在 `parse_list` 和 `parse_args` 里；",
    "我先把这三处一起改掉，",
    "然后跑一遍 `cargo test -p parser`。",
    "\n\n",
    "测试这边我加了两条：",
    "一条钉住 `1+2+`，",
    "一条钉住 `[1,2,`",
    "和 `f(a,` 的空参数情况。",
    "\n\n",
    "顺便说一句，",
    "这类 off-by-one 都长成同一个形状：",
    "**先推进、后判断**。",
    "所以在 review 里看到 `next()` 出现在条件之前，",
    "就值得多看一眼，",
    "哪怕当下测试是绿的。",
    "\n\n",
    "要我把这三处的改动拆成两个 commit 吗？",
    "一个只做循环边界的调整，",
    "另一个补测试，",
    "这样 revert 的粒度",
];

/// One delta every this many milliseconds: slow enough that a screenshot catches
/// the reply in flight, fast enough that a capture does not wait on prose.
const STREAM_DELTA: std::time::Duration = std::time::Duration::from_millis(120);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// The demo agent's handle on its event stream.
///
/// `stream_events` is where the client's subscription lands, and a scripted turn
/// has to write into that very stream — so the sender is kept here instead of
/// being parked inside the function (see the note there). `None` before any
/// client subscribes: a prompt that arrives that early has nowhere to go, and
/// the surface is not watching anyway.
#[derive(Clone, Default)]
struct DemoAgent {
    events:
        std::sync::Arc<std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<StreamEvent>>>>,
}

impl DemoAgent {
    /// Emit the scripted turn on the subscribed stream: `agent_start`, then one
    /// `text_chunk` per [`STREAM_DELTAS`] entry, then nothing.
    ///
    /// Nothing after the last delta is the point — an `agent_end` would take the
    /// activity row away and leave a *finished* reply on screen, while a
    /// screenshot of streaming output needs the turn still in flight. The
    /// surface keeps drawing its spinner for as long as the capture runs.
    fn start_turn(&self, run_id: &str) {
        let Some(tx) = self.events.lock().ok().and_then(|slot| slot.clone()) else {
            return;
        };
        let run_id = run_id.to_string();
        tokio::spawn(async move {
            let event = |kind: &str, data: Value| StreamEvent {
                r#type: kind.to_string(),
                session_id: SESSION_ID.to_string(),
                run_id: run_id.clone(),
                data: data.to_string(),
                ..Default::default()
            };
            if tx
                .send(event(
                    "agent_start",
                    json!({"model": MODEL, "sessionId": SESSION_ID}),
                ))
                .is_err()
            {
                return;
            }
            for delta in STREAM_DELTAS {
                tokio::time::sleep(STREAM_DELTA).await;
                if tx
                    .send(event("text_chunk", json!({"text": delta})))
                    .is_err()
                {
                    return;
                }
            }
        });
    }
}

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
            // A launchpad submit creates the session it then prompts; answer with
            // the id the scripted session has so both halves name one session.
            "new_session" => json!({"sessionId": SESSION_ID}),
            // A prompt needs a model in a real agent. Here it starts the scripted
            // turn: the ack is what the surface expects from a running run, and
            // the deltas arrive on the event stream this agent was subscribed to.
            "prompt" => {
                let run_id = format!("demo-run-{}", now_ms());
                self.start_turn(&run_id);
                // `RunAck` reads snake_case (`run_id`, `run_epoch`,
                // `accepted_state`): a camelCase ack is what produced a
                // "Failed to send: missing field `run_id`" notice the first
                // time a scripted turn was screenshotted.
                json!({"run_id": run_id, "run_epoch": 1, "accepted_state": "running"})
            }
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
        // The scripted turn writes into this same stream, so the sender is kept
        // on the service rather than only in the parked task below.
        if let Ok(mut slot) = self.events.lock() {
            *slot = Some(tx.clone());
        }
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
        .add_service(FutureAgentServer::new(DemoAgent::default()))
        .serve_with_incoming(incoming)
        .await;
}
