//! RPC Server — 1:1 compatible with Go internal/rpc/
//!
//! Protocol: JSONL over stdin/stdout. Commands are JSON objects with `type` field.
//! Responses are JSON objects with `type: "response"`, `command`, `success`.
//! Events are AgentSessionEvent objects streamed as they occur.

use crate::events::EventBus;
use crate::types::{StreamEvent, ConvertToLLM};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::mpsc;

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
    fn ok(id: &str, command: &str, data: impl Into<serde_json::Value>) -> Self {
        Self {
            resp_type: "response".to_string(),
            id: id.to_string(),
            command: command.to_string(),
            success: true,
            data: Some(data.into()),
            error: None,
        }
    }
    fn fail(id: &str, command: &str, err: &str) -> Self {
        Self {
            resp_type: "response".to_string(),
            id: id.to_string(),
            command: command.to_string(),
            success: false,
            data: None,
            error: Some(err.to_string()),
        }
    }
}

// ─── RPC Session State ──────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcSessionState {
    pub model: String,
    pub thinking_level: String,
    pub is_streaming: bool,
    pub is_compacting: bool,
    pub steering_mode: String,
    pub follow_up_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    pub session_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub session_name: String,
    pub explicit_session: bool,
    pub auto_compaction_enabled: bool,
    pub message_count: i32,
    pub pending_message_count: i32,

    // Welcome info
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,

    // Context usage
    pub context_tokens: i32,
    pub context_window: i32,
    pub context_percent: f64,

    // Token usage
    pub tokens_in: i32,
    pub tokens_out: i32,
    pub total_cost: f64,
}

// ─── Extension UI ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcExtensionUIRequest {
    #[serde(rename = "type")]
    pub req_type: String,
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default)]
    pub prefill: String,
    #[serde(default)]
    pub timeout: i32,
    #[serde(default)]
    pub notify_type: String,
    #[serde(default)]
    pub status_key: String,
    #[serde(default)]
    pub status_text: String,
    #[serde(default)]
    pub widget_key: String,
    #[serde(default)]
    pub widget_lines: Vec<String>,
    #[serde(default)]
    pub widget_placement: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcExtensionUIResponse {
    #[serde(rename = "type")]
    pub resp_type: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(default)]
    pub confirmed: bool,
    #[serde(default)]
    pub cancelled: bool,
}

// ─── Server ───────────────────────────────────────────────────────────────

pub struct Server {
    pub session: Arc<RwLock<ServerSession>>,
    writer: Arc<RwLock<Box<dyn tokio::io::AsyncWrite + Send + Unpin>>>,
    pending_ui: Arc<RwLock<HashMap<String, mpsc::Sender<RpcExtensionUIResponse>>>>,
    unsub_event: RwLock<Option<Box<dyn Fn(crate::types::StreamEvent) + Send + Sync>>>,
    welcome_version: String,
    welcome_cwd: String,
    welcome_skills: Vec<String>,
    welcome_context: Vec<String>,
    welcome_exts: Vec<String>,
    explicit_session: bool,
}


