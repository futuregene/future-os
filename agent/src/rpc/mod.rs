//! RPC Server - Command handling for gRPC

use crate::session::Manager;
use crate::types::ConvertToLLM;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;
use crate::events::EventBus;

// ─── RPC Command (stdin) ────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcCommand {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type")]
    pub cmd_type: String,

    // Prompting
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub images: Vec<crate::types::ImageContent>,
    #[serde(default)]
    pub streaming_behavior: String,

    // new_session
    #[serde(default)]
    pub parent_session: String,

    // set_model
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model_id: String,

    // set_thinking_level
    #[serde(default)]
    pub level: String,

    // set_steering_mode / set_follow_up_mode
    #[serde(default)]
    pub mode: String,

    // compact
    #[serde(default)]
    pub custom_instructions: String,

    // set_auto_compaction / set_auto_retry
    #[serde(default)]
    pub enabled: bool,

    // bash
    #[serde(default)]
    pub command: String,

    // Session
    #[serde(default)]
    pub session_path: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub entry_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub output_path: String,
}

// ─── RPC Response (stdout) ───────────────────────────────────────────────

// ─── RPC Response (stdout) ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    #[serde(rename = "type")]
    pub resp_type: String,
    #[serde(default)]
    pub id: String,
    pub command: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl RpcResponse {
    fn ok(id: &str, command: &str, data: impl Into<serde_json::Value>) -> String {
        let resp = Self {
            resp_type: "response".to_string(),
            id: id.to_string(),
            command: command.to_string(),
            success: true,
            data: Some(data.into()),
            error: None,
        };
        serde_json::to_string(&resp).unwrap_or_default()
    }

    pub fn build_fail(id: &str, command: &str, err: &str) -> String {
        let resp = Self {
            resp_type: "response".to_string(),
            id: id.to_string(),
            command: command.to_string(),
            success: false,
            data: None,
            error: Some(err.to_string()),
        };
        serde_json::to_string(&resp).unwrap_or_default()
    }
}

// ─── SSE Event Broadcaster ──────────────────────────────────────────────

/// Global SSE broadcaster shared across all connections
#[derive(Clone)]
pub struct SseBroadcaster {
    tx: broadcast::Sender<SseEvent>,
}

impl SseBroadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(64);
        Self { tx }
    }

    /// Subscribe to SSE events
    pub fn subscribe(&self) -> broadcast::Receiver<SseEvent> {
        self.tx.subscribe()
    }

    /// Broadcast an event to all subscribers
    pub fn broadcast(&self, event: SseEvent) {
        let _ = self.tx.send(event);
    }
}

impl Default for SseBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

/// SSE Event structure
#[derive(Debug, Clone)]
pub struct SseEvent {
    pub event_type: String,
    pub data: String,
}

impl SseEvent {
    pub fn new(event_type: &str, data: serde_json::Value) -> Self {
        Self {
            event_type: event_type.to_string(),
            data: serde_json::to_string(&data).unwrap_or_default(),
        }
    }
}

// ─── ServerSession ────────────────────────────────────────────────────────

pub struct ServerSession {
    pub agent_loop: Arc<tokio::sync::RwLock<crate::agent::Loop>>,
    pub messages: Vec<crate::types::AgentMessage>,
    pub model: String,
    pub thinking_level: String,
    pub steering_mode: String,
    pub follow_up_mode: String,
    pub auto_compaction: bool,
    pub auto_retry: bool,
    pub session_manager: Arc<Manager>,
    pub cwd: String,
    pub is_streaming: bool,
    pub session_name: String,
    pub event_bus: Arc<EventBus>,
    pub broadcaster: Arc<SseBroadcaster>,
}

impl ServerSession {
    pub fn new(agent_loop: Arc<tokio::sync::RwLock<crate::agent::Loop>>, manager: Arc<Manager>, cwd: &str, event_bus: Arc<EventBus>, broadcaster: Arc<SseBroadcaster>) -> Self {
        Self {
            agent_loop,
            messages: vec![],
            model: String::new(),
            thinking_level: "high".to_string(),  // Match Go default
            steering_mode: "all".to_string(),
            follow_up_mode: "all".to_string(),
            auto_compaction: true,  // Match Go default
            auto_retry: true,
            session_manager: manager,
            cwd: cwd.to_string(),
            is_streaming: false,
            session_name: String::new(),
            event_bus,
            broadcaster,
        }
    }

