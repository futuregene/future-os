//! gRPC Server for xihu agent
//! 
//! This module implements the xihu gRPC service using tonic.
//! The proto definition is in the ../proto/ directory.
//!
//! HTTP endpoints:
//! - POST / - RPC commands (JSON)
//! - GET /events - SSE event stream
//! 
//! gRPC service: proto.XihuAgent (on grpc_port)

use crate::rpc::{AppState, handle_command_internal, RpcResponse};
use anyhow::Result;
use std::net::SocketAddr;
use std::pin::Pin;
use tokio::sync::broadcast;
use tokio_stream::Stream;
use futures::StreamExt;

// Include the generated proto code
pub mod proto {
    include!("grpc/generated/proto.rs");
}

/// Start a combined HTTP + gRPC server.
/// 
/// HTTP endpoints:
/// - POST / - RPC commands (JSON)  
/// - GET /events - SSE event stream
/// 
/// gRPC service: proto.XihuAgent (on grpc_port)
pub async fn serve_combined(
    state: AppState,
    http_port: u16,
    grpc_port: u16,
) -> Result<()> {
    use axum::{routing::get, routing::post, extract::State, Json, Router};
    use axum::response::sse::{Event, Sse};
    use futures::Stream;
    use tokio::time::{interval, Duration};
    
    // Create a closure-based handler for POST /
    let rpc_handler = |State(state): State<AppState>, Json(body): Json<serde_json::Value>| async move {
        // Try batch first
        if let Ok(cmds) = serde_json::from_value::<Vec<crate::rpc::RpcCommand>>(body.clone()) {
            let responses: Vec<serde_json::Value> = cmds
                .into_iter()
                .map(|cmd| {
                    let resp_str = handle_command_internal(&state, cmd);
                    serde_json::from_str(&resp_str).unwrap_or_default()
                })
                .collect();
            serde_json::to_string(&responses).unwrap_or_default()
        } else if let Ok(cmd) = serde_json::from_value::<crate::rpc::RpcCommand>(body) {
            handle_command_internal(&state, cmd)
        } else {
            RpcResponse::build_fail("", "rpc", "invalid JSON")
        }
    };
    
    // SSE handler
    let sse_handler = |State(state): State<AppState>| {
        let mut rx = state.broadcaster.subscribe();
        async move {
            let stream = async_stream::stream! {
                let mut heartbeat = interval(Duration::from_secs(30));
                
                // Initial ping (matching Go format)
                yield Ok::<_, std::convert::Infallible>(Event::default().comment(" ping"));
                
                loop {
                    tokio::select! {
                        event = rx.recv() => {
                            match event {
                                Ok(evt) => {
                                    yield Ok(Event::default()
                                        .event(evt.event_type)
                                        .data(evt.data));
                                }
                                Err(broadcast::error::RecvError::Lagged(n)) => {
                                    eprintln!("SSE lagged {} events", n);
                                    continue;
                                }
                                Err(broadcast::error::RecvError::Closed) => break,
                            }
                        }
                        _ = heartbeat.tick() => {
                            yield Ok(Event::default().comment(" heartbeat"));
                        }
                    }
                }
            };
            Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
        }
    };
    
    // Build HTTP router
    let app = Router::new()
        .route("/", post(rpc_handler))
        .route("/events", get(sse_handler))
        .with_state(state);
    
    // Build gRPC service
    let grpc_service = XihuAgentService {
        state: app.state().clone(),
    };
    
    eprintln!("HTTP server listening on 0.0.0.0:{}", http_port);
    eprintln!("gRPC server listening on 0.0.0.0:{}", grpc_port);
    
    // Start both servers
    let http_addr: SocketAddr = format!("0.0.0.0:{}", http_port).parse().unwrap();
    let grpc_addr: SocketAddr = format!("0.0.0.0:{}", grpc_port).parse().unwrap();
    
    // Start HTTP server
    let http_handle = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(http_addr).await.unwrap();
        axum::serve(listener, app).await
    });
    
    // Start gRPC server
    let grpc_handle = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(proto::xihu_agent_server::XihuAgentServer::new(grpc_service))
            .serve(grpc_addr)
            .await
    });
    
    // Wait for both servers
    tokio::select! {
        result = http_handle => {
            if let Err(e) = result {
                eprintln!("HTTP server error: {}", e);
            }
        }
        result = grpc_handle => {
            if let Err(e) = result {
                eprintln!("gRPC server error: {}", e);
            }
        }
    }
    
    Ok(())
}

