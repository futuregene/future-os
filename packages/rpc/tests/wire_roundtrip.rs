//! In-process gRPC coverage for the generated tonic client/server plumbing
//! in `future_rpc::proto` (the checked-in `generated/proto.rs`): client
//! constructors and tuning methods, server builders, the unknown-path
//! `Unimplemented` fallback, and a real ExecuteCommand / StreamEvents
//! exchange over a loopback socket.

use std::sync::Arc;

use future_rpc::proto;
use future_rpc::proto::future_agent_client::FutureAgentClient;
use future_rpc::proto::future_agent_server::{FutureAgent, FutureAgentServer};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::codec::CompressionEncoding;
use tonic::codegen::http;
use tonic::codegen::tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

/// Minimal in-process agent: echoes the command and emits one ping event.
struct StubAgent;

#[tonic::async_trait]
impl FutureAgent for StubAgent {
    async fn execute_command(
        &self,
        request: Request<proto::RpcCommand>,
    ) -> Result<Response<proto::RpcResponse>, Status> {
        let command = request.into_inner();
        Ok(Response::new(proto::RpcResponse {
            id: command.id,
            r#type: "response".to_string(),
            command: command.r#type,
            success: true,
            data: r#"{"ok":true}"#.to_string(),
            ..Default::default()
        }))
    }

    type StreamEventsStream = ReceiverStream<Result<proto::StreamEvent, Status>>;

    async fn stream_events(
        &self,
        _request: Request<proto::StreamRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tx.send(Ok(proto::StreamEvent {
            r#type: "ping".to_string(),
            ..Default::default()
        }))
        .await
        .map_err(|_| Status::internal("stream send failed"))?;
        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

/// A client transport whose `poll_ready` always fails — drives the
/// service-not-ready error mapping in the generated RPC methods without
/// needing a socket.
#[derive(Clone)]
struct NeverReady;

impl tonic::codegen::Service<http::Request<tonic::body::BoxBody>> for NeverReady {
    type Response = http::Response<tonic::body::BoxBody>;
    // Boxed: keeps clippy::result_large_err happy (tonic::Status is large).
    type Error = Box<Status>;
    type Future = std::future::Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Err(Box::new(Status::unavailable("never ready"))))
    }

    fn call(&mut self, _req: http::Request<tonic::body::BoxBody>) -> Self::Future {
        std::future::ready(Err(Box::new(Status::unavailable("never ready"))))
    }
}

