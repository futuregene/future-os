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

use crate::rpc::{AppState, handle_command_internal};
use anyhow::Result;
use std::net::SocketAddr;
use std::pin::Pin;
use tokio::sync::broadcast;
use tokio_stream::Stream;


// Include the generated proto code
pub mod proto {
    include!("generated/proto.rs");
}

/// Start a gRPC-only server (no HTTP).
pub async fn serve(
    state: AppState,
    host: &str,
    port: u16,
) -> Result<()> {
    eprintln!("gRPC server listening on {}:{}", host, port);
    
    // Build gRPC service
    let grpc_service = XihuAgentService { state };
    
    // Start gRPC server
    let grpc_addr: SocketAddr = format!("{}:{}", host, port).parse().unwrap();
    
    tonic::transport::Server::builder()
        .add_service(proto::xihu_agent_server::XihuAgentServer::new(grpc_service))
        .serve(grpc_addr)
        .await?;
    
    Ok(())
}

#[derive(Clone)]
struct XihuAgentService {
    state: AppState,
}

#[tonic::async_trait]
impl proto::xihu_agent_server::XihuAgent for XihuAgentService {
    async fn execute_command(
        &self,
        request: tonic::Request<proto::RpcCommand>,
    ) -> Result<tonic::Response<proto::RpcResponse>, tonic::Status> {
        let cmd = request.into_inner();
        
        // Convert proto command to internal command
        let internal_images: Vec<crate::types::ImageContent> = cmd.images.into_iter().map(|img| {
            let (data, source) = match img.content {
                Some(proto::image_content::Content::Url(url)) => (
                    Some(url.clone()),
                    Some(crate::types::ImageSource {
                        source_type: "url".to_string(),
                        media_type: String::new(),
                        data: url,
                    }),
                ),
                Some(proto::image_content::Content::Base64(base64)) => (
                    Some(base64.clone()),
                    Some(crate::types::ImageSource {
                        source_type: "base64".to_string(),
                        media_type: String::new(),
                        data: base64,
                    }),
                ),
                None => (None, None),
            };
            crate::types::ImageContent {
                content_type: img.r#type,
                mime_type: None,
                data,
                source,
            }
        }).collect();
        
        let internal_cmd = crate::rpc::RpcCommand {
            id: cmd.id,
            cmd_type: cmd.r#type,
            message: cmd.message,
            images: internal_images,
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
        
        // Convert to proto response - error is Option<String>, need to handle None
        let proto_resp = proto::RpcResponse {
            id: json_resp.id,
            r#type: json_resp.resp_type,
            command: json_resp.command,
            success: json_resp.success,
            data: json_resp.data.map(|d| serde_json::to_string(&d).unwrap_or_default()).unwrap_or_default(),
            error: json_resp.error.unwrap_or_default(),
        };
        
        Ok(tonic::Response::new(proto_resp))
    }
    
    type StreamEventsStream = Pin<Box<dyn Stream<Item = Result<proto::StreamEvent, tonic::Status>> + Send>>;
    
    async fn stream_events(
        &self,
        _request: tonic::Request<proto::StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        let mut rx = self.state.broadcaster.subscribe();
        
        let stream = async_stream::stream! {
            // Send initial ping
            yield Ok(proto::StreamEvent {
                r#type: "ping".to_string(),
                data: r#"{"type":"ping"}"#.to_string(),
            });
            
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        yield Ok(proto::StreamEvent {
                            r#type: event.event_type,
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
    
    type ExtensionUIStream = Pin<Box<dyn Stream<Item = Result<proto::ExtensionUiResponse, tonic::Status>> + Send>>;
    
    async fn extension_ui(
        &self,
        _request: tonic::Request<tonic::Streaming<proto::ExtensionUiRequest>>,
    ) -> Result<tonic::Response<Self::ExtensionUIStream>, tonic::Status> {
        Err(tonic::Status::unimplemented("Extension UI not yet implemented"))
    }
}