    pub fn session_id(&self) -> String {
        self.session_manager
            .list(&self.cwd)
            .ok()
            .and_then(|v| v.first().map(|s| s.id.clone()))
            .unwrap_or_default()
    }

    pub fn session_name(&self) -> String {
        self.session_name.clone()
    }

    pub fn set_session_name(&mut self, name: &str) {
        self.session_name = name.to_string();
    }

    pub fn prompt(&mut self, msg: &str, _images: &[crate::types::ImageContent], _behavior: &str) -> Result<()> {
        // Add message to session
        self.messages
            .push(crate::types::AgentMessage::new_user("user", serde_json::json!([{"type": "text", "text": msg}])));
        
        // Set streaming flag
        self.is_streaming = true;
        
        // Clone messages for the background task
        let messages = self.messages.clone();
        let agent_loop = self.agent_loop.clone();
        let broadcaster = self.broadcaster.clone();
        
        // Spawn background task to run agent loop
        tokio::spawn(async move {
            // Clone broadcaster for each closure
            let broadcaster_text = broadcaster.clone();
            let broadcaster_event = broadcaster.clone();
            
            // Run with timeout
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                async {
                    let r#loop = agent_loop.write().await;
                    r#loop.run_streaming_with_messages(
                        messages,
                        move |text| {
                            broadcaster_text.broadcast(crate::rpc::SseEvent {
                                event_type: "text_chunk".to_string(),
                                data: serde_json::json!({"text": text}).to_string(),
                            });
                        },
                        move |event| {
                            let (event_type, data) = match event.event_type.as_str() {
                                "agent_start" => ("agent_start".to_string(), serde_json::json!({})),
                                "agent_end" => ("agent_end".to_string(), serde_json::json!({})),
                                "turn_start" => ("turn_start".to_string(), serde_json::json!({})),
                                "message_start" => ("message_start".to_string(), serde_json::json!({})),
                                "thinking_start" => ("thinking_start".to_string(), serde_json::json!({})),
                                "thinking_end" => ("thinking_end".to_string(), serde_json::json!({})),
                                "text_start" => ("text_start".to_string(), serde_json::json!({})),
                                "text_end" => ("text_end".to_string(), serde_json::json!({})),
                                "tool_start" => {
                                    ("tool_start".to_string(), serde_json::json!({"tool_name": event.tool_name, "tool_id": event.tool_id}))
                                }
                                "tool_end" => {
                                    ("tool_end".to_string(), serde_json::json!({"tool_name": event.tool_name, "tool_id": event.tool_id}))
                                }
                                "error" => {
                                    ("error".to_string(), serde_json::json!({"error": event.error_text}))
                                }
                                _ => (event.event_type.clone(), serde_json::json!({
                                    "text": event.text,
                                    "tool_name": event.tool_name,
                                    "tool_id": event.tool_id,
                                    "error": event.error_text,
                                })),
                            };
                            broadcaster_event.broadcast(crate::rpc::SseEvent {
                                event_type,
                                data: data.to_string(),
                            });
                        },
                    ).await
                }
            ).await;
            
            match result {
                Ok(Ok((_final_text, _final_messages))) => {
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "agent_end".to_string(),
                        data: serde_json::json!({"type": "stop"}).to_string(),
                    });
                }
                Ok(Err(e)) => {
                    eprintln!("Agent loop error: {}", e);
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "error".to_string(),
                        data: serde_json::json!({"error": e.to_string()}).to_string(),
                    });
                }
                Err(_timeout) => {
                    eprintln!("Agent loop timed out after 60s");
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "error".to_string(),
                        data: serde_json::json!({"error": "timeout: agent loop timed out after 60 seconds"}).to_string(),
                    });
                }
            }
        });
        
        Ok(())
    }

    pub fn steer(&mut self, msg: &str) -> Result<()> {
        self.agent_loop.try_write().unwrap().steering_queue.enqueue(msg.to_string());
        Ok(())
    }

    pub fn follow_up(&mut self, msg: &str) -> Result<()> {
        self.agent_loop.try_write().unwrap().follow_up_queue.enqueue(msg.to_string());
        Ok(())
    }

    pub fn abort(&self) {
        self.agent_loop.try_write().unwrap().abort();
    }

    pub fn new_session(&mut self) -> Result<()> {
        self.messages.clear();
        Ok(())
    }

    pub fn get_messages(&self) -> Vec<crate::types::Message> {
        ConvertToLLM(&self.messages)
    }

    pub fn set_model(&mut self, model: &str) -> Result<()> {
        self.model = model.to_string();
        Ok(())
    }

    pub fn set_thinking_level(&mut self, level: &str) {
        self.thinking_level = level.to_string();
        let budget = match level {
            "off" => 0,
            "minimal" => 2000,
            "low" => 4000,
            "medium" => 8000,
            "high" => 16000,
            "xhigh" => 24000,
            _ => 0,
        };
        self.agent_loop.try_write().unwrap().config.thinking_budget = budget;
    }

    pub fn set_steering_mode(&mut self, mode: &str) {
        self.steering_mode = mode.to_string();
        self.agent_loop.try_write().unwrap().steering_queue.mode = mode.to_string();
    }

    pub fn set_follow_up_mode(&mut self, mode: &str) {
        self.follow_up_mode = mode.to_string();
        self.agent_loop.try_write().unwrap().follow_up_queue.mode = mode.to_string();
    }

    pub fn compact(&self, _instructions: &str) -> Result<serde_json::Value> {
        let messages: Vec<crate::types::Message> = ConvertToLLM(&self.messages);
        let tokens_before = crate::compaction::estimate_context_tokens(&messages);

        let (compacted, result) = crate::compaction::compact(
            messages,
            &crate::compaction::CompactOptions {
                reserve_tokens: 160000,
                keep_recent_tokens: 80000,
                context_window: 0,
            },
        );

        if let Some(r) = result {
            let tokens_after = crate::compaction::estimate_context_tokens(&compacted);
            let messages_removed = (tokens_before - tokens_after).max(0) as i32;
            Ok(serde_json::json!({
                "tokensBefore": r.tokens_before,
                "tokensAfter": tokens_after,
                "summary": r.summary,
                "messagesRemoved": messages_removed,
            }))
        } else {
            Ok(serde_json::json!({
                "tokensBefore": tokens_before,
                "tokensAfter": tokens_before,
                "summary": "",
                "messagesRemoved": 0,
            }))
        }
    }

    pub fn set_auto_compaction(&mut self, enabled: bool) {
        self.auto_compaction = enabled;
    }

    pub fn set_auto_retry(&mut self, enabled: bool) {
        self.auto_retry = enabled;
        self.agent_loop.try_write().unwrap().config.max_retries = if enabled { 3 } else { 0 };
    }

    pub fn execute_bash(&self, command: &str) -> Result<serde_json::Value> {
        let output = std::process::Command::new("bash")
            .args(["-c", command])
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(serde_json::json!({
            "output": format!(
                "{}{}",
                stdout,
                if stderr.is_empty() {
                    String::new()
                } else {
                    format!("\n{}", stderr)
                }
            ),
            "exitCode": output.status.code().unwrap_or(-1),
        }))
    }

    pub fn get_session_stats(&self) -> serde_json::Value {
        serde_json::json!({
            "sessionFile": "",
            "sessionId": self.session_id(),
            "userMessages": self.messages.iter().filter(|m| m.role == "user").count(),
            "assistantMessages": self.messages.iter().filter(|m| m.role == "assistant").count(),
            "toolCalls": self.messages.iter().filter(|m| !m.tool_calls.is_empty()).count(),
            "toolResults": self.messages.iter().filter(|m| m.role == "tool").count(),
            "totalMessages": self.messages.len(),
            "tokens": {
                "input": 0,
                "output": 0,
                "cacheRead": 0,
                "total": 0,
            },
            "cost": 0,
        })
    }

    pub fn list_sessions(&self) -> Result<Vec<serde_json::Value>> {
        let sessions = self.session_manager.list(&self.cwd)?;
        Ok(sessions
            .into_iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "cwd": s.cwd,
                    "model": s.model,
                    "updatedAt": s.updated_at,
                })
            })
            .collect())
    }

    pub fn switch_session(&mut self, _path: &str, _id: &str) -> Result<()> {
        Ok(())
    }

    pub fn delete_session(&self, _id: &str) -> Result<()> {
        Ok(())
    }

    pub fn fork(&mut self, _entry_id: &str) -> Result<()> {
        Ok(())
    }

    pub fn get_last_assistant_text(&self) -> String {
        self.messages
            .iter()
            .filter(|m| m.role == "assistant")
            .last()
            .map(|m| m.text())
            .unwrap_or_default()
    }
}

