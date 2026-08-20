//! gRPC Server for FutureAgent
//!
//! This module implements the FutureAgent gRPC service using tonic.
//! The proto definition is in the packages/rpc/proto/ directory.
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
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

// Generated proto code lives in the future-rpc crate (single codegen owner;
// typed-RPC milestone). Re-exported under the historical module name so call
// sites keep their `proto::...` paths.
pub use future_rpc::proto;

/// Start a gRPC-only server (no HTTP). Runs until process exit.
pub async fn serve(state: AppState, host: &str, port: u16) -> Result<()> {
    serve_with(state, host, port, std::future::pending()).await
}

/// `serve` with an injectable shutdown trigger so tests can drive the server
/// to a clean `Ok(())` completion (production never exits).
async fn serve_with(
    state: AppState,
    host: &str,
    port: u16,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<()> {
    tracing::info!("gRPC server listening on {}:{}", host, port);

    // Build gRPC service
    let grpc_service = FutureAgentService { state };

    // Start gRPC server
    let grpc_addr: SocketAddr = format!("{}:{}", host, port).parse().unwrap();

    // Raise the message-size limits above tonic's 4MB default. Image bytes no
    // longer cross the wire (the agent reads them from the path), but a large
    // session's get_session_entries / export_html response can still exceed 4MB.
    const MAX_GRPC_MESSAGE_SIZE: usize = 32 * 1024 * 1024;

    tonic::transport::Server::builder()
        .add_service(
            proto::future_agent_server::FutureAgentServer::new(grpc_service)
                .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE)
                .max_encoding_message_size(MAX_GRPC_MESSAGE_SIZE),
        )
        .serve_with_shutdown(grpc_addr, shutdown)
        .await?;

    Ok(())
}

#[derive(Clone)]
struct FutureAgentService {
    state: AppState,
}

/// Test seam driving execute_command's two transport-failure arms:
/// "__test_panic__" makes the blocking command task panic (JoinError), and
/// "__test_bad_json__" makes it return a non-JSON payload.
#[cfg(test)]
fn execute_command_test_override(cmd_type: &str) -> Option<String> {
    match cmd_type {
        "__test_panic__" => panic!("injected command task panic"),
        "__test_bad_json__" => Some("not json".to_string()),
        _ => None,
    }
}

#[allow(clippy::result_large_err)] // tonic stream items require `tonic::Status` directly.
fn map_broadcast_event(
    result: Result<crate::rpc::SseEvent, BroadcastStreamRecvError>,
    broadcaster: &crate::rpc::SseBroadcaster,
    session_id: &str,
) -> Result<proto::StreamEvent, tonic::Status> {
    match result {
        Ok(event) => {
            // Typed-RPC dual-write: encode the JSON payload into its typed
            // form alongside the unchanged `data` string (pass-through event
            // types return None and stay JSON-only).
            let payload = future_rpc::encode::event_payload(&event.event_type, &event.data);
            Ok(proto::StreamEvent {
                r#type: event.event_type,
                data: event.data,
                run_id: event.run_id,
                idx: event.idx,
                projection_snapshot: false,
                snapshot_events: Vec::new(),
                snapshot_cursor: 0,
                session_id: session_id.to_string(),
                epoch: event.epoch,
                event_id: event.event_id,
                timestamp: event.timestamp,
                session_idx: event.session_idx,
                run_sequence: event.run_sequence,
                payload,
            })
        }
        Err(error) => {
            // Non-atomic observers can remain subscribed across multiple runs,
            // so resolve the canonical run at the moment lag is observed rather
            // than freezing the run that happened to be active at subscribe.
            let run_id = broadcaster.current_run_id();
            let count = broadcaster.record_lag();
            tracing::warn!(
                session_id,
                run_id = %run_id,
                lag_count = count,
                error = %error,
                "SSE stream lagged; terminating for cursor resume"
            );
            Err(tonic::Status::data_loss(format!(
                "event stream gap for session {session_id}, run {run_id}; reconnect with atomic attach"
            )))
        }
    }
}

