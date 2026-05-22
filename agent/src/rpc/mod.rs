//! RPC Server - Command handling for gRPC

use crate::events::EventBus;
use crate::session::Manager;
use crate::types::ConvertToLLM;
use anyhow::Result;
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
    #[serde(default)]
    pub cwd: String,

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

    // set_enabled_models
    #[serde(default)]
    pub enabled_models: Option<Vec<String>>,
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
        let (tx, _) = broadcast::channel(4096);
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
    pub messages: Arc<std::sync::RwLock<Vec<crate::types::AgentMessage>>>,
    pub model: String,
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
}

impl ServerSession {
    pub fn new(
        session_id: String,
        agent_loop: Arc<tokio::sync::RwLock<crate::agent::Loop>>,
        manager: Arc<Manager>,
        cwd: &str,
        event_bus: Arc<EventBus>,
        broadcaster: Arc<SseBroadcaster>,
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
            thinking_level: "high".to_string(), // Match Go default
            steering_mode: "one-at-a-time".to_string(),
            follow_up_mode: "one-at-a-time".to_string(),
            auto_compaction: true, // Match Go default
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

    pub fn prompt(
        &mut self,
        msg: &str,
        _images: &[crate::types::ImageContent],
        _behavior: &str,
    ) -> Result<()> {
        // Add message to session
        self.messages
            .write()
            .unwrap()
            .push(crate::types::AgentMessage::new_user(
                "user",
                serde_json::json!([{"type": "text", "text": msg}]),
            ));

        // Set streaming flag
        self.is_streaming
            .store(true, std::sync::atomic::Ordering::Relaxed);

        // Swap per-session token counters into the agent loop so updates are tracked per-session
        {
            if let Ok(mut r#loop) = self.agent_loop.try_write() {
                r#loop.cumulative_input_tokens = self.tokens_in.clone();
                r#loop.cumulative_output_tokens = self.tokens_out.clone();
                r#loop.cumulative_cache_read_tokens = self.tokens_cache_r.clone();
                r#loop.cumulative_cache_write_tokens = self.tokens_cache_w.clone();
                r#loop.last_prompt_tokens = self.last_prompt_tokens.clone();
            }
        }

        // Wire auto-compaction transform (checked before each turn)
        if self.auto_compaction {
            if let Ok(mut r#loop) = self.agent_loop.try_write() {
                let comp_tokens = self.last_prompt_tokens.clone();
                let comp_model = self.model.clone();
                let comp_result = r#loop.last_compaction_result.clone();
                r#loop.config.transform_context = Some(Arc::new(move |msgs, _| {
                    use std::sync::atomic::Ordering;
                    let context_tokens = comp_tokens.load(Ordering::Relaxed) as i32;
                    if context_tokens == 0 {
                        return msgs; // No API call made yet, nothing to compact
                    }
                    let context_window = crate::models::Registry::new()
                        .resolve(&comp_model)
                        .map(|m| m.context_window)
                        .unwrap_or(200000);
                    let (compacted, result) = crate::compaction::compact(
                        msgs,
                        &crate::compaction::CompactOptions {
                            reserve_tokens: 16384,
                            keep_recent_tokens: 20000,
                            context_window,
                            tokens_before: context_tokens,
                        },
                    );
                    if let Some(r) = result {
                        *comp_result.lock().unwrap() = Some(r);
                        compacted
                    } else {
                        compacted
                    }
                }));
            }
        }

        // Clone shared state for the background task
        let messages_arc = self.messages.clone();
        let initial_messages = messages_arc.read().unwrap().clone();
        let agent_loop = self.agent_loop.clone();
        let broadcaster = self.broadcaster.clone();
        let is_streaming = self.is_streaming.clone();
        let session_manager = self.session_manager.clone();
        let session_id = self.session_id.clone();
        let session_cwd = self.cwd.clone();
        let session_model = self.model.clone();
        let session_thinking = self.thinking_level.clone();
        let tokens_in = self.tokens_in.clone();
        let tokens_out = self.tokens_out.clone();
        let tokens_cache_r = self.tokens_cache_r.clone();
        let tokens_cache_w = self.tokens_cache_w.clone();
        let last_prompt = self.last_prompt_tokens.clone();
        let session_name = self.session_name.clone();
        let auto_compaction = self.auto_compaction;

        // Set tool event callback so tool_start/tool_end reach the TUI
        {
            let broadcaster_tool = broadcaster.clone();
            if let Ok(mut r#loop) = agent_loop.try_write() {
                r#loop.tool_event_callback =
                    Some(Arc::new(move |event: crate::types::StreamEvent| {
                        let mut data = serde_json::Map::new();
                        data.insert("type".to_string(), serde_json::json!(&event.event_type));
                        if !event.tool_name.is_empty() {
                            data.insert(
                                "tool_name".to_string(),
                                serde_json::json!(&event.tool_name),
                            );
                        }
                        if !event.tool_id.is_empty() {
                            data.insert("tool_id".to_string(), serde_json::json!(&event.tool_id));
                        }
                        if !event.text.is_empty() {
                            data.insert("text".to_string(), serde_json::json!(&event.text));
                        }
                        if !event.error_text.is_empty() {
                            data.insert("error".to_string(), serde_json::json!(&event.error_text));
                        }
                        if let Some(ref tc) = event.tool_call {
                            data.insert("tool_args".to_string(), tc.function.arguments.clone());
                        }
                        broadcaster_tool.broadcast(crate::rpc::SseEvent {
                            event_type: event.event_type.clone(),
                            data: serde_json::to_string(&data).unwrap_or_default(),
                        });
                    }));
            }
        }

        // agent_start is now emitted inside run_streaming_with_messages via on_event,
        // for both initial prompts and follow-up turns.

        // Create interrupt channel so steer()/abort() can stop the current stream
        let (interrupt_tx, interrupt_rx) = tokio::sync::mpsc::channel::<()>(1);
        self.interrupt_tx = Some(interrupt_tx);

        // Spawn background task to run agent loop
        tokio::spawn(async move {
            // Run with timeout — generous limit for complex multi-turn tasks
            let result = tokio::time::timeout(std::time::Duration::from_secs(3600), async {
                let mut current_messages = initial_messages;
                let mut current_interrupt_rx = Some(interrupt_rx);

                loop {
                    let bt = broadcaster.clone();
                    let be = broadcaster.clone();
                    let r#loop = agent_loop.write().await;

                    match r#loop
                        .run_streaming_with_messages(
                            current_messages,
                            move |text| {
                                bt.broadcast(crate::rpc::SseEvent {
                                    event_type: "text_chunk".to_string(),
                                    data: serde_json::json!({"text": text}).to_string(),
                                });
                            },
                            move |event| {
                                let mut data = serde_json::Map::new();
                                data.insert(
                                    "type".to_string(),
                                    serde_json::json!(&event.event_type),
                                );
                                if !event.text.is_empty() {
                                    data.insert("text".to_string(), serde_json::json!(&event.text));
                                }
                                if !event.tool_name.is_empty() {
                                    data.insert(
                                        "tool_name".to_string(),
                                        serde_json::json!(&event.tool_name),
                                    );
                                }
                                if !event.tool_id.is_empty() {
                                    data.insert(
                                        "tool_id".to_string(),
                                        serde_json::json!(&event.tool_id),
                                    );
                                }
                                if !event.error_text.is_empty() {
                                    data.insert(
                                        "error".to_string(),
                                        serde_json::json!(&event.error_text),
                                    );
                                }
                                if !event.stop_reason.is_empty() {
                                    data.insert(
                                        "stopReason".to_string(),
                                        serde_json::json!(&event.stop_reason),
                                    );
                                }
                                if let Some(usage) = &event.usage {
                                    data.insert("usage".to_string(), serde_json::json!(usage));
                                }
                                if let Some(ref tc) = event.tool_call {
                                    data.insert(
                                        "tool_args".to_string(),
                                        tc.function.arguments.clone(),
                                    );
                                }
                                be.broadcast(crate::rpc::SseEvent {
                                    event_type: event.event_type.clone(),
                                    data: serde_json::to_string(&data).unwrap_or_default(),
                                });
                            },
                            current_interrupt_rx.take(),
                        )
                        .await
                    {
                        Ok((_, final_messages)) => {
                            current_messages = final_messages;

                            let follow_ups = r#loop.follow_up_queue.drain();
                            drop(r#loop);

                            if follow_ups.is_empty() {
                                return Ok(current_messages);
                            }
                            for msg in follow_ups {
                                current_messages.push(crate::types::AgentMessage::new_user(
                                    "user",
                                    serde_json::json!([{"type": "text", "text": msg}]),
                                ));
                            }
                            // No interrupt channel for follow-up re-runs
                            current_interrupt_rx = None;
                        }
                        Err(e) => return Err(e),
                    }
                }
            })
            .await;

            match result {
                Ok(Ok(final_messages)) => {
                    // Update shared messages so next prompt includes the full context
                    match messages_arc.write() {
                        Ok(mut msgs) => {
                            *msgs = final_messages;
                        }
                        Err(e) => {
                            let mut msgs = e.into_inner();
                            *msgs = final_messages;
                        }
                    }
                    // Save session to disk
                    {
                        let msgs = messages_arc.read().unwrap();
                        let mut entries: Vec<crate::session::SessionEntry> = msgs
                            .iter()
                            .map(crate::session::agent_message_to_entry)
                            .collect();

                        // Prepend session_info entry with metadata
                        use std::sync::atomic::Ordering;
                        // Preserve parent_session_id from existing session on disk
                        let parent_session_id = session_manager
                            .load(&session_id)
                            .map(|s| s.parent_session_id)
                            .unwrap_or_default();
                        let info = serde_json::json!({
                            "cwd": session_cwd,
                            "model": session_model,
                            "thinking_level": session_thinking,
                            "tokens_in": tokens_in.load(Ordering::Relaxed),
                            "tokens_out": tokens_out.load(Ordering::Relaxed),
                            "tokens_cache_r": tokens_cache_r.load(Ordering::Relaxed),
                            "tokens_cache_w": tokens_cache_w.load(Ordering::Relaxed),
                            "last_prompt_tokens": last_prompt.load(Ordering::Relaxed),
                            "session_name": session_name,
                            "auto_compaction": auto_compaction,
                            "parent_session_id": parent_session_id,
                        });
                        let info_entry = crate::session::SessionEntry {
                            id: crate::utils::generate_entry_id(),
                            parent_id: String::new(),
                            entry_type: crate::session::ENTRY_TYPE_SESSION_INFO.to_string(),
                            role: "system".to_string(),
                            content: Some(info),
                            tool_calls: vec![],
                            timestamp: chrono::Local::now(),
                            summary: String::new(),
                            model: session_model.clone(),
                            label: String::new(),
                            thinking_level: session_thinking.clone(),
                            branch_summary: None,
                            custom_type: String::new(),
                            custom_data: None,
                            display: String::new(),
                            provider: String::new(),
                            tool_call_id: String::new(),
                        };
                        entries.insert(0, info_entry);

                        let session = crate::session::Session {
                            id: session_id.clone(),
                            version: crate::session::CURRENT_SESSION_VERSION,
                            cwd: session_cwd.clone(),
                            model: session_model.clone(),
                            base_url: String::new(),
                            name: String::new(),
                            parent_session_id,
                            leaf_id: String::new(),
                            entries,
                            created_at: chrono::Local::now(),
                            updated_at: chrono::Local::now(),
                        };
                        if let Err(e) = session_manager.save(&session) {
                            eprintln!("Failed to save session: {}", e);
                        }
                    }
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "agent_end".to_string(),
                        data: serde_json::json!({"type": "agent_end"}).to_string(),
                    });
                    is_streaming.store(false, std::sync::atomic::Ordering::Relaxed);
                }
                Ok(Err(e)) => {
                    eprintln!("Agent loop error: {}", e);
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "error".to_string(),
                        data: serde_json::json!({"error": e.to_string()}).to_string(),
                    });
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "agent_end".to_string(),
                        data: serde_json::json!({"type": "agent_end", "error": e.to_string()})
                            .to_string(),
                    });
                    is_streaming.store(false, std::sync::atomic::Ordering::Relaxed);
                }
                Err(_timeout) => {
                    eprintln!("Agent loop timed out after 10 min");
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "error".to_string(),
                        data: serde_json::json!({"error": "The request took too long (10 minute timeout). Try a simpler prompt, or break the task into smaller steps."}).to_string(),
                    });
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "agent_end".to_string(),
                        data: serde_json::json!({"type": "agent_end", "error": "Request timed out after 10 minutes."})
                            .to_string(),
                    });
                    is_streaming.store(false, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });

        Ok(())
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
        // Primary path: send via interrupt_tx (works even when agent_loop is locked)
        if let Some(ref tx) = self.interrupt_tx {
            let _ = tx.try_send(());
        }
        // Fallback: try loop directly (only works when not streaming)
        if let Ok(loop_) = self.agent_loop.try_write() {
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
                // maxTokensField: pi compat field controlling max_tokens vs max_completion_tokens
                if let Some(field) = model_config
                    .compat
                    .get("maxTokensField")
                    .and_then(|v| v.as_str())
                {
                    loop_.provider.update_max_tokens_field(field);
                } else if model_config.reasoning
                    && !model_config.compat.contains_key("thinkingFormat")
                {
                    // Fallback: reasoning models without thinkingFormat need max_completion_tokens
                    loop_
                        .provider
                        .update_max_tokens_field("max_completion_tokens");
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

    pub fn get_last_assistant_text(&self) -> String {
        let msgs = self.messages.read().unwrap();
        msgs.iter()
            .rfind(|m| m.role == "assistant")
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
    pub welcome_skills: Arc<RwLock<Vec<String>>>,
    pub welcome_context: Arc<RwLock<Vec<String>>>,
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
        {
            let sessions = self.sessions.read().unwrap();
            if let Some(sess) = sessions.get(session_id) {
                return sess.clone();
            }
        }
        // Session not found in map — if it matches the default session's own
        // ID, return it silently. Otherwise warn and fall back to default.
        let default_id = self.session.read().unwrap().session_id.clone();
        if session_id == default_id {
            return self.session.clone();
        }
        eprintln!(
            "[get_session] session_id={} not found in sessions map, falling back to default",
            session_id
        );
        self.session.clone()
    }

    /// Create a new session and return its ID.
    /// Each session gets its own private SseBroadcaster so events are only
    /// delivered to subscribers of that specific session (not globally).
    pub fn create_session(&self, mut session: ServerSession) -> String {
        let id = session.session_id.clone();
        session.broadcaster = Arc::new(SseBroadcaster::new());
        self.sessions
            .write()
            .unwrap()
            .insert(id.clone(), Arc::new(RwLock::new(session)));
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
            let mut sess = session.write().unwrap();
            if sess.is_streaming.load(std::sync::atomic::Ordering::Relaxed) {
                RpcResponse::build_fail(
                    id,
                    "prompt",
                    "agent is still streaming; wait or abort first",
                )
            } else {
                let _ = sess.prompt(&cmd.message, &cmd.images, &cmd.streaming_behavior);
                RpcResponse::ok(id, "prompt", serde_json::json!({}))
            }
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
            if let Ok(sess) = session.try_write() {
                sess.abort();
            }
            RpcResponse::ok(id, "abort", serde_json::json!({}))
        }
        "new_session" => {
            // Create a new session with shared agent_loop, preserving model/thinking
            // Use TUI-provided cwd if available, otherwise home directory
            let session_cwd = if !cmd.cwd.is_empty() {
                cmd.cwd.clone()
            } else {
                dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                    .to_string_lossy()
                    .to_string()
            };
            let active_id = state.get_active_session_id();
            let session = state.get_session(&active_id);
            let sess = session.read().unwrap();
            let mut new_sess = ServerSession::new_with_shared_loop(
                crate::utils::generate_id(),
                sess.agent_loop.clone(),
                sess.session_manager.clone(),
                &session_cwd,
                sess.event_bus.clone(),
                sess.broadcaster.clone(),
            );
            // Preserve model and thinking level from the current session
            new_sess.model = sess.model.clone();
            new_sess.thinking_level = sess.thinking_level.clone();
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
            session
                .write()
                .unwrap()
                .set_system_prompt(&cmd.system_prompt);
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
            session
                .write()
                .unwrap()
                .append_system_prompt(&cmd.system_prompt);
            RpcResponse::ok(id, "append_system_prompt", serde_json::json!({}))
        }
        "set_ephemeral" => {
            session.write().unwrap().set_ephemeral(cmd.ephemeral);
            RpcResponse::ok(
                id,
                "set_ephemeral",
                serde_json::json!({"ephemeral": cmd.ephemeral}),
            )
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
            let summaries = session
                .read()
                .unwrap()
                .session_manager
                .list_all()
                .unwrap_or_default();
            // Convert to the format expected by TUI
            let sessions: Vec<serde_json::Value> = summaries
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id,
                        "name": s.name,
                        "model": s.model,
                        "cwd": s.cwd,
                        "updated_at": s.updated_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                        "parent_session_id": s.parent_session_id,
                    })
                })
                .collect();
            RpcResponse::ok(
                id,
                "list_sessions",
                serde_json::json!({"sessions": sessions}),
            )
        }
        "switch_session" => {
            if cmd.session_id.is_empty() {
                return RpcResponse::build_fail(
                    id,
                    "switch_session",
                    "No session selected. Choose a session from the list to switch to.",
                );
            }
            let mut sess = session.write().unwrap();
            let result = match sess.switch_session(&cmd.session_id) {
                Ok(()) => {
                    if let Ok(mut active_id) = state.active_session_id.try_write() {
                        *active_id = cmd.session_id.clone();
                    }
                    // Give this session its own private broadcaster so events
                    // are only delivered to subscribers of this session.
                    sess.broadcaster = Arc::new(SseBroadcaster::new());
                    // Insert into sessions map so subsequent lookups by this
                    // session_id succeed (avoids fallback-to-default warning).
                    if let Ok(mut sessions) = state.sessions.try_write() {
                        sessions.insert(cmd.session_id.clone(), session.clone());
                    }
                    RpcResponse::ok(
                        id,
                        "switch_session",
                        serde_json::json!({"cancelled": false}),
                    )
                }
                Err(e) => RpcResponse::build_fail(id, "switch_session", &e.to_string()),
            };
            result
        }
        "delete_session" => {
            if cmd.session_id.is_empty() {
                return RpcResponse::build_fail(
                    id,
                    "delete_session",
                    "No session selected to delete. Choose a session first.",
                );
            }
            // Delete from disk
            if let Err(e) = session
                .read()
                .unwrap()
                .session_manager
                .delete(&cmd.session_id)
            {
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
                return RpcResponse::build_fail(
                    id,
                    "fork",
                    "No message selected to fork from. Choose a user message to fork at.",
                );
            }

            // Extract needed data from session
            let (agent_loop, session_manager, event_bus, broadcaster, _cwd, session_id) = {
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
            let parent = match session_manager.load(&session_id) {
                Ok(s) => s,
                Err(_) => {
                    return RpcResponse::build_fail(
                        id,
                        "fork",
                        "Session not found on disk — it may have been deleted or moved.",
                    );
                }
            };

            // Fork a new session
            let forked = crate::session::fork_session(&parent, entry_id);
            let forked_id = forked.id.clone();

            // Save the forked session
            if let Err(e) = session_manager.save(&forked) {
                return RpcResponse::build_fail(
                    id,
                    "fork",
                    &format!("failed to save forked session: {}", e),
                );
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
            // Load session from disk to get entry IDs (needed for fork)
            let (session_manager, session_id) = {
                let sess = session.read().unwrap();
                (sess.session_manager.clone(), sess.session_id.clone())
            };
            let user_entries: Vec<serde_json::Value> =
                session_manager
                    .load(&session_id)
                    .map(|s| {
                        s.entries
                        .iter()
                        .filter(|e| e.entry_type == crate::session::ENTRY_TYPE_USER)
                        .map(|e| {
                            let content_text = e.content.as_ref()
                                .map(|c| {
                                    if let Some(arr) = c.as_array() {
                                        arr.iter()
                                            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                            .collect::<Vec<_>>()
                                            .join(" ")
                                    } else {
                                        c.as_str().unwrap_or("").to_string()
                                    }
                                })
                                .unwrap_or_default();
                            serde_json::json!({
                                "id": e.id,
                                "role": e.role,
                                "content": content_text,
                                "timestamp": e.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
                            })
                        })
                        .collect()
                    })
                    .unwrap_or_default();
            RpcResponse::ok(
                id,
                "get_fork_messages",
                serde_json::json!({"messages": user_entries}),
            )
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
            let (session_manager, session_id) = {
                let mut sess = session.write().unwrap();
                sess.set_session_name(&cmd.name);
                (sess.session_manager.clone(), sess.session_id.clone())
            };
            // Persist label entry to session JSONL so name survives restarts
            if let Ok(mut s) = session_manager.load(&session_id) {
                s.entries.push(crate::session::SessionEntry {
                    id: crate::utils::generate_entry_id(),
                    parent_id: String::new(),
                    entry_type: crate::session::ENTRY_TYPE_LABEL.to_string(),
                    role: String::new(),
                    content: None,
                    tool_calls: vec![],
                    timestamp: chrono::Local::now(),
                    summary: String::new(),
                    model: String::new(),
                    label: cmd.name.clone(),
                    thinking_level: String::new(),
                    branch_summary: None,
                    custom_type: String::new(),
                    custom_data: None,
                    display: String::new(),
                    provider: String::new(),
                    tool_call_id: String::new(),
                });
                s.name = cmd.name.clone();
                let _ = session_manager.save(&s);
            }
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

            let commands: Vec<serde_json::Value> = skills
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "name": s.name,
                        "description": s.description,
                        "source": "skill"
                    })
                })
                .collect();

            RpcResponse::ok(
                id,
                "get_commands",
                serde_json::json!({"commands": commands}),
            )
        }
        "abort_retry" => {
            if let Ok(sess) = session.try_write() {
                sess.abort();
            }
            RpcResponse::ok(id, "abort_retry", serde_json::json!({}))
        }
        "abort_bash" => {
            // Bash abort is handled by the agent loop
            RpcResponse::ok(id, "abort_bash", serde_json::json!({}))
        }
        "cycle_model" => {
            // Cycle to next model, respecting enabled_models from settings.
            let registry = crate::models::Registry::new();
            let auth = crate::AuthStore::load();
            let settings_path = std::path::PathBuf::from(crate::models::settings_path());
            let settings = crate::config::load_settings(&settings_path).unwrap_or_default();

            let models: Vec<String> = if !settings.enabled_models.is_empty() {
                // Use scoped models (from enabled_models) filtered by auth
                let scoped = registry.resolve_scope(&settings.enabled_models, &auth);
                if scoped.is_empty() {
                    return RpcResponse::ok(
                        id,
                        "cycle_model",
                        serde_json::json!({"model": "", "thinkingLevel": "", "isScoped": true}),
                    );
                }
                scoped
            } else {
                // Fall back to all auth-configured models
                let available: Vec<String> = registry
                    .all_models()
                    .into_iter()
                    .filter(|m| !m.api_key.is_empty() || auth.get(&m.provider).is_some())
                    .map(|m| m.id)
                    .collect();
                available
            };

            if models.is_empty() {
                return RpcResponse::ok(
                    id,
                    "cycle_model",
                    serde_json::json!({"model": "", "thinkingLevel": "", "isScoped": false}),
                );
            }

            let is_scoped = !settings.enabled_models.is_empty();
            let current = session.read().unwrap().model.clone();
            let idx = models.iter().position(|m| m == &current).unwrap_or(0);
            let next_idx = (idx + 1) % models.len();
            let next_model = &models[next_idx];

            // Use set_model to update session, agent_loop, compat, and endpoint
            let _ = session.write().unwrap().set_model(next_model);

            RpcResponse::ok(
                id,
                "cycle_model",
                serde_json::json!({
                    "model": next_model,
                    "thinkingLevel": session.read().unwrap().thinking_level.clone(),
                    "isScoped": is_scoped
                }),
            )
        }
        "cycle_thinking_level" => {
            // Cycle thinking level: off -> minimal -> low -> medium -> high -> xhigh -> off
            let levels = ["off", "minimal", "low", "medium", "high", "xhigh"];
            let current = session.read().unwrap().thinking_level.clone();
            let idx = levels.iter().position(|l| *l == current).unwrap_or(0);
            let next_idx = (idx + 1) % levels.len();
            let next_level = levels[next_idx];

            // Update session thinking level and propagate to provider
            session.write().unwrap().set_thinking_level(next_level);

            RpcResponse::ok(
                id,
                "cycle_thinking_level",
                serde_json::json!({"level": next_level}),
            )
        }
        "get_available_models" => {
            let registry = crate::models::Registry::new();
            let auth = crate::AuthStore::load();
            let models: Vec<serde_json::Value> = registry
                .all_models()
                .into_iter()
                .filter(|m| {
                    // Model is available only if its specific provider has an API key in auth.json,
                    // or the model itself has an API key set (from models.json).
                    !m.api_key.is_empty() || auth.get(&m.provider).is_some()
                })
                .map(|m| {
                    let has_image = m.input.contains(&"image".to_string());
                    serde_json::json!({
                        "id": m.id,
                        "name": m.name,
                        "provider": m.provider,
                        "reasoning": m.reasoning,
                        "image": has_image,
                        "contextWindow": m.context_window,
                        "maxTokens": m.max_tokens,
                    })
                })
                .collect();
            // Include current enabled_models from settings so the TUI knows the scope
            let settings_path = std::path::PathBuf::from(crate::models::settings_path());
            let settings = crate::config::load_settings(&settings_path).unwrap_or_default();
            let enabled_model_ids: Vec<String> = if settings.enabled_models.is_empty() {
                models
                    .iter()
                    .map(|m| m["id"].as_str().unwrap_or("").to_string())
                    .collect()
            } else {
                settings.enabled_models.clone()
            };
            RpcResponse::ok(
                id,
                "get_available_models",
                serde_json::json!({"models": models, "enabled_model_ids": enabled_model_ids}),
            )
        }
        "set_enabled_models" => {
            let enabled: Vec<String> = cmd.enabled_models.clone().unwrap_or_default();
            let settings_path = std::path::PathBuf::from(crate::models::settings_path());
            let mut settings = crate::config::load_settings(&settings_path).unwrap_or_default();
            settings.enabled_models = enabled;
            if let Err(e) = settings.save(&settings_path) {
                RpcResponse::build_fail(id, "set_enabled_models", &e.to_string())
            } else {
                RpcResponse::ok(id, "set_enabled_models", serde_json::json!({}))
            }
        }
        "clone" => {
            // Extract needed data from session
            let (agent_loop, session_manager, event_bus, broadcaster, _cwd, session_id) = {
                let sess = session.read().unwrap();
                if sess.messages.read().unwrap().is_empty() {
                    return RpcResponse::build_fail(
                        id,
                        "clone",
                        "Nothing to clone — the current session has no messages yet.",
                    );
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
            let parent = match session_manager.load(&session_id) {
                Ok(s) => s,
                Err(_) => {
                    return RpcResponse::build_fail(
                        id,
                        "clone",
                        "Session not found on disk — it may have been deleted or moved.",
                    );
                }
            };

            let leaf_id = parent
                .entries
                .last()
                .map(|e| e.id.clone())
                .unwrap_or_default();
            if leaf_id.is_empty() {
                return RpcResponse::build_fail(
                    id,
                    "clone",
                    "Nothing to clone — no messages found in session.",
                );
            }

            // Fork from leaf
            let forked = crate::session::fork_session(&parent, &leaf_id);
            let forked_id = forked.id.clone();

            // Save the forked session
            if let Err(e) = session_manager.save(&forked) {
                return RpcResponse::build_fail(
                    id,
                    "clone",
                    &format!("failed to save cloned session: {}", e),
                );
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
                return RpcResponse::build_fail(
                    id,
                    "export_html",
                    &format!("failed to write file: {}", e),
                );
            }

            RpcResponse::ok(id, "export_html", serde_json::json!({"path": output_path}))
        }
        "reload_config" => {
            // Re-discover skills and re-read context files, then rebuild system prompt.
            let (cwd, tools) = {
                let sess = session.read().unwrap();
                let loop_ = match sess.agent_loop.try_read() {
                    Ok(l) => l,
                    Err(_) => return RpcResponse::build_fail(
                        id, "reload_config", "agent is busy, retry in a moment",
                    ),
                };
                (sess.cwd.clone(), loop_.tools.clone())
            };

            // Re-discover skills (blocking I/O, no locks held)
            let skill_dirs = vec![
                crate::skills::USER_SKILLS_DIR.to_string(),
                format!("{}/{}", cwd, crate::skills::PROJECT_SKILLS_DIR),
                crate::skills::AGENTS_SKILLS_DIR.to_string(),
            ];
            let skills = crate::skills::discover_skills(&skill_dirs).unwrap_or_default();
            let skill_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();

            // Re-read context files
            let mut agent_content = String::new();
            for fname in &["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
                let p = std::path::Path::new(&cwd).join(fname);
                if p.exists() {
                    if let Ok(content) = std::fs::read_to_string(&p) {
                        agent_content = content;
                        break;
                    }
                }
            }
            let context_lines: Vec<String> = if agent_content.is_empty() {
                vec![]
            } else {
                vec![agent_content.clone()]
            };

            // Rebuild system prompt
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let new_prompt = crate::prompt::build_prompt(&crate::prompt::PromptOptions {
                working_directory: cwd.clone(),
                date: today,
                tools: tools.clone(),
                skills: skills.clone(),
                agent_content: agent_content.clone(),
                ..Default::default()
            });

            // Update welcome_* state for get_state
            *state.welcome_skills.write().unwrap() = skill_names.clone();
            *state.welcome_context.write().unwrap() = context_lines;

            // Update running session's system prompt
            let sess = session.read().unwrap();
            if let Ok(mut r#loop) = sess.agent_loop.try_write() {
                r#loop.system_prompt = new_prompt.clone();
                r#loop.config.system_prompt = new_prompt;
            }

            RpcResponse::ok(id, "reload_config", serde_json::json!({
                "skills": skill_names,
                "contextFiles": if agent_content.is_empty() { vec![] } else { vec!["CLAUDE.md".to_string()] },
            }))
        }
        _ => RpcResponse::build_fail(id, cmd_type, &format!("unknown command: {}", cmd_type)),
    }
}

