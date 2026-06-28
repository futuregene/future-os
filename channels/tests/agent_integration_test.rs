//! Integration test: verify agent processes prompts correctly.
//! Requires a running agent on 127.0.0.1:50051.
//! Run with: cargo test --test agent_integration_test -- --nocapture --ignored

use std::time::Duration;
use tokio::time::timeout;

// Reuse the channel's gRPC client types
mod proto {
    tonic::include_proto!("proto");
}

use proto::future_agent_client::FutureAgentClient;
use proto::{RpcCommand, StreamRequest};

#[derive(Debug, Clone)]
enum AgentEvent {
    TextChunk(String),
    ThinkingStart,
    ThinkingDelta(String),
    ThinkingEnd,
    ToolStart {
        tool_id: String,
        tool_name: String,
    },
    ToolEnd {
        tool_id: String,
        text: Option<String>,
    },
    AgentStart,
    AgentEnd {
        error: Option<String>,
    },
    Error(String),
    Ping,
}

async fn call(
    client: &mut FutureAgentClient<tonic::transport::Channel>,
    cmd_type: &str,
    session_id: &str,
    extra: RpcCommand,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let request = tonic::Request::new(RpcCommand {
        id: uuid::Uuid::new_v4().to_string(),
        r#type: cmd_type.to_string(),
        session_id: session_id.to_string(),
        ..extra
    });
    let response = client.execute_command(request).await?.into_inner();
    if !response.success {
        return Err(format!("Command '{}' failed: {}", cmd_type, response.error).into());
    }
    if response.data.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    Ok(serde_json::from_str(&response.data)?)
}