impl Server {
    pub fn new<W: tokio::io::AsyncWrite + Send + Unpin + 'static>(session: Arc<RwLock<ServerSession>>, writer: W) -> Self {
        Self {
            session,
            writer: Arc::new(RwLock::new(Box::new(writer) as Box<dyn tokio::io::AsyncWrite + Send + Unpin>)),
            pending_ui: Arc::new(RwLock::new(HashMap::new())),
            unsub_event: RwLock::new(None),
            welcome_version: crate::utils::VERSION.to_string(),
            welcome_cwd: String::new(),
            welcome_skills: vec![],
            welcome_context: vec![],
            welcome_exts: vec![],
            explicit_session: false,
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

    fn write_json(&self, value: &impl Serialize) {
        if let Ok(json) = serde_json::to_string(value) {
            let line = json + "\n";
            if let Ok(mut w) = self.writer.write() {
                let _ = w.write_all(line.as_bytes());
            }
        }
    }

    fn handle_command(&self, cmd: RpcCommand) {
        let id = &cmd.id;
        let cmd_type = &cmd.cmd_type;

        let resp = match cmd_type.as_str() {
            "prompt" => {
                // Start prompt/agent loop
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
            "new_session_with_prompt" => {
                let _ = self.session.write().unwrap().new_session();
                let _ = self.session.write().unwrap().prompt(&cmd.message, &[], "");
                RpcResponse::ok(id, "new_session_with_prompt", serde_json::json!({}))
            }
            "fork" => {
                match self.session.write().unwrap().fork(&cmd.entry_id) {
                    Ok(_) => RpcResponse::ok(id, "fork", serde_json::json!({})),
                    Err(e) => RpcResponse::fail(id, "fork", &e.to_string()),
                }
            }
            _ => {
                RpcResponse::fail(id, cmd_type, &format!("unknown command: {}", cmd_type))
            }
        };

        self.write_json(&resp);
    }

    fn get_state(&self) -> serde_json::Value {
        let sess = self.session.read().unwrap();
        serde_json::json!({
            "model": sess.model,
            "thinkingLevel": sess.thinking_level,
            "isStreaming": false,
            "isCompacting": false,
            "steeringMode": sess.steering_mode,
            "followUpMode": sess.follow_up_mode,
            "sessionId": sess.session_id(),
            "explicitSession": self.explicit_session,
            "autoCompactionEnabled": sess.auto_compaction,
            "messageCount": sess.messages.len() as i32,
            "pendingMessageCount": 0,
            "version": self.welcome_version,
            "cwd": self.welcome_cwd,
            "skills": self.welcome_skills,
            "contextFiles": self.welcome_context,
            "extensions": self.welcome_exts,
        })
    }

    /// Run the server over stdin/stdout (JSONL protocol).
    pub async fn run_stdio(&self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        // Send welcome event
        self.write_json(&serde_json::json!({
            "type": "welcome",
            "version": self.welcome_version,
            "cwd": self.welcome_cwd,
            "skills": self.welcome_skills,
            "contextFiles": self.welcome_context,
            "extensions": self.welcome_exts,
        }));

        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<RpcCommand>(&line) {
                Ok(cmd) => self.handle_command(cmd),
                Err(e) => {
                    let resp = RpcResponse::fail("", "unknown", &format!("parse error: {}", e));
                    self.write_json(&resp);
                }
            }
        }
        Ok(())
    }

    /// Run over TCP port
    pub async fn run_tcp(&self, addr: &str) -> Result<()> {
        let listener = TcpListener::bind(addr).await?;
        let (socket, _) = listener.accept().await?;
        let (reader, writer) = socket.into_split();
        let mut server = Server::new(self.session.clone(), writer);
        server.set_welcome(&self.welcome_version, &self.welcome_cwd, self.welcome_skills.clone(), self.welcome_context.clone(), self.welcome_exts.clone());
        server.explicit_session = self.explicit_session;
        let reader = BufReader::new(reader);
        let mut lines = reader.lines();

        // Send welcome
        server.write_json(&serde_json::json!({
            "type": "welcome",
            "version": server.welcome_version,
            "cwd": server.welcome_cwd,
            "skills": server.welcome_skills,
            "contextFiles": server.welcome_context,
            "extensions": server.welcome_exts,
        }));

        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() { continue; }
            match serde_json::from_str::<RpcCommand>(&line) {
                Ok(cmd) => server.handle_command(cmd),
                Err(e) => {
                    let resp = RpcResponse::fail("", "unknown", &format!("parse error: {}", e));
                    server.write_json(&resp);
                }
            }
        }
        Ok(())
    }