// ─── App State ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub session: Arc<RwLock<ServerSession>>,
    pub welcome_version: String,
    pub welcome_cwd: String,
    pub welcome_skills: Vec<String>,
    pub welcome_context: Vec<String>,
    pub welcome_exts: Vec<String>,
    pub explicit_session: bool,
    pub broadcaster: Arc<SseBroadcaster>,
    pub event_bus: Arc<EventBus>,
}

pub fn handle_command_internal(state: &AppState, cmd: RpcCommand) -> String {
    let id = &cmd.id;
    let cmd_type = &cmd.cmd_type;

    match cmd_type.as_str() {
        "prompt" => {
            let _ = state.session.write().unwrap().prompt(&cmd.message, &cmd.images, &cmd.streaming_behavior);
            RpcResponse::ok(id, "prompt", serde_json::json!({}))
        }
        "steer" => {
            let _ = state.session.write().unwrap().steer(&cmd.message);
            RpcResponse::ok(id, "steer", serde_json::json!({}))
        }
        "follow_up" => {
            let _ = state.session.write().unwrap().follow_up(&cmd.message);
            RpcResponse::ok(id, "follow_up", serde_json::json!({}))
        }
        "abort" => {
            state.session.write().unwrap().abort();
            RpcResponse::ok(id, "abort", serde_json::json!({}))
        }
        "new_session" => {
            let _ = state.session.write().unwrap().new_session();
            RpcResponse::ok(id, "new_session", serde_json::json!({"cancelled": false}))
        }
        "get_state" => {
            let state_val = get_state_internal(state);
            RpcResponse::ok(id, "get_state", state_val)
        }
        "get_messages" => {
            let msgs = state.session.read().unwrap().get_messages();
            RpcResponse::ok(id, "get_messages", serde_json::json!({"messages": msgs}))
        }
        "set_model" => {
            let _ = state.session.write().unwrap().set_model(&cmd.model_id);
            RpcResponse::ok(id, "set_model", serde_json::json!({"model": cmd.model_id}))
        }
        "set_thinking_level" => {
            state.session.write().unwrap().set_thinking_level(&cmd.level);
            RpcResponse::ok(id, "set_thinking_level", serde_json::json!({}))
        }
        "set_steering_mode" => {
            state.session.write().unwrap().set_steering_mode(&cmd.mode);
            RpcResponse::ok(id, "set_steering_mode", serde_json::json!({}))
        }
        "set_follow_up_mode" => {
            state.session.write().unwrap().set_follow_up_mode(&cmd.mode);
            RpcResponse::ok(id, "set_follow_up_mode", serde_json::json!({}))
        }
        "compact" => {
            let result = state.session.write().unwrap().compact(&cmd.custom_instructions);
            match result {
                Ok(r) => RpcResponse::ok(id, "compact", r),
                Err(e) => RpcResponse::build_fail(id, "compact", &e.to_string()),
            }
        }
        "set_auto_compaction" => {
            state.session.write().unwrap().set_auto_compaction(cmd.enabled);
            RpcResponse::ok(id, "set_auto_compaction", serde_json::json!({}))
        }
        "set_auto_retry" => {
            state.session.write().unwrap().set_auto_retry(cmd.enabled);
            RpcResponse::ok(id, "set_auto_retry", serde_json::json!({}))
        }
        "bash" => {
            let result = state.session.write().unwrap().execute_bash(&cmd.command);
            match result {
                Ok(r) => RpcResponse::ok(id, "bash", r),
                Err(e) => RpcResponse::build_fail(id, "bash", &e.to_string()),
            }
        }
        "get_session_stats" => {
            let stats = state.session.read().unwrap().get_session_stats();
            RpcResponse::ok(id, "get_session_stats", stats)
        }
        "list_sessions" => {
            let sessions = state.session.read().unwrap().list_sessions().unwrap_or_default();
            RpcResponse::ok(id, "list_sessions", serde_json::json!({"sessions": sessions}))
        }
        "switch_session" => {
            let _ = state.session.write().unwrap().switch_session(&cmd.session_path, &cmd.session_id);
            RpcResponse::ok(id, "switch_session", serde_json::json!({"cancelled": false}))
        }
        "delete_session" => {
            let _ = state.session.write().unwrap().delete_session(&cmd.session_id);
            RpcResponse::ok(id, "delete_session", serde_json::json!({}))
        }
        "fork" => {
            let _ = state.session.write().unwrap().fork(&cmd.entry_id);
            RpcResponse::ok(id, "fork", serde_json::json!({}))
        }
        "get_fork_messages" => RpcResponse::ok(id, "get_fork_messages", serde_json::json!({"messages": []})),
        "get_last_assistant_text" => {
            let text = state.session.read().unwrap().get_last_assistant_text();
            RpcResponse::ok(
                id,
                "get_last_assistant_text",
                serde_json::json!({"text": if text.is_empty() { None } else { Some(text) }}),
            )
        }
        "set_session_name" => {
            state.session.write().unwrap().set_session_name(&cmd.name);
            RpcResponse::ok(id, "set_session_name", serde_json::json!({}))
        }
        "get_commands" => RpcResponse::ok(id, "get_commands", serde_json::json!({"commands": []})),
        "abort_retry" => RpcResponse::ok(id, "abort_retry", serde_json::json!({})),
        "abort_bash" => RpcResponse::ok(id, "abort_bash", serde_json::json!({})),
        "cycle_model" => RpcResponse::ok(id, "cycle_model", serde_json::json!({"model": ""})),
        "cycle_thinking_level" => RpcResponse::ok(id, "cycle_thinking_level", serde_json::json!({"level": ""})),
        "get_available_models" => {
            let registry = crate::models::Registry::new();
            let models = registry.all_models().into_iter().map(|m| m.id).collect::<Vec<_>>();
            RpcResponse::ok(id, "get_available_models", serde_json::json!({"models": models}))
        }
        "clone" => RpcResponse::ok(id, "clone", serde_json::json!({})),
        "export_html" => RpcResponse::ok(id, "export_html", serde_json::json!({"path": ""})),
        "ui_response" => RpcResponse::ok(id, "ui_response", serde_json::json!({})),
        _ => RpcResponse::build_fail(id, cmd_type, &format!("unknown command: {}", cmd_type)),
    }
}

