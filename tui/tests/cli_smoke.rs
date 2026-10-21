//! CLI smoke tests: run the real `future-tui` binary end-to-end.
//!
//! These cover `main.rs` (the binary entry point) and the top-level
//! `index::run` flow, which unit tests cannot reach.

use std::process::Command;

/// Run the binary with a scratch HOME so no user config is touched.
fn run_with_args(args: &[&str]) -> std::process::Output {
    let home = tempfile::tempdir().expect("tempdir");
    Command::new(env!("CARGO_BIN_EXE_future-tui"))
        .args(args)
        .env("HOME", home.path())
        .output()
        .expect("spawn future-tui")
}

#[test]
fn help_flag_prints_usage_and_exits_zero() {
    let out = run_with_args(&["--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("future-tui"));
    assert!(stdout.contains("--help"));
}

#[test]
fn version_flag_prints_version_and_exits_zero() {
    let out = run_with_args(&["--version"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("future-tui v"));
}

#[test]
fn unknown_option_exits_one() {
    let out = run_with_args(&["--definitely-not-an-option"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Unknown option"));
}

/// An agent that only answers the readiness handshake — enough to be discovered
/// on an explicit `--grpc-addr`, so the binary never tries to host an Agent. A TCP endpoint has no expected home, so any 64-hex data root validates.
struct ReadinessMock;

#[tonic::async_trait]
impl future_rpc::proto::future_agent_server::FutureAgent for ReadinessMock {
    async fn execute_command(
        &self,
        request: tonic::Request<future_rpc::proto::RpcCommand>,
    ) -> Result<tonic::Response<future_rpc::proto::RpcResponse>, tonic::Status> {
        let cmd = request.into_inner();
        let data = if cmd.r#type == "get_agent_readiness" {
            serde_json::json!({
                "version": future_rpc::connection::EXPECTED_AGENT_VERSION,
                "agentInstanceId": "cli-smoke-fixture",
                "protocolVersion": future_rpc::connection::AGENT_PROTOCOL_VERSION,
                "dataRootId": "0".repeat(64),
                "storageSchemaVersion": future_rpc::connection::AGENT_STORAGE_SCHEMA_VERSION,
            })
            .to_string()
        } else {
            "{}".to_string()
        };
        Ok(tonic::Response::new(future_rpc::proto::RpcResponse {
            id: cmd.id,
            r#type: "response".into(),
            command: cmd.r#type,
            success: true,
            data,
            ..Default::default()
        }))
    }

    type StreamEventsStream = std::pin::Pin<
        Box<
            dyn tokio_stream::Stream<Item = Result<future_rpc::proto::StreamEvent, tonic::Status>>
                + Send,
        >,
    >;

    async fn stream_events(
        &self,
        _request: tonic::Request<future_rpc::proto::StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        Ok(tonic::Response::new(Box::pin(
            futures_util::stream::pending(),
        )))
    }
}

/// Exit code 1 in print mode has **two** possible causes: when an Agent is
/// reachable, `index::run` reaches its own "No prompt provided" check; when
/// discovery fails first, the same code comes back from that path instead. Asserting only
/// `Some(1)` therefore accepts either path, and the difference is visible only
/// in coverage counters — an infrastructure accident, not a behaviour.
///
/// So the path is pinned here: a readiness mock on the explicit address is
/// discovered as an external agent (no launch, nothing host-dependent), and the
/// assertion is on the message the print-mode branch prints.
#[test]
fn print_mode_without_message_exits_one() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let (addr, server) = runtime.block_on(async {
        use future_rpc::proto::future_agent_server::FutureAgentServer;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let incoming = tonic::transport::server::TcpIncoming::from_listener(listener, true, None)
            .expect("tcp incoming");
        let server = tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(FutureAgentServer::new(ReadinessMock))
                .serve_with_incoming(incoming),
        );
        (addr, server)
    });

    let out = run_with_args(&["--print", "--grpc-addr", &addr]);
    server.abort();
    drop(runtime);

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("No prompt provided"),
        "print mode must report the missing prompt (the agent was reachable, so \
         this is the print branch, not a discovery failure): {stderr}"
    );
}

#[test]
fn list_models_without_agent_exits_one() {
    // Nothing listens on this addr — the connect must fail fast.
    let out = run_with_args(&["--list-models", "--grpc-addr", "127.0.0.1:1"]);
    assert_eq!(out.status.code(), Some(1));
}