    /// Run over Unix socket
    pub async fn run_unix(&self, path: &str) -> Result<()> {
        let listener = UnixListener::bind(path)?;
        let (socket, _) = listener.accept().await?;
        let (reader, writer) = socket.into_split();
        let mut server = Server::new(self.session.clone(), writer);
        server.set_welcome(&self.welcome_version, &self.welcome_cwd, self.welcome_skills.clone(), self.welcome_context.clone(), self.welcome_exts.clone());
        server.explicit_session = self.explicit_session;
        let reader = BufReader::new(reader);
        let mut lines = reader.lines();

        server.write_json(&serde_json::json!({
            "type": "welcome",
            "version": server.welcome_version,
            "cwd": server.welcome_cwd,
            "skills": server.welcome_skills,
            "contextFiles": server.welcome_context,
            "extensions": server.welcome_exts,
        }));

        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() { continue; }
            match serde_json::from_str::<RpcCommand>(&line) {
                Ok(cmd) => server.handle_command(cmd),
                Err(e) => {
                    let resp = RpcResponse::fail("", "unknown", &format!("parse error: {}", e));
                    server.write_json(&resp);
                }
            }
        }
        Ok(())
    }
}

// ─── ServerSession (minimal placeholder) ─────────────────────────────────────

use crate::session::Manager as SessionManager;

pub struct ServerSession {
    pub agent_loop: crate::agent::Loop,
    pub messages: Vec<crate::types::AgentMessage>,
    pub model: String,
    pub thinking_level: String,
    pub steering_mode: String,
    pub follow_up_mode: String,
    pub auto_compaction: bool,
    pub auto_retry: bool,
    pub session_manager: Arc<SessionManager>,
    pub cwd: String,
}

impl ServerSession {
    pub fn new(agent_loop: crate::agent::Loop, manager: Arc<SessionManager>, cwd: &str) -> Self {
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
        }
    }

    pub fn session_id(&self) -> String {
        String::new()
    }

    pub fn prompt(&mut self, msg: &str, _images: &[crate::types::ImageContent], _behavior: &str) -> Result<()> {
        self.messages.push(crate::types::AgentMessage::new_user("user", serde_json::json!([{"type": "text", "text": msg}])));
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

    pub fn abort(&self) {}

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
    }

    pub fn set_steering_mode(&mut self, mode: &str) {
        self.steering_mode = mode.to_string();
    }

    pub fn set_follow_up_mode(&mut self, mode: &str) {
        self.follow_up_mode = mode.to_string();
    }

    pub fn compact(&self, _instructions: &str) -> Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    pub fn set_auto_compaction(&mut self, enabled: bool) {
        self.auto_compaction = enabled;
    }

    pub fn set_auto_retry(&mut self, enabled: bool) {
        self.auto_retry = enabled;
    }

    pub fn execute_bash(&self, command: &str) -> Result<serde_json::Value> {
        let output = std::process::Command::new("bash")
            .args(["-c", command])
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(serde_json::json!({
            "output": format!("{}{}", stdout, if stderr.is_empty() { String::new() } else { format!("\n{}", stderr) }),
            "exitCode": output.status.code().unwrap_or(-1),
        }))
    }

    pub fn get_session_stats(&self) -> serde_json::Value {
        serde_json::json!({
            "sessionId": self.session_id(),
            "userMessages": self.messages.iter().filter(|m| m.role == "user").count(),
            "assistantMessages": self.messages.iter().filter(|m| m.role == "assistant").count(),
            "toolCalls": self.messages.iter().filter(|m| !m.tool_calls.is_empty()).count(),
            "toolResults": self.messages.iter().filter(|m| m.role == "tool").count(),
            "totalMessages": self.messages.len(),
        })
    }

    pub fn list_sessions(&self) -> Result<Vec<serde_json::Value>> {
        let sessions = self.session_manager.list(&self.cwd)?;
        Ok(sessions.into_iter().map(|s| {
            serde_json::json!({
                "id": s.id,
                "cwd": s.cwd,
                "model": s.model,
                "updatedAt": s.updated_at,
            })
        }).collect())
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
}