// =============================================================================
// gRPC Service Implementation
// =============================================================================

#[derive(Clone)]
struct XihuAgentService {
    state: AppState,
}

#[tonic::async_trait]
impl proto::xihu_agent_server::XihuAgent for XihuAgentService {
    /// Execute a single RPC command and return the response
    async fn execute_command(
        &self,
        request: tonic::Request<proto::RpcCommand>,
    ) -> Result<tonic::Response<proto::RpcResponse>, tonic::Status> {
        let cmd = request.into_inner();
        
        // Convert proto command to internal command
        let internal_cmd = crate::rpc::RpcCommand {
            id: cmd.id,
            cmd_type: cmd.r#type,
            message: cmd.message,
            images: cmd.images.into_iter().map(|img| {
                crate::types::ImageContent {
                    r#type: img.r#type,
                    url: img.url.unwrap_or_default(),
                    base64: img.base64.unwrap_or_default(),
                }
            }).collect(),
            streaming_behavior: cmd.streaming_behavior,
            parent_session: cmd.parent_session,
            provider: cmd.provider,
            model_id: cmd.model_id,
            level: cmd.level,
            mode: cmd.mode,
            custom_instructions: cmd.custom_instructions,
            enabled: cmd.enabled,
            command: cmd.command,
            session_path: cmd.session_path,
            session_id: cmd.session_id,
            entry_id: cmd.entry_id,
            name: cmd.name,
            output_path: cmd.output_path,
        };
        
        // Handle the command
        let resp_str = handle_command_internal(&self.state, internal_cmd);
        
        // Parse the response
        #[derive(serde::Deserialize)]
        struct JsonResp {
            id: String,
            #[serde(rename = "type")]
            resp_type: String,
            command: String,
            success: bool,
            data: Option<serde_json::Value>,
            error: Option<String>,
        }
        
        let json_resp: JsonResp = serde_json::from_str(&resp_str)
            .map_err(|e| tonic::Status::internal(format!("Failed to parse response: {}", e)))?;
        
        // Convert to proto response
        let proto_resp = proto::RpcResponse {
            id: json_resp.id,
            resp_type: json_resp.resp_type,
            command: json_resp.command,
            success: json_resp.success,
            data: json_resp.data.map(|d| prost_types::Any {
                type_url: "type.googleapis.com/google.protobuf.Value".to_string(),
                value: serde_json::to_vec(&d).unwrap_or_default(),
            }),
            error: json_resp.error,
        };
        
        Ok(tonic::Response::new(proto_resp))
    }
    
    /// Stream events to the client (server-side streaming)
    type StreamEventsStream = Pin<Box<dyn Stream<Item = Result<proto::StreamEvent, tonic::Status>> + Send>>;
    
    async fn stream_events(
        &self,
        _request: tonic::Request<proto::StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        let mut rx = self.state.broadcaster.subscribe();
        
        let stream = async_stream::stream! {
            // Send initial ping
            yield Ok(proto::StreamEvent {
                event_type: "ping".to_string(),
                data: r#"{"type":"ping"}"#.to_string(),
            });
            
            // Stream events until client disconnects
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        yield Ok(proto::StreamEvent {
                            event_type: event.event_type,
                            data: event.data,
                        });
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("SSE lagged {} events", n);
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        };
        
        Ok(tonic::Response::new(Box::pin(stream)))
    }
    
    /// Bidirectional streaming for extension UI
    type ExtensionUIStream = Pin<Box<dyn Stream<Item = Result<proto::ExtensionUiResponse, tonic::Status>> + Send>>;
    
    async fn extension_ui(
        &self,
        _request: tonic::Request<tonic::Streaming<proto::ExtensionUiRequest>>,
    ) -> Result<tonic::Response<Self::ExtensionUIStream>, tonic::Status> {
        // Extension UI not implemented
        Err(tonic::Status::unimplemented("Extension UI not yet implemented"))
    }
}

// Need prost_types for Any
pub mod prost_types {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Any {
        #[prost(string, tag = "1")]
        pub type_url: ::prost::alloc::string::String,
        #[prost(bytes, tag = "2")]
        pub value: ::prost::alloc::vec::Vec<u8>,
    }
}
