//! gRPC Server for FutureAgent
//!
//! This module implements the FutureAgent gRPC service using tonic.
//! The proto definition is in the ../proto/ directory.
//!
//! HTTP endpoints:
//! - POST / - RPC commands (JSON)
//! - GET /events - SSE event stream
//!
//! gRPC service: proto.FutureAgent (on grpc_port)

use crate::rpc::{handle_command_internal, AppState};
use anyhow::Result;
use std::net::SocketAddr;
use std::pin::Pin;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

// Include the generated proto code
pub mod proto {
    include!("generated/proto.rs");
}

/// Start a gRPC-only server (no HTTP).
pub async fn serve(state: AppState, host: &str, port: u16) -> Result<()> {
    eprintln!("gRPC server listening on {}:{}", host, port);

    // Build gRPC service
    let grpc_service = FutureAgentService { state };

    // Start gRPC server
    let grpc_addr: SocketAddr = format!("{}:{}", host, port).parse().unwrap();

    tonic::transport::Server::builder()
        .add_service(proto::future_agent_server::FutureAgentServer::new(
            grpc_service,
        ))
        .serve(grpc_addr)
        .await?;

    Ok(())
}

#[derive(Clone)]
struct FutureAgentService {
    state: AppState,
}

#[tonic::async_trait]
impl proto::future_agent_server::FutureAgent for FutureAgentService {
    type StreamEventsStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<proto::StreamEvent, tonic::Status>> + Send>>;
    async fn execute_command(
        &self,
        request: tonic::Request<proto::RpcCommand>,
    ) -> Result<tonic::Response<proto::RpcResponse>, tonic::Status> {
        let cmd = request.into_inner();

        // Log requests in verbose mode
        if self.state.verbose {
            eprintln!(
                "[grpc] {} session={} msg={:.80}",
                cmd.r#type,
                if cmd.session_id.is_empty() {
                    "-"
                } else {
                    &cmd.session_id
                },
                if cmd.message.is_empty() {
                    "-"
                } else {
                    &cmd.message
                }
            );
        }

        // Convert proto command to internal command
        let internal_images: Vec<crate::types::ImageContent> = cmd
            .images
            .into_iter()
            .map(|img| {
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
            })
            .collect();

        let internal_cmd = crate::rpc::RpcCommand {
            id: cmd.id,
            cmd_type: cmd.r#type,
            message: cmd.message,
            images: internal_images,
            streaming_behavior: cmd.streaming_behavior,
            parent_session: cmd.parent_session,
            model_id: cmd.model_id,
            level: cmd.level,
            mode: cmd.mode,
            custom_instructions: cmd.custom_instructions,
            enabled: cmd.enabled,
            command: cmd.command,
            session_id: cmd.session_id,
            entry_id: cmd.entry_id,
            name: cmd.name,
            system_prompt: cmd.system_prompt,
            tools: cmd.tools,
            ephemeral: cmd.ephemeral,
            cwd: cmd.cwd,
            enabled_models: Some(cmd.enabled_models),
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
            data: json_resp
                .data
                .map(|d| serde_json::to_string(&d).unwrap_or_default())
                .unwrap_or_default(),
            error: json_resp.error.unwrap_or_default(),
        };

        Ok(tonic::Response::new(proto_resp))
    }

    async fn stream_events(
        &self,
        request: tonic::Request<proto::StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        let req = request.into_inner();
        let session_id = req.session_id;

        let rx = if session_id.is_empty() {
            self.state.broadcaster.subscribe()
        } else {
            let session = self.state.get_session(&session_id);
            let sess = session.read().unwrap();
            if self.state.verbose {
                eprintln!(
                    "[stream] subscribe session={} has_msgs={}",
                    session_id,
                    sess.messages.read().unwrap().len()
                );
            }
            sess.broadcaster.subscribe()
        };

        // Convert broadcast receiver into a Stream, adding an initial ping.
        // Using StreamExt directly instead of async_stream::stream! gives tonic
        // a proper poll-based stream — yields are not buffered internally.
        let ping = tokio_stream::once(Ok(proto::StreamEvent {
            r#type: "ping".to_string(),
            data: r#"{"type":"ping"}"#.to_string(),
        }));
        let events = BroadcastStream::new(rx).map(|r| match r {
            Ok(event) => Ok(proto::StreamEvent {
                r#type: event.event_type,
                data: event.data,
            }),
            Err(e) => {
                eprintln!("SSE stream error: {}", e);
                Err(tonic::Status::internal(e.to_string()))
            }
        });
        let stream = ping.chain(events);

        Ok(tonic::Response::new(Box::pin(stream)))
    }
}
