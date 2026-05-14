//! RPC Server - Command handling for gRPC

use crate::session::Manager;
use crate::types::ConvertToLLM;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

    // set_system_prompt
    #[serde(default)]
    pub system_prompt: String,

    // set_tools / disable_tools
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub no_tools: bool,

    // set_ephemeral
    #[serde(default)]
    pub ephemeral: bool,
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
    pub session_id: String,
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
    pub ephemeral: bool,
}

impl ServerSession {
    pub fn new(session_id: String, agent_loop: Arc<tokio::sync::RwLock<crate::agent::Loop>>, manager: Arc<Manager>, cwd: &str, event_bus: Arc<EventBus>, broadcaster: Arc<SseBroadcaster>) -> Self {
        Self {
            session_id: session_id.clone(),
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
            ephemeral: false,
        }
    }
    
    /// Create a new session with the same agent_loop but cleared state
    pub fn new_with_shared_loop(session_id: String,
        agent_loop: Arc<tokio::sync::RwLock<crate::agent::Loop>>,
        manager: Arc<Manager>,
        cwd: &str,
        event_bus: Arc<EventBus>,
        broadcaster: Arc<SseBroadcaster>,
    ) -> Self {
        Self {
            session_id: session_id.clone(),
            agent_loop,
            messages: vec![],
            model: String::new(),
            thinking_level: "high".to_string(),
            steering_mode: "all".to_string(),
            follow_up_mode: "all".to_string(),
            auto_compaction: true,
            auto_retry: true,
            session_manager: manager,
            cwd: cwd.to_string(),
            is_streaming: false,
            session_name: String::new(),
            event_bus,
            broadcaster,
            ephemeral: false,
        }
    }