fn get_state_internal(state: &AppState, session_id: &str) -> serde_json::Value {
    let session = state.get_session(session_id);
    let sess = session.read().unwrap();

    // Resolve context window: registry first (user models), then builtin, then default
    let registry = crate::models::Registry::new();
    let context_window = registry
        .resolve(&sess.model)
        .map(|m| m.context_window)
        .or_else(|| {
            crate::models::builtin_models()
                .into_iter()
                .find(|m| m.id == sess.model)
                .map(|m| m.context_window)
        })
        .unwrap_or(200000) as i64;

    let session_id = sess.session_id();
    let cwd = sess.cwd.clone();

    // Read cumulative token usage directly from Arc<AtomicI64> — lock-free
    use std::sync::atomic::Ordering;
    let tokens_in = sess.tokens_in.load(Ordering::Relaxed);
    let tokens_out = sess.tokens_out.load(Ordering::Relaxed);
    let cache_r = sess.tokens_cache_r.load(Ordering::Relaxed);
    let cache_w = sess.tokens_cache_w.load(Ordering::Relaxed);

    // Compute cost from model pricing
    let total_cost = if let Some(model_config) = registry.resolve(&sess.model) {
        let input_cost = (tokens_in as f64 / 1_000_000.0) * model_config.cost.input;
        let output_cost = (tokens_out as f64 / 1_000_000.0) * model_config.cost.output;
        let cache_read_cost = (cache_r as f64 / 1_000_000.0) * model_config.cost.cache_read;
        let cache_write_cost = (cache_w as f64 / 1_000_000.0) * model_config.cost.cache_write;
        input_cost + output_cost + cache_read_cost + cache_write_cost
    } else {
        0.0
    };

    // Use API-reported prompt_tokens from the last request as actual context usage
    let context_tokens = sess.last_prompt_tokens.load(Ordering::Relaxed);
    let msg_count = sess.messages.read().unwrap().len();
    let context_percent = if context_window > 0 {
        (context_tokens as f64 / context_window as f64) * 100.0
    } else {
        0.0
    };

    serde_json::json!({
        "model": sess.model,
        "thinkingLevel": sess.thinking_level,
        "isStreaming": sess.is_streaming.load(std::sync::atomic::Ordering::Relaxed),
        "isCompacting": false,
        "steeringMode": sess.steering_mode,
        "followUpMode": sess.follow_up_mode,
        "sessionFile": if session_id.is_empty() { serde_json::Value::Null } else { serde_json::Value::String("".to_string()) },
        "sessionId": if session_id.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(session_id) },
        "sessionName": serde_json::Value::Null,
        "explicitSession": state.explicit_session,
        "autoCompactionEnabled": sess.auto_compaction,
        "messageCount": msg_count,
        "pendingMessageCount": sess.agent_loop.try_read().map(|l|l.pending_message_count()).unwrap_or(0),
        "version": crate::utils::VERSION,
        "cwd": cwd,
        "skills": state.welcome_skills.read().unwrap().clone(),
        "contextFiles": state.welcome_context.read().unwrap().clone(),
        "extensions": serde_json::Value::Null,
        "contextWindow": context_window,
        "contextTokens": context_tokens,
        "contextPercent": context_percent,
        "tokensIn": tokens_in,
        "tokensOut": tokens_out,
        "tokensCacheR": cache_r,
        "tokensCacheW": cache_w,
        "totalCost": total_cost,
    })
}

/// Generate HTML representation of a session (matches Go exportSessionToHTML)
fn generate_session_html(
    session_id: &str,
    model: &str,
    cwd: &str,
    messages: &[crate::types::Message],
) -> String {
    let mut html = String::new();

    html.push_str("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">");
    html.push_str(&format!(
        "<title>FutureAgent session {}</title>",
        session_id
    ));
    html.push_str("<style>");
    html.push_str("body{font-family:system-ui;max-width:800px;margin:auto;padding:20px;background:#1a1a2e;color:#e0e0e0}");
    html.push_str(".user{background:#16213e;padding:10px;margin:5px 0;border-radius:8px}");
    html.push_str(".assistant{background:#0f3460;padding:10px;margin:5px 0;border-radius:8px}");
    html.push_str(
        ".tool{background:#1a1a1a;padding:10px;margin:5px 0;border-radius:8px;font-size:0.9em}",
    );
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