fn parse_event(event: proto::StreamEvent) -> Option<AgentEvent> {
    match event.r#type.as_str() {
        "ping" => Some(AgentEvent::Ping),
        "agent_start" => Some(AgentEvent::AgentStart),
        "agent_end" => {
            let error = serde_json::from_str::<serde_json::Value>(&event.data)
                .ok()
                .and_then(|d| d["error"].as_str().map(|s| s.to_string()));
            Some(AgentEvent::AgentEnd { error })
        }
        "text_chunk" => {
            let text = serde_json::from_str::<serde_json::Value>(&event.data)
                .ok()
                .and_then(|d| d["text"].as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            Some(AgentEvent::TextChunk(text))
        }
        "thinking_start" => Some(AgentEvent::ThinkingStart),
        "thinking_delta" => {
            let text = serde_json::from_str::<serde_json::Value>(&event.data)
                .ok()
                .and_then(|d| d["text"].as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            Some(AgentEvent::ThinkingDelta(text))
        }
        "thinking_end" => Some(AgentEvent::ThinkingEnd),
        "tool_start" => {
            let data = serde_json::from_str::<serde_json::Value>(&event.data).ok()?;
            Some(AgentEvent::ToolStart {
                tool_id: data["tool_id"].as_str().unwrap_or("").to_string(),
                tool_name: data["tool_name"].as_str().unwrap_or("").to_string(),
            })
        }
        "tool_end" => {
            let data = serde_json::from_str::<serde_json::Value>(&event.data).ok()?;
            Some(AgentEvent::ToolEnd {
                tool_id: data["tool_id"].as_str().unwrap_or("").to_string(),
                text: data["text"].as_str().map(|s| s.to_string()),
            })
        }
        "error" => {
            let msg = serde_json::from_str::<serde_json::Value>(&event.data)
                .ok()
                .and_then(|d| d["error"].as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown error".to_string());
            Some(AgentEvent::Error(msg))
        }
        _ => None,
    }
}

#[tokio::test]
#[ignore] // Requires running agent
async fn test_agent_prompt_flow() {
    let addr = "http://127.0.0.1:50051";
    let channel = tonic::transport::Endpoint::new(addr.to_string())
        .unwrap()
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
        .expect("Failed to connect to agent");
    let mut client = FutureAgentClient::new(channel);

    // 1. Create a new session
    println!("--- Creating new session ---");
    let resp = call(
        &mut client,
        "new_session",
        "",
        RpcCommand {
            cwd: "/tmp".to_string(),
            ..Default::default()
        },
    )
    .await
    .expect("new_session failed");
    let session_id = resp["sessionId"].as_str().unwrap().to_string();
    println!("Session created: {}", session_id);

    // 2. Send a prompt
    println!("--- Sending prompt ---");
    call(
        &mut client,
        "prompt",
        &session_id,
        RpcCommand {
            message: "Reply with exactly: pong".to_string(),
            ..Default::default()
        },
    )
    .await
    .expect("prompt failed");
    println!("Prompt sent");

    // 3. Stream events and collect the response
    println!("--- Streaming events ---");
    let stream_request = tonic::Request::new(StreamRequest {
        session_id: session_id.clone(),
        event_types: vec![],
    });
    let mut stream = client
        .stream_events(stream_request)
        .await
        .expect("stream_events failed")
        .into_inner();

    let mut response_text = String::new();
    let deadline = Duration::from_secs(60);

    loop {
        match timeout(Duration::from_secs(2), stream.message()).await {
            Ok(Ok(Some(event))) => {
                if let Some(parsed) = parse_event(event) {
                    match &parsed {
                        AgentEvent::TextChunk(text) => {
                            response_text.push_str(text);
                            print!("{}", text);
                        }
                        AgentEvent::ThinkingDelta(text) => {
                            print!("[think:{}]", text);
                        }
                        AgentEvent::ToolStart { tool_name, .. } => {
                            println!("\n[TOOL:{}]", tool_name);
                        }
                        AgentEvent::ToolEnd { text, .. } => {
                            if let Some(t) = text {
                                println!("[TOOL RESULT: {:.100}]", t);
                            }
                        }
                        AgentEvent::AgentEnd { error } => {
                            if let Some(err) = error {
                                println!("\nAgent ended with error: {}", err);
                            } else {
                                println!("\nAgent ended successfully");
                            }
                            break;
                        }
                        AgentEvent::Error(msg) => {
                            println!("\nError: {}", msg);
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Ok(Ok(None)) => {
                println!("\nStream ended");
                break;
            }
            Ok(Err(e)) => {
                println!("\nStream error: {}", e);
                break;
            }
            Err(_) => {
                // 2s timeout — check if we've been waiting too long
                if tokio::time::Instant::now()
                    .duration_since(tokio::time::Instant::now() - Duration::from_secs(0))
                    .as_secs()
                    > deadline.as_secs()
                {
                    println!("\nTimeout after {} seconds", deadline.as_secs());
                    break;
                }
                println!("(waiting...)");
            }
        }
    }

    println!("\n=== Full response ===");
    println!("{}", response_text);
    assert!(
        !response_text.is_empty(),
        "Should have received a response from the agent"
    );
    println!("\n✅ Agent prompt flow works correctly");
}

#[tokio::test]
#[ignore] // Requires running agent
async fn test_old_session_prompt_flow() {
    // Test what happens when channel uses an old session ID after agent restart.
    let session_id =
        std::env::var("OLD_SESSION_ID").unwrap_or_else(|_| "20260604-171411-78b7bb".to_string());
    println!("Using old session ID: {}", session_id);

    let addr = "http://127.0.0.1:50051";
    let channel = tonic::transport::Endpoint::new(addr.to_string())
        .unwrap()
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
        .expect("Failed to connect to agent");
    let mut client = FutureAgentClient::new(channel);

    // Simulate what channel does for existing sessions:
    // 1. Try switch_session (channel code sends it but it's broken with empty session_id)
    println!("--- Simulating channel ensure_session: switch_session ---");
    let switch_resp = call(
        &mut client,
        "switch_session",
        &session_id,
        RpcCommand {
            session_id: session_id.clone(),
            ..Default::default()
        },
    )
    .await;
    println!(
        "switch_session result: {:?}",
        switch_resp.as_ref().map(|_| "ok").unwrap_or_else(|e| "err")
    );

    // 2. Send prompt with old session ID
    println!("--- Sending prompt with OLD session ID ---");
    call(
        &mut client,
        "prompt",
        &session_id,
        RpcCommand {
            message: "reply pong".to_string(),
            ..Default::default()
        },
    )
    .await
    .expect("prompt failed");
    println!("Prompt sent");

    // 3. Stream events
    println!("--- Streaming events ---");
    let stream_request = tonic::Request::new(StreamRequest {
        session_id: session_id.clone(),
        event_types: vec![],
    });
    let mut stream = client
        .stream_events(stream_request)
        .await
        .expect("stream_events failed")
        .into_inner();

    let mut response_text = String::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(5), stream.message()).await {
            Ok(Ok(Some(event))) => {
                if let Some(parsed) = parse_event(event) {
                    match &parsed {
                        AgentEvent::TextChunk(text) => {
                            response_text.push_str(text);
                            print!("{}", text);
                        }
                        AgentEvent::AgentEnd { error } => {
                            if let Some(err) = error {
                                println!("\nAgent ended with error: {}", err);
                            } else {
                                println!("\nAgent ended successfully");
                            }
                            break;
                        }
                        AgentEvent::Error(msg) => {
                            println!("\nError: {}", msg);
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Ok(Ok(None)) => {
                println!("\nStream ended");
                break;
            }
            Ok(Err(e)) => {
                println!("\nStream error: {}", e);
                break;
            }
            Err(_) => {
                println!("\nTimeout waiting for events");
                break;
            }
        }
    }

    println!("\n=== Full response ===");
    println!("{}", response_text);
    assert!(
        !response_text.is_empty(),
        "Should have received a response from the agent"
    );
    println!("\n✅ Old session prompt flow works correctly");
}
