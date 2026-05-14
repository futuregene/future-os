//! RPC Server — 1:1 compatible with Go internal/rpc/
//!
//! Protocol: HTTP + JSON for commands, SSE for events.
//! Uses axum for HTTP/SSE server (tiny_http replaced for proper SSE streaming support).

use crate::session::Manager;
use crate::types::ConvertToLLM;
use anyhow::Result;
use axum::{
    extract::State,
    response::sse::{Event, Sse},
    routing::get,
    Json, Router,
};
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

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

    fn fail(id: &str, command: &str, err: &str) -> String {
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
    pub agent_loop: crate::agent::Loop,
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
}

impl ServerSession {
    pub fn new(agent_loop: crate::agent::Loop, manager: Arc<Manager>, cwd: &str) -> Self {
        Self {
            agent_loop,
            messages: vec![],
            model: String::new(),
            thinking_level: "off".to_string(),
            steering_mode: "all".to_string(),
            follow_up_mode: "all".to_string(),
            auto_compaction: false,
            auto_retry: true,
            session_manager: manager,
            cwd: cwd.to_string(),
            is_streaming: false,
            session_name: String::new(),
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
        self.messages
            .push(crate::types::AgentMessage::new_user("user", serde_json::json!([{"type": "text", "text": msg}])));
        Ok(())
    }

    pub fn steer(&mut self, msg: &str) -> Result<()> {
        self.agent_loop.steering_queue.enqueue(msg.to_string());
        Ok(())
    }

    pub fn follow_up(&mut self, msg: &str) -> Result<()> {
        self.agent_loop.follow_up_queue.enqueue(msg.to_string());
        Ok(())
    }

    pub fn abort(&self) {
        self.agent_loop.abort();
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
        self.agent_loop.config.thinking_budget = budget;
    }

    pub fn set_steering_mode(&mut self, mode: &str) {
        self.steering_mode = mode.to_string();
        self.agent_loop.steering_queue.mode = mode.to_string();
    }

    pub fn set_follow_up_mode(&mut self, mode: &str) {
        self.follow_up_mode = mode.to_string();
        self.agent_loop.follow_up_queue.mode = mode.to_string();
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
        self.agent_loop.config.max_retries = if enabled { 3 } else { 0 };
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
    pub broadcaster: SseBroadcaster,
}

// ─── Server ───────────────────────────────────────────────────────────────

pub struct Server {
    pub session: Arc<RwLock<ServerSession>>,
    welcome_version: String,
    welcome_cwd: String,
    welcome_skills: Vec<String>,
    welcome_context: Vec<String>,
    welcome_exts: Vec<String>,
    explicit_session: bool,
    broadcaster: SseBroadcaster,
}

impl Server {
    pub fn new(session: Arc<RwLock<ServerSession>>) -> Self {
        Self {
            session,
            welcome_version: crate::utils::VERSION.to_string(),
            welcome_cwd: String::new(),
            welcome_skills: vec![],
            welcome_context: vec![],
            welcome_exts: vec![],
            explicit_session: false,
            broadcaster: SseBroadcaster::new(),
        }
    }

    pub fn into_app_state(self) -> AppState {
        AppState {
            session: self.session,
            welcome_version: self.welcome_version,
            welcome_cwd: self.welcome_cwd,
            welcome_skills: self.welcome_skills,
            welcome_context: self.welcome_context,
            welcome_exts: self.welcome_exts,
            explicit_session: self.explicit_session,
            broadcaster: self.broadcaster,
        }
    }

    pub fn set_welcome(&mut self, version: &str, cwd: &str, skills: Vec<String>, context: Vec<String>, exts: Vec<String>) {
        self.welcome_version = version.to_string();
        self.welcome_cwd = cwd.to_string();
        self.welcome_skills = skills;
        self.welcome_context = context;
        self.welcome_exts = exts;
    }

    pub fn set_explicit_session(&mut self, v: bool) {
        self.explicit_session = v;
    }

    fn handle_command(&self, cmd: RpcCommand) -> String {
        let id = &cmd.id;
        let cmd_type = &cmd.cmd_type;

        match cmd_type.as_str() {
            "prompt" => {
                let _ = self.session.write().unwrap().prompt(&cmd.message, &cmd.images, &cmd.streaming_behavior);
                RpcResponse::ok(id, "prompt", serde_json::json!({}))
            }
            "steer" => {
                let _ = self.session.write().unwrap().steer(&cmd.message);
                RpcResponse::ok(id, "steer", serde_json::json!({}))
            }
            "follow_up" => {
                let _ = self.session.write().unwrap().follow_up(&cmd.message);
                RpcResponse::ok(id, "follow_up", serde_json::json!({}))
            }
            "abort" => {
                self.session.write().unwrap().abort();
                RpcResponse::ok(id, "abort", serde_json::json!({}))
            }
            "new_session" => {
                let _ = self.session.write().unwrap().new_session();
                RpcResponse::ok(id, "new_session", serde_json::json!({"cancelled": false}))
            }
            "get_state" => {
                let state = self.get_state();
                RpcResponse::ok(id, "get_state", state)
            }
            "get_messages" => {
                let msgs = self.session.read().unwrap().get_messages();
                RpcResponse::ok(id, "get_messages", serde_json::json!({"messages": msgs}))
            }
            "set_model" => {
                let _ = self.session.write().unwrap().set_model(&cmd.model_id);
                RpcResponse::ok(id, "set_model", serde_json::json!({"model": cmd.model_id}))
            }
            "set_thinking_level" => {
                self.session.write().unwrap().set_thinking_level(&cmd.level);
                RpcResponse::ok(id, "set_thinking_level", serde_json::json!({}))
            }
            "set_steering_mode" => {
                self.session.write().unwrap().set_steering_mode(&cmd.mode);
                RpcResponse::ok(id, "set_steering_mode", serde_json::json!({}))
            }
            "set_follow_up_mode" => {
                self.session.write().unwrap().set_follow_up_mode(&cmd.mode);
                RpcResponse::ok(id, "set_follow_up_mode", serde_json::json!({}))
            }
            "compact" => {
                let result = self.session.write().unwrap().compact(&cmd.custom_instructions);
                match result {
                    Ok(r) => RpcResponse::ok(id, "compact", r),
                    Err(e) => RpcResponse::fail(id, "compact", &e.to_string()),
                }
            }
            "set_auto_compaction" => {
                self.session.write().unwrap().set_auto_compaction(cmd.enabled);
                RpcResponse::ok(id, "set_auto_compaction", serde_json::json!({}))
            }
            "set_auto_retry" => {
                self.session.write().unwrap().set_auto_retry(cmd.enabled);
                RpcResponse::ok(id, "set_auto_retry", serde_json::json!({}))
            }
            "bash" => {
                let result = self.session.write().unwrap().execute_bash(&cmd.command);
                match result {
                    Ok(r) => RpcResponse::ok(id, "bash", r),
                    Err(e) => RpcResponse::fail(id, "bash", &e.to_string()),
                }
            }
            "get_session_stats" => {
                let stats = self.session.read().unwrap().get_session_stats();
                RpcResponse::ok(id, "get_session_stats", stats)
            }
            "list_sessions" => {
                let sessions = self.session.read().unwrap().list_sessions().unwrap_or_default();
                RpcResponse::ok(id, "list_sessions", serde_json::json!({"sessions": sessions}))
            }
            "switch_session" => {
                let _ = self.session.write().unwrap().switch_session(&cmd.session_path, &cmd.session_id);
                RpcResponse::ok(id, "switch_session", serde_json::json!({"cancelled": false}))
            }
            "delete_session" => {
                let _ = self.session.write().unwrap().delete_session(&cmd.session_id);
                RpcResponse::ok(id, "delete_session", serde_json::json!({}))
            }
            "fork" => {
                let _ = self.session.write().unwrap().fork(&cmd.entry_id);
                RpcResponse::ok(id, "fork", serde_json::json!({}))
            }
            "get_fork_messages" => {
                RpcResponse::ok(id, "get_fork_messages", serde_json::json!({"messages": []}))
            }
            "get_last_assistant_text" => {
                let text = self.session.read().unwrap().get_last_assistant_text();
                RpcResponse::ok(
                    id,
                    "get_last_assistant_text",
                    serde_json::json!({"text": if text.is_empty() { None } else { Some(text) } }),
                )
            }
            "set_session_name" => {
                self.session.write().unwrap().set_session_name(&cmd.name);
                RpcResponse::ok(id, "set_session_name", serde_json::json!({}))
            }
            "get_commands" => {
                RpcResponse::ok(id, "get_commands", serde_json::json!({"commands": []}))
            }
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
            _ => RpcResponse::fail(id, cmd_type, &format!("unknown command: {}", cmd_type)),
        }
    }

    fn get_state(&self) -> serde_json::Value {
        let sess = self.session.read().unwrap();

        let context_window = crate::models::builtin_models()
            .into_iter()
            .find(|m| m.id == sess.model)
            .map(|m| m.context_window)
            .unwrap_or(1000000);

        let context_tokens = (sess.messages.len() * 200) as i32;
        let context_percent = ((context_tokens as f64 / context_window as f64) * 100.0) as i32;

        serde_json::json!({
            "model": sess.model,
            "thinkingLevel": sess.thinking_level,
            "isStreaming": sess.is_streaming,
            "isCompacting": false,
            "steeringMode": sess.steering_mode,
            "followUpMode": sess.follow_up_mode,
            "sessionFile": "",
            "sessionId": sess.session_id(),
            "sessionName": sess.session_name(),
            "explicitSession": self.explicit_session,
            "autoCompactionEnabled": sess.auto_compaction,
            "messageCount": sess.messages.len(),
            "pendingMessageCount": sess.agent_loop.pending_message_count(),
            "version": crate::utils::VERSION,
            "cwd": self.welcome_cwd,
            "skills": self.welcome_skills,
            "contextFiles": self.welcome_context,
            "extensions": self.welcome_exts,
            "contextWindow": context_window,
            "contextTokens": context_tokens,
            "contextPercent": context_percent,
            "tokensIn": 0,
            "tokensOut": 0,
            "totalCost": 0.0,
        })
    }

    /// Run over HTTP on a TCP port using axum
    pub async fn run_tcp(&mut self, addr: &str) -> Result<()> {
        let addr = if addr.parse::<u16>().is_ok() {
            format!("0.0.0.0:{}", addr)
        } else if addr.starts_with(':') {
            format!("0.0.0.0{}", addr)
        } else {
            addr.to_string()
        };

        let state = self.clone_as_app_state();

        let app = Router::new()
            .route("/", axum::routing::post(rpc_handler))
            .route("/events", get(sse_handler))
            .with_state(state);

        eprintln!("xihu server listening on {}", addr);

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }

    /// Run over Unix socket
    pub async fn run_unix(&mut self, path: &str) -> Result<()> {
        use std::os::unix::net::UnixListener;

        let _ = std::fs::remove_file(path);

        let listener = UnixListener::bind(path)?;
        eprintln!("xihu server listening on {}", path);

        loop {
            let (mut stream, _) = listener.accept()?;
            let mut buf = vec![];
            use std::io::Read;
            stream.read_to_end(&mut buf)?;
            let body_str = String::from_utf8_lossy(&buf);

            if let Ok(cmd) = serde_json::from_str::<RpcCommand>(&body_str) {
                let resp_str = self.handle_command(cmd);
                use std::io::Write;
                let _ = stream.write_all(resp_str.as_bytes());
            }
        }
    }

    /// Run over stdio (for pipe mode)
    pub fn run_stdio(&mut self) -> Result<()> {
        use std::io::{BufRead, Write};

        let stdin = std::io::stdin();
        let reader = stdin.lock();
        let mut lines = reader.lines();

        let welcome = serde_json::json!({
            "type": "welcome",
            "version": self.welcome_version,
            "cwd": self.welcome_cwd,
            "skills": self.welcome_skills,
            "contextFiles": self.welcome_context,
            "extensions": self.welcome_exts,
        });
        if let Ok(line) = serde_json::to_string(&welcome) {
            println!("{}", line);
        }

        for line in lines {
            if let Ok(line) = line {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(cmd) = serde_json::from_str::<RpcCommand>(&line) {
                    let resp_str = self.handle_command(cmd);
                    println!("{}", resp_str);
                }
            }
        }

        Ok(())
    }

    fn clone_as_app_state(&self) -> AppState {
        AppState {
            session: self.session.clone(),
            welcome_version: self.welcome_version.clone(),
            welcome_cwd: self.welcome_cwd.clone(),
            welcome_skills: self.welcome_skills.clone(),
            welcome_context: self.welcome_context.clone(),
            welcome_exts: self.welcome_exts.clone(),
            explicit_session: self.explicit_session,
            broadcaster: self.broadcaster.clone(),
        }
    }
}

// ─── HTTP Handlers ─────────────────────────────────────────────────────

async fn rpc_handler(State(state): State<AppState>, Json(body): Json<serde_json::Value>) -> String {
    // Try batch first
    if let Ok(cmds) = serde_json::from_value::<Vec<RpcCommand>>(body.clone()) {
        let responses: Vec<serde_json::Value> = cmds
            .into_iter()
            .map(|cmd| {
                let resp_str = handle_command_internal(&state, cmd);
                serde_json::from_str(&resp_str).unwrap_or_default()
            })
            .collect();
        serde_json::to_string(&responses).unwrap_or_default()
    } else if let Ok(cmd) = serde_json::from_value::<RpcCommand>(body) {
        handle_command_internal(&state, cmd)
    } else {
        RpcResponse::fail("", "rpc", "invalid JSON")
    }
}

fn handle_command_internal(state: &AppState, cmd: RpcCommand) -> String {
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
                Err(e) => RpcResponse::fail(id, "compact", &e.to_string()),
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
                Err(e) => RpcResponse::fail(id, "bash", &e.to_string()),
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
        _ => RpcResponse::fail(id, cmd_type, &format!("unknown command: {}", cmd_type)),
    }
}

fn get_state_internal(state: &AppState) -> serde_json::Value {
    let sess = state.session.read().unwrap();

    let context_window = crate::models::builtin_models()
        .into_iter()
        .find(|m| m.id == sess.model)
        .map(|m| m.context_window)
        .unwrap_or(1000000);

    let context_tokens = (sess.messages.len() * 200) as i32;
    let context_percent = ((context_tokens as f64 / context_window as f64) * 100.0) as i32;

    serde_json::json!({
        "model": sess.model,
        "thinkingLevel": sess.thinking_level,
        "isStreaming": sess.is_streaming,
        "isCompacting": false,
        "steeringMode": sess.steering_mode,
        "followUpMode": sess.follow_up_mode,
        "sessionFile": "",
        "sessionId": sess.session_id(),
        "sessionName": sess.session_name(),
        "explicitSession": state.explicit_session,
        "autoCompactionEnabled": sess.auto_compaction,
        "messageCount": sess.messages.len(),
        "pendingMessageCount": sess.agent_loop.pending_message_count(),
        "version": crate::utils::VERSION,
        "cwd": state.welcome_cwd,
        "skills": state.welcome_skills,
        "contextFiles": state.welcome_context,
        "extensions": state.welcome_exts,
        "contextWindow": context_window,
        "contextTokens": context_tokens,
        "contextPercent": context_percent,
        "tokensIn": 0,
        "tokensOut": 0,
        "totalCost": 0.0,
    })
}

// ─── SSE Handler ─────────────────────────────────────────────────────────

async fn sse_handler(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, axum::Error>>> {
    let mut rx = state.broadcaster.subscribe();

    // Create stream using BroadcastStream from tokio_stream with sync feature
    // For now, use a simple manual stream implementation
    use futures_util::StreamExt;
    use tokio::time::{interval, Duration};

    let stream = async_stream::stream! {
        let mut heartbeat = interval(Duration::from_secs(30));

        // Send initial ping (Go format: ": ping\n\n")
        yield Ok(Event::default().comment(" ping"));

        loop {
            tokio::select! {
                // Event from broadcaster
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
                        Err(broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
                // Heartbeat
                _ = heartbeat.tick() => {
                    yield Ok(Event::default().comment(" heartbeat"));
                }
            }
        }
    };

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::default()
    )
}

// ─── Event Broadcasting ─────────────────────────────────────────────────

impl Server {
    /// Broadcast an event to all SSE subscribers
    pub fn broadcast_event(&self, event_type: &str, data: serde_json::Value) {
        self.broadcaster
            .broadcast(SseEvent::new(event_type, data));
    }
}