    pub fn session_id(&self) -> String {
        self.session_id.clone()
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
                            // Build full event data matching pi's format
                            let mut data = serde_json::Map::new();
                            data.insert("type".to_string(), serde_json::json!(&event.event_type));
                            if !event.text.is_empty() {
                                data.insert("delta".to_string(), serde_json::json!(&event.text));
                            }
                            if !event.tool_name.is_empty() {
                                data.insert("tool_name".to_string(), serde_json::json!(&event.tool_name));
                            }
                            if !event.tool_id.is_empty() {
                                data.insert("tool_id".to_string(), serde_json::json!(&event.tool_id));
                            }
                            if !event.error_text.is_empty() {
                                data.insert("error".to_string(), serde_json::json!(&event.error_text));
                            }
                            if !event.stop_reason.is_empty() {
                                data.insert("stopReason".to_string(), serde_json::json!(&event.stop_reason));
                            }
                            if let Some(usage) = &event.usage {
                                data.insert("usage".to_string(), serde_json::json!(usage));
                            }
                            broadcaster_event.broadcast(crate::rpc::SseEvent {
                                event_type: event.event_type.clone(),
                                data: serde_json::to_string(&data).unwrap_or_default(),
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
        if let Ok(loop_) = self.agent_loop.try_write() {
            loop_.abort();
        }
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

    pub fn set_system_prompt(&mut self, prompt: &str) {
        let mut loop_ = self.agent_loop.try_write().unwrap();
        loop_.system_prompt = prompt.to_string();
        loop_.config.system_prompt = prompt.to_string();
    }

    pub fn set_tools(&mut self, tool_names: &[String]) {
        let all_tools = crate::tools::all_tools();
        let selected: Vec<_> = all_tools.into_iter()
            .filter(|t| tool_names.contains(&t.def.function.name))
            .collect();
        self.agent_loop.try_write().unwrap().tools = selected;
    }

    pub fn disable_tools(&mut self) {
        self.agent_loop.try_write().unwrap().tools = vec![];
    }

    pub fn disable_builtin_tools(&mut self) {
        // For now, same as disable_tools - all tools disabled
        // TODO: distinguish built-in vs extension tools
        self.agent_loop.try_write().unwrap().tools = vec![];
    }

    pub fn append_system_prompt(&mut self, append: &str) {
        let current = self.agent_loop.try_read().unwrap().system_prompt.clone();
        let new_prompt = if current.is_empty() {
            append.to_string()
        } else {
            format!("{}\n{}", current, append)
        };
        self.agent_loop.try_write().unwrap().system_prompt = new_prompt;
    }

    pub fn set_ephemeral(&mut self, ephemeral: bool) {
        self.ephemeral = ephemeral;
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
    /// Default session (used when no session_id specified)
    pub session: Arc<RwLock<ServerSession>>,
    /// Additional sessions keyed by session_id
    pub sessions: Arc<RwLock<HashMap<String, Arc<RwLock<ServerSession>>>>>,
    /// Active session ID (for get_state display)
    pub active_session_id: Arc<RwLock<String>>,
    pub welcome_version: String,
    pub welcome_cwd: String,
    pub welcome_skills: Vec<String>,
    pub welcome_context: Vec<String>,
    pub welcome_exts: Vec<String>,
    pub explicit_session: bool,
    pub broadcaster: Arc<SseBroadcaster>,
    pub event_bus: Arc<EventBus>,
}

impl AppState {
    /// Get session by ID, or return default session if id is empty/None
    pub fn get_session(&self, session_id: &str) -> Arc<RwLock<ServerSession>> {
        if session_id.is_empty() {
            return self.session.clone();
        }
        if let Ok(sessions) = &self.sessions.try_read() {
            if let Some(sess) = sessions.get(session_id) {
                return sess.clone();
            }
        }
        self.session.clone()
    }
    
    /// Create a new session and return its ID
    /// Uses the session's own session_id as the key
    pub fn create_session(&self, session: ServerSession) -> String {
        let id = session.session_id.clone();
        if let Ok(mut sessions) = self.sessions.try_write() {
            sessions.insert(id.clone(), Arc::new(RwLock::new(session)));
        }
        if let Ok(mut active_id) = self.active_session_id.try_write() {
            *active_id = id.clone();
        }
        id
    }
    
    /// Get active session ID
    pub fn get_active_session_id(&self) -> String {
        self.active_session_id.read().unwrap().clone()
    }
}

pub fn handle_command_internal(state: &AppState, cmd: RpcCommand) -> String {
    let id = &cmd.id;
    let cmd_type = &cmd.cmd_type;
    
    // Get the target session based on session_id, or use default
    let session = state.get_session(&cmd.session_id);
    
    match cmd_type.as_str() {
        "prompt" => {
            let _ = session.write().unwrap().prompt(&cmd.message, &cmd.images, &cmd.streaming_behavior);
            RpcResponse::ok(id, "prompt", serde_json::json!({}))
        }
        "steer" => {
            let _ = session.write().unwrap().steer(&cmd.message);
            RpcResponse::ok(id, "steer", serde_json::json!({}))
        }
        "follow_up" => {
            let _ = session.write().unwrap().follow_up(&cmd.message);
            RpcResponse::ok(id, "follow_up", serde_json::json!({}))
        }
        "abort" => {
            // Try to abort, but don't fail if lock is busy
            if let Ok(mut sess) = session.try_write() {
                sess.abort();
            }
            RpcResponse::ok(id, "abort", serde_json::json!({}))
        }
        "new_session" => {
            // Create a new session with shared agent_loop
            let active_id = state.get_active_session_id();
            let session = state.get_session(&active_id);
            let sess = session.read().unwrap();
            let new_sess = ServerSession::new_with_shared_loop(
                crate::utils::generate_id(),
                sess.agent_loop.clone(),
                sess.session_manager.clone(),
                &sess.cwd,
                sess.event_bus.clone(),
                sess.broadcaster.clone(),
            );
            drop(sess);
            
            // Add to sessions map
            let new_id = state.create_session(new_sess);
            
            RpcResponse::ok(id, "new_session", serde_json::json!({"sessionId": new_id}))
        }
        "get_state" => {
            let state_val = get_state_internal(state, &cmd.session_id);
            RpcResponse::ok(id, "get_state", state_val)
        }
        "get_messages" => {
            let msgs = session.read().unwrap().get_messages();
            RpcResponse::ok(id, "get_messages", serde_json::json!({"messages": msgs}))
        }
        "set_model" => {
            let _ = session.write().unwrap().set_model(&cmd.model_id);
            RpcResponse::ok(id, "set_model", serde_json::json!({"model": cmd.model_id}))
        }
        "set_thinking_level" => {
            session.write().unwrap().set_thinking_level(&cmd.level);
            RpcResponse::ok(id, "set_thinking_level", serde_json::json!({}))
        }
        "set_steering_mode" => {
            session.write().unwrap().set_steering_mode(&cmd.mode);
            RpcResponse::ok(id, "set_steering_mode", serde_json::json!({}))
        }
        "set_follow_up_mode" => {
            session.write().unwrap().set_follow_up_mode(&cmd.mode);
            RpcResponse::ok(id, "set_follow_up_mode", serde_json::json!({}))
        }
        "compact" => {
            let result = session.write().unwrap().compact(&cmd.custom_instructions);
            match result {
                Ok(r) => RpcResponse::ok(id, "compact", r),
                Err(e) => RpcResponse::build_fail(id, "compact", &e.to_string()),
            }
        }
        "set_auto_compaction" => {
            session.write().unwrap().set_auto_compaction(cmd.enabled);
            RpcResponse::ok(id, "set_auto_compaction", serde_json::json!({}))
        }
        "set_auto_retry" => {
            session.write().unwrap().set_auto_retry(cmd.enabled);
            RpcResponse::ok(id, "set_auto_retry", serde_json::json!({}))
        }
        "set_system_prompt" => {
            session.write().unwrap().set_system_prompt(&cmd.system_prompt);
            RpcResponse::ok(id, "set_system_prompt", serde_json::json!({}))
        }
        "set_tools" => {
            session.write().unwrap().set_tools(&cmd.tools);
            RpcResponse::ok(id, "set_tools", serde_json::json!({"tools": cmd.tools}))
        }
        "disable_tools" => {
            session.write().unwrap().disable_tools();
            RpcResponse::ok(id, "disable_tools", serde_json::json!({}))
        }
        "disable_builtin_tools" => {
            session.write().unwrap().disable_builtin_tools();
            RpcResponse::ok(id, "disable_builtin_tools", serde_json::json!({}))
        }
        "append_system_prompt" => {
            session.write().unwrap().append_system_prompt(&cmd.system_prompt);
            RpcResponse::ok(id, "append_system_prompt", serde_json::json!({}))
        }
        "set_ephemeral" => {
            session.write().unwrap().set_ephemeral(cmd.ephemeral);
            RpcResponse::ok(id, "set_ephemeral", serde_json::json!({"ephemeral": cmd.ephemeral}))
        }
        "bash" => {
            let result = session.write().unwrap().execute_bash(&cmd.command);
            match result {
                Ok(r) => RpcResponse::ok(id, "bash", r),
                Err(e) => RpcResponse::build_fail(id, "bash", &e.to_string()),
            }
        }
        "get_session_stats" => {
            let stats = session.read().unwrap().get_session_stats();
            RpcResponse::ok(id, "get_session_stats", stats)
        }
        "list_sessions" => {
            // Use session_manager.list_all() to get all sessions from disk
            let summaries = session.read().unwrap().session_manager.list_all().unwrap_or_default();
            // Convert to the format expected by TUI
            let sessions: Vec<serde_json::Value> = summaries.into_iter().map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "name": s.name,
                    "model": s.model,
                    "cwd": s.cwd,
                    "updated_at": s.updated_at.format("%Y-%m-%d %H:%M:%S").to_string()
                })
            }).collect();
            RpcResponse::ok(id, "list_sessions", serde_json::json!({"sessions": sessions}))
        }
        "switch_session" => {
            // Switch to the session specified by session_id
            if cmd.session_id.is_empty() {
                return RpcResponse::build_fail(id, "switch_session", "session_id is required");
            }
            // Update active_session_id
            if let Ok(mut active_id) = state.active_session_id.try_write() {
                *active_id = cmd.session_id.clone();
            }
            RpcResponse::ok(id, "switch_session", serde_json::json!({"cancelled": false}))
        }
        "delete_session" => {
            if cmd.session_id.is_empty() {
                return RpcResponse::build_fail(id, "delete_session", "session_id is required");
            }
            // Get cwd before deleting
            let cwd = session.read().unwrap().cwd.clone();
            // Delete from disk
            if let Err(e) = session.read().unwrap().session_manager.delete(&cmd.session_id, &cwd) {
                return RpcResponse::build_fail(id, "delete_session", &e.to_string());
            }
            // Remove from memory if present
            if let Ok(mut sessions) = state.sessions.try_write() {
                sessions.remove(&cmd.session_id);
            }
            RpcResponse::ok(id, "delete_session", serde_json::json!({"deleted": true}))
        }
        "fork" => {
            let entry_id = &cmd.entry_id;
            if entry_id.is_empty() {
                return RpcResponse::build_fail(id, "fork", "entry_id is required");
            }
            
            // Extract needed data from session
            let (agent_loop, session_manager, event_bus, broadcaster, cwd, session_id) = {
                let sess = session.read().unwrap();
                (
                    sess.agent_loop.clone(),
                    sess.session_manager.clone(),
                    sess.event_bus.clone(),
                    sess.broadcaster.clone(),
                    sess.cwd.clone(),
                    sess.session_id.clone(),
                )
            };
            
            // Get parent session from manager
            let parent = match session_manager.load(&cwd, &session_id) {
                Ok(s) => s,
                Err(_) => {
                    return RpcResponse::build_fail(id, "fork", "parent session not found");
                }
            };
            
            // Fork a new session
            let forked = crate::session::fork_session(&parent, entry_id);
            let forked_id = forked.id.clone();
            
            // Save the forked session
            if let Err(e) = session_manager.save(&forked) {
                return RpcResponse::build_fail(id, "fork", &format!("failed to save forked session: {}", e));
            }
            
            // Add to sessions map
            let new_sess = ServerSession::new_with_shared_loop(
                forked_id.clone(),
                agent_loop,
                session_manager,
                &forked.cwd,
                event_bus,
                broadcaster,
            );
            state.create_session(new_sess);
            
            RpcResponse::ok(id, "fork", serde_json::json!({"cancelled": false}))
        }
        "get_fork_messages" => {
            let msgs = session.read().unwrap().get_messages();
            RpcResponse::ok(id, "get_fork_messages", serde_json::json!({"messages": msgs}))
        }
        "get_last_assistant_text" => {
            let text = session.read().unwrap().get_last_assistant_text();
            RpcResponse::ok(
                id,
                "get_last_assistant_text",
                serde_json::json!({"text": if text.is_empty() { None } else { Some(text) }}),
            )
        }
        "set_session_name" => {
            session.write().unwrap().set_session_name(&cmd.name);
            RpcResponse::ok(id, "set_session_name", serde_json::json!({}))
        }
        "get_commands" => {
            // Return commands from skills (similar to Go's extensions + prompts)
            let skill_dirs = vec![
                crate::skills::USER_SKILLS_DIR.to_string(),
                crate::skills::PROJECT_SKILLS_DIR.to_string(),
                crate::skills::AGENTS_SKILLS_DIR.to_string(),
            ];
            let skills = crate::skills::discover_skills(&skill_dirs).unwrap_or_default();
            
            let commands: Vec<serde_json::Value> = skills.into_iter().map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                    "source": "skill"
                })
            }).collect();
            
            RpcResponse::ok(id, "get_commands", serde_json::json!({"commands": commands}))
        }
        "abort_retry" => {
            if let Ok(mut sess) = session.try_write() {
                sess.abort();
            }
            RpcResponse::ok(id, "abort_retry", serde_json::json!({}))
        }
        "abort_bash" => {
            // Bash abort is handled by the agent loop
            RpcResponse::ok(id, "abort_bash", serde_json::json!({}))
        }
        "cycle_model" => {
            // Cycle to next model
            let models: Vec<String> = crate::models::Registry::new()
                .all_models()
                .into_iter()
                .map(|m| m.id)
                .collect();
            if models.is_empty() {
                return RpcResponse::ok(id, "cycle_model", serde_json::json!({"model": "", "thinkingLevel": "", "isScoped": false}));
            }
            let current = session.read().unwrap().model.clone();
            let idx = models.iter().position(|m| m == &current).unwrap_or(0);
            let next_idx = (idx + 1) % models.len();
            let next_model = &models[next_idx];
            
            // Update session model
            session.write().unwrap().model = next_model.clone();
            
            RpcResponse::ok(id, "cycle_model", serde_json::json!({
                "model": next_model,
                "thinkingLevel": session.read().unwrap().thinking_level.clone(),
                "isScoped": false
            }))
        }
        "cycle_thinking_level" => {
            // Cycle thinking level: off -> minimal -> low -> medium -> high -> xhigh -> off
            let levels = ["off", "minimal", "low", "medium", "high", "xhigh"];
            let current = session.read().unwrap().thinking_level.clone();
            let idx = levels.iter().position(|l| *l == current).unwrap_or(0);
            let next_idx = (idx + 1) % levels.len();
            let next_level = levels[next_idx];
            
            // Update session thinking level
            session.write().unwrap().thinking_level = next_level.to_string();
            
            RpcResponse::ok(id, "cycle_thinking_level", serde_json::json!({"level": next_level}))
        }
        "get_available_models" => {
            let registry = crate::models::Registry::new();
            let models = registry.all_models().into_iter().map(|m| m.id).collect::<Vec<_>>();
            RpcResponse::ok(id, "get_available_models", serde_json::json!({"models": models}))
        }
        "clone" => {
            // Extract needed data from session
            let (agent_loop, session_manager, event_bus, broadcaster, cwd, session_id) = {
                let sess = session.read().unwrap();
                let entries = &sess.messages;
                if entries.is_empty() {
                    return RpcResponse::build_fail(id, "clone", "no entries to clone");
                }
                (
                    sess.agent_loop.clone(),
                    sess.session_manager.clone(),
                    sess.event_bus.clone(),
                    sess.broadcaster.clone(),
                    sess.cwd.clone(),
                    sess.session_id.clone(),
                )
            };
            
            // Get parent session from manager
            let parent = match session_manager.load(&cwd, &session_id) {
                Ok(s) => s,
                Err(_) => {
                    return RpcResponse::build_fail(id, "clone", "parent session not found");
                }
            };
            
            let leaf_id = parent.entries.last().map(|e| e.id.clone()).unwrap_or_default();
            if leaf_id.is_empty() {
                return RpcResponse::build_fail(id, "clone", "no entries to clone");
            }
            
            // Fork from leaf
            let forked = crate::session::fork_session(&parent, &leaf_id);
            let forked_id = forked.id.clone();
            
            // Save the forked session
            if let Err(e) = session_manager.save(&forked) {
                return RpcResponse::build_fail(id, "clone", &format!("failed to save cloned session: {}", e));
            }
            
            // Add to sessions map
            let new_sess = ServerSession::new_with_shared_loop(
                forked_id.clone(),
                agent_loop,
                session_manager,
                &forked.cwd,
                event_bus,
                broadcaster,
            );
            state.create_session(new_sess);
            
            RpcResponse::ok(id, "clone", serde_json::json!({"cancelled": false}))
        }
        "export_html" => {
            // Export session to HTML file
            let sess = session.read().unwrap();
            let session_id = sess.session_id();
            let model = sess.model.clone();
            let cwd = sess.cwd.clone();
            let messages = sess.get_messages();
            drop(sess);
            
            // Generate HTML
            let html = generate_session_html(&session_id, &model, &cwd, &messages);
            
            // Write to file
            let output_path = format!("/tmp/future_agent_export_{}.html", session_id);
            if let Err(e) = std::fs::write(&output_path, html) {
                return RpcResponse::build_fail(id, "export_html", &format!("failed to write file: {}", e));
            }
            
            RpcResponse::ok(id, "export_html", serde_json::json!({"path": output_path}))
        }
        "ui_response" => RpcResponse::ok(id, "ui_response", serde_json::json!({})),
        _ => RpcResponse::build_fail(id, cmd_type, &format!("unknown command: {}", cmd_type)),
    }
}

