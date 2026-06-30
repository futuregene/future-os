use crate::events::EventBus;
use crate::session::Manager;
use crate::types::ConvertToLLM;
use anyhow::Result;
use std::{collections::HashMap, sync::Arc};

use super::{ApprovalGate, SseBroadcaster};

const DEFAULT_PERMISSION_LEVEL: &str = "all";

// ─── ServerSession ────────────────────────────────────────────────────────

pub struct ServerSession {
    pub session_id: String,
    pub agent_loop: Arc<tokio::sync::RwLock<crate::agent::Loop>>,
    pub messages: Arc<std::sync::RwLock<Vec<crate::types::AgentMessage>>>,
    pub model: String,
    /// Shared model name for the auto-compaction closure — updated by
    /// set_model so compaction always uses the current context_window.
    pub compaction_model: Arc<std::sync::RwLock<String>>,
    pub thinking_level: String,
    pub steering_mode: String,
    pub follow_up_mode: String,
    pub auto_compaction: bool,
    pub auto_retry: bool,
    pub session_manager: Arc<Manager>,
    pub cwd: String,
    pub is_streaming: Arc<std::sync::atomic::AtomicBool>,
    pub session_name: String,
    pub event_bus: Arc<EventBus>,
    pub broadcaster: Arc<SseBroadcaster>,
    pub ephemeral: bool,
    /// Cumulative token counters (Arc<AtomicI64> — read lock-free without agent_loop lock)
    pub tokens_in: Arc<std::sync::atomic::AtomicI64>,
    pub tokens_out: Arc<std::sync::atomic::AtomicI64>,
    pub tokens_cache_r: Arc<std::sync::atomic::AtomicI64>,
    pub tokens_cache_w: Arc<std::sync::atomic::AtomicI64>,
    /// Last API call's prompt_tokens (actual context size, reset each call)
    pub last_prompt_tokens: Arc<std::sync::atomic::AtomicI64>,
    /// Sender for steering queue (cloned from loop, usable without loop lock)
    pub steering_tx: tokio::sync::mpsc::Sender<String>,
    /// Sender for follow_up queue
    pub follow_up_tx: tokio::sync::mpsc::Sender<String>,
    /// Sender for interrupting the current stream (per-stream, set in prompt())
    pub interrupt_tx: Option<tokio::sync::mpsc::Sender<()>>,
    pub approval_gate: ApprovalGate,
    /// Permission level for tool execution: "all" | "workspace" | "none"
    pub permission_level: String,
}

/// Default workspace directory for new sessions.
pub fn default_workspace() -> String {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".future")
        .join("agent")
        .join("workspace")
        .to_string_lossy()
        .to_string()
}

impl ServerSession {
    pub fn new(
        session_id: String,
        agent_loop: Arc<tokio::sync::RwLock<crate::agent::Loop>>,
        manager: Arc<Manager>,
        cwd: &str,
        event_bus: Arc<EventBus>,
        broadcaster: Arc<SseBroadcaster>,
        approval_gate: ApprovalGate,
    ) -> Self {
        // Clone token counter Arcs and queue senders from the agent loop for lock-free access
        let (ti, to, tcr, tcw, lpt, stx, ftx) = if let Ok(loop_) = agent_loop.try_read() {
            (
                loop_.cumulative_input_tokens.clone(),
                loop_.cumulative_output_tokens.clone(),
                loop_.cumulative_cache_read_tokens.clone(),
                loop_.cumulative_cache_write_tokens.clone(),
                loop_.last_prompt_tokens.clone(),
                loop_.steering_queue.tx.clone(),
                loop_.follow_up_queue.tx.clone(),
            )
        } else {
            let (stx, _) = tokio::sync::mpsc::channel(64);
            let (ftx, _) = tokio::sync::mpsc::channel(64);
            (
                Arc::new(std::sync::atomic::AtomicI64::new(0)),
                Arc::new(std::sync::atomic::AtomicI64::new(0)),
                Arc::new(std::sync::atomic::AtomicI64::new(0)),
                Arc::new(std::sync::atomic::AtomicI64::new(0)),
                Arc::new(std::sync::atomic::AtomicI64::new(0)),
                stx,
                ftx,
            )
        };
        Self {
            session_id: session_id.clone(),
            agent_loop,
            messages: Arc::new(std::sync::RwLock::new(vec![])),
            model: String::new(),
            thinking_level: "high".to_string(), // Match default
            steering_mode: "one-at-a-time".to_string(),
            follow_up_mode: "one-at-a-time".to_string(),
            auto_compaction: true, // Match default
            auto_retry: true,
            session_manager: manager,
            cwd: cwd.to_string(),
            is_streaming: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session_name: String::new(),
            event_bus,
            broadcaster,
            ephemeral: false,
            tokens_in: ti,
            tokens_out: to,
            tokens_cache_r: tcr,
            tokens_cache_w: tcw,
            last_prompt_tokens: lpt,
            steering_tx: stx,
            follow_up_tx: ftx,
            interrupt_tx: None,
            approval_gate,
            permission_level: DEFAULT_PERMISSION_LEVEL.to_string(),
            compaction_model: Arc::new(std::sync::RwLock::new(String::new())),
        }
    }

