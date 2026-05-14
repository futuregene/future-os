//! gRPC Server for xihu agent
//! 
//! This module implements combined HTTP + gRPC server for the xihu agent.
//! 
//! HTTP endpoints:
//! - POST / - RPC commands (JSON)
//! - GET /events - SSE event stream
//! 
//! gRPC service: xihu.XihuAgent (on grpc_port)
//! 
//! Note: Full gRPC service implementation requires proto codegen from xihu-proto

use crate::rpc::{AppState, handle_command_internal, RpcResponse};
use anyhow::Result;
use std::net::SocketAddr;
use tokio::sync::broadcast;

/// Start a combined HTTP + gRPC server.
/// 
/// HTTP endpoints:
/// - POST / - RPC commands (JSON)  
/// - GET /events - SSE event stream
/// 
/// gRPC service: xihu.XihuAgent (on grpc_port)
pub async fn serve_combined(
    state: AppState,
    http_port: u16,
    grpc_port: u16,
) -> Result<()> {
    use axum::{routing::get, routing::post, extract::State, Json, Router};
    use axum::response::sse::{Event, Sse};
    use futures::Stream;
    use std::pin::Pin;
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
    
    eprintln!("HTTP server listening on 0.0.0.0:{}", http_port);
    eprintln!("gRPC server would listen on 0.0.0.0:{} (proto codegen pending)", grpc_port);
    
    // Start HTTP server
    let http_addr: SocketAddr = format!("0.0.0.0:{}", http_port).parse().unwrap();
    let listener = tokio::net::TcpListener::bind(http_addr).await?;
    axum::serve(listener, app).await?;
    
    Ok(())
}