fn get_state_internal(state: &AppState, session_id: &str) -> serde_json::Value {
    let session = state.get_session(session_id);
    let sess = session.read().unwrap();

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

/// Generate HTML representation of a session (matches Go exportSessionToHTML)
fn generate_session_html(session_id: &str, model: &str, cwd: &str, messages: &[crate::types::Message]) -> String {
    let mut html = String::new();
    
    html.push_str("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">");
    html.push_str(&format!("<title>FutureAgent session {}</title>", session_id));
    html.push_str("<style>");
    html.push_str("body{font-family:system-ui;max-width:800px;margin:auto;padding:20px;background:#1a1a2e;color:#e0e0e0}");
    html.push_str(".user{background:#16213e;padding:10px;margin:5px 0;border-radius:8px}");
    html.push_str(".assistant{background:#0f3460;padding:10px;margin:5px 0;border-radius:8px}");
    html.push_str(".tool{background:#1a1a1a;padding:10px;margin:5px 0;border-radius:8px;font-size:0.9em}");
    html.push_str("pre{white-space:pre-wrap;word-wrap:break-word}");
    html.push_str("</style></head><body>\n");
    html.push_str(&format!("<h1>FutureAgent Session: {}</h1>\n", session_id));
    html.push_str(&format!("<p>Model: {} | CWD: {}</p>\n", model, cwd));
    
    for msg in messages {
        let cls = match msg.role.as_str() {
            "assistant" => "assistant",
            "tool" => "tool",
            _ => "user",
        };
        let content = match &msg.content {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => String::new(),
        };
        html.push_str(&format!(
            "<div class=\"{}\"><strong>{}</strong><pre>{}</pre></div>\n",
            cls,
            escape_html(&msg.role),
            escape_html(&content)
        ));
    }
    
    html.push_str("</body></html>");
    html
}

/// Escape HTML special characters
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