fn get_state_internal(state: &AppState) -> serde_json::Value {
    let sess = state.session.read().unwrap();

    let context_window = crate::models::builtin_models()
        .into_iter()
        .find(|m| m.id == sess.model)
        .map(|m| m.context_window)
        .unwrap_or(1000000);

    let session_id = sess.session_id();
    let cwd = if state.welcome_cwd.contains("workspace") && state.welcome_cwd.contains("(main)") {
        state.welcome_cwd.clone()
    } else if state.welcome_cwd.contains("workspace") {
        format!("{} (main)", state.welcome_cwd)
    } else {
        state.welcome_cwd.clone()
    };

    serde_json::json!({
        "model": sess.model,
        "thinkingLevel": sess.thinking_level,
        "isStreaming": sess.is_streaming,
        "isCompacting": false,
        "steeringMode": sess.steering_mode,
        "followUpMode": sess.follow_up_mode,
        "sessionFile": if session_id.is_empty() { serde_json::Value::Null } else { serde_json::Value::String("".to_string()) },
        "sessionId": if session_id.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(session_id) },
        "sessionName": serde_json::Value::Null,
        "explicitSession": state.explicit_session,
        "autoCompactionEnabled": sess.auto_compaction,
        "messageCount": sess.messages.len(),
        "pendingMessageCount": sess.agent_loop.try_read().map(|l|l.pending_message_count()).unwrap_or(0),
        "version": crate::utils::VERSION,
        "cwd": cwd,
        "skills": state.welcome_skills,
        "contextFiles": serde_json::Value::Null,
        "extensions": serde_json::Value::Null,
        "contextWindow": context_window,
        "contextTokens": serde_json::Value::Null,
        "contextPercent": serde_json::Value::Null,
        "tokensIn": serde_json::Value::Null,
        "tokensOut": serde_json::Value::Null,
        "totalCost": serde_json::Value::Null,
    })
}

