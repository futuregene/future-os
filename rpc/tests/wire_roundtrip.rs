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