/// Proto string field → optional domain value: empty strings are "absent"
/// (proto3 has no presence for plain scalars; the config-write sub-messages
/// rely on this to mean "leave the field untouched").
fn nonempty(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
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
            tracing::debug!(
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
                    file_path: if img.file_path.is_empty() {
                        None
                    } else {
                        Some(img.file_path)
                    },
                }
            })
            .collect();

        let internal_attachments: Vec<crate::types::Attachment> = cmd
            .attachments
            .into_iter()
            .map(|att| crate::types::Attachment {
                path: att.path,
                kind: att.kind,
                name: att.name,
                thumbnail: if att.thumbnail.is_empty() {
                    None
                } else {
                    Some(att.thumbnail)
                },
            })
            .collect();

        let internal_cmd = crate::rpc::RpcCommand {
            id: cmd.id,
            cmd_type: cmd.r#type,
            message: cmd.message,
            model_context: cmd.model_context,
            images: internal_images,
            attachments: internal_attachments,
            parent_session: cmd.parent_session,
            model_id: cmd.model_id,
            level: cmd.level,
            mode: cmd.mode,
            custom_instructions: cmd.custom_instructions,
            created_by: cmd.created_by,
            source_meta: cmd.source_meta,
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
            run_id: cmd.run_id,
            since_idx: cmd.since_idx,
            max_events: cmd.max_events,
            requested_run_id: cmd.requested_run_id,
            client_request_id: cmd.client_request_id,
            busy_policy: cmd.busy_policy,
            include_builtin_providers: cmd.include_builtin_providers,
            sandbox_policy: cmd
                .sandbox_policy
                .map(|policy| crate::sandbox::SandboxPolicy {
                    tier: crate::sandbox::SandboxTier::parse(&policy.tier),
                }),
            auth_update: cmd
                .auth_update
                .map(|update| crate::config::providers::AuthMutation {
                    provider: update.provider,
                    key: nonempty(update.key),
                    clear_key: update.clear_key,
                    base_url: nonempty(update.base_url),
                    clear_base_url: update.clear_base_url,
                    remove_entry: update.remove_entry,
                    remove_platform_base_url: update.remove_platform_base_url,
                }),
            provider_config: cmd.provider_config.map(|config| {
                crate::config::providers::ProviderUpsertSpec {
                    id: config.id,
                    name: nonempty(config.name),
                    api: nonempty(config.api),
                    base_url: nonempty(config.base_url),
                    clear_base_url: config.clear_base_url,
                    models: config
                        .models
                        .into_iter()
                        .map(|model| crate::config::providers::ProviderModelSpec {
                            id: model.id,
                            name: model.name,
                            modalities: model.modalities,
                            context_window: model.context_window,
                            max_tokens: model.max_tokens,
                        })
                        .collect(),
                    replace_models: config.replace_models,
                    create_only: config.create_only,
                    api_key: nonempty(config.api_key),
                    clear_api_key: config.clear_api_key,
                }
            }),
        };

        // The command dispatcher still contains legacy synchronous JSONL and
        // shell paths. Keep those off tonic's async workers while the per-
        // session ordered persistence worker is introduced incrementally.
        let command_state = self.state.clone();
        let resp_str = tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            if let Some(override_resp) = execute_command_test_override(&internal_cmd.cmd_type) {
                return override_resp;
            }
            handle_command_internal(&command_state, internal_cmd)
        })
        .await
        .map_err(|error| tonic::Status::internal(format!("command task failed: {error}")))?;

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
            #[serde(default)]
            error_code: Option<String>,
            #[serde(default)]
            error_data: Option<serde_json::Value>,
        }

        let json_resp: JsonResp = serde_json::from_str(&resp_str)
            .map_err(|e| tonic::Status::internal(format!("Failed to parse response: {}", e)))?;

        // Typed payload (dual-write): encode the JSON payload into its typed
        // wire form for commands that have a ResponsePayload member. Untyped
        // commands keep serving the JSON `data` string only; clients always
        // fall back to `data` when `payload` is absent.
        let typed_payload = json_resp
            .data
            .as_ref()
            .and_then(|value| future_rpc::encode::response_payload(&json_resp.command, value));

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
            error_code: json_resp.error_code.unwrap_or_default(),
            error_data: json_resp
                .error_data
                .map(|data| serde_json::to_string(&data).unwrap_or_default())
                .unwrap_or_default(),
            payload: typed_payload,
        };

        Ok(tonic::Response::new(proto_resp))
    }

    #[allow(clippy::result_large_err)]
    async fn stream_events(
        &self,
        request: tonic::Request<proto::StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        let req = request.into_inner();
        let session_id = req.session_id;
        let event_types: std::collections::HashSet<String> = req.event_types.into_iter().collect();
        let filter_enabled = !event_types.is_empty();
        let atomic_attach = req.atomic_attach;
        let requested_run_id = req.run_id;
        let after_idx = req.after_idx;

        if req.global_events {
            if !session_id.is_empty() || atomic_attach || !requested_run_id.is_empty() {
                return Err(tonic::Status::invalid_argument(
                    "global_events cannot be combined with session/run attachment fields",
                ));
            }
            let broadcaster = crate::rpc::global_config_broadcaster();
            let rx = broadcaster.subscribe();
            let revision = crate::rpc::current_config_revision();
            let mut initial = vec![proto::StreamEvent {
                r#type: "ping".to_string(),
                data: serde_json::json!({
                    "type": "ping",
                    "configRevision": revision,
                })
                .to_string(),
                session_idx: -1,
                run_sequence: -1,
                ..Default::default()
            }];
            if filter_enabled {
                initial.retain(|event| event_types.contains(&event.r#type));
            }
            let filter_types = event_types.clone();
            let snapshot = tokio_stream::iter(initial.into_iter().map(Ok));
            let lag_broadcaster = broadcaster.clone();
            let events = BroadcastStream::new(rx)
                .filter(move |result| {
                    !filter_enabled
                        || result
                            .as_ref()
                            .map(|event| filter_types.contains(&event.event_type))
                            .unwrap_or(true)
                })
                .map(move |result| map_broadcast_event(result, &lag_broadcaster, ""));
            return Ok(tonic::Response::new(Box::pin(snapshot.chain(events))));
        }

        // Sessions are equal peers — every subscription must name its
        // session.  An empty id previously subscribed to a global/default
        // broadcaster, which could leak other sessions' events.
        if session_id.is_empty() {
            return Err(tonic::Status::failed_precondition(
                "session_id is required for StreamEvents",
            ));
        }
        let Some(session) = self.state.get_session(&session_id) else {
            return Err(tonic::Status::not_found(format!(
                "session {session_id} not found"
            )));
        };
        let (rx, mut initial_events, lag_broadcaster) = {
            let sess = session.read();
            if self.state.verbose {
                tracing::debug!(
                    "[stream] subscribe session={} has_msgs={}",
                    session_id,
                    sess.messages.read().len()
                );
            }
            if atomic_attach {
                let attachment = sess
                    .broadcaster
                    .attach(&requested_run_id, after_idx)
                    .map_err(|error| tonic::Status::failed_precondition(error.to_string()))?;
                let mut initial = Vec::new();
                if let Some(projection) = attachment.projection {
                    initial.push(proto::StreamEvent {
                        r#type: "run_snapshot".to_string(),
                        data: String::new(),
                        run_id: projection.run_id,
                        idx: projection.cursor,
                        projection_snapshot: true,
                        snapshot_events: projection
                            .events
                            .into_iter()
                            .map(|event| {
                                let payload = future_rpc::encode::event_payload(
                                    &event.event_type,
                                    &event.data,
                                );
                                proto::ProjectedRunEvent {
                                    r#type: event.event_type,
                                    data: event.data,
                                    idx: event.idx,
                                    payload,
                                }
                            })
                            .collect(),
                        snapshot_cursor: projection.cursor,
                        session_id: session_id.clone(),
                        epoch: projection.epoch,
                        event_id: String::new(),
                        timestamp: String::new(),
                        session_idx: -1,
                        run_sequence: projection.run_sequence,
                        payload: None,
                    });
                }
                initial.extend(attachment.events.into_iter().map(|event| {
                    let payload = future_rpc::encode::event_payload(&event.event_type, &event.data);
                    proto::StreamEvent {
                        r#type: event.event_type,
                        data: event.data,
                        run_id: event.run_id,
                        idx: event.idx,
                        projection_snapshot: false,
                        snapshot_events: Vec::new(),
                        snapshot_cursor: 0,
                        session_id: session_id.clone(),
                        epoch: event.epoch,
                        event_id: event.event_id,
                        timestamp: event.timestamp,
                        session_idx: event.session_idx,
                        run_sequence: event.run_sequence,
                        payload,
                    }
                }));
                (attachment.receiver, initial, sess.broadcaster.clone())
            } else {
                (
                    sess.broadcaster.subscribe(),
                    vec![proto::StreamEvent {
                        r#type: "ping".to_string(),
                        data: r#"{"type":"ping"}"#.to_string(),
                        run_id: String::new(),
                        idx: 0,
                        projection_snapshot: false,
                        snapshot_events: Vec::new(),
                        snapshot_cursor: 0,
                        session_id: session_id.clone(),
                        epoch: 0,
                        event_id: String::new(),
                        timestamp: String::new(),
                        session_idx: -1,
                        run_sequence: -1,
                        payload: None,
                    }],
                    sess.broadcaster.clone(),
                )
            }
        };

        if filter_enabled {
            initial_events.retain(|event| {
                event.projection_snapshot
                    || event.r#type == "stream_gap"
                    || event_types.contains(&event.r#type)
            });
        }

        // Clone for the lag-warning closure below — `session_id` is moved
        // into the `ping` stream and can't be borrowed across the chain.
        let lag_session_id = session_id.clone();

        let snapshot = tokio_stream::iter(initial_events.into_iter().map(Ok));
        let events = BroadcastStream::new(rx)
            .filter(move |r| {
                if !filter_enabled {
                    return true;
                }
                match r {
                    Ok(event) => event_types.contains(&event.event_type),
                    // Lag must bypass event-type filtering so the mapper below
                    // can terminate the gRPC stream explicitly. The client then
                    // reconnects with atomic attach and recovers from its cursor.
                    Err(_) => true,
                }
            })
            .map(move |result| map_broadcast_event(result, &lag_broadcaster, &lag_session_id));
        let stream = snapshot.chain(events);

        Ok(tonic::Response::new(Box::pin(stream)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::{SseBroadcaster, SseEvent};
    use proto::future_agent_server::FutureAgent;
    use std::sync::Arc;
    use tokio_stream::wrappers::ReceiverStream;

    /// Provider double for the service harness: an immediately-empty stream.
    struct NoopProvider;

    #[async_trait::async_trait]
    impl crate::types::LLMProvider for NoopProvider {
        async fn stream_model(
            &self,
            _request: crate::llm::schema::ModelRequest,
        ) -> anyhow::Result<ReceiverStream<crate::llm::schema::ModelStreamEvent>> {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(ReceiverStream::new(rx))
        }
    }

    #[tokio::test]
    async fn broadcast_overflow_records_lag_through_grpc_mapper() {
        let broadcaster = SseBroadcaster::new();
        broadcaster.start_run("run-lag".to_string(), 7);
        let receiver = broadcaster.subscribe();

        // The production broadcast channel holds 256 events. Keeping this
        // receiver idle while publishing more than that deterministically
        // produces BroadcastStreamRecvError::Lagged on its first read.
        for idx in 0..300 {
            broadcaster.broadcast(SseEvent::new(
                "text_chunk",
                serde_json::json!({"text": idx.to_string()}),
            ));
        }
        let result = BroadcastStream::new(receiver)
            .next()
            .await
            .expect("lagged stream yields an item");
        assert!(matches!(&result, Err(BroadcastStreamRecvError::Lagged(_))));

        let status = map_broadcast_event(result, &broadcaster, "session-lag").unwrap_err();
        assert_eq!(status.code(), tonic::Code::DataLoss);
        assert!(status.message().contains("session-lag"));
        assert!(status.message().contains("run-lag"));
        assert_eq!(broadcaster.lag_count(), 1);
    }

    // ─── service-level tests (direct handler calls) ─────────────────────────

    fn grpc_app_state(verbose: bool) -> AppState {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cwd = std::env::temp_dir()
            .join(format!("futureos-grpc-test-{stamp}"))
            .to_string_lossy()
            .to_string();
        let session_dir = std::env::temp_dir().join(format!(
            "futureos-grpc-sess-{}",
            crate::utils::generate_id()
        ));
        let model_registry = Arc::new(parking_lot::RwLock::new(crate::models::Registry::new()));
        let session_manager = Arc::new(crate::session::Manager::new(session_dir));
        let approval_gate = crate::rpc::ApprovalGate::default();
        let queue_budget = Arc::new(crate::runtime::GlobalQueueBudget::defaults());
        let session = crate::rpc::ServerSession::new_with_queue_budget(
            "default".to_string(),
            Arc::new(tokio::sync::RwLock::new(crate::agent::Loop::new(
                Arc::new(NoopProvider),
                "mock",
            ))),
            session_manager.clone(),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            approval_gate.clone(),
            model_registry.clone(),
            queue_budget.clone(),
        );
        let sessions: std::collections::HashMap<
            String,
            Arc<parking_lot::RwLock<crate::rpc::ServerSession>>,
        > = [(
            "default".to_string(),
            Arc::new(parking_lot::RwLock::new(session)),
        )]
        .into_iter()
        .collect();
        AppState {
            agent_instance_id: "grpc-test".to_string(),
            sessions: Arc::new(parking_lot::RwLock::new(sessions)),
            queue_budget,
            session_manager,
            welcome_version: "0.0.0".to_string(),
            welcome_cwd: cwd,
            welcome_skills: Arc::new(parking_lot::RwLock::new(vec![])),
            welcome_context: Arc::new(parking_lot::RwLock::new(vec![])),
            welcome_exts: vec![],
            explicit_session: false,
            approval_gate,
            verbose,
            shutting_down: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            model_registry,
            loop_template: Arc::new(crate::agent::Loop::new(Arc::new(NoopProvider), "mock")),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_converts_full_proto_command() {
        let service = FutureAgentService {
            state: grpc_app_state(true), // verbose logging branch
        };
        let cmd = proto::RpcCommand {
            id: "cmd-1".to_string(),
            r#type: "get_agent_info".to_string(),
            session_id: "default".to_string(),
            images: vec![
                proto::ImageContent {
                    r#type: "image".to_string(),
                    file_path: "/tmp/pic.png".to_string(),
                    content: Some(proto::image_content::Content::Url(
                        "https://example.com/p.png".to_string(),
                    )),
                },
                proto::ImageContent {
                    r#type: "image".to_string(),
                    file_path: String::new(),
                    content: Some(proto::image_content::Content::Base64("aGk=".to_string())),
                },
                proto::ImageContent {
                    r#type: "image".to_string(),
                    file_path: String::new(),
                    content: None,
                },
            ],
            attachments: vec![
                proto::Attachment {
                    path: "/tmp/a.pdf".to_string(),
                    kind: "file".to_string(),
                    name: "a.pdf".to_string(),
                    thumbnail: "/tmp/a.thumb".to_string(),
                },
                proto::Attachment {
                    path: "/tmp/b.pdf".to_string(),
                    kind: "file".to_string(),
                    name: "b.pdf".to_string(),
                    thumbnail: String::new(),
                },
            ],
            sandbox_policy: Some(proto::SandboxPolicy {
                tier: "manual".to_string(),
            }),
            auth_update: Some(proto::AuthUpdate {
                provider: "custom".to_string(),
                key: "k".to_string(),
                clear_key: false,
                base_url: "https://x".to_string(),
                clear_base_url: false,
                remove_entry: false,
                remove_platform_base_url: false,
            }),
            provider_config: Some(proto::ProviderUpsert {
                id: "custom".to_string(),
                name: "Custom".to_string(),
                api: "openai".to_string(),
                base_url: "https://x".to_string(),
                clear_base_url: false,
                models: vec![proto::ProviderModel {
                    id: "m1".to_string(),
                    name: "Model One".to_string(),
                    modalities: vec!["text".to_string()],
                    context_window: 128000,
                    max_tokens: 16384,
                }],
                replace_models: true,
                create_only: false,
                api_key: "k".to_string(),
                clear_api_key: false,
            }),
            message: "hello".to_string(),
            ..Default::default()
        };
        let response = service
            .execute_command(tonic::Request::new(cmd))
            .await
            .expect("command succeeds");
        let resp = response.into_inner();
        assert!(resp.success, "{}", resp.error);
        assert_eq!(resp.id, "cmd-1");
        assert_eq!(resp.command, "get_agent_info");
        assert!(resp.data.contains("agentInstanceId"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_unknown_command_returns_error_envelope() {
        let service = FutureAgentService {
            state: grpc_app_state(false),
        };
        let cmd = proto::RpcCommand {
            id: "cmd-2".to_string(),
            r#type: "frobnicate".to_string(),
            session_id: "default".to_string(),
            ..Default::default()
        };
        let response = service
            .execute_command(tonic::Request::new(cmd))
            .await
            .expect("handler returns Ok envelope");
        let resp = response.into_inner();
        assert!(!resp.success);
        assert!(resp.error.contains("unknown command"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_get_state_emits_typed_payload() {
        let service = FutureAgentService {
            state: grpc_app_state(false),
        };
        let cmd = proto::RpcCommand {
            id: "cmd-3".to_string(),
            r#type: "get_state".to_string(),
            session_id: "default".to_string(),
            ..Default::default()
        };
        let response = service
            .execute_command(tonic::Request::new(cmd))
            .await
            .expect("command succeeds");
        let resp = response.into_inner();
        assert!(resp.success, "{}", resp.error);
        assert!(resp.data.contains("\"sessionId\":\"default\""));
        // get_state is a Tier-1 command — the typed payload is dual-written.
        assert!(resp.payload.is_some(), "typed payload present");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_events_requires_and_resolves_session() {
        let service = FutureAgentService {
            state: grpc_app_state(true),
        };

        // Empty session id → failed_precondition.
        let err = service
            .stream_events(tonic::Request::new(proto::StreamRequest::default()))
            .await
            .map(|_| ())
            .expect_err("expected an error");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);

        // Unknown session → not_found.
        let err = service
            .stream_events(tonic::Request::new(proto::StreamRequest {
                session_id: "ghost".to_string(),
                ..Default::default()
            }))
            .await
            .map(|_| ())
            .expect_err("expected an error");
        assert_eq!(err.code(), tonic::Code::NotFound);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn global_stream_reports_committed_provider_revision() {
        let service = FutureAgentService {
            state: grpc_app_state(false),
        };
        let response = service
            .stream_events(tonic::Request::new(proto::StreamRequest {
                event_types: vec!["provider_config_changed".to_string()],
                global_events: true,
                ..Default::default()
            }))
            .await
            .expect("global subscription succeeds");
        let mut stream = response.into_inner();
        let revision = crate::rpc::publish_provider_config_changed("custom", "updated", true, true);
        let event = stream.next().await.unwrap().unwrap();
        assert_eq!(event.r#type, "provider_config_changed");
        let data: serde_json::Value = serde_json::from_str(&event.data).unwrap();
        assert_eq!(data["revision"], revision);
        assert_eq!(data["providerId"], "custom");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_events_pings_then_streams_broadcasts() {
        let service = FutureAgentService {
            state: grpc_app_state(false),
        };
        let session = service.state.get_session("default").unwrap();
        let response = service
            .stream_events(tonic::Request::new(proto::StreamRequest {
                session_id: "default".to_string(),
                ..Default::default()
            }))
            .await
            .expect("subscribes");
        let mut stream = response.into_inner();
        // First event is the initial ping.
        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first.r#type, "ping");
        // Broadcasts arrive with the session id stamped by the mapper.
        session.read().broadcaster.broadcast(SseEvent::new(
            "session_name_changed",
            serde_json::json!({"name": "renamed"}),
        ));
        let second = stream.next().await.unwrap().unwrap();
        assert_eq!(second.r#type, "session_name_changed");
        assert_eq!(second.session_id, "default");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_events_applies_event_type_filter() {
        let service = FutureAgentService {
            state: grpc_app_state(false),
        };
        let session = service.state.get_session("default").unwrap();
        let response = service
            .stream_events(tonic::Request::new(proto::StreamRequest {
                session_id: "default".to_string(),
                event_types: vec!["model_changed".to_string()],
                ..Default::default()
            }))
            .await
            .expect("subscribes");
        let mut stream = response.into_inner();
        // The initial ping is filtered out (not in event_types).
        session.read().broadcaster.broadcast(SseEvent::new(
            "thinking_level_changed",
            serde_json::json!({}),
        ));
        session.read().broadcaster.broadcast(SseEvent::new(
            "model_changed",
            serde_json::json!({"model": "mock"}),
        ));
        let event = stream.next().await.unwrap().unwrap();
        assert_eq!(event.r#type, "model_changed");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_events_atomic_attach_replays_active_run() {
        let service = FutureAgentService {
            state: grpc_app_state(false),
        };
        let session = service.state.get_session("default").unwrap();
        {
            let sess = session.read();
            sess.broadcaster.start_run("run-live".to_string(), 1);
            sess.broadcaster.broadcast(SseEvent::new(
                "text_chunk",
                serde_json::json!({"text": "in-flight"}),
            ));
        }
        let response = service
            .stream_events(tonic::Request::new(proto::StreamRequest {
                session_id: "default".to_string(),
                atomic_attach: true,
                run_id: "run-live".to_string(),
                after_idx: -1,
                ..Default::default()
            }))
            .await
            .expect("atomic attach succeeds");
        let mut stream = response.into_inner();
        let event = stream.next().await.unwrap().unwrap();
        assert_eq!(event.r#type, "text_chunk");
        assert_eq!(event.run_id, "run-live");

        // Attaching to a different (stale) run fails.
        let err = service
            .stream_events(tonic::Request::new(proto::StreamRequest {
                session_id: "default".to_string(),
                atomic_attach: true,
                run_id: "run-stale".to_string(),
                after_idx: -1,
                ..Default::default()
            }))
            .await
            .map(|_| ())
            .expect_err("expected an error");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }

    #[test]
    fn nonempty_maps_empty_to_none() {
        assert_eq!(nonempty(String::new()), None);
        assert_eq!(nonempty("x".to_string()), Some("x".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn serve_with_shutdown_completes_cleanly() {
        let state = grpc_app_state(false);
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve_with(state, "127.0.0.1", 0, async move {
            let _ = rx.await;
        }));
        // Give the server a moment to bind, then trigger shutdown.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let _ = tx.send(());
        tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .expect("server shuts down promptly")
            .expect("server task did not panic")
            .expect("clean shutdown is Ok");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_serializes_error_data() {
        let service = FutureAgentService {
            state: grpc_app_state(false),
        };
        // prompt with an unknown busy policy → error_code + error_data.
        let cmd = proto::RpcCommand {
            id: "cmd-err".to_string(),
            r#type: "prompt".to_string(),
            session_id: "default".to_string(),
            message: "hello".to_string(),
            busy_policy: "frobnicate".to_string(),
            ..Default::default()
        };
        let resp = service
            .execute_command(tonic::Request::new(cmd))
            .await
            .expect("command executes")
            .into_inner();
        assert!(!resp.success);
        assert_eq!(resp.error_code, "invalid_busy_policy");
        assert!(
            resp.error_data.contains("frobnicate"),
            "error_data must be serialized onto the wire: {}",
            resp.error_data
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_maps_task_panic_and_bad_json_to_internal() {
        let service = FutureAgentService {
            state: grpc_app_state(false),
        };
        // The command task itself panicked → JoinError → internal status.
        let err = service
            .execute_command(tonic::Request::new(proto::RpcCommand {
                id: "cmd-panic".to_string(),
                r#type: "__test_panic__".to_string(),
                session_id: "default".to_string(),
                ..Default::default()
            }))
            .await
            .expect_err("panic surfaces as an error");
        assert_eq!(err.code(), tonic::Code::Internal);
        assert!(err.message().contains("command task failed"), "{err}");

        // The dispatcher returned non-JSON → parse failure → internal status.
        let err = service
            .execute_command(tonic::Request::new(proto::RpcCommand {
                id: "cmd-badjson".to_string(),
                r#type: "__test_bad_json__".to_string(),
                session_id: "default".to_string(),
                ..Default::default()
            }))
            .await
            .expect_err("bad json surfaces as an error");
        assert_eq!(err.code(), tonic::Code::Internal);
        assert!(err.message().contains("Failed to parse response"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_events_atomic_attach_includes_projection_snapshot_events() {
        let service = FutureAgentService {
            state: grpc_app_state(false),
        };
        let session = service.state.get_session("default").unwrap();
        {
            let mut sess = session.write();
            // A journal-less broadcaster: a truncated attach then has no disk
            // to fall back on and returns the projection snapshot instead.
            sess.broadcaster = std::sync::Arc::new(SseBroadcaster::new());
            sess.broadcaster.start_run("run-long".to_string(), 1);
            // Overflow the in-memory ring (MAX_RUN_EVENTS) so an attach at
            // after_idx=-1 is truncated → projection snapshot path.
            for idx in 0..2100 {
                sess.broadcaster.broadcast(SseEvent::new(
                    "text_chunk",
                    serde_json::json!({"text": format!("tok{idx} ")}),
                ));
            }
        }
        let response = service
            .stream_events(tonic::Request::new(proto::StreamRequest {
                session_id: "default".to_string(),
                atomic_attach: true,
                run_id: "run-long".to_string(),
                after_idx: -1,
                ..Default::default()
            }))
            .await
            .expect("truncated attach succeeds");
        let mut stream = response.into_inner();
        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first.r#type, "run_snapshot");
        assert!(first.projection_snapshot);
        assert!(
            !first.snapshot_events.is_empty(),
            "projection carries the coalesced event stream"
        );
        assert_eq!(first.snapshot_events[0].r#type, "text_chunk");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_events_lag_bypasses_type_filter_as_dataloss() {
        let service = FutureAgentService {
            state: grpc_app_state(false),
        };
        // Subscribe with a type filter, then overflow the broadcast channel
        // (capacity 256) without draining: the next poll yields Lagged,
        // which must bypass the filter and surface as DataLoss.
        let response = service
            .stream_events(tonic::Request::new(proto::StreamRequest {
                session_id: "default".to_string(),
                event_types: vec!["text_chunk".to_string()],
                ..Default::default()
            }))
            .await
            .expect("filtered subscribe succeeds");
        {
            let sess = service.state.get_session("default").unwrap();
            let sess = sess.read();
            sess.broadcaster.start_run("run-flood".to_string(), 1);
            for idx in 0..300 {
                sess.broadcaster.broadcast(SseEvent::new(
                    "text_chunk",
                    serde_json::json!({"text": idx.to_string()}),
                ));
            }
        }
        let mut stream = response.into_inner();
        let item = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .expect("lagged item arrives")
            .expect("stream yields the lag");
        let status = item.expect_err("lag maps to an error status");
        assert_eq!(status.code(), tonic::Code::DataLoss);
    }

    #[tokio::test]
    async fn noop_provider_stream_model_is_an_empty_stream() {
        // Exercise the harness provider double's stream body directly.
        use crate::types::LLMProvider;
        use tokio_stream::StreamExt;
        let mut stream = NoopProvider
            .stream_model(crate::llm::schema::ModelRequest {
                model: "m".into(),
                system_prompt: String::new(),
                messages: vec![],
                tools: vec![],
            })
            .await
            .unwrap();
        assert!(stream.next().await.is_none());
    }
}