#[tokio::test]
async fn execute_command_and_stream_events_roundtrip() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    let server = tonic::transport::Server::builder()
        .add_service(FutureAgentServer::new(StubAgent))
        .serve_with_incoming(TcpListenerStream::new(listener));
    tokio::spawn(server);

    let mut client = FutureAgentClient::connect(format!("http://{addr}"))
        .await
        .expect("connect to in-process server");

    let response = client
        .execute_command(proto::RpcCommand {
            id: "req-1".to_string(),
            r#type: "get_state".to_string(),
            ..Default::default()
        })
        .await
        .expect("execute_command")
        .into_inner();
    assert_eq!(response.id, "req-1");
    assert_eq!(response.command, "get_state");
    assert!(response.success);
    assert_eq!(response.data, r#"{"ok":true}"#);

    let mut stream = client
        .stream_events(proto::StreamRequest::default())
        .await
        .expect("stream_events")
        .into_inner();
    let event = stream.message().await.unwrap().expect("one ping event");
    assert_eq!(event.r#type, "ping");
}

/// An agent answering with a response above tonic's 4 MiB decoding default —
/// one `get_messages` for a long session — is decoded by
/// [`future_rpc::transport::agent_client`] and rejected by a plain generated
/// client. That asymmetry is the bug the TUI reported as "Switched to session:
/// … — its transcript could not be loaded (Error, decoded message length too
/// large: found 10342115 bytes, the limit is: 4194304 bytes)": the Agent's own
/// encoding cap is `MAX_GRPC_MESSAGE_SIZE`, so the 4 MiB limit is purely the
/// client's, and only clients built through the helper inherit the raised one.
#[tokio::test]
async fn agent_client_decodes_a_response_past_tonics_default_limit() {
    /// Past the 4 MiB decoding default, far below the 32 MiB cap.
    const FILLER_BYTES: usize = 5 * 1024 * 1024;

    struct BigAgent;

    #[tonic::async_trait]
    impl FutureAgent for BigAgent {
        async fn execute_command(
            &self,
            request: Request<proto::RpcCommand>,
        ) -> Result<Response<proto::RpcResponse>, Status> {
            let command = request.into_inner();
            Ok(Response::new(proto::RpcResponse {
                id: command.id,
                r#type: "response".to_string(),
                command: command.r#type,
                success: true,
                data: format!(
                    r#"{{"messages":[{{"role":"user","blocks":[{{"type":"text","text":"{}"}}]}}]}}"#,
                    "x".repeat(FILLER_BYTES)
                ),
                ..Default::default()
            }))
        }

        type StreamEventsStream = ReceiverStream<Result<proto::StreamEvent, Status>>;

        async fn stream_events(
            &self,
            _request: Request<proto::StreamRequest>,
        ) -> Result<Response<Self::StreamEventsStream>, Status> {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            drop(tx);
            Ok(Response::new(ReceiverStream::new(rx)))
        }
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(FutureAgentServer::new(BigAgent))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    let command = proto::RpcCommand {
        id: "req-1".to_string(),
        r#type: "get_messages".to_string(),
        ..Default::default()
    };

    // Untuned client: its 4 MiB decoding default rejects the response.
    let mut plain = FutureAgentClient::connect(format!("http://{addr}"))
        .await
        .expect("connect to in-process server");
    let error = plain
        .execute_command(command.clone())
        .await
        .expect_err("a 5 MiB response cannot fit tonic's default decoding cap");
    assert!(
        error.message().contains("decoded message length too large"),
        "{error}"
    );

    // The shared client carries the cap, so the same call succeeds.
    let connected = future_rpc::transport::connect_channel(
        Some(&format!("http://{addr}")),
        std::time::Duration::from_secs(5),
        None,
    )
    .await
    .expect("discover the in-process server");
    let mut client = future_rpc::transport::agent_client(connected.channel);
    let response = client
        .execute_command(command)
        .await
        .expect("the shared client decodes the oversized response")
        .into_inner();
    assert!(response.success);
    assert!(response.data.len() > FILLER_BYTES);
}

/// Every generated client tuning method, then the not-ready error mapping
/// for both RPCs.
#[tokio::test]
async fn client_builders_and_not_ready_errors() {
    let mut client = FutureAgentClient::new(NeverReady)
        .send_compressed(CompressionEncoding::Gzip)
        .accept_compressed(CompressionEncoding::Gzip)
        .max_decoding_message_size(1024)
        .max_encoding_message_size(2048);
    let err = client
        .execute_command(proto::RpcCommand::default())
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unknown);

    let mut client = FutureAgentClient::new(NeverReady);
    let err = client
        .stream_events(proto::StreamRequest::default())
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unknown);
}

#[test]
// tonic's Interceptor trait fixes the `Result<_, Status>` closure signature.
#[allow(clippy::result_large_err)]
fn client_constructor_variants() {
    let origin: http::Uri = "http://127.0.0.1:1".parse().unwrap();
    let _client = FutureAgentClient::with_origin(NeverReady, origin);
    let _client = FutureAgentClient::with_interceptor(NeverReady, |req: Request<()>| Ok(req));
}

#[test]
// tonic's Interceptor trait fixes the `Result<_, Status>` closure signature.
#[allow(clippy::result_large_err)]
fn server_constructor_variants_and_tuning() {
    let server = FutureAgentServer::from_arc(Arc::new(StubAgent));
    let _server = server
        .accept_compressed(CompressionEncoding::Gzip)
        .send_compressed(CompressionEncoding::Gzip)
        .max_decoding_message_size(1024)
        .max_encoding_message_size(2048);
    let _intercepted = FutureAgentServer::with_interceptor(StubAgent, |req: Request<()>| Ok(req));
}

/// A request to an unknown path gets the generated Unimplemented response.
#[tokio::test]
async fn server_unknown_path_is_unimplemented() {
    use tonic::codegen::Service;

    type Req = http::Request<tonic::body::BoxBody>;

    let mut server = FutureAgentServer::new(StubAgent);
    std::future::poll_fn(|cx| Service::<Req>::poll_ready(&mut server, cx))
        .await
        .expect("server is always ready");
    let request = http::Request::builder()
        .uri("/proto.FutureAgent/BogusMethod")
        .body(tonic::body::empty_body())
        .unwrap();
    let response = Service::<Req>::call(&mut server, request)
        .await
        .expect("unimplemented response");
    let status = response.headers().get(Status::GRPC_STATUS).unwrap();
    assert_eq!(
        status.to_str().unwrap(),
        (tonic::Code::Unimplemented as i32).to_string()
    );
}