    /// Create a new session with the same agent_loop but cleared state
    pub fn new_with_shared_loop(
        session_id: String,
        agent_loop: Arc<tokio::sync::RwLock<crate::agent::Loop>>,
        manager: Arc<Manager>,
        cwd: &str,
        event_bus: Arc<EventBus>,
        broadcaster: Arc<SseBroadcaster>,
        approval_gate: ApprovalGate,
    ) -> Self {
        let (stx, ftx) = if let Ok(loop_) = agent_loop.try_read() {
            (
                loop_.steering_queue.tx.clone(),
                loop_.follow_up_queue.tx.clone(),
            )
        } else {
            let (stx, _) = tokio::sync::mpsc::channel(64);
            let (ftx, _) = tokio::sync::mpsc::channel(64);
            (stx, ftx)
        };
        Self {
            session_id: session_id.clone(),
            agent_loop,
            messages: Arc::new(std::sync::RwLock::new(vec![])),
            model: String::new(),
            thinking_level: "high".to_string(),
            steering_mode: "one-at-a-time".to_string(),
            follow_up_mode: "one-at-a-time".to_string(),
            auto_compaction: true,
            auto_retry: true,
            session_manager: manager,
            cwd: cwd.to_string(),
            is_streaming: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session_name: String::new(),
            event_bus,
            broadcaster,
            ephemeral: false,
            tokens_in: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            tokens_out: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            tokens_cache_r: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            tokens_cache_w: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            last_prompt_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            steering_tx: stx,
            follow_up_tx: ftx,
            interrupt_tx: None,
            approval_gate,
            permission_level: DEFAULT_PERMISSION_LEVEL.to_string(),
            compaction_model: Arc::new(std::sync::RwLock::new(String::new())),
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

    pub fn steer(&mut self, msg: &str) -> Result<()> {
        let _ = self.steering_tx.try_send(msg.to_string());
        if let Some(ref tx) = self.interrupt_tx {
            let _ = tx.try_send(());
        }
        Ok(())
    }

    pub fn follow_up(&mut self, msg: &str) -> Result<()> {
        if !self.is_streaming.load(std::sync::atomic::Ordering::Relaxed) {
            return self.prompt(msg, &[], "");
        }
        let _ = self.follow_up_tx.try_send(msg.to_string());
        Ok(())
    }

    pub fn abort(&self) {
        // Primary path: send via interrupt_tx for streaming-mode abort
        if let Some(ref tx) = self.interrupt_tx {
            let _ = tx.try_send(());
        }
        // Set AgentLoop interrupt flag (works with read lock, even during tool execution)
        if let Ok(loop_) = self.agent_loop.try_read() {
            loop_.abort();
        }
        self.is_streaming
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn new_session(&mut self) -> Result<()> {
        self.messages.write().unwrap().clear();
        Ok(())
    }

    pub fn get_messages(&self) -> Vec<crate::types::Message> {
        let msgs = self.messages.read().unwrap();
        ConvertToLLM(&msgs)
    }

    pub fn set_model(&mut self, model: &str) -> Result<()> {
        // Resolve model config from registry to get base_url, compat settings, etc.
        let registry = crate::models::Registry::new();
        let resolved = registry.resolve(model);
        // Use canonical model ID (strip provider prefix if present)
        self.model = resolved
            .as_ref()
            .map(|m| m.id.clone())
            .unwrap_or_else(|| model.to_string());
        // Keep compaction closure in sync so /model changes are reflected.
        *self.compaction_model.write().unwrap() = self.model.clone();

        // Update the agent loop in one shot — both model name and provider endpoint.
        // Uses try_write: if a prompt is actively streaming (holding the write lock),
        // skip this update. The caller should retry or set_model before prompting.
        if let Ok(mut loop_) = self.agent_loop.try_write() {
            // Use resolved model's canonical ID (strip provider prefix if present)
            if let Some(ref mc) = resolved {
                loop_.model = mc.id.clone();
            } else {
                loop_.model = model.to_string();
            }

            if let Some(model_config) = resolved {
                if !model_config.input.iter().any(|input| input == "image") {
                    self.strip_image_content_from_messages();
                }

                let thinking_format = model_config
                    .compat
                    .get("thinkingFormat")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let supports_reasoning_effort = model_config
                    .compat
                    .get("supportsReasoningEffort")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let requires_reasoning_on_assistant = model_config
                    .compat
                    .get("requiresReasoningContentOnAssistantMessages")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let tlm: HashMap<String, String> = model_config
                    .thinking_level_map
                    .into_iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                    .collect();

                let auth = crate::AuthStore::load();
                let api_key = auth
                    .get(model)
                    .or_else(|| auth.get(&model_config.provider))
                    .or_else(|| {
                        if model_config.api_key.is_empty() {
                            None
                        } else {
                            Some(model_config.api_key.clone())
                        }
                    })
                    .or_else(|| auth.default_key())
                    .unwrap_or_default();

                loop_.provider.update_compat(
                    &thinking_format,
                    supports_reasoning_effort,
                    requires_reasoning_on_assistant,
                    tlm,
                );
                // maxTokensField: compat field controlling max_tokens vs max_completion_tokens
                if let Some(field) = model_config
                    .compat
                    .get("maxTokensField")
                    .and_then(|v| v.as_str())
                {
                    loop_.provider.update_max_tokens_field(field);
                } else {
                    loop_.provider.update_max_tokens_field("max_tokens");
                }
                loop_
                    .provider
                    .update_endpoint(&model_config.base_url, &api_key);
            }
        }
        Ok(())
    }

    fn strip_image_content_from_messages(&self) {
        if let Ok(mut messages) = self.messages.write() {
            for message in messages.iter_mut() {
                message
                    .content
                    .retain(|block| !matches!(block, crate::types::ContentBlock::Image { .. }));
            }
        }
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
        if let Ok(mut loop_) = self.agent_loop.try_write() {
            loop_.config.thinking_budget = budget;
            loop_.provider.update_thinking(level, budget);
        }
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
        use std::sync::atomic::Ordering;

        let messages: Vec<crate::types::Message> = {
            let msgs = self.messages.read().unwrap();
            ConvertToLLM(&msgs)
        };

        // Use API-reported prompt_tokens (same as getState's contextTokens)
        let tokens_before = self.last_prompt_tokens.load(Ordering::Relaxed) as i32;

        // Resolve context_window from model registry (same as getState's contextWindow)
        let context_window = crate::models::Registry::new()
            .resolve(&self.model)
            .map(|m| m.context_window)
            .or_else(|| {
                crate::models::builtin_models()
                    .into_iter()
                    .find(|m| m.id == self.model)
                    .map(|m| m.context_window)
            })
            .unwrap_or(200000);

        let (compacted, result) = crate::compaction::compact(
            messages,
            &crate::compaction::CompactOptions {
                reserve_tokens: 160000,
                keep_recent_tokens: 80000,
                context_window,
                tokens_before,
            },
        );

        if let Some(r) = result {
            let tokens_after = crate::compaction::estimate_context_tokens(&compacted);
            let messages_removed = (tokens_before - tokens_after).max(0);
            Ok(serde_json::json!({
                "tokensBefore": tokens_before,
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
        let selected: Vec<_> = all_tools
            .into_iter()
            .filter(|t| tool_names.contains(&t.def.function.name))
            .collect();
        self.agent_loop.try_write().unwrap().tools = selected;
    }

    pub fn disable_tools(&mut self) {
        self.agent_loop.try_write().unwrap().tools = vec![];
    }

    pub fn disable_builtin_tools(&mut self) {
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
            .current_dir(&self.cwd)
            .env("PWD", &self.cwd)
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
        let msgs = self.messages.read().unwrap();
        serde_json::json!({
            "sessionFile": "",
            "sessionId": self.session_id(),
            "userMessages": msgs.iter().filter(|m| m.role == "user").count(),
            "assistantMessages": msgs.iter().filter(|m| m.role == "assistant").count(),
            "toolCalls": msgs.iter().filter(|m| !m.tool_calls.is_empty()).count(),
            "toolResults": msgs.iter().filter(|m| m.role == "tool").count(),
            "totalMessages": msgs.len(),
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

    pub fn switch_session(&mut self, id: &str) -> Result<()> {
        if let Some(path) = self.session_manager.find(id) {
            let session = self.session_manager.load_path(&path, id)?;
            let msgs = crate::session::entries_to_agent_messages(&session.entries);
            if !session.model.is_empty() {
                self.model = session.model.clone();
            }
            // Restore session name from label entries (via load_path) or session_info
            if !session.name.is_empty() {
                self.session_name = session.name.clone();
            }
            // Restore metadata from session_info entry
            if let Some(info) = session.get_session_info() {
                if let Some(tl) = info.get("thinking_level").and_then(|v| v.as_str()) {
                    self.thinking_level = tl.to_string();
                }
                if self.session_name.is_empty() {
                    if let Some(name) = info.get("session_name").and_then(|v| v.as_str()) {
                        self.session_name = name.to_string();
                    }
                }
                if let Some(v) = info.get("auto_compaction").and_then(|v| v.as_bool()) {
                    self.auto_compaction = v;
                }
                // Restore cwd from session_info (previously lost after agent restart)
                if let Some(saved_cwd) = info.get("cwd").and_then(|v| v.as_str()) {
                    self.cwd = saved_cwd.to_string();
                }
                use std::sync::atomic::Ordering;
                let restore_i64 = |key: &str, target: &std::sync::atomic::AtomicI64| {
                    if let Some(v) = info.get(key).and_then(|v| v.as_i64()) {
                        target.store(v, Ordering::Relaxed);
                    }
                };
                restore_i64("tokens_in", &self.tokens_in);
                restore_i64("tokens_out", &self.tokens_out);
                restore_i64("tokens_cache_r", &self.tokens_cache_r);
                restore_i64("tokens_cache_w", &self.tokens_cache_w);
                restore_i64("last_prompt_tokens", &self.last_prompt_tokens);
            }
            *self.messages.write().unwrap() = msgs;
            self.session_id = id.to_string();
        }
        Ok(())
    }

    pub fn delete_session(&self, _id: &str) -> Result<()> {
        Ok(())
    }

    pub fn fork(&mut self, _entry_id: &str) -> Result<()> {
        Ok(())
    }

    pub fn set_cwd(&mut self, cwd: &str) {
        self.cwd = cwd.to_string();
    }

    pub fn set_permission_level(&mut self, level: &str) {
        self.permission_level = level.to_string();
    }

    pub fn get_permission_level(&self) -> &str {
        &self.permission_level
    }

    pub fn get_last_assistant_text(&self) -> String {
        let msgs = self.messages.read().unwrap();
        msgs.iter()
            .rfind(|m| m.role == "assistant")
            .map(|m| m.text())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::Loop,
        types::{LLMProvider, Message, StreamEvent, ToolDef},
    };
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    struct EmptyProvider;

    #[async_trait::async_trait]
    impl LLMProvider for EmptyProvider {
        async fn stream_chat(
            &self,
            _model: String,
            _messages: Vec<Message>,
            _tools: Vec<ToolDef>,
            _system_prompt: String,
        ) -> anyhow::Result<ReceiverStream<StreamEvent>> {
            let (_tx, rx) = mpsc::channel(1);
            Ok(ReceiverStream::new(rx))
        }
    }

    fn test_workspace() -> String {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("futureos-session-default-permission-{stamp}"))
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn new_sessions_default_to_workspace_permission() {
        let cwd = test_workspace();
        let session = ServerSession::new(
            "session_test".to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(EmptyProvider),
                "mock",
            ))),
            Arc::new(Manager::default_for(&cwd)),
            &cwd,
            Arc::new(EventBus::new()),
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
        );

        assert_eq!(session.get_permission_level(), "workspace");
    }
}
